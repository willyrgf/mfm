use std::fmt;

use crate::commands::result::{CommandOutput, CommandResult, PublicError};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::{connect_run_read_services, RunStoresArgs};
use clap::{Args, Subcommand};
use mfm_app::{
    PublicFactDescriptorSummary, PublicFactExplain, PublicFactKindSummary, PublicFactQueryPage,
    PublicFactQueryRequest, PublicFactQuerySelector, PublicFactRef, PublicFactRefId,
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
            FactsCommand::Kinds { args } => finish_fact_command(ctx, execute_kinds(args).await),
            FactsCommand::Describe { args } => {
                finish_fact_command(ctx, execute_describe(args).await)
            }
            FactsCommand::Explain { args } => finish_fact_command(ctx, execute_explain(args).await),
            FactsCommand::Query { args } => finish_fact_command(ctx, execute_query(args).await),
            FactsCommand::Latest { args } => finish_fact_command(ctx, execute_latest(args).await),
            FactsCommand::History { args } => {
                finish_fact_command(ctx, execute_limited_kind_query(args).await)
            }
            FactsCommand::Top { args } => {
                finish_fact_command(ctx, execute_limited_kind_query(args).await)
            }
            FactsCommand::Show { args } => finish_fact_command(ctx, execute_show(args).await),
        }
    }
}

fn finish_fact_command<T>(ctx: &CommandContext, result: CommandResult<T>) -> !
where
    T: Serialize + fmt::Display,
{
    handle_command_result(result, &ctx.output_format)
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

    /// Shared fact query selector arguments.
    #[command(flatten)]
    pub(crate) query: FactQuerySelectorArgs,

    /// Maximum fact rows to return.
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: u64,

    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// Shared public fact query selector arguments.
#[derive(Args)]
pub(crate) struct FactQuerySelectorArgs {
    /// Optional descriptor shape selector.
    #[arg(long)]
    pub(crate) shape: Option<String>,

    /// Descriptor ordering policy name.
    #[arg(long)]
    pub(crate) order: String,

    /// Subject predicate, for example `network=bitcoin-mainnet` or `subject.network.eq=bitcoin-mainnet`.
    #[arg(long = "subject")]
    pub(crate) subjects: Vec<String>,

    /// Result predicate, for example `amount_sat.gt=1000` or `result.amount_sat.gt=1000`.
    #[arg(long = "result")]
    pub(crate) results: Vec<String>,

    /// Full field predicate, for example `subject.bitcoin_network.eq=main`.
    #[arg(long = "where")]
    pub(crate) where_predicates: Vec<String>,

    /// Returnable field id to request from the public fact service.
    #[arg(long = "field", required = true)]
    pub(crate) fields: Vec<String>,
}

/// Shared arguments for kind-first latest queries.
#[derive(Args)]
pub(crate) struct KindQueryArgs {
    /// Public fact kind to query.
    pub(crate) kind: String,

    /// Shared fact query selector arguments.
    #[command(flatten)]
    pub(crate) query: FactQuerySelectorArgs,

    /// Storage configuration for certified typed run events and fact projections.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// Shared arguments for kind-first limited fact queries.
#[derive(Args)]
pub(crate) struct LimitedKindQueryArgs {
    /// Public fact kind to query.
    pub(crate) kind: String,

    /// Shared fact query selector arguments.
    #[command(flatten)]
    pub(crate) query: FactQuerySelectorArgs,

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
#[serde(transparent)]
pub(crate) struct QueryOutput(PublicFactQueryPage);

impl From<PublicFactQueryPage> for QueryOutput {
    fn from(page: PublicFactQueryPage) -> Self {
        Self(page)
    }
}

impl fmt::Display for QueryOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.facts.is_empty() {
            writeln!(f, "facts 0")?;
        } else {
            for fact in &self.0.facts {
                write_fact(f, fact)?;
            }
        }
        if let Some(cursor) = &self.0.next_cursor {
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

async fn execute_kinds(args: &KindsArgs) -> CommandResult<KindsOutput> {
    let services = connect_run_read_services(&args.stores).await?;
    Ok(CommandOutput::new(KindsOutput {
        kinds: services.fact_kinds().await?,
    }))
}

async fn execute_describe(args: &DescribeArgs) -> CommandResult<DescribeOutput> {
    let services = connect_run_read_services(&args.stores).await?;
    Ok(CommandOutput::new(DescribeOutput {
        descriptors: services.describe_fact_kind(&args.kind).await?,
    }))
}

async fn execute_explain(args: &ExplainArgs) -> CommandResult<ExplainOutput> {
    let services = connect_run_read_services(&args.stores).await?;
    Ok(CommandOutput::new(ExplainOutput {
        explain: services.explain_fact_kind(&args.kind).await?,
    }))
}

async fn execute_query(args: &QueryArgs) -> CommandResult<QueryOutput> {
    execute_kind_query(&args.stores, &args.kind, &args.query, Some(args.limit)).await
}

async fn execute_latest(args: &KindQueryArgs) -> CommandResult<QueryOutput> {
    execute_kind_query(&args.stores, &args.kind, &args.query, Some(1)).await
}

async fn execute_show(args: &ShowArgs) -> CommandResult<ShowOutput> {
    let public_ref = PublicFactRefId::new(args.public_ref.clone())?;
    let services = connect_run_read_services(&args.stores).await?;
    Ok(CommandOutput::new(ShowOutput {
        fact: services.resolve_public_fact_ref(&public_ref).await?,
    }))
}

async fn execute_limited_kind_query(args: &LimitedKindQueryArgs) -> CommandResult<QueryOutput> {
    execute_kind_query(&args.stores, &args.kind, &args.query, Some(args.limit)).await
}

async fn execute_kind_query(
    stores: &RunStoresArgs,
    kind: &str,
    query: &FactQuerySelectorArgs,
    limit: Option<u64>,
) -> CommandResult<QueryOutput> {
    let services =
        mfm_app::connect_production_fact_public_query_service(stores.database_url.as_deref())
            .await?;
    let request = public_fact_query_request(kind, query, limit)?;
    Ok(CommandOutput::new(services.query(request).await?.into()))
}

fn public_fact_query_request(
    kind: &str,
    query: &FactQuerySelectorArgs,
    limit: Option<u64>,
) -> Result<PublicFactQueryRequest, PublicError> {
    PublicFactQueryRequest::from_selector(
        kind,
        PublicFactQuerySelector {
            shape: query.shape.clone(),
            subject_predicates: query.subjects.clone(),
            result_predicates: query.results.clone(),
            field_predicates: query.where_predicates.clone(),
            return_fields: query.fields.clone(),
            ordering: Some(query.order.clone()),
            limit,
        },
    )
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
        writeln!(f, "  {}={}", field.field_id, field.value)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
