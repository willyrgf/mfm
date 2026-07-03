use super::*;

/// Result of a Postgres fact query execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresFactQueryResult {
    rows: Vec<PostgresFactQueryRow>,
    receipt: mfm_facts::FactQueryReceipt,
}

impl PostgresFactQueryResult {
    /// Returns matching fact query rows in plan ordering.
    pub fn rows(&self) -> &[PostgresFactQueryRow] {
        &self.rows
    }

    /// Returns the authenticated store receipt for this query result.
    pub const fn receipt(&self) -> &mfm_facts::FactQueryReceipt {
        &self.receipt
    }
}

/// One Postgres fact query result row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresFactQueryRow {
    fact_ref: mfm_facts::InternalFactRef,
    returned_fields: Vec<mfm_facts::ReturnedFieldValueSummary>,
}

impl PostgresFactQueryRow {
    /// Returns the internal fact reference for this row.
    pub const fn fact_ref(&self) -> &mfm_facts::InternalFactRef {
        &self.fact_ref
    }

    /// Returns requested returned field summaries present on this row.
    pub fn returned_fields(&self) -> &[mfm_facts::ReturnedFieldValueSummary] {
        &self.returned_fields
    }
}

impl PostgresRunStore {
    /// Executes a descriptor-scoped canonical fact query plan against Postgres fact indexes.
    pub async fn execute_fact_query(
        &self,
        plan: &mfm_facts::CanonicalFactQueryPlan,
    ) -> Result<PostgresFactQueryResult> {
        let signer = require_fact_receipt_signer(self.fact_receipt_signer.as_ref())?;
        let trust_root = self
            .authority
            .fact_receipt_trust_root()
            .ok_or_else(|| receipt_authentication_store_error("missing fact receipt trust root"))?;
        execute_fact_query_client(&self.pool, plan, signer, trust_root).await
    }
}

async fn execute_fact_query_client(
    pool: &PgPool,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    signer: &PostgresFactReceiptSigner,
    trust_root: &mfm_store::v1::FactQueryReceiptTrustRoot,
) -> Result<PostgresFactQueryResult> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start fact query transaction", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to set fact query transaction mode", error))?;
    let result = execute_fact_query_tx(&mut tx, plan, signer, trust_root).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit fact query transaction", error))?;
    Ok(result)
}

async fn execute_fact_query_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    signer: &PostgresFactReceiptSigner,
    trust_root: &mfm_store::v1::FactQueryReceiptTrustRoot,
) -> Result<PostgresFactQueryResult> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).map_err(fact_error)?;
    let rows = load_matching_fact_index_rows_tx(tx, plan, &shape).await?;
    let mut result_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let fact_ref = internal_fact_ref_from_row(&row)?;
        let returned_fields =
            load_returned_field_summaries_tx(tx, &fact_ref, shape.return_fields()).await?;
        result_rows.push(PostgresFactQueryRow {
            fact_ref,
            returned_fields,
        });
    }
    let receipt = build_fact_query_receipt_tx(tx, plan, signer, &result_rows).await?;
    let plan_hash = mfm_facts::fact_query_plan_hash(plan).map_err(fact_error)?;
    mfm_store::v1::verify_fact_query_receipt_authentication(&plan_hash, &receipt, trust_root)
        .map_err(PostgresStoreError::Store)?;
    Ok(PostgresFactQueryResult {
        rows: result_rows,
        receipt,
    })
}

async fn build_fact_query_receipt_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    signer: &PostgresFactReceiptSigner,
    rows: &[PostgresFactQueryRow],
) -> Result<mfm_facts::FactQueryReceipt> {
    let plan_hash = mfm_facts::fact_query_plan_hash(plan).map_err(fact_error)?;
    let returned_refs = rows
        .iter()
        .map(|row| row.fact_ref.clone())
        .collect::<Vec<_>>();
    let returned_field_summaries = returned_field_summaries_for_receipt(plan, rows)?;
    let result_set_digest =
        mfm_facts::fact_query_result_set_digest(&returned_refs, returned_field_summaries.as_ref())
            .map_err(fact_error)?;
    let result_cardinality = query_result_cardinality(plan, rows.len())?;
    let read_frontier = load_store_read_frontier_tx(tx, plan).await?;
    let receipt_hash = mfm_facts::fact_query_receipt_body_hash_from_parts(
        &plan_hash,
        &read_frontier,
        mfm_facts::StoreReadFrontierType::Snapshot,
        &returned_refs,
        returned_field_summaries.as_ref(),
        &result_set_digest,
        result_cardinality,
    )
    .map_err(fact_error)?;
    let auth = signer.sign_receipt_hash(&receipt_hash)?;
    Ok(mfm_facts::FactQueryReceipt::new(
        read_frontier,
        mfm_facts::StoreReadFrontierType::Snapshot,
        returned_refs,
        returned_field_summaries,
        result_set_digest,
        result_cardinality,
        receipt_hash,
        auth,
    ))
}

fn returned_field_summaries_for_receipt(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    rows: &[PostgresFactQueryRow],
) -> Result<Option<mfm_facts::ReturnedFieldSummaries>> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).map_err(fact_error)?;
    if shape.return_fields().is_empty() {
        return Ok(None);
    }
    let summaries = rows
        .iter()
        .map(|row| {
            mfm_facts::ReturnedFactFieldSummary::new(
                row.fact_ref.fact_claim_id().clone(),
                row.returned_fields.clone(),
            )
        })
        .collect();
    Ok(Some(mfm_facts::ReturnedFieldSummaries::new(summaries)))
}

fn query_result_cardinality(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    row_count: usize,
) -> Result<mfm_facts::QueryResultCardinality> {
    let row_count = u64::try_from(row_count)
        .map_err(|_| PostgresStoreError::Corruption("fact query row count overflow".to_owned()))?;
    match plan.limit() {
        Some(limit) if row_count == limit => {
            Ok(mfm_facts::QueryResultCardinality::AtLeast(row_count))
        }
        _ => Ok(mfm_facts::QueryResultCardinality::Exact(row_count)),
    }
}

async fn load_store_read_frontier_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> Result<mfm_facts::StoreReadFrontier> {
    let descriptor_catalog_watermark = load_descriptor_catalog_watermark_tx(tx).await?;
    let projection_generation = load_fact_projection_generation_tx(tx).await?;
    let max_included_store_commit_order =
        load_max_included_store_commit_order_tx(tx, plan.query_scope()).await?;
    Ok(mfm_facts::StoreReadFrontier::new(
        plan.store_scope().clone(),
        plan.query_scope().clone(),
        descriptor_catalog_watermark,
        projection_generation,
        max_included_store_commit_order,
        mfm_facts::StoreCommitWatermark::new(max_included_store_commit_order),
    ))
}

async fn load_descriptor_catalog_watermark_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<mfm_facts::DescriptorCatalogWatermark> {
    let row = sqlx::query("SELECT COUNT(*)::bigint AS descriptor_count FROM fact_descriptor_index")
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load descriptor catalog watermark", error))?;
    let count = row_i64(
        &row,
        "descriptor_count",
        "fact_descriptor_index descriptor count",
    )?;
    Ok(mfm_facts::DescriptorCatalogWatermark::new(
        i64_to_nonnegative_u64(count, "fact descriptor catalog watermark")?,
    ))
}

async fn load_fact_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<mfm_facts::FactProjectionGeneration> {
    let row =
        sqlx::query("SELECT projection_generation FROM fact_projection_metadata WHERE singleton")
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| database_error("failed to load fact projection generation", error))?;
    let generation = row_i64(
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

async fn load_max_included_store_commit_order_tx(
    tx: &mut Transaction<'_, Postgres>,
    query_scope: &mfm_facts::FactQueryScope,
) -> Result<u64> {
    let row = sqlx::query(
        "SELECT COALESCE(MAX(store_commit_order), 0)::bigint AS commit_watermark \
         FROM fact_index \
         WHERE audience = $1 AND visibility_scope = $2",
    )
    .bind(fact_audience_tag(query_scope.audience()))
    .bind(fact_visibility_scope_tag(query_scope.scope()))
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load fact query commit watermark", error))?;
    i64_to_nonnegative_u64(
        row_i64(
            &row,
            "commit_watermark",
            "fact_index.store_commit_order watermark",
        )?,
        "fact query commit watermark",
    )
}

async fn load_matching_fact_index_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    shape: &mfm_facts::CompiledFactQueryShape,
) -> Result<Vec<PgRow>> {
    let mut builder = QueryBuilder::new(
        "SELECT f.source_run_id, f.source_seq, f.source_ordinal, f.source_event_id, \
         f.producer_node_id, f.recorded_at, f.observed_at, f.audience, f.visibility_scope, f.fact_kind, \
         f.fact_descriptor_hash, f.fact_subject_namespace_hash, f.fact_key, \
         f.subject_material_hash, f.request_schema_id, f.request_hash, f.response_schema_id, \
         f.response_hash, f.response_artifact_id, f.response_artifact_evidence_hash, \
         f.capability_kind, f.capability_version, f.adapter_kind, f.adapter_version \
         FROM fact_index f",
    );
    for (index, term) in plan.ordering().terms().iter().enumerate() {
        push_ordering_join(&mut builder, index, term);
    }
    builder
        .push(" WHERE f.fact_descriptor_hash = ")
        .push_bind(plan.resolved_descriptor().as_str().to_owned())
        .push(" AND f.audience = ")
        .push_bind(fact_audience_tag(plan.query_scope().audience()))
        .push(" AND f.visibility_scope = ")
        .push_bind(fact_visibility_scope_tag(plan.query_scope().scope()));
    for (index, predicate) in shape.predicates().iter().enumerate() {
        push_predicate_exists(&mut builder, index, plan.resolved_descriptor(), predicate);
    }
    push_ordering_clause(&mut builder, plan.ordering());
    if let Some(limit) = plan.limit() {
        builder
            .push(" LIMIT ")
            .push_bind(u64_to_i64(limit, "fact query limit")?);
    }
    builder
        .build()
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to execute fact query", error))
}

fn push_ordering_join(
    builder: &mut QueryBuilder<Postgres>,
    index: usize,
    term: &mfm_facts::FactOrderingTerm,
) {
    let alias = ordering_alias(index);
    builder
        .push(" LEFT JOIN fact_index_terms ")
        .push(&alias)
        .push(" ON ")
        .push(&alias)
        .push(".source_run_id = f.source_run_id AND ")
        .push(&alias)
        .push(".source_seq = f.source_seq AND ")
        .push(&alias)
        .push(".source_ordinal = f.source_ordinal AND ")
        .push(&alias)
        .push(".fact_descriptor_hash = f.fact_descriptor_hash AND ")
        .push(&alias)
        .push(".field_id = ")
        .push_bind(term.field_id().as_str().to_owned());
}

fn push_predicate_exists(
    builder: &mut QueryBuilder<Postgres>,
    index: usize,
    descriptor_hash: &ContentDigest,
    predicate: &mfm_facts::FactQueryPredicate,
) {
    let alias = predicate_alias(index);
    builder
        .push(" AND EXISTS (SELECT 1 FROM fact_index_terms ")
        .push(&alias)
        .push(" WHERE ")
        .push(&alias)
        .push(".source_run_id = f.source_run_id AND ")
        .push(&alias)
        .push(".source_seq = f.source_seq AND ")
        .push(&alias)
        .push(".source_ordinal = f.source_ordinal AND ")
        .push(&alias)
        .push(".fact_descriptor_hash = ")
        .push_bind(descriptor_hash.as_str().to_owned())
        .push(" AND ")
        .push(&alias)
        .push(".field_id = ")
        .push_bind(predicate.field_id().as_str().to_owned())
        .push(" AND ")
        .push(&alias)
        .push(".value_type = ")
        .push_bind(fact_field_value_type_tag(predicate.value().value_type()))
        .push(" AND ");
    push_scalar_predicate(builder, &alias, predicate.operator(), predicate.value());
    builder.push(")");
}

fn push_scalar_predicate(
    builder: &mut QueryBuilder<Postgres>,
    alias: &str,
    operator: mfm_facts::FactQueryOperator,
    value: &mfm_facts::FactCanonicalScalar,
) {
    match value {
        mfm_facts::FactCanonicalScalar::String(value) => {
            push_simple_comparison(builder, alias, "value_text", operator, value.clone());
        }
        mfm_facts::FactCanonicalScalar::Boolean(value) => {
            builder.push(alias).push(".value_bool = ").push_bind(*value);
        }
        mfm_facts::FactCanonicalScalar::SignedInteger(value) => {
            push_simple_comparison(builder, alias, "value_i64", operator, *value);
        }
        mfm_facts::FactCanonicalScalar::UnsignedInteger(value) => {
            push_u64_comparison(builder, alias, operator, *value);
        }
        mfm_facts::FactCanonicalScalar::Timestamp(value) => {
            push_simple_comparison(builder, alias, "value_timestamp", operator, value.clone());
        }
        mfm_facts::FactCanonicalScalar::DecimalString(value) => {
            builder.push(alias).push(".value_decimal::numeric ");
            push_sql_operator(builder, operator);
            builder
                .push(" ")
                .push_bind(value.as_str().to_owned())
                .push("::numeric");
        }
        mfm_facts::FactCanonicalScalar::Digest(value) => {
            builder
                .push(alias)
                .push(".value_digest = ")
                .push_bind(value.as_str().to_owned());
        }
    }
}

fn push_simple_comparison<T>(
    builder: &mut QueryBuilder<Postgres>,
    alias: &str,
    column: &str,
    operator: mfm_facts::FactQueryOperator,
    value: T,
) where
    T: 'static + Send + sqlx::Encode<'static, Postgres> + sqlx::Type<Postgres>,
{
    builder.push(alias).push(".").push(column).push(" ");
    push_sql_operator(builder, operator);
    builder.push(" ").push_bind(value);
}

fn push_u64_comparison(
    builder: &mut QueryBuilder<Postgres>,
    alias: &str,
    operator: mfm_facts::FactQueryOperator,
    value: u64,
) {
    let value = value.to_string();
    match operator {
        mfm_facts::FactQueryOperator::Equal => {
            builder.push(alias).push(".value_u64 = ").push_bind(value);
        }
        mfm_facts::FactQueryOperator::LessThan
        | mfm_facts::FactQueryOperator::LessThanOrEqual
        | mfm_facts::FactQueryOperator::GreaterThan
        | mfm_facts::FactQueryOperator::GreaterThanOrEqual => {
            let length_operator = match operator {
                mfm_facts::FactQueryOperator::LessThan
                | mfm_facts::FactQueryOperator::LessThanOrEqual => "<",
                mfm_facts::FactQueryOperator::GreaterThan
                | mfm_facts::FactQueryOperator::GreaterThanOrEqual => ">",
                mfm_facts::FactQueryOperator::Equal => unreachable!("handled above"),
            };
            builder
                .push("(length(")
                .push(alias)
                .push(".value_u64) ")
                .push(length_operator)
                .push(" length(")
                .push_bind(value.clone())
                .push(") OR (length(")
                .push(alias)
                .push(".value_u64) = length(")
                .push_bind(value.clone())
                .push(") AND ")
                .push(alias)
                .push(".value_u64 ");
            push_sql_operator(builder, operator);
            builder.push(" ").push_bind(value).push("))");
        }
    }
}

fn push_sql_operator(builder: &mut QueryBuilder<Postgres>, operator: mfm_facts::FactQueryOperator) {
    builder.push(match operator {
        mfm_facts::FactQueryOperator::Equal => "=",
        mfm_facts::FactQueryOperator::LessThan => "<",
        mfm_facts::FactQueryOperator::LessThanOrEqual => "<=",
        mfm_facts::FactQueryOperator::GreaterThan => ">",
        mfm_facts::FactQueryOperator::GreaterThanOrEqual => ">=",
    });
}

fn push_ordering_clause(builder: &mut QueryBuilder<Postgres>, ordering: &mfm_facts::FactOrderingPolicy) {
    builder.push(" ORDER BY ");
    for (index, term) in ordering.terms().iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        push_ordering_term(builder, &ordering_alias(index), term);
    }
    builder.push(", f.source_run_id ASC, f.source_seq ASC, f.source_ordinal ASC");
}

fn push_ordering_term(
    builder: &mut QueryBuilder<Postgres>,
    alias: &str,
    term: &mfm_facts::FactOrderingTerm,
) {
    let direction = match term.direction() {
        mfm_facts::SortDirection::Ascending => "ASC",
        mfm_facts::SortDirection::Descending => "DESC",
    };
    let nulls = match term.nulls() {
        mfm_facts::NullOrdering::First => "NULLS FIRST",
        mfm_facts::NullOrdering::Last => "NULLS LAST",
    };
    let columns = [
        "value_i64",
        "length(value_u64)",
        "value_u64",
        "value_decimal::numeric",
        "value_timestamp",
    ];
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        if column.contains('(') {
            builder
                .push(column.replace("value_u64", &format!("{alias}.value_u64")))
                .push(" ");
        } else {
            builder.push(alias).push(".").push(*column).push(" ");
        }
        builder.push(direction).push(" ").push(nulls);
    }
}

async fn load_returned_field_summaries_tx(
    tx: &mut Transaction<'_, Postgres>,
    fact_ref: &mfm_facts::InternalFactRef,
    return_fields: &[mfm_facts::FactQueryReturnField],
) -> Result<Vec<mfm_facts::ReturnedFieldValueSummary>> {
    let mut summaries = Vec::new();
    for field in return_fields {
        let row = sqlx::query(
            "SELECT field_id, value_type, value_text, value_bool, value_i64, value_u64, \
             value_decimal, value_timestamp, value_digest \
             FROM fact_index_terms \
             WHERE source_run_id = $1 AND source_seq = $2 AND source_ordinal = $3 \
               AND field_id = $4",
        )
        .bind(fact_ref.fact_claim_id().source_run_id().as_str())
        .bind(u64_to_i64(
            fact_ref.fact_claim_id().source_seq(),
            "fact_index_terms.source_seq",
        )?)
        .bind(
            i32::try_from(fact_ref.fact_claim_id().source_ordinal()).map_err(|_| {
                PostgresStoreError::Corruption(
                    "fact_index_terms.source_ordinal overflow".to_owned(),
                )
            })?,
        )
        .bind(field.field_id().as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load fact query return field", error))?;
        let Some(row) = row else {
            continue;
        };
        let field_id =
            mfm_facts::FactFieldId::new(row_string(&row, "field_id", "fact_index_terms.field_id")?)
                .map_err(fact_error)?;
        let value_type = parse_fact_field_value_type(&row_string(
            &row,
            "value_type",
            "fact_index_terms.value_type",
        )?)?;
        let value = parse_term_value(&row, value_type)?;
        summaries.push(
            mfm_facts::ReturnedFieldValueSummary::new(field_id, value_type, value)
                .map_err(fact_error)?,
        );
    }
    Ok(summaries)
}

fn internal_fact_ref_from_row(row: &PgRow) -> Result<mfm_facts::InternalFactRef> {
    let source_run_id = parse_identity::<RunId>(&row_string(
        row,
        "source_run_id",
        "fact_index.source_run_id",
    )?)?;
    let source_seq = i64_to_positive_u64(
        row_i64(row, "source_seq", "fact_index.source_seq")?,
        "fact_index.source_seq",
    )?;
    let source_ordinal =
        u32::try_from(row_i32(row, "source_ordinal", "fact_index.source_ordinal")?).map_err(
            |_| PostgresStoreError::Corruption("fact_index.source_ordinal overflow".into()),
        )?;
    let audience = parse_fact_audience(&row_string(row, "audience", "fact_index.audience")?)?;
    let scope = parse_fact_visibility_scope(&row_string(
        row,
        "visibility_scope",
        "fact_index.visibility_scope",
    )?)?;
    let parts = mfm_facts::InternalFactRefParts {
        fact_claim_id: mfm_facts::FactClaimId::new(
            source_run_id.clone(),
            source_seq,
            source_ordinal,
        )
        .map_err(fact_error)?,
        source_event_id: parse_identity(&row_string(
            row,
            "source_event_id",
            "fact_index.source_event_id",
        )?)?,
        recorded_at: row_string(row, "recorded_at", "fact_index.recorded_at")?,
        producer_node_id: parse_identity(&row_string(
            row,
            "producer_node_id",
            "fact_index.producer_node_id",
        )?)?,
        observed_at: row_optional_string(row, "observed_at")?,
        visibility: mfm_facts::FactVisibility::Indexed { audience, scope },
        fact_kind: mfm_facts::FactKind::new(row_string(row, "fact_kind", "fact_index.fact_kind")?)
            .map_err(fact_error)?,
        fact_descriptor_hash: parse_identity(&row_string(
            row,
            "fact_descriptor_hash",
            "fact_index.fact_descriptor_hash",
        )?)?,
        fact_subject_namespace_hash: parse_identity(&row_string(
            row,
            "fact_subject_namespace_hash",
            "fact_index.fact_subject_namespace_hash",
        )?)?,
        fact_key: mfm_facts::FactKey::from_digest(parse_identity(&row_string(
            row,
            "fact_key",
            "fact_index.fact_key",
        )?)?),
        subject_material_hash: parse_identity(&row_string(
            row,
            "subject_material_hash",
            "fact_index.subject_material_hash",
        )?)?,
        request_schema_id: parse_optional_identity(row_optional_string(row, "request_schema_id")?)?,
        request_hash: parse_optional_identity(row_optional_string(row, "request_hash")?)?,
        response_schema_id: parse_identity(&row_string(
            row,
            "response_schema_id",
            "fact_index.response_schema_id",
        )?)?,
        response_hash: parse_identity(&row_string(
            row,
            "response_hash",
            "fact_index.response_hash",
        )?)?,
        artifact_id: parse_identity(&row_string(
            row,
            "response_artifact_id",
            "fact_index.response_artifact_id",
        )?)?,
        artifact_evidence_hash: parse_identity(&row_string(
            row,
            "response_artifact_evidence_hash",
            "fact_index.response_artifact_evidence_hash",
        )?)?,
        capability_kind: parse_identity(&row_string(
            row,
            "capability_kind",
            "fact_index.capability_kind",
        )?)?,
        capability_version: row_string(row, "capability_version", "fact_index.capability_version")?
            .parse()
            .map_err(|error| PostgresStoreError::Corruption(format!("{error}")))?,
        adapter_kind: parse_identity(&row_string(row, "adapter_kind", "fact_index.adapter_kind")?)?,
        adapter_version: row_string(row, "adapter_version", "fact_index.adapter_version")?
            .parse()
            .map_err(|error| PostgresStoreError::Corruption(format!("{error}")))?,
    };
    mfm_facts::InternalFactRef::new(parts).map_err(fact_error)
}

fn ordering_alias(index: usize) -> String {
    format!("order_term_{index}")
}

fn predicate_alias(index: usize) -> String {
    format!("predicate_term_{index}")
}

fn fact_field_value_type_tag(value: mfm_facts::FactFieldValueType) -> &'static str {
    match value {
        mfm_facts::FactFieldValueType::String => "string",
        mfm_facts::FactFieldValueType::Boolean => "boolean",
        mfm_facts::FactFieldValueType::SignedInteger => "signed_integer",
        mfm_facts::FactFieldValueType::UnsignedInteger => "unsigned_integer",
        mfm_facts::FactFieldValueType::Timestamp => "timestamp",
        mfm_facts::FactFieldValueType::DecimalString => "decimal_string",
        mfm_facts::FactFieldValueType::Digest => "digest",
    }
}

fn row_string(row: &PgRow, column: &str, field: &'static str) -> Result<String> {
    row.try_get::<String, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
}

fn row_optional_string(row: &PgRow, column: &str) -> Result<Option<String>> {
    row.try_get::<Option<String>, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{column} was invalid")))
}

fn row_i64(row: &PgRow, column: &str, field: &'static str) -> Result<i64> {
    row.try_get::<i64, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
}

fn row_i32(row: &PgRow, column: &str, field: &'static str) -> Result<i32> {
    row.try_get::<i32, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
}
