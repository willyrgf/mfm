use std::fmt;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::{connect_run_read_services, RunStoresArgs};
use clap::{Args, Subcommand};
use mfm_app::{
    PublicFactDescriptorSummary, PublicFactExplain, PublicFactFieldValue, PublicFactKindSummary,
    PublicFactPredicate, PublicFactQueryPage, PublicFactQueryRequest, PublicFactRef,
    PublicFactRefId, PublicFactScalarValue,
};
use serde::Serialize;

/// Subcommands under `mfm facts`.
#[derive(Subcommand)]
pub(crate) enum FactsCommand {
    /// List public fact kinds.
    Kinds {
        /// Parsed arguments for the kinds command.
        #[command(flatten)]
        args: KindsArgs,
    },
    /// Describe public descriptors for a fact kind.
    Describe {
        /// Parsed arguments for the describe command.
        #[command(flatten)]
        args: DescribeArgs,
    },
    /// Explain public query fields and orderings for a fact kind.
    Explain {
        /// Parsed arguments for the explain command.
        #[command(flatten)]
        args: ExplainArgs,
    },
    /// Query public facts by kind, predicates, ordering, and limit.
    Query {
        /// Parsed arguments for the query command.
        #[command(flatten)]
        args: QueryArgs,
    },
    /// Query the latest public fact for a kind using an explicit ordering.
    Latest {
        /// Parsed arguments for the latest command.
        #[command(flatten)]
        args: KindQueryArgs,
    },
    /// Query public fact history for a kind.
    History {
        /// Parsed arguments for the history command.
        #[command(flatten)]
        args: LimitedKindQueryArgs,
    },
    /// Query top public facts for a kind.
    Top {
        /// Parsed arguments for the top command.
        #[command(flatten)]
        args: LimitedKindQueryArgs,
    },
    /// Show one public fact by opaque public reference.
    Show {
        /// Parsed arguments for the show command.
        #[command(flatten)]
        args: ShowArgs,
    },
}

impl FactsCommand {
    /// Dispatches the selected facts subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            FactsCommand::Kinds { args } => kinds::execute(ctx, args).await,
            FactsCommand::Describe { args } => describe::execute(ctx, args).await,
            FactsCommand::Explain { args } => explain::execute(ctx, args).await,
            FactsCommand::Query { args } => query::execute(ctx, args).await,
            FactsCommand::Latest { args } => latest::execute(ctx, args).await,
            FactsCommand::History { args } => history::execute(ctx, args).await,
            FactsCommand::Top { args } => top::execute(ctx, args).await,
            FactsCommand::Show { args } => show::execute(ctx, args).await,
        }
    }
}

/// Arguments for `mfm facts kinds`.
#[derive(Args)]
pub(crate) struct KindsArgs {
    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// Arguments for `mfm facts describe`.
#[derive(Args)]
pub(crate) struct DescribeArgs {
    /// Public fact kind to describe.
    pub(crate) kind: String,

    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// Arguments for `mfm facts explain`.
#[derive(Args)]
pub(crate) struct ExplainArgs {
    /// Public fact kind to explain.
    pub(crate) kind: String,

    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// Arguments for `mfm facts query`.
#[derive(Args)]
pub(crate) struct QueryArgs {
    /// Public fact kind to query.
    #[arg(long)]
    pub(crate) kind: String,

    /// Optional descriptor shape selector.
    #[arg(long)]
    pub(crate) shape: Option<String>,

    /// Descriptor ordering policy name.
    #[arg(long)]
    pub(crate) order: String,

    /// Subject predicate, for example `chain=bitcoin` or `subject.chain.eq=bitcoin`.
    #[arg(long = "subject")]
    pub(crate) subjects: Vec<String>,

    /// Result predicate, for example `amount_sat.gt=1000` or `result.amount_sat.gt=1000`.
    #[arg(long = "result")]
    pub(crate) results: Vec<String>,

    /// Full field predicate, for example `subject.chain.eq=bitcoin`.
    #[arg(long = "where")]
    pub(crate) where_predicates: Vec<String>,

    /// Returnable field id to request from the public fact service.
    #[arg(long = "field", required = true)]
    pub(crate) fields: Vec<String>,

    /// Maximum fact rows to return.
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: u64,

    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// Shared arguments for kind-first latest queries.
#[derive(Args)]
pub(crate) struct KindQueryArgs {
    /// Public fact kind to query.
    pub(crate) kind: String,

    /// Optional descriptor shape selector.
    #[arg(long)]
    pub(crate) shape: Option<String>,

    /// Descriptor ordering policy name.
    #[arg(long)]
    pub(crate) order: String,

    /// Subject predicate, for example `chain=bitcoin` or `subject.chain.eq=bitcoin`.
    #[arg(long = "subject")]
    pub(crate) subjects: Vec<String>,

    /// Result predicate, for example `amount_sat.gt=1000` or `result.amount_sat.gt=1000`.
    #[arg(long = "result")]
    pub(crate) results: Vec<String>,

    /// Full field predicate, for example `subject.chain.eq=bitcoin`.
    #[arg(long = "where")]
    pub(crate) where_predicates: Vec<String>,

    /// Returnable field id to request from the public fact service.
    #[arg(long = "field", required = true)]
    pub(crate) fields: Vec<String>,

    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// Shared arguments for kind-first limited fact queries.
#[derive(Args)]
pub(crate) struct LimitedKindQueryArgs {
    /// Public fact kind to query.
    pub(crate) kind: String,

    /// Optional descriptor shape selector.
    #[arg(long)]
    pub(crate) shape: Option<String>,

    /// Descriptor ordering policy name.
    #[arg(long)]
    pub(crate) order: String,

    /// Subject predicate, for example `chain=bitcoin` or `subject.chain.eq=bitcoin`.
    #[arg(long = "subject")]
    pub(crate) subjects: Vec<String>,

    /// Result predicate, for example `amount_sat.gt=1000` or `result.amount_sat.gt=1000`.
    #[arg(long = "result")]
    pub(crate) results: Vec<String>,

    /// Full field predicate, for example `subject.chain.eq=bitcoin`.
    #[arg(long = "where")]
    pub(crate) where_predicates: Vec<String>,

    /// Returnable field id to request from the public fact service.
    #[arg(long = "field", required = true)]
    pub(crate) fields: Vec<String>,

    /// Maximum fact rows to return.
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: u64,

    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// Arguments for `mfm facts show`.
#[derive(Args)]
pub(crate) struct ShowArgs {
    /// Opaque public fact reference returned by `mfm facts query`.
    pub(crate) public_ref: String,

    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct KindsOutput {
    kinds: Vec<PublicFactKindSummary>,
}

impl fmt::Display for KindsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kinds.is_empty() {
            return writeln!(f, "fact_kinds 0");
        }
        for kind in &self.kinds {
            writeln!(
                f,
                "{} descriptors={}",
                kind.fact_kind, kind.descriptor_count
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DescribeOutput {
    descriptors: Vec<PublicFactDescriptorSummary>,
}

impl fmt::Display for DescribeOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.descriptors.is_empty() {
            return writeln!(f, "descriptors 0");
        }
        for descriptor in &self.descriptors {
            write_descriptor(f, descriptor)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExplainOutput {
    explain: PublicFactExplain,
}

impl fmt::Display for ExplainOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "kind {}", self.explain.fact_kind)?;
        if self.explain.descriptors.is_empty() {
            return writeln!(f, "descriptors 0");
        }
        for descriptor in &self.explain.descriptors {
            write_descriptor(f, descriptor)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct QueryOutput {
    facts: Vec<PublicFactRef>,
    next_cursor: Option<String>,
}

impl From<PublicFactQueryPage> for QueryOutput {
    fn from(page: PublicFactQueryPage) -> Self {
        Self {
            facts: page.facts,
            next_cursor: page.next_cursor,
        }
    }
}

impl fmt::Display for QueryOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.facts.is_empty() {
            writeln!(f, "facts 0")?;
        } else {
            for fact in &self.facts {
                write_fact(f, fact)?;
            }
        }
        if let Some(cursor) = &self.next_cursor {
            writeln!(f, "next_cursor {cursor}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ShowOutput {
    fact: PublicFactRef,
}

impl fmt::Display for ShowOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_fact(f, &self.fact)
    }
}

mod kinds {
    use super::*;

    pub(crate) async fn execute(ctx: &CommandContext, args: &KindsArgs) -> ! {
        let result = execute_internal(args).await;
        handle_command_result(result, &ctx.output_format);
    }

    async fn execute_internal(args: &KindsArgs) -> CommandResult<KindsOutput> {
        let services = connect_run_read_services(&args.stores).await?;
        Ok(CommandOutput::new(KindsOutput {
            kinds: services.fact_kinds().await?,
        }))
    }
}

mod describe {
    use super::*;

    pub(crate) async fn execute(ctx: &CommandContext, args: &DescribeArgs) -> ! {
        let result = execute_internal(args).await;
        handle_command_result(result, &ctx.output_format);
    }

    async fn execute_internal(args: &DescribeArgs) -> CommandResult<DescribeOutput> {
        let services = connect_run_read_services(&args.stores).await?;
        Ok(CommandOutput::new(DescribeOutput {
            descriptors: services.describe_fact_kind(&args.kind).await?,
        }))
    }
}

mod explain {
    use super::*;

    pub(crate) async fn execute(ctx: &CommandContext, args: &ExplainArgs) -> ! {
        let result = execute_internal(args).await;
        handle_command_result(result, &ctx.output_format);
    }

    async fn execute_internal(args: &ExplainArgs) -> CommandResult<ExplainOutput> {
        let services = connect_run_read_services(&args.stores).await?;
        Ok(CommandOutput::new(ExplainOutput {
            explain: services.explain_fact_kind(&args.kind).await?,
        }))
    }
}

mod query {
    use super::*;

    pub(crate) async fn execute(ctx: &CommandContext, args: &QueryArgs) -> ! {
        let result = execute_internal(args).await;
        handle_command_result(result, &ctx.output_format);
    }

    async fn execute_internal(args: &QueryArgs) -> CommandResult<QueryOutput> {
        validate_limit(args.limit)?;
        let services = connect_run_read_services(&args.stores).await?;
        let request = PublicFactQueryRequest {
            fact_kind: args.kind.clone(),
            shape: args.shape.clone(),
            predicates: parse_predicates(
                args.subjects.iter(),
                args.results.iter(),
                args.where_predicates.iter(),
            )?,
            return_fields: args.fields.clone(),
            ordering: args.order.clone(),
            limit: Some(args.limit),
        };
        Ok(CommandOutput::new(
            services.query_public_facts(request).await?.into(),
        ))
    }
}

mod latest {
    use super::*;

    pub(crate) async fn execute(ctx: &CommandContext, args: &KindQueryArgs) -> ! {
        let result = execute_internal(args).await;
        handle_command_result(result, &ctx.output_format);
    }

    async fn execute_internal(args: &KindQueryArgs) -> CommandResult<QueryOutput> {
        let services = connect_run_read_services(&args.stores).await?;
        let request = PublicFactQueryRequest {
            fact_kind: args.kind.clone(),
            shape: args.shape.clone(),
            predicates: parse_predicates(
                args.subjects.iter(),
                args.results.iter(),
                args.where_predicates.iter(),
            )?,
            return_fields: args.fields.clone(),
            ordering: args.order.clone(),
            limit: Some(1),
        };
        Ok(CommandOutput::new(
            services.query_public_facts(request).await?.into(),
        ))
    }
}

mod history {
    use super::*;

    pub(crate) async fn execute(ctx: &CommandContext, args: &LimitedKindQueryArgs) -> ! {
        let result = execute_internal(args).await;
        handle_command_result(result, &ctx.output_format);
    }

    async fn execute_internal(args: &LimitedKindQueryArgs) -> CommandResult<QueryOutput> {
        execute_limited_kind_query(args).await
    }
}

mod top {
    use super::*;

    pub(crate) async fn execute(ctx: &CommandContext, args: &LimitedKindQueryArgs) -> ! {
        let result = execute_internal(args).await;
        handle_command_result(result, &ctx.output_format);
    }

    async fn execute_internal(args: &LimitedKindQueryArgs) -> CommandResult<QueryOutput> {
        execute_limited_kind_query(args).await
    }
}

mod show {
    use super::*;

    pub(crate) async fn execute(ctx: &CommandContext, args: &ShowArgs) -> ! {
        let result = execute_internal(args).await;
        handle_command_result(result, &ctx.output_format);
    }

    async fn execute_internal(args: &ShowArgs) -> CommandResult<ShowOutput> {
        let public_ref = PublicFactRefId::new(args.public_ref.clone())?;
        let services = connect_run_read_services(&args.stores).await?;
        Ok(CommandOutput::new(ShowOutput {
            fact: services.resolve_public_fact_ref(&public_ref).await?,
        }))
    }
}

async fn execute_limited_kind_query(args: &LimitedKindQueryArgs) -> CommandResult<QueryOutput> {
    validate_limit(args.limit)?;
    let services = connect_run_read_services(&args.stores).await?;
    let request = PublicFactQueryRequest {
        fact_kind: args.kind.clone(),
        shape: args.shape.clone(),
        predicates: parse_predicates(
            args.subjects.iter(),
            args.results.iter(),
            args.where_predicates.iter(),
        )?,
        return_fields: args.fields.clone(),
        ordering: args.order.clone(),
        limit: Some(args.limit),
    };
    Ok(CommandOutput::new(
        services.query_public_facts(request).await?.into(),
    ))
}

fn validate_limit(limit: u64) -> Result<(), CommandError> {
    if limit == 0 {
        return Err(CommandError::new(
            "FactQueryLimitInvalid",
            "--limit must be greater than zero",
        ));
    }
    Ok(())
}

fn parse_predicates<'a>(
    subjects: impl Iterator<Item = &'a String>,
    results: impl Iterator<Item = &'a String>,
    where_predicates: impl Iterator<Item = &'a String>,
) -> Result<Vec<PublicFactPredicate>, CommandError> {
    let mut predicates = Vec::new();
    for value in subjects {
        predicates.push(parse_prefixed_predicate("subject", value)?);
    }
    for value in results {
        predicates.push(parse_prefixed_predicate("result", value)?);
    }
    for value in where_predicates {
        predicates.push(parse_field_predicate(value)?);
    }
    Ok(predicates)
}

fn parse_prefixed_predicate(
    prefix: &'static str,
    value: &str,
) -> Result<PublicFactPredicate, CommandError> {
    let mut predicate = parse_field_predicate(value)?;
    if !predicate.field_id.starts_with("subject.") && !predicate.field_id.starts_with("result.") {
        predicate.field_id = format!("{prefix}.{}", predicate.field_id);
    }
    Ok(predicate)
}

fn parse_field_predicate(value: &str) -> Result<PublicFactPredicate, CommandError> {
    let (left, raw_value) = value.split_once('=').ok_or_else(|| {
        CommandError::new(
            "FactPredicateInvalid",
            "Fact predicates must use `field[.operator]=value`",
        )
    })?;
    if left.is_empty() {
        return Err(CommandError::new(
            "FactPredicateInvalid",
            "Fact predicate field must not be empty",
        ));
    }
    let (field_id, operator) = parse_field_and_operator(left)?;
    Ok(PublicFactPredicate {
        field_id,
        operator,
        value: parse_scalar_value(raw_value)?,
    })
}

fn parse_field_and_operator(value: &str) -> Result<(String, String), CommandError> {
    let operators = [
        (".lte", "less_than_or_equal"),
        (".gte", "greater_than_or_equal"),
        (".eq", "equal"),
        (".lt", "less_than"),
        (".gt", "greater_than"),
    ];
    for (suffix, operator) in operators {
        if let Some(field_id) = value.strip_suffix(suffix) {
            if field_id.is_empty() {
                return Err(CommandError::new(
                    "FactPredicateInvalid",
                    "Fact predicate field must not be empty",
                ));
            }
            return Ok((field_id.to_owned(), operator.to_owned()));
        }
    }
    Ok((value.to_owned(), "equal".to_owned()))
}

fn parse_scalar_value(value: &str) -> Result<PublicFactScalarValue, CommandError> {
    if let Some((type_name, typed_value)) = value.split_once(':') {
        return match type_name {
            "string" => Ok(PublicFactScalarValue::String(typed_value.to_owned())),
            "bool" => parse_bool_scalar(typed_value),
            "i64" => parse_i64_scalar(typed_value),
            "u64" => parse_u64_scalar(typed_value),
            "timestamp" => Ok(PublicFactScalarValue::Timestamp(typed_value.to_owned())),
            "decimal" => Ok(PublicFactScalarValue::DecimalString(typed_value.to_owned())),
            "digest" => Ok(PublicFactScalarValue::Digest(typed_value.to_owned())),
            _ => Ok(infer_scalar_value(value)),
        };
    }
    Ok(infer_scalar_value(value))
}

fn infer_scalar_value(value: &str) -> PublicFactScalarValue {
    if let Ok(parsed) = value.parse::<bool>() {
        return PublicFactScalarValue::Boolean(parsed);
    }
    if value.starts_with('-') {
        if let Ok(parsed) = value.parse::<i64>() {
            return PublicFactScalarValue::SignedInteger(parsed);
        }
    } else if let Ok(parsed) = value.parse::<u64>() {
        return PublicFactScalarValue::UnsignedInteger(parsed);
    }
    if value.contains('.') && value.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
        return PublicFactScalarValue::DecimalString(value.to_owned());
    }
    PublicFactScalarValue::String(value.to_owned())
}

fn parse_bool_scalar(value: &str) -> Result<PublicFactScalarValue, CommandError> {
    value
        .parse::<bool>()
        .map(PublicFactScalarValue::Boolean)
        .map_err(|_| CommandError::new("FactPredicateInvalid", "Boolean fact value is invalid"))
}

fn parse_i64_scalar(value: &str) -> Result<PublicFactScalarValue, CommandError> {
    value
        .parse::<i64>()
        .map(PublicFactScalarValue::SignedInteger)
        .map_err(|_| {
            CommandError::new(
                "FactPredicateInvalid",
                "Signed integer fact value is invalid",
            )
        })
}

fn parse_u64_scalar(value: &str) -> Result<PublicFactScalarValue, CommandError> {
    value
        .parse::<u64>()
        .map(PublicFactScalarValue::UnsignedInteger)
        .map_err(|_| {
            CommandError::new(
                "FactPredicateInvalid",
                "Unsigned integer fact value is invalid",
            )
        })
}

fn write_descriptor(
    f: &mut fmt::Formatter<'_>,
    descriptor: &PublicFactDescriptorSummary,
) -> fmt::Result {
    writeln!(
        f,
        "{} descriptor_schema={} subject_schema={} response_schema={}",
        descriptor.fact_kind,
        descriptor.descriptor.descriptor_schema_id,
        descriptor.descriptor.subject_schema_id,
        descriptor.descriptor.response_schema_id,
    )?;
    for field in &descriptor.fields {
        writeln!(
            f,
            "  field {} source={} path={} type={} exposure={} operators={} sortable={}",
            field.field_id,
            field.source,
            field.path,
            field.value_type,
            field.exposure,
            field.operators.join(","),
            field.sortable
        )?;
    }
    for ordering in &descriptor.orderings {
        let terms = ordering
            .terms
            .iter()
            .map(|term| {
                format!(
                    "{}:{}:{}:tie_breaker={}",
                    term.field_id, term.direction, term.nulls, term.tie_breaker
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        writeln!(f, "  ordering {} {}", ordering.name, terms)?;
    }
    Ok(())
}

fn write_fact(f: &mut fmt::Formatter<'_>, fact: &PublicFactRef) -> fmt::Result {
    writeln!(
        f,
        "{} {} recorded_at={} observed_at={}",
        fact.public_ref,
        fact.fact_kind,
        fact.recorded_at,
        fact.observed_at.as_deref().unwrap_or("none")
    )?;
    for field in &fact.fields {
        writeln!(f, "  {}={}", field.field_id, public_scalar_display(field))?;
    }
    Ok(())
}

fn public_scalar_display(field: &PublicFactFieldValue) -> String {
    match &field.value {
        PublicFactScalarValue::String(value)
        | PublicFactScalarValue::Timestamp(value)
        | PublicFactScalarValue::DecimalString(value)
        | PublicFactScalarValue::Digest(value) => value.clone(),
        PublicFactScalarValue::Boolean(value) => value.to_string(),
        PublicFactScalarValue::SignedInteger(value) => value.to_string(),
        PublicFactScalarValue::UnsignedInteger(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_app::{PublicFactDescriptorRef, PublicFactFieldValue};

    #[test]
    fn predicate_flags_decode_to_app_public_query_dtos() {
        let predicates = parse_predicates(
            [&"chain=bitcoin".to_owned()].into_iter(),
            [&"amount_sat.gt=1000".to_owned()].into_iter(),
            [&"metadata.observed_at.lte=timestamp:2026-07-02T00:00:00Z".to_owned()].into_iter(),
        )
        .expect("predicates");

        assert_eq!(predicates[0].field_id, "subject.chain");
        assert_eq!(predicates[0].operator, "equal");
        assert_eq!(
            predicates[0].value,
            PublicFactScalarValue::String("bitcoin".to_owned())
        );
        assert_eq!(predicates[1].field_id, "result.amount_sat");
        assert_eq!(predicates[1].operator, "greater_than");
        assert_eq!(
            predicates[1].value,
            PublicFactScalarValue::UnsignedInteger(1000)
        );
        assert_eq!(predicates[2].field_id, "metadata.observed_at");
        assert_eq!(predicates[2].operator, "less_than_or_equal");
        assert_eq!(
            predicates[2].value,
            PublicFactScalarValue::Timestamp("2026-07-02T00:00:00Z".to_owned())
        );
    }

    #[test]
    fn rendered_public_fact_outputs_do_not_include_internal_fields() {
        let fact = sample_public_fact();
        let output = QueryOutput {
            facts: vec![fact],
            next_cursor: None,
        };

        let json = serde_json::to_string(&output).expect("json");
        let text = output.to_string();
        for forbidden in [
            "artifact_id",
            "artifact_evidence_hash",
            "fact_descriptor_hash",
            "subject_material",
            "subject_material_hash",
            "response_hash",
            "source_run_id",
            "source_seq",
            "source_ordinal",
            "source_event_id",
            "event_id",
            "fact_key",
        ] {
            assert!(!json.contains(forbidden), "json leaked {forbidden}");
            assert!(!text.contains(forbidden), "text leaked {forbidden}");
        }
    }

    fn sample_public_fact() -> PublicFactRef {
        PublicFactRef {
            public_ref: PublicFactRefId::new(format!("pfr_{}", "a".repeat(64)))
                .expect("public ref"),
            fact_kind: "wallet.balance".to_owned(),
            descriptor: PublicFactDescriptorRef {
                descriptor_schema_id: "mfm.wallet.balance.v1".to_owned(),
                subject_schema_id: "mfm.wallet.balance.subject.v1".to_owned(),
                response_schema_id: "mfm.wallet.balance.response.v1".to_owned(),
            },
            recorded_at: "2026-07-02T00:00:00Z".to_owned(),
            observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
            fields: vec![PublicFactFieldValue {
                field_id: "result.amount_sat".to_owned(),
                path: "result.amount_sat".to_owned(),
                source: "result".to_owned(),
                value_type: "unsigned_integer".to_owned(),
                value: PublicFactScalarValue::UnsignedInteger(1000),
                unit: Some("sat".to_owned()),
                scale: None,
            }],
        }
    }
}
