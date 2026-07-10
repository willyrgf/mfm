use super::*;

/// Result of a Postgres fact query execution.
pub type PostgresFactQueryResult = mfm_facts::FactQueryResult;

/// One Postgres fact query result row.
pub type PostgresFactQueryRow = mfm_facts::FactQueryResultRow;

struct AuthoritativeFactQueryRow {
    index: mfm_store::v1::FactIndexProjection,
    terms: BTreeMap<mfm_facts::FactFieldId, mfm_facts::FactCanonicalScalar>,
    row: PostgresFactQueryRow,
}

impl PostgresRunStore {
    /// Executes a descriptor-scoped canonical fact query plan against Postgres fact indexes.
    pub async fn execute_fact_query(
        &self,
        plan: &mfm_facts::CanonicalFactQueryPlan,
    ) -> Result<PostgresFactQueryResult> {
        let mut results = self
            .execute_fact_queries(std::slice::from_ref(plan))
            .await?;
        Ok(results
            .pop()
            .expect("single-plan fact query batch always returns one result"))
    }

    /// Executes many descriptor-scoped plans under one REPEATABLE READ snapshot.
    pub async fn execute_fact_queries(
        &self,
        plans: &[mfm_facts::CanonicalFactQueryPlan],
    ) -> Result<Vec<PostgresFactQueryResult>> {
        if plans.is_empty() {
            return Ok(Vec::new());
        }
        let signer = require_fact_receipt_signer(self.fact_receipt_signer.as_ref())?;
        let trust_root = self
            .authority
            .fact_receipt_trust_root()
            .ok_or_else(|| receipt_authentication_store_error("missing fact receipt trust root"))?;
        execute_fact_queries_client(&self.pool, plans, signer, trust_root).await
    }
}

async fn execute_fact_queries_client(
    pool: &PgPool,
    plans: &[mfm_facts::CanonicalFactQueryPlan],
    signer: &PostgresFactReceiptSigner,
    trust_root: &mfm_store::v1::FactQueryReceiptTrustRoot,
) -> Result<Vec<PostgresFactQueryResult>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start fact query transaction", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to set fact query transaction mode", error))?;
    let mut results = Vec::with_capacity(plans.len());
    for plan in plans {
        results.push(execute_fact_query_tx(&mut tx, plan, signer, trust_root).await?);
    }
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit fact query transaction", error))?;
    Ok(results)
}

async fn execute_fact_query_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    signer: &PostgresFactReceiptSigner,
    trust_root: &mfm_store::v1::FactQueryReceiptTrustRoot,
) -> Result<PostgresFactQueryResult> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).map_err(fact_error)?;
    let authority_events = load_fact_authority_events_tx(tx).await?;
    let result_rows =
        load_authoritative_fact_query_rows_tx(tx, plan, &shape, &authority_events).await?;
    let receipt =
        build_fact_query_receipt_tx(tx, plan, &shape, signer, &result_rows, &authority_events)
            .await?;
    let plan_hash = mfm_facts::fact_query_plan_hash(plan).map_err(fact_error)?;
    mfm_store::v1::verify_fact_query_receipt_authentication(&plan_hash, &receipt, trust_root)
        .map_err(PostgresStoreError::Store)?;
    PostgresFactQueryResult::new(result_rows, receipt).map_err(fact_error)
}

async fn build_fact_query_receipt_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    shape: &mfm_facts::CompiledFactQueryShape,
    signer: &PostgresFactReceiptSigner,
    rows: &[PostgresFactQueryRow],
    authority_events: &[KernelEventEnvelope],
) -> Result<mfm_facts::FactQueryReceipt> {
    let plan_hash = mfm_facts::fact_query_plan_hash(plan).map_err(fact_error)?;
    let read_frontier = load_store_read_frontier_tx(tx, plan, authority_events).await?;
    let material = mfm_facts::FactQueryReceiptMaterial::from_rows(
        &plan_hash,
        read_frontier,
        mfm_facts::StoreReadFrontierType::Snapshot,
        rows,
        !shape.return_fields().is_empty(),
        plan.limit(),
    )
    .map_err(fact_error)?;
    let auth = signer.sign_receipt_hash(material.store_receipt_hash())?;
    Ok(material.into_receipt(auth))
}

async fn load_store_read_frontier_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    authority_events: &[KernelEventEnvelope],
) -> Result<mfm_facts::StoreReadFrontier> {
    let descriptor_catalog_watermark = descriptor_catalog_watermark(authority_events)?;
    let projection_generation = load_fact_projection_generation_tx(tx).await?;
    let store_commit_order = load_store_commit_order_tx(tx).await?;
    Ok(mfm_facts::StoreReadFrontier::new(
        plan.store_scope().clone(),
        plan.query_scope().clone(),
        descriptor_catalog_watermark,
        projection_generation,
        mfm_facts::StoreCommitOrder::new(store_commit_order),
    ))
}

fn descriptor_catalog_watermark(
    authority_events: &[KernelEventEnvelope],
) -> Result<mfm_facts::DescriptorCatalogWatermark> {
    let mut descriptors = BTreeSet::new();
    for event in authority_events {
        if let events::KernelEventPayload::RunAdmitted(payload) = event.payload() {
            descriptors.extend(
                payload
                    .fact_descriptor_artifacts
                    .iter()
                    .map(|artifact| artifact.content_digest.clone()),
            );
        }
    }
    let count = u64::try_from(descriptors.len()).map_err(|_| {
        PostgresStoreError::Corruption("fact descriptor catalog watermark overflow".to_owned())
    })?;
    Ok(mfm_facts::DescriptorCatalogWatermark::new(count))
}

async fn load_fact_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<mfm_facts::FactProjectionGeneration> {
    let row =
        sqlx::query("SELECT projection_generation FROM fact_projection_metadata WHERE singleton")
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| database_error("failed to load fact projection generation", error))?;
    let generation = required_i64(
        &row,
        "projection_generation",
        "fact_projection_metadata.projection_generation",
    )?;
    let generation = i64_to_nonnegative_u64(generation, "fact projection generation")?;
    if generation == 0 {
        return Err(PostgresStoreError::Corruption(
            "fact projection generation was zero".to_owned(),
        ));
    }
    Ok(mfm_facts::FactProjectionGeneration::new(generation))
}

async fn load_store_commit_order_tx(tx: &mut Transaction<'_, Postgres>) -> Result<u64> {
    let row = sqlx::query(
        "SELECT current_order AS store_commit_order FROM store_commit_order WHERE singleton",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load store commit watermark", error))?;
    i64_to_nonnegative_u64(
        required_i64(
            &row,
            "store_commit_order",
            "store_commit_order.current_order",
        )?,
        "store commit watermark",
    )
}

async fn load_fact_authority_events_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Vec<KernelEventEnvelope>> {
    let run_admitted_schema = event_schema_id("mfm.events.v1.run_admitted")?;
    let fact_recorded_schema = event_schema_id("mfm.events.v1.fact_recorded")?;
    let rows = sqlx::query(
        "SELECT e.run_id, e.seq, e.ordinal, c.store_commit_order, e.event_id, e.event_schema_id, \
         e.spec_hash, e.commit_key, e.logical_key, e.payload_hash, e.payload_canonical_json \
         FROM run_events e JOIN commits c ON c.run_id = e.run_id AND c.seq = e.seq \
         WHERE e.event_schema_id IN ($1, $2) \
         ORDER BY e.run_id ASC, e.seq ASC, e.ordinal ASC",
    )
    .bind(run_admitted_schema)
    .bind(fact_recorded_schema)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load authoritative fact events", error))?;
    rows.into_iter().map(event_envelope_from_row).collect()
}

fn event_schema_id(schema_name: &str) -> Result<String> {
    let descriptor = events::all_event_schema_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.schema_name == schema_name)
        .ok_or_else(|| {
            PostgresStoreError::Corruption(format!(
                "missing compiled event schema descriptor {schema_name}"
            ))
        })?;
    Ok(descriptor.schema_id()?.to_string())
}

async fn load_authoritative_fact_query_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    shape: &mfm_facts::CompiledFactQueryShape,
    authority_events: &[KernelEventEnvelope],
) -> Result<Vec<PostgresFactQueryRow>> {
    let run_ids = authority_events
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::FactRecorded(_) => Some(event.run_id().clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut snapshots = BTreeMap::new();
    for run_id in run_ids {
        let stream = load_run_stream_tx(tx, &run_id).await?;
        let artifact_bytes = load_fact_rebuild_artifact_bytes_tx(tx, &stream).await?;
        let snapshot = ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
            &stream,
            &artifact_bytes,
        )?;
        snapshots.insert(run_id, snapshot);
    }

    let mut rows = Vec::new();
    for snapshot in snapshots.values() {
        for (claim_id, index) in snapshot.fact_index_entries() {
            if index.fact_descriptor_hash != *plan.resolved_descriptor()
                || index.audience != plan.query_scope().audience()
                || index.visibility_scope != plan.query_scope().scope()
            {
                continue;
            }
            let terms = snapshot
                .fact_terms_for_claim(claim_id)
                .map(|term| (term.field_id.clone(), term.value.clone()))
                .collect::<BTreeMap<_, _>>();
            if !shape.predicates().iter().all(|predicate| {
                terms
                    .get(predicate.field_id())
                    .is_some_and(|value| predicate.matches_scalar(value))
            }) {
                continue;
            }
            let returned_fields = shape
                .return_fields()
                .iter()
                .filter_map(|field_id| {
                    terms.get(field_id).map(|value| {
                        mfm_facts::FactFieldValue::new(
                            field_id.clone(),
                            value.value_type(),
                            value.clone(),
                        )
                        .map_err(fact_error)
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            rows.push(AuthoritativeFactQueryRow {
                index: index.clone(),
                terms,
                row: PostgresFactQueryRow::new(index.internal_ref()?, returned_fields),
            });
        }
    }
    rows.sort_by(|left, right| compare_authoritative_fact_rows(left, right, plan));
    if let Some(limit) = plan.limit() {
        rows.truncate(rows.len().min(usize::try_from(limit).unwrap_or(usize::MAX)));
    }
    Ok(rows.into_iter().map(|row| row.row).collect())
}

fn compare_authoritative_fact_rows(
    left: &AuthoritativeFactQueryRow,
    right: &AuthoritativeFactQueryRow,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> std::cmp::Ordering {
    for term in plan.ordering().terms() {
        match term.compare_values(
            left.terms.get(term.field_id()),
            right.terms.get(term.field_id()),
        ) {
            Some(std::cmp::Ordering::Equal) => {}
            Some(ordering) => return ordering,
            None => return std::cmp::Ordering::Equal,
        }
    }
    left.index
        .source_run_id
        .cmp(&right.index.source_run_id)
        .then_with(|| left.index.source_seq.cmp(&right.index.source_seq))
        .then_with(|| left.index.source_ordinal.cmp(&right.index.source_ordinal))
}
