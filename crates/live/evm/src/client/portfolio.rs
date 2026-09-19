//! EVM-owned Portfolio configuration and semantic output conversion.
//! These pure functions require no provider, signer, configuration repository or Runtime handle.
mod balance_config;
use balance_config::{PortfolioBalanceRequestConfig, PortfolioBalanceSourceConfig};
use mfm_capabilities::CallbackFailure;
use mfm_chain::balance::BalanceExecutionConfig;
use mfm_evm::{EvmBalanceLedger, EvmBalanceRoute, EvmBalanceTarget, EvmBlockPoint, EvmEndpoint};
use mfm_ids::EntryPointId;
use mfm_portfolio::{
    EnrichmentProvenance, PortfolioAdmission, PortfolioCollectionDemand, PortfolioEnrichmentInput,
    PortfolioEnrichmentOutput, PortfolioError, PortfolioId, PortfolioSnapshotInput,
    PortfolioSnapshotOutput, PortfolioSnapshotSelector, QuoteCode,
    PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_values::{InvocationDiagnostic, Object};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, num::NonZeroU64};

/// Native client rejection with its owning admission, codec or projection cause.
#[derive(Debug, Serialize, thiserror::Error)]
pub enum EvmPortfolioClientError {
    /// Shared checked product rejection.
    #[error("Portfolio admission failed")]
    Portfolio(#[from] PortfolioError),
    /// Native checked-value construction rejected the supplied facts.
    #[error("EVM configuration value is invalid")]
    Domain(#[from] mfm_evm::EvmDomainError),
    /// Native/shared Object encoding rejected the supplied value.
    #[error("native value encoding failed")]
    Value(#[from] mfm_values::ValueError),
    /// Native codec or agreement check failed with reviewed context.
    #[error("native Portfolio qualification failed")]
    Native(InvocationDiagnostic),
    /// Shared source construction rejected its public identity.
    #[error("balance source is invalid")]
    Source(#[from] mfm_chain::balance::BalanceTextError),
    /// Shared request admission failed.
    #[error("balance request is invalid")]
    Request(#[from] mfm_chain::balance::BalanceRequestError),
    /// Checked entry-point identity failed.
    #[error("native entry point is invalid")]
    Identity(#[from] mfm_ids::CheckedStringError),
    /// JSON conversion retains its category, location and supplied rejection message.
    #[error("native output encoding failed")]
    Json(#[from] mfm_canonical::JsonError),
}
impl From<CallbackFailure> for EvmPortfolioClientError {
    fn from(cause: CallbackFailure) -> Self {
        Self::Native(cause.into_diagnostic())
    }
}
impl From<InvocationDiagnostic> for EvmPortfolioClientError {
    fn from(cause: InvocationDiagnostic) -> Self {
        Self::Native(cause)
    }
}
fn invalid(reason: &'static str) -> EvmPortfolioClientError {
    EvmPortfolioClientError::Native(InvocationDiagnostic::from_fields(
        "native_config",
        "admit_portfolio",
        reason,
        None,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteWire {
    chain_id: NonZeroU64,
    endpoint_id: String,
}
impl RouteWire {
    fn route(&self) -> Result<EvmBalanceRoute, EvmPortfolioClientError> {
        Ok(EvmBalanceRoute::new(
            self.chain_id,
            EvmEndpoint::new(&self.endpoint_id)?,
        ))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CollectionConfig {
    correlation: String,
    request: PortfolioBalanceRequestConfig,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioConfig {
    portfolio_id: PortfolioId,
    quotes: Vec<QuoteCode>,
    collections: Vec<CollectionConfig>,
}

/// Supported native Portfolio transport input; admission validates route and collection agreement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvmPortfolioInput {
    routes: Vec<RouteWire>,
    selector: PortfolioSnapshotSelector,
    portfolio: PortfolioConfig,
    #[serde(default)]
    provenance: Option<EnrichmentProvenance>,
}
/// The sole EVM Portfolio configuration wire accepted by native clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry_point", content = "input", deny_unknown_fields)]
pub enum EvmPortfolioConfig {
    /// Snapshot of explicitly configured sources.
    #[serde(rename = "mfm.portfolio/snapshot@1")]
    Snapshot(EvmPortfolioInput),
    /// Discovery from explicitly configured candidates.
    #[serde(rename = "mfm.portfolio/enrich@1")]
    Enrichment(EvmPortfolioInput),
}
impl EvmPortfolioConfig {
    fn input(&self) -> &EvmPortfolioInput {
        match self {
            Self::Snapshot(input) | Self::Enrichment(input) => input,
        }
    }
    /// Checked supported entry-point identity.
    pub fn entry_point_id(&self) -> Result<EntryPointId, EvmPortfolioClientError> {
        Ok(EntryPointId::new(match self {
            Self::Snapshot(_) => PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
            Self::Enrichment(_) => PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID,
        })?)
    }
    /// Native admission validates all configuration without compiling or invoking IO.
    pub fn validate(&self) -> Result<(), EvmPortfolioClientError> {
        match self {
            Self::Snapshot(_) => {
                admit_snapshot(self, None)?;
            }
            Self::Enrichment(_) => {
                admit_enrichment(self, None)?;
            }
        }
        Ok(())
    }
    /// Application-owned enrichment provenance, never interpreted by native execution.
    pub fn provenance(&self) -> Option<&EnrichmentProvenance> {
        self.input().provenance.as_ref()
    }
    /// Attaches Application's verified history identity to a generated snapshot configuration.
    pub fn with_provenance(
        mut self,
        provenance: EnrichmentProvenance,
    ) -> Result<Self, EvmPortfolioClientError> {
        let Self::Snapshot(input) = &mut self else {
            return Err(invalid("enrichment_cannot_reference_enrichment"));
        };
        input.provenance = Some(provenance);
        Ok(self)
    }
}

fn admit(
    config: &EvmPortfolioConfig,
    admission: Option<PortfolioAdmission>,
) -> Result<PortfolioSnapshotInput, EvmPortfolioClientError> {
    let input = config.input();
    if matches!(config, EvmPortfolioConfig::Enrichment(_)) && input.provenance.is_some() {
        return Err(invalid("enrichment_provenance"));
    }
    if input.selector.target() != &input.portfolio.portfolio_id
        || !input.portfolio.quotes.contains(input.selector.quote())
    {
        return Err(invalid("selector_mismatch"));
    }
    if input.routes.is_empty()
        || input.routes.len() > 64
        || input
            .routes
            .windows(2)
            .any(|pair| pair[0].chain_id >= pair[1].chain_id)
    {
        return Err(invalid("routes_not_unique_sorted_nonempty"));
    }
    let routes = input
        .routes
        .iter()
        .map(|wire| Ok((wire.chain_id, wire.route()?)))
        .collect::<Result<BTreeMap<_, _>, EvmPortfolioClientError>>()?;
    let mut used = std::collections::BTreeSet::new();
    let mut demands = Vec::with_capacity(input.portfolio.collections.len());
    for collection in &input.portfolio.collections {
        let request = collection.request.to_request()?;
        let mut executions = Vec::with_capacity(request.sources().len());
        for source in collection.request.sources() {
            let route = routes
                .get(&source.chain_id)
                .ok_or_else(|| invalid("missing_chain_route"))?;
            used.insert(source.chain_id);
            executions.push(BalanceExecutionConfig::new(
                route.route_ref()?,
                Object::from_value(route)?,
            ));
        }
        demands.push(PortfolioCollectionDemand::new(
            collection.correlation.clone(),
            request,
            executions,
        )?);
    }
    if used.len() != routes.len() {
        return Err(invalid("unused_chain_route"));
    }
    Ok(PortfolioSnapshotInput::new(
        input.portfolio.portfolio_id.clone(),
        demands,
        input.selector.quote().clone(),
        input.portfolio.quotes.clone(),
        admission,
    )?)
}
/// Admits EVM snapshot demand while leaving compilation to the shared compiler.
pub fn admit_snapshot(
    config: &EvmPortfolioConfig,
    admission: Option<PortfolioAdmission>,
) -> Result<PortfolioSnapshotInput, EvmPortfolioClientError> {
    if !matches!(config, EvmPortfolioConfig::Snapshot(_)) {
        return Err(invalid("expected_snapshot"));
    }
    admit(config, admission)
}
/// Admits native candidate requirements; Portfolio itself never inspects native/token tags.
pub fn admit_enrichment(
    config: &EvmPortfolioConfig,
    admission: Option<PortfolioAdmission>,
) -> Result<PortfolioEnrichmentInput, EvmPortfolioClientError> {
    if !matches!(config, EvmPortfolioConfig::Enrichment(_)) {
        return Err(invalid("expected_enrichment"));
    }
    let required = config
        .input()
        .portfolio
        .collections
        .iter()
        .flat_map(|collection| collection.request.sources())
        .filter(|source| source.token.is_none())
        .map(|source| source.source_id.clone())
        .collect();
    Ok(PortfolioEnrichmentInput::new(
        admit(config, admission)?,
        required,
    )?)
}

/// Reconstructs the supported native snapshot config solely from retained enrichment output.
pub fn snapshot_config(
    output: &PortfolioEnrichmentOutput,
) -> Result<EvmPortfolioConfig, EvmPortfolioClientError> {
    let mut routes = BTreeMap::new();
    let mut collections = Vec::with_capacity(output.collections().len());
    for collection in output.collections() {
        let demand = collection.demand();
        let mut sources = Vec::with_capacity(demand.request().sources().len());
        for (source, execution) in demand.request().sources().iter().zip(demand.executions()) {
            let route = EvmBalanceRoute::from_execution(execution)?;
            let ledger = source
                .target()
                .ledger()
                .native()
                .decode::<EvmBalanceLedger>()?;
            if ledger.chain_id() != route.chain_id() {
                return Err(invalid("source_route_ledger_mismatch"));
            }
            let target = source.target().native().decode::<EvmBalanceTarget>()?;
            let wire = RouteWire {
                chain_id: route.chain_id(),
                endpoint_id: route.endpoint().endpoint_id().to_owned(),
            };
            if routes.get(&wire.chain_id).is_some_and(|old| old != &wire) {
                return Err(invalid("conflicting_chain_routes"));
            }
            routes.insert(wire.chain_id, wire);
            sources.push(PortfolioBalanceSourceConfig {
                source_id: source.source_id().to_owned(),
                chain_id: ledger.chain_id(),
                address: target.account().clone(),
                token: target.token().cloned(),
            });
        }
        collections.push(CollectionConfig {
            correlation: demand.correlation().to_owned(),
            request: PortfolioBalanceRequestConfig::new(sources, demand.request().decimals())?,
        });
    }
    let config = EvmPortfolioConfig::Snapshot(EvmPortfolioInput {
        routes: routes.into_values().collect(),
        selector: PortfolioSnapshotSelector::new(
            output.portfolio_id().clone(),
            output.quote().clone(),
        )?,
        portfolio: PortfolioConfig {
            portfolio_id: output.portfolio_id().clone(),
            quotes: output.quotes().to_vec(),
            collections,
        },
        provenance: None,
    });
    config.validate()?;
    Ok(config)
}
/// Checks the native config projection independently of Application's history authorization.
pub fn matches_snapshot(
    config: &EvmPortfolioConfig,
    output: &PortfolioEnrichmentOutput,
) -> Result<bool, EvmPortfolioClientError> {
    let EvmPortfolioConfig::Snapshot(expected) = snapshot_config(output)? else {
        return Err(invalid("expected_snapshot"));
    };
    let EvmPortfolioConfig::Snapshot(actual) = config else {
        return Ok(false);
    };
    Ok(actual.routes == expected.routes
        && actual.selector == expected.selector
        && actual.portfolio == expected.portfolio)
}

/// Converts confirmed shared balances to the established native snapshot presentation wire.
pub fn render_snapshot(
    output: &PortfolioSnapshotOutput,
) -> Result<serde_json::Value, EvmPortfolioClientError> {
    let mut collections = Vec::with_capacity(output.collections().len());
    for collection in output.collections() {
        let mut holdings = Vec::with_capacity(collection.balances().len());
        let first = collection
            .balances()
            .first()
            .ok_or_else(|| invalid("empty_confirmed_collection"))?;
        let ledger = first
            .source()
            .target()
            .ledger()
            .native()
            .decode::<EvmBalanceLedger>()?;
        let point = first.observed_at().native().decode::<EvmBlockPoint>()?;
        for (balance, execution) in collection.balances().iter().zip(collection.executions()) {
            let route = EvmBalanceRoute::from_execution(execution)?;
            if route.chain_id() != ledger.chain_id() {
                return Err(invalid("snapshot_route_ledger_mismatch"));
            }
            let target = balance
                .source()
                .target()
                .native()
                .decode::<EvmBalanceTarget>()?;
            let asset = match target.token() {
                None => serde_json::json!({"kind": "native"}),
                Some(token) => {
                    serde_json::json!({"kind": "token", "value": {"contract": token.to_string()}})
                }
            };
            holdings.push(serde_json::json!({
                "source_id": balance.source().source_id(), "asset": asset,
                "decimals": balance.source_decimals().get(), "raw_units": balance.raw_units().as_str(),
                "amount_dec": decimal_amount(balance.raw_units().as_str(), balance.source_decimals().get()),
            }));
        }
        collections.push(serde_json::json!({"collection_ordinal": collection.metadata().collection_ordinal(), "chain_id": ledger.chain_id(),
            "anchor": {"number": point.number().to_string(), "hash": point.hash().to_string()}, "holdings": holdings}));
    }
    Ok(
        serde_json::json!({"snapshot": {"schema_version": 1, "portfolio_id": output.portfolio_id(), "collections": collections}, "report": output.report()}),
    )
}
fn decimal_amount(raw: &str, decimals: u8) -> String {
    let decimals = usize::from(decimals);
    if decimals == 0 {
        return raw.to_owned();
    }
    let padded = if raw.len() <= decimals {
        format!("{:0>width$}", raw, width = decimals + 1)
    } else {
        raw.to_owned()
    };
    let split = padded.len() - decimals;
    format!("{}.{}", &padded[..split], &padded[split..])
}

// Publishing a new source here makes its executable code available after restart. Consumers
// recomposing installed components need no registration changes: dependent codecs, handlers and
// injected States are derived by discovery rather than separately listed. Automatic discovery of
// arbitrary downstream code remains deferred; see docs/known-gaps.md#downstream-component-discovery-in-the-dsl-refactor.
/// Installed native environment for both maintained Portfolio continuations.
pub type PortfolioResources = crate::EvmResources<(
    mfm_portfolio::PortfolioSnapshotOperation,
    mfm_portfolio::PortfolioEnrichmentOperation,
)>;

#[cfg(test)]
mod tests;

mod failure;
pub use failure::{enrichment_failure, snapshot_failure};

/// Converts semantic enrichment observations into the established native product view.
/// This rendering has a different representation from the retained semantic output Object.
pub fn render_enrichment(
    output: &PortfolioEnrichmentOutput,
) -> Result<serde_json::Value, EvmPortfolioClientError> {
    let config = snapshot_config(output)?;
    let mut collections = Vec::with_capacity(output.collections().len());
    for (collection, native) in output
        .collections()
        .iter()
        .zip(&config.input().portfolio.collections)
    {
        let point = collection
            .observed_at()
            .native()
            .decode::<EvmBlockPoint>()?;
        collections.push(serde_json::json!({
            "config": native,
            "route_ref": collection.demand().route_ref()?,
            "anchor": {"number": point.number(), "hash": point.hash()},
        }));
    }
    Ok(
        serde_json::json!({"portfolio_id": output.portfolio_id(), "quotes": output.quotes(), "quote": output.quote(), "collections": collections}),
    )
}
