//! Explicit candidate selection and immutable admission provenance.

use super::*;
use mfm_ids::{ContentDigest, DigestAlgorithm, RunId};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioAdmission {
    name: String,
    config_digest_hex: String,
    entry_point: EntryPointId,
    enrichment: Option<EnrichmentProvenance>,
}
impl_checked_deserialize!(PortfolioAdmission {
    name: String,
    config_digest_hex: String,
    entry_point: EntryPointId,
    enrichment: Option<EnrichmentProvenance>,
});
impl PortfolioAdmission {
    /// Retains the selected public revision and optional verified enrichment linkage.
    pub fn new(
        name: String,
        config_digest_hex: String,
        entry_point: EntryPointId,
        enrichment: Option<EnrichmentProvenance>,
    ) -> Result<Self, PortfolioError> {
        let value = Self {
            name,
            config_digest_hex,
            entry_point,
            enrichment,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), PortfolioError> {
        if !valid_public_text(&self.name, 64)
            || self.config_digest_hex.len() != 64
            || !self
                .config_digest_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(PortfolioError::InvalidValue);
        }
        Ok(())
    }
    /// Returns the selected source name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Returns the lowercase SHA-256 hex digest of the canonical configuration document.
    pub fn config_digest_hex(&self) -> &str {
        &self.config_digest_hex
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

/// One resolved collection's exact public binding and observed anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EnrichmentCollection {
    chain_id: NonZeroU64,
    route_ref: ContentRef,
    anchor: PortfolioAnchor,
}

/// Checked candidate selection; it makes no claim about assets outside the supplied list.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioEnrichmentOutput {
    portfolio: PortfolioConfig,
    collections: Vec<EnrichmentCollection>,
    selector: PortfolioSnapshotSelector,
}
impl_checked_deserialize!(PortfolioEnrichmentOutput {
    portfolio: PortfolioConfig,
    collections: Vec<EnrichmentCollection>,
    selector: PortfolioSnapshotSelector,
});
impl PortfolioEnrichmentOutput {
    fn validate(&self) -> Result<(), PortfolioError> {
        validate_portfolio_config(&self.portfolio)?;
        self.selector.validate()?;
        if self.selector.target != self.portfolio.portfolio_id
            || !self.portfolio.quotes.contains(&self.selector.quote)
            || self.collections.len() != self.portfolio.collections.len()
            || self
                .collections
                .iter()
                .zip(&self.portfolio.collections)
                .any(|(bound, collection)| {
                    bound.anchor.validate().is_err()
                        || collection
                            .request
                            .sources()
                            .first()
                            .is_none_or(|source| source.chain_id() != bound.chain_id)
                        || !collection
                            .request
                            .sources()
                            .iter()
                            .any(|source| source.token().is_none())
                })
        {
            return Err(PortfolioError::InvalidValue);
        }
        Ok(())
    }
    /// Returns the checked resolved configuration in original candidate order.
    pub fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }
    /// Returns the original snapshot selector.
    pub fn selector(&self) -> &PortfolioSnapshotSelector {
        &self.selector
    }
    /// Returns declaration-ordered chain and binding references for publication verification.
    pub fn bindings(&self) -> impl Iterator<Item = (NonZeroU64, &ContentRef)> {
        self.collections
            .iter()
            .map(|collection| (collection.chain_id, &collection.route_ref))
    }
}

/// Retains native sources and tokens with nonzero balance at the verified collection anchor.
pub struct ResolvePortfolioAssets;
impl_portfolio_state!(
    ResolvePortfolioAssets,
    PortfolioContinuation,
    PortfolioEnrichmentOutput,
    "mfm.portfolio.state.resolve-assets@1",
    "Resolves configured asset candidates at verified anchors."
);
impl PureState for ResolvePortfolioAssets {
    fn evaluate(
        input: PortfolioContinuation,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, mfm_program::StateExecutionError>
    {
        let resolve = || -> Result<PortfolioEnrichmentOutput, PortfolioError> {
            input.validate()?;
            if input.completed_collections.len() != input.input.collections.len() {
                return Err(PortfolioError::InvalidContinuation);
            }
            let mut configs = Vec::with_capacity(input.input.collections.len());
            let mut collections = Vec::with_capacity(configs.capacity());
            for (demand, result) in input
                .input
                .collections
                .iter()
                .zip(&input.completed_collections)
            {
                let sources = demand
                    .request
                    .sources()
                    .iter()
                    .zip(&result.holdings)
                    .filter(|(source, holding)| {
                        source.token().is_none() || holding.raw_units != "0"
                    })
                    .map(|(source, _)| source.clone())
                    .collect();
                configs.push(PortfolioCollectionConfig {
                    correlation: demand.correlation.clone(),
                    request: EvmBalanceRequest::new(sources, demand.request.decimals())
                        .map_err(|_| PortfolioError::InvalidValue)?,
                });
                collections.push(EnrichmentCollection {
                    chain_id: result.chain_id,
                    route_ref: demand.route_ref.clone(),
                    anchor: result.anchor.clone(),
                });
            }
            let output = PortfolioEnrichmentOutput {
                portfolio: PortfolioConfig {
                    portfolio_id: input.input.portfolio_id.clone(),
                    quotes: input.input.quotes.clone(),
                    collections: configs,
                },
                selector: PortfolioSnapshotSelector {
                    target: input.input.portfolio_id.clone(),
                    quote: input.input.quote.clone(),
                },
                collections,
            };
            output.validate()?;
            Ok(output)
        };
        Ok(match resolve() {
            Ok(output) => portfolio_success(output),
            Err(_) => portfolio_failure(PortfolioSnapshotFailure::ConsolidationFailed),
        })
    }
}

/// Plans bounded anchored candidate discovery without implicit publication or dependent admission.
pub fn plan_enrichment(
    selector: PortfolioSnapshotSelector,
    config: &PortfolioConfig,
    targets: &[EvmPhysicalTarget],
    admission: Option<PortfolioAdmission>,
) -> Result<(Program, PortfolioSnapshotInput), PortfolioError> {
    if config.collections.iter().any(|collection| {
        !collection
            .request
            .sources()
            .iter()
            .any(|source| source.token().is_none())
    }) {
        return Err(PortfolioError::InvalidValue);
    }
    plan::<ResolvePortfolioAssets>(
        selector,
        config,
        targets,
        admission,
        EntryPointId::new(PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID)
            .map_err(|_| PortfolioError::Program)?,
    )
}

#[cfg(test)]
mod tests;
