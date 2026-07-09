use super::*;

/// Result of a Postgres fact query execution.
pub type PostgresFactQueryResult = mfm_facts::FactQueryResult;

/// One Postgres fact query result row.
pub type PostgresFactQueryRow = mfm_facts::FactQueryResultRow;

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
    let rows = load_matching_fact_index_rows_tx(tx, plan, &shape).await?;
    let mut result_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let fact_ref = internal_fact_ref_from_row(&row)?;
        let returned_fields =
            load_returned_field_summaries_tx(tx, &fact_ref, shape.return_fields()).await?;
        result_rows.push(PostgresFactQueryRow::new(fact_ref, returned_fields));
    }
    let receipt = build_fact_query_receipt_tx(tx, plan, &shape, signer, &result_rows).await?;
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
) -> Result<mfm_facts::FactQueryReceipt> {
    let plan_hash = mfm_facts::fact_query_plan_hash(plan).map_err(fact_error)?;
    let read_frontier = load_store_read_frontier_tx(tx, plan).await?;
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
    let count = required_i64(
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

async fn load_max_included_store_commit_order_tx(
    tx: &mut Transaction<'_, Postgres>,
    query_scope: &mfm_facts::FactQueryScope,
) -> Result<u64> {
    let row = sqlx::query(
        "SELECT COALESCE(MAX(store_commit_order), 0)::bigint AS commit_watermark \
         FROM fact_index \
         WHERE audience = $1 AND visibility_scope = $2",
    )
    .bind(query_scope.audience().as_str())
    .bind(query_scope.scope().as_str())
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load fact query commit watermark", error))?;
    i64_to_nonnegative_u64(
        required_i64(
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
    let mut builder = QueryBuilder::new("SELECT ");
    super::fact_projections::push_fact_index_projection_select_list(&mut builder, Some("f"));
    builder.push(" FROM fact_index f");
    for (index, term) in plan.ordering().terms().iter().enumerate() {
        push_ordering_join(&mut builder, index, term);
    }
    builder
        .push(" WHERE f.fact_descriptor_hash = ")
        .push_bind(plan.resolved_descriptor().as_str().to_owned())
        .push(" AND f.audience = ")
        .push_bind(plan.query_scope().audience().as_str())
        .push(" AND f.visibility_scope = ")
        .push_bind(plan.query_scope().scope().as_str());
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
        .push_bind(predicate.value().value_type().as_str())
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
    let column = super::fact_projections::FactTermValueColumn::for_scalar(value);
    match value {
        mfm_facts::FactCanonicalScalar::String(value) => {
            push_simple_comparison(
                builder,
                alias,
                column.storage_column(),
                operator,
                value.clone(),
            );
        }
        mfm_facts::FactCanonicalScalar::Boolean(value) => {
            push_qualified_value_column(builder, alias, column);
            builder.push(" = ").push_bind(*value);
        }
        mfm_facts::FactCanonicalScalar::SignedInteger(value) => {
            push_simple_comparison(builder, alias, column.storage_column(), operator, *value);
        }
        mfm_facts::FactCanonicalScalar::UnsignedInteger(value) => {
            push_u64_comparison(builder, alias, column, operator, *value);
        }
        mfm_facts::FactCanonicalScalar::Timestamp(value) => {
            push_simple_comparison(
                builder,
                alias,
                column.storage_column(),
                operator,
                value.clone(),
            );
        }
        mfm_facts::FactCanonicalScalar::DecimalString(value) => {
            push_qualified_value_column(builder, alias, column);
            builder.push("::numeric ");
            push_sql_operator(builder, operator);
            builder
                .push(" ")
                .push_bind(value.as_str().to_owned())
                .push("::numeric");
        }
        mfm_facts::FactCanonicalScalar::Digest(value) => {
            push_qualified_value_column(builder, alias, column);
            builder.push(" = ").push_bind(value.as_str().to_owned());
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
    column: super::fact_projections::FactTermValueColumn,
    operator: mfm_facts::FactQueryOperator,
    value: u64,
) {
    let value = value.to_string();
    match operator {
        mfm_facts::FactQueryOperator::Equal => {
            push_qualified_value_column(builder, alias, column);
            builder.push(" = ").push_bind(value);
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
                .push(".")
                .push(column.storage_column())
                .push(") ")
                .push(length_operator)
                .push(" length(")
                .push_bind(value.clone())
                .push(") OR (length(")
                .push(alias)
                .push(".")
                .push(column.storage_column())
                .push(") = length(")
                .push_bind(value.clone())
                .push(") AND ")
                .push(alias)
                .push(".")
                .push(column.storage_column())
                .push(" ");
            push_sql_operator(builder, operator);
            builder.push(" ").push_bind(value).push("))");
        }
    }
}

fn push_qualified_value_column(
    builder: &mut QueryBuilder<Postgres>,
    alias: &str,
    column: super::fact_projections::FactTermValueColumn,
) {
    builder.push(alias).push(".").push(column.storage_column());
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

fn push_ordering_clause(
    builder: &mut QueryBuilder<Postgres>,
    ordering: &mfm_facts::FactOrderingPolicy,
) {
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
    for (index, expression) in FACT_TERM_ORDERING_EXPRESSIONS.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        expression.push_sql(builder, alias);
        builder.push(" ");
        builder.push(direction).push(" ").push(nulls);
    }
}

const FACT_TERM_ORDERING_EXPRESSIONS: &[FactTermOrderingExpression] = &[
    FactTermOrderingExpression::Column(super::fact_projections::FactTermValueColumn::I64),
    FactTermOrderingExpression::U64Length,
    FactTermOrderingExpression::Column(super::fact_projections::FactTermValueColumn::U64),
    FactTermOrderingExpression::DecimalNumeric,
    FactTermOrderingExpression::Column(super::fact_projections::FactTermValueColumn::Timestamp),
];

enum FactTermOrderingExpression {
    Column(super::fact_projections::FactTermValueColumn),
    U64Length,
    DecimalNumeric,
}

impl FactTermOrderingExpression {
    fn push_sql(&self, builder: &mut QueryBuilder<Postgres>, alias: &str) {
        match self {
            Self::Column(column) => push_qualified_value_column(builder, alias, *column),
            Self::U64Length => {
                builder.push("length(");
                push_qualified_value_column(
                    builder,
                    alias,
                    super::fact_projections::FactTermValueColumn::U64,
                );
                builder.push(")");
            }
            Self::DecimalNumeric => {
                push_qualified_value_column(
                    builder,
                    alias,
                    super::fact_projections::FactTermValueColumn::Decimal,
                );
                builder.push("::numeric");
            }
        }
    }
}

async fn load_returned_field_summaries_tx(
    tx: &mut Transaction<'_, Postgres>,
    fact_ref: &mfm_facts::InternalFactRef,
    return_fields: &[mfm_facts::FactFieldId],
) -> Result<Vec<mfm_facts::FactFieldValue>> {
    let mut summaries = Vec::new();
    for field_id in return_fields {
        let mut builder = QueryBuilder::new("SELECT field_id, value_type");
        super::fact_projections::push_fact_index_term_value_select_list(&mut builder);
        builder.push(
            " FROM fact_index_terms \
             WHERE source_run_id = ",
        );
        builder
            .push_bind(fact_ref.fact_claim_id().source_run_id().as_str())
            .push(" AND source_seq = ")
            .push_bind(u64_to_i64(
                fact_ref.fact_claim_id().source_seq(),
                "fact_index_terms.source_seq",
            )?)
            .push(" AND source_ordinal = ")
            .push_bind(
                i32::try_from(fact_ref.fact_claim_id().source_ordinal()).map_err(|_| {
                    PostgresStoreError::Corruption(
                        "fact_index_terms.source_ordinal overflow".to_owned(),
                    )
                })?,
            )
            .push(" AND field_id = ")
            .push_bind(field_id.as_str());
        let row = builder
            .build()
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| database_error("failed to load fact query return field", error))?;
        let Some(row) = row else {
            continue;
        };
        summaries.push(super::fact_projections::returned_field_summary_from_row(
            &row,
        )?);
    }
    Ok(summaries)
}

fn internal_fact_ref_from_row(row: &PgRow) -> Result<mfm_facts::InternalFactRef> {
    Ok(super::fact_projections::fact_index_projection_from_row(row)?.internal_ref()?)
}

fn ordering_alias(index: usize) -> String {
    format!("order_term_{index}")
}

fn predicate_alias(index: usize) -> String {
    format!("predicate_term_{index}")
}
