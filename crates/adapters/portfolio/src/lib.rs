#![warn(missing_docs)]
//! Portfolio adapter runners for fact-backed receipt-pinned portfolio snapshots.
//!
//! This crate binds certified portfolio state descriptors to typed runners over explicit artifact
//! and Platform fact-index capability contracts. Live chain transports are not used by the report
//! graph after the collectors cutover.

use std::collections::BTreeSet;
use std::sync::Arc;

use mfm_artifact_capabilities::{fact_response_artifact_requirement, hydrate_fact_response_json};
use mfm_events::v1 as events;
use mfm_fact_capabilities::{FactIndexReadProvider, FactIndexReadRequest};
use mfm_facts::{
    fact_query_result_rows_from_receipt, CanonicalFactQueryPlan, FactCanonicalScalar,
    FactQueryEvidence, QueryResultCardinality, ScopeDecisionEvidence, StoreReadFrontier,
    StoreReadFrontierType, StoreScopeRef,
};
use mfm_portfolio_model::symbol::{HoldingSourceConfig, ObservationAnchor};
use mfm_program::{MfmFactType, StateSpec, ValidatedConfig};
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config_for_node, ErasedNodeRunner, ErasedRunCtx,
    ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry, RunnerExecutableIdentityTemplate,
    RunnerOutputBuilder, RunnerRegistrationBuilder,
};
use mfm_state_portfolio::{
    assemble_snapshot, holding_candidate_from_normalized, observations_from_selected_holdings,
    portfolio_adapter_kind, portfolio_adapter_version,
    portfolio_holding_select_scope_decision_hash, project_network_pins_from_observations,
    symbols_by_id_map, validate_receipt_against_portfolio, AssembleSnapshotConfig,
    AssembleSnapshotInput, AssembleSnapshotState, CollectedHoldingReceipt, HoldingCandidate,
    HoldingRequirementKey, HoldingSourceKey, NormalizedHoldingFields, PortfolioCollectionReceipt,
    PortfolioHoldingErrorCode, PortfolioHoldingSelectionError, ProjectReportConfig,
    ProjectReportInput, ProjectReportState, SelectHoldingsConfig, SelectHoldingsInput,
    SelectHoldingsState, SelectedHolding, SelectedHoldings,
};
use mfm_states_btc::{
    normalize_btc_address_balance, platform_address_balance_at_anchor_plan,
    BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact, BtcAddressBalanceSubject,
};
use mfm_states_evm::{
    normalize_evm_address_native_balance, platform_erc20_balance_at_anchor_plan,
    platform_native_balance_at_anchor_plan, EvmAddressErc20BalanceResponse,
    EvmAddressErc20BalanceSnapshotFact, EvmAddressErc20BalanceSubject,
    EvmAddressNativeBalanceResponse, EvmAddressNativeBalanceSnapshotFact,
    EvmAddressNativeBalanceSubject,
};
use mfm_store::v1 as store;
use mfm_values::MfmValue;
use serde::de::DeserializeOwned;

#[path = "replay.rs"]
mod replay;
#[path = "selection.rs"]
mod selection;
pub use self::replay::verify_portfolio_replay;
#[cfg(test)]
pub(crate) use self::selection::{holding_fact_index_request, select_holdings};

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const ADAPTER_FACTORY: &str = "portfolio_adapter";

/// Runtime capabilities used by portfolio adapter runners.
#[derive(Clone)]
pub struct PortfolioRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl PortfolioRunnerCapabilities {
    /// Creates portfolio runner capabilities from artifact and Platform fact-index providers.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        fact_index: Arc<dyn FactIndexReadProvider>,
    ) -> Self {
        Self {
            artifacts,
            fact_index,
        }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn fact_index(&self) -> Arc<dyn FactIndexReadProvider> {
        Arc::clone(&self.fact_index)
    }
}

/// Registers typed portfolio runners for receipt-pinned selection and pure snapshot projection.
pub fn register_portfolio_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: PortfolioRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let artifacts = capabilities.artifacts();
    let fact_index = capabilities.fact_index();
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-portfolio",
        "typed-portfolio",
        env!("CARGO_PKG_VERSION"),
    )?;
    let pure_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(PURE_FACTORY)?);
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    registrations.register_adapter_executable_with_factory(
        portfolio_adapter_kind()?,
        portfolio_adapter_version()?,
        &adapter_factory,
    )?;
    registrations.register_state_runner_with_factory::<SelectHoldingsState>(
        &read_factory,
        Arc::new(SelectHoldingsRunner {
            artifacts: artifacts.clone(),
            fact_index,
        }),
    )?;
    registrations.register_state_runner_with_factory::<AssembleSnapshotState>(
        &pure_factory,
        Arc::new(AssembleSnapshotRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ProjectReportState>(
        &pure_factory,
        Arc::new(ProjectReportRunner { artifacts }),
    )?;
    Ok(())
}

struct SelectHoldingsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl ErasedNodeRunner for SelectHoldingsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<SelectHoldingsConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<SelectHoldingsInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let (selected, evidences) = selection::select_holdings(
                config,
                input,
                self.artifacts.as_ref(),
                self.fact_index.as_ref(),
            )
            .await?;
            selection::select_holdings_output(ctx, &selected, &evidences)
        })
    }
}

struct AssembleSnapshotRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleSnapshotRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<AssembleSnapshotConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<AssembleSnapshotInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = assemble_snapshot(config.as_ref(), input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            state_output(ctx, &output)
        })
    }
}

struct ProjectReportRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ProjectReportRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config = load_runner_config_for_node::<ProjectReportConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<ProjectReportInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = mfm_state_portfolio::project_report_from_snapshot(input.snapshot)
                .map_err(|error| {
                    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                })?;
            state_output(ctx, &output)
        })
    }
}

fn state_output<T>(ctx: ErasedRunCtx<'_>, value: &T) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue,
{
    ErasedRunnerOutput::state_output(&ctx, value)
}

#[cfg(test)]
mod tests;
