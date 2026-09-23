//! Semantic candidate selection and immutable admission provenance.
use super::*;
use mfm_chain::ObservationPoint;
use mfm_ids::{ConfigName, ContentDigest, DigestAlgorithm, DigestBytes, RunId};
use mfm_values::InvocationDiagnostic;

fn validate_candidates(
    input: &PortfolioSnapshotInput,
    required: &[String],
) -> Result<(), PortfolioError> {
    if duplicate_text(required.iter().map(String::as_str))
        || required.iter().any(|id| {
            !input.collections.iter().any(|demand| {
                demand
                    .request
                    .sources()
                    .iter()
                    .any(|source| source.source_id() == id)
            })
        })
        || input.collections.iter().any(|demand| {
            !demand
                .request
                .sources()
                .iter()
                .any(|source| required.iter().any(|id| id == source.source_id()))
        })
    {
        return Err(PortfolioError::InvalidValue);
    }
    Ok(())
}

/// Candidate progress and caller-required source identities, without native asset interpretation.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "enrichment-continuation",
    version = "2",
    schema = "mfm.portfolio-enrichment-continuation"
)]
pub struct EnrichmentContinuation {
    progress: PortfolioContinuation,
    required_sources: Vec<String>,
}
impl_checked_deserialize!(EnrichmentContinuation { progress: PortfolioContinuation, required_sources: Vec<String> });
impl EnrichmentContinuation {
    /// Projects an exact collection failure only after checking its retained enrichment context.
    pub fn project_collection_failure(
        context: &BalanceContext<Self>,
        code: BalanceFailureCode,
    ) -> Result<PortfolioSnapshotFailure, InvocationDiagnostic> {
        collection::project_failure(context, &context.caller().progress, code)
    }
    fn validate(&self) -> Result<(), PortfolioError> {
        validate_candidates(&self.progress.input, &self.required_sources)
    }
}

/// Checked candidate discovery demand and native-client-selected required sources.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "enrichment-input",
    version = "2",
    schema = "mfm.portfolio-enrichment-input"
)]
pub struct PortfolioEnrichmentInput {
    pub(super) input: PortfolioSnapshotInput,
    required_sources: Vec<String>,
}
impl_checked_deserialize!(PortfolioEnrichmentInput { input: PortfolioSnapshotInput, required_sources: Vec<String> });
impl PortfolioEnrichmentInput {
    /// Checks required-source membership, uniqueness and coverage for every collection.
    pub fn new(
        input: PortfolioSnapshotInput,
        required_sources: Vec<String>,
    ) -> Result<Self, PortfolioError> {
        let value = Self {
            input,
            required_sources,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), PortfolioError> {
        validate_candidates(&self.input, &self.required_sources)
    }
    /// Exact admitted source revision and enrichment linkage, when supplied.
    pub fn admission(&self) -> Option<&PortfolioAdmission> {
        self.input.admission()
    }
    /// Checked semantic collection admission.
    pub fn snapshot_input(&self) -> &PortfolioSnapshotInput {
        &self.input
    }
}

/// Initializes checked candidate collection progress.
pub struct InitializeEnrichment;
/// Enters a shared collection with an enrichment continuation.
pub struct EnterEnrichmentCollection;
/// Resumes enrichment after confirmation of every source.
pub struct ResumeEnrichmentCollection;
impl_portfolio_state!(
    InitializeEnrichment,
    PortfolioEnrichmentInput,
    EnrichmentContinuation,
    Never,
    "mfm.portfolio.state.initialize-enrichment@1",
    "Initializes checked enrichment progress."
);
impl_portfolio_state!(
    EnterEnrichmentCollection,
    EnrichmentContinuation,
    BalanceContext<EnrichmentContinuation>,
    Never,
    "mfm.portfolio.state.enter-enrichment-collection@1",
    "Enters the next enrichment collection."
);
impl_portfolio_state!(
    ResumeEnrichmentCollection,
    BalanceCollectionCompletion<EnrichmentContinuation>,
    EnrichmentContinuation,
    Never,
    "mfm.portfolio.state.resume-enrichment-collection@1",
    "Resumes enrichment after confirmed collection."
);
impl PureState for InitializeEnrichment {
    fn evaluate(
        input: Self::Input,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, mfm_values::InvocationDiagnostic>
    {
        Ok(portfolio_success(EnrichmentContinuation {
            progress: PortfolioContinuation::new(input.input),
            required_sources: input.required_sources,
        }))
    }
}
impl PureState for EnterEnrichmentCollection {
    fn evaluate(
        input: Self::Input,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, mfm_values::InvocationDiagnostic>
    {
        let (request, metadata) = collection::enter(&input.progress)?;
        Ok(portfolio_success(BalanceContext::new(
            request, input, metadata,
        )))
    }
}
impl PureState for ResumeEnrichmentCollection {
    fn evaluate(
        input: Self::Input,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, mfm_values::InvocationDiagnostic>
    {
        let (context, total_scaled) = input.into_parts();
        let (request, continuation, metadata, confirmed) = context.into_parts();
        let progress = collection::resume(
            continuation.progress,
            request,
            metadata,
            confirmed,
            total_scaled,
        )?;
        Ok(portfolio_success(EnrichmentContinuation {
            progress,
            required_sources: continuation.required_sources,
        }))
    }
}

/// Entry point for resolving a caller-supplied candidate asset list.
pub const PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID: &str = "mfm.portfolio/enrich@1";

/// Exact successful enrichment history and output selected for dependent admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EnrichmentProvenance {
    run_id: RunId,
    head: ContentDigest,
    output: ContentRef,
}
impl_checked_deserialize!(EnrichmentProvenance {
    run_id: RunId,
    head: ContentDigest,
    output: ContentRef,
});
impl EnrichmentProvenance {
    /// Constructs a bounded reference to one terminal enrichment result.
    pub fn new(
        run_id: RunId,
        head: ContentDigest,
        output: ContentRef,
    ) -> Result<Self, PortfolioError> {
        let value = Self {
            run_id,
            head,
            output,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), PortfolioError> {
        (self.head.algorithm() == DigestAlgorithm::Sha256V1)
            .then_some(())
            .ok_or(PortfolioError::InvalidValue)
    }
    /// Returns the enrichment run identity.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }
    /// Returns its exact terminal head.
    pub fn head(&self) -> &ContentDigest {
        &self.head
    }
    /// Returns its exact checked output identity.
    pub fn output(&self) -> &ContentRef {
        &self.output
    }
}

/// Bounded public source revision identity; Application owns repository interpretation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioAdmission {
    name: ConfigName,
    config_digest_hex: DigestBytes,
    entry_point: EntryPointId,
    enrichment: Option<EnrichmentProvenance>,
}
impl PortfolioAdmission {
    /// Retains the selected public revision and optional verified enrichment linkage.
    pub fn new(
        name: ConfigName,
        config_digest_hex: DigestBytes,
        entry_point: EntryPointId,
        enrichment: Option<EnrichmentProvenance>,
    ) -> Self {
        Self {
            name,
            config_digest_hex,
            entry_point,
            enrichment,
        }
    }
    /// Returns the selected source name.
    pub fn name(&self) -> &ConfigName {
        &self.name
    }
    /// Returns the lowercase SHA-256 hex digest of the canonical configuration document.
    pub fn config_digest_hex(&self) -> DigestBytes {
        self.config_digest_hex
    }
    /// Returns the selected entry point.
    pub fn entry_point(&self) -> &EntryPointId {
        &self.entry_point
    }
    /// Returns the verified enrichment linkage, when present.
    pub fn enrichment(&self) -> Option<&EnrichmentProvenance> {
        self.enrichment.as_ref()
    }
}

/// Filtered semantic demand and its confirmed common observation point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EnrichmentCollection {
    demand: PortfolioCollectionDemand,
    observed_at: ObservationPoint,
}
impl EnrichmentCollection {
    /// Retained source-aligned demand for configuration-free native publication.
    pub fn demand(&self) -> &PortfolioCollectionDemand {
        &self.demand
    }
    /// Confirmed point at which candidate inclusion was determined.
    pub fn observed_at(&self) -> &ObservationPoint {
        &self.observed_at
    }
}

/// Checked selected sources retaining public execution descriptors for cold publication.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "enrichment-output",
    version = "2",
    schema = "mfm.portfolio-enrichment-output"
)]
pub struct PortfolioEnrichmentOutput {
    portfolio_id: PortfolioId,
    quotes: Vec<QuoteCode>,
    quote: QuoteCode,
    collections: Vec<EnrichmentCollection>,
}
impl_checked_deserialize!(PortfolioEnrichmentOutput {
    portfolio_id: PortfolioId, quotes: Vec<QuoteCode>, quote: QuoteCode, collections: Vec<EnrichmentCollection>,
});
impl PortfolioEnrichmentOutput {
    fn validate(&self) -> Result<(), PortfolioError> {
        validate_demand_parts(
            &self.portfolio_id,
            &self.quote,
            &self.quotes,
            self.collections.iter().map(|collection| &collection.demand),
        )?;
        if self.collections.iter().any(|collection| {
            collection
                .demand
                .request
                .sources()
                .iter()
                .any(|source| source.target().ledger() != collection.observed_at.ledger())
        }) {
            return Err(PortfolioError::InvalidContinuation);
        }
        Ok(())
    }
    /// Portfolio identity selected by admission.
    pub fn portfolio_id(&self) -> &PortfolioId {
        &self.portfolio_id
    }
    /// Admitted supported quote currencies.
    pub fn quotes(&self) -> &[QuoteCode] {
        &self.quotes
    }
    /// Selected quote currency.
    pub fn quote(&self) -> &QuoteCode {
        &self.quote
    }
    /// Filtered source/descriptor pairs and their observation points.
    pub fn collections(&self) -> &[EnrichmentCollection] {
        &self.collections
    }
}

/// Retains required sources and nonzero observed balances without interpreting native asset tags.
pub struct ResolvePortfolioAssets;
impl_portfolio_state!(
    ResolvePortfolioAssets,
    EnrichmentContinuation,
    PortfolioEnrichmentOutput,
    Never,
    "mfm.portfolio.state.resolve-assets@1",
    "Retains required and nonzero confirmed candidate sources."
);
impl PureState for ResolvePortfolioAssets {
    fn evaluate(
        input: Self::Input,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, mfm_values::InvocationDiagnostic>
    {
        let resolve = || -> Result<PortfolioEnrichmentOutput, PortfolioError> {
            if input.progress.next_collection_ordinal().is_some() {
                return Err(PortfolioError::InvalidContinuation);
            }
            let mut collections = Vec::with_capacity(input.progress.completed_collections.len());
            for (demand, result) in input
                .progress
                .input
                .collections
                .iter()
                .zip(&input.progress.completed_collections)
            {
                let first = result
                    .balances
                    .first()
                    .ok_or(PortfolioError::InvalidContinuation)?;
                let mut sources = Vec::new();
                let mut executions = Vec::new();
                for (balance, execution) in result.balances.iter().zip(&demand.executions) {
                    if input
                        .required_sources
                        .iter()
                        .any(|id| id == balance.source().source_id())
                        || balance.raw_units().as_str() != "0"
                    {
                        sources.push(balance.source().clone());
                        executions.push(execution.clone());
                    }
                }
                collections.push(EnrichmentCollection {
                    demand: PortfolioCollectionDemand::new(
                        demand.correlation.clone(),
                        BalanceRequest::new(sources, demand.request.decimals())?,
                        executions,
                    )?,
                    observed_at: first.observed_at().clone(),
                });
            }
            let output = PortfolioEnrichmentOutput {
                portfolio_id: input.progress.input.portfolio_id,
                quotes: input.progress.input.quotes,
                quote: input.progress.input.quote,
                collections,
            };
            output.validate()?;
            Ok(output)
        };
        resolve()
            .map(portfolio_success)
            .map_err(|source| source.into_diagnostic("resolve_portfolio_assets"))
    }
}

#[cfg(test)]
mod tests;
