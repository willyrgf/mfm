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

struct AuthoritativeFactQuerySnapshot {
    projection: ProjectionSnapshot,
    descriptor_catalog_watermark: mfm_facts::DescriptorCatalogWatermark,
    store_commit_order: mfm_facts::StoreCommitOrder,
}

impl AuthoritativeFactQuerySnapshot {
    fn read_frontier(
        &self,
        plan: &mfm_facts::CanonicalFactQueryPlan,
    ) -> mfm_facts::StoreReadFrontier {
        mfm_facts::StoreReadFrontier::new(
            plan.store_scope().clone(),
            plan.query_scope().clone(),
            self.descriptor_catalog_watermark,
            self.store_commit_order,
        )
    }
}

impl PostgresStore {
    /// Executes a descriptor-scoped canonical fact query plan from authoritative run evidence.
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
        execute_fact_queries_client(&self.pool, plans).await
    }
}

async fn execute_fact_queries_client(
    pool: &PgPool,
    plans: &[mfm_facts::CanonicalFactQueryPlan],
) -> Result<Vec<PostgresFactQueryResult>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start fact query transaction", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to set fact query transaction mode", error))?;
    let authority_events = load_fact_authority_events_tx(&mut tx).await?;
    let snapshot = AuthoritativeFactQuerySnapshot {
        projection: load_authoritative_fact_projection_snapshot_tx(&mut tx, &authority_events)
            .await?,
        descriptor_catalog_watermark: descriptor_catalog_watermark(&authority_events)?,
        store_commit_order: mfm_facts::StoreCommitOrder::new(
            load_store_commit_order_tx(&mut tx).await?,
        ),
    };
    let mut results = Vec::with_capacity(plans.len());
    for plan in plans {
        results.push(execute_fact_query(plan, &snapshot)?);
    }
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit fact query transaction", error))?;
    Ok(results)
}

fn execute_fact_query(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    snapshot: &AuthoritativeFactQuerySnapshot,
) -> Result<PostgresFactQueryResult> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).map_err(fact_error)?;
    let result_rows = load_authoritative_fact_query_rows(plan, &shape, &snapshot.projection)?;
    let receipt = build_fact_query_receipt(plan, &shape, &result_rows, snapshot)?;
    PostgresFactQueryResult::new(result_rows, receipt).map_err(fact_error)
}

fn build_fact_query_receipt(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    shape: &mfm_facts::CompiledFactQueryShape,
    rows: &[PostgresFactQueryRow],
    snapshot: &AuthoritativeFactQuerySnapshot,
) -> Result<mfm_facts::FactQueryReceipt> {
    mfm_facts::FactQueryReceipt::from_rows(
        snapshot.read_frontier(plan),
        mfm_facts::StoreReadFrontierType::Snapshot,
        rows,
        !shape.return_fields().is_empty(),
        plan.limit(),
    )
    .map_err(fact_error)
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

pub(super) async fn load_fact_authority_events_tx(
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

pub(super) async fn load_authoritative_fact_projection_snapshot_tx(
    tx: &mut Transaction<'_, Postgres>,
    authority_events: &[KernelEventEnvelope],
) -> Result<ProjectionSnapshot> {
    let run_ids = authority_events
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(_)
            | events::KernelEventPayload::FactRecorded(_) => Some(event.run_id().clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut parts = ProjectionSnapshotParts::default();
    for run_id in run_ids {
        let stream = load_run_stream_tx(tx, &run_id).await?;
        let artifact_bytes = load_fact_rebuild_artifact_bytes_tx(tx, &stream).await?;
        let snapshot = ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
            &stream,
            &artifact_bytes,
        )?;
        merge_fact_projection_family(
            &mut parts.fact_descriptors,
            snapshot
                .fact_descriptors()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            "descriptor",
        )?;
        merge_fact_projection_family(
            &mut parts.fact_records,
            snapshot
                .fact_records()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            "record",
        )?;
        merge_fact_projection_family(
            &mut parts.fact_index_entries,
            snapshot
                .fact_index_entries()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            "index",
        )?;
        merge_fact_projection_family(
            &mut parts.fact_term_entries,
            snapshot
                .fact_term_entries()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            "term",
        )?;
    }
    Ok(ProjectionSnapshot::from_parts(parts)?)
}

fn merge_fact_projection_family<K, V>(
    target: &mut BTreeMap<K, V>,
    source: BTreeMap<K, V>,
    family: &str,
) -> Result<()>
where
    K: Ord,
    V: PartialEq,
{
    for (key, value) in source {
        if let Some(existing) = target.get(&key) {
            if existing != &value {
                return Err(PostgresStoreError::Corruption(format!(
                    "conflicting authoritative fact {family} projection"
                )));
            }
        } else {
            target.insert(key, value);
        }
    }
    Ok(())
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

fn load_authoritative_fact_query_rows(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    shape: &mfm_facts::CompiledFactQueryShape,
    snapshot: &ProjectionSnapshot,
) -> Result<Vec<PostgresFactQueryRow>> {
    let mut rows = Vec::new();
    for (claim_id, index) in snapshot.fact_index_entries() {
        if index.fact_descriptor_hash != *plan.resolved_descriptor()
            || index.audience != plan.query_scope().audience()
            || index.visibility_scope != plan.query_scope().scope()
        {
            continue;
        }
        let fact_ref = index.internal_ref()?;
        if shape
            .content_identity()
            .is_some_and(|identity| !identity.matches_internal_ref(&fact_ref))
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
            row: PostgresFactQueryRow::new(fact_ref, returned_fields),
        });
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
