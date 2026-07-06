#![warn(missing_docs)]
//! Portfolio adapter runners.
//!
//! This crate binds certified portfolio state descriptors to typed runners over explicit artifact
//! and EVM capability contracts. Concrete artifact stores and live EVM transports are supplied by
//! app assembly.

use std::{fmt, sync::Arc};

use alloy_primitives::{Address, U256};
use mfm_btc_capabilities::{
    BtcAddress, BtcBalanceReadProvider, BtcBalanceReadRequest, BtcChain, BtcChainGuard,
    BtcChainHeadReadProvider, BtcChainHeadRequest, BtcHeadSelection, BtcNetworkId,
    BtcSourceIdentity,
};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBlockReadProvider, EvmBlockReadRequest,
    EvmBlockSelector, EvmCallReadProvider, EvmCallReadRequest, EvmCapabilityError, EvmChainGuard,
    EvmNetworkId,
};
use mfm_evm_core::encoding::{encode_erc20_balance_of, encode_erc20_decimals, parse_u8_u256};
use mfm_evm_core::hex::hex_to_bytes;
use mfm_ids::SchemaId;
use mfm_portfolio_model::portfolio::{ExecutionAnchor, NetworkConfig, NetworkFamilyConfig};
use mfm_portfolio_model::symbol::BalanceReaderConfig;
use mfm_program::ValidatedConfig;
use mfm_runtime::{
    load_launch_config, load_materialized_input_value, load_materialized_struct_input,
    load_non_empty_materialized_input, load_runner_config, CapabilityImplementationId,
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerRegistrationBuilder,
};
use mfm_state_portfolio::{
    observe_batch_with_backend, pin_views_with_backend, portfolio_adapter_kind,
    portfolio_adapter_version, prepare_sources_from_config, resolve_subjects_from_config,
    resolve_valuations_from_config, AssembleSnapshotConfig, AssembleSnapshotInput,
    AssembleSnapshotState, MergeObservationsConfig, MergeObservationsState, ObservationBatch,
    ObserveBatchConfig, ObserveBatchInput, ObserveBatchState, PinViewsConfig, PinViewsState,
    PortfolioReadBackend, PortfolioReadError, PortfolioReadFuture, PrepareSourcesConfig,
    PrepareSourcesState, ProjectReportConfig, ProjectReportInput, ProjectReportState,
    RawBalanceObservation, ResolveSubjectsConfig, ResolveSubjectsState, ResolveValuationsConfig,
    ResolveValuationsState,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue};
use serde::de::DeserializeOwned;

const READ_FACTORY: &str = "read_external";
const PURE_FACTORY: &str = "pure";
const ADAPTER_FACTORY: &str = "portfolio_adapter";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.portfolio.runtime.v1";

/// EVM read capabilities required by portfolio adapter runners.
pub trait PortfolioEvmProvider:
    EvmBlockReadProvider + EvmBalanceReadProvider + EvmCallReadProvider
{
}

impl<T> PortfolioEvmProvider for T where
    T: EvmBlockReadProvider + EvmBalanceReadProvider + EvmCallReadProvider
{
}

/// Bitcoin read capabilities required by portfolio adapter runners.
pub trait PortfolioBtcProvider: BtcChainHeadReadProvider + BtcBalanceReadProvider {}

impl<T> PortfolioBtcProvider for T where T: BtcChainHeadReadProvider + BtcBalanceReadProvider {}

/// Runtime validator for portfolio read routes required before run admission.
pub trait PortfolioRuntimeValidator: Send + Sync {
    /// Validates that the process can resolve an EVM guard without live network IO.
    fn validate_evm_guard(&self, guard: &EvmChainGuard) -> mfm_runtime::Result<()>;

    /// Validates that the process has a Bitcoin read provider for a guard.
    fn validate_btc_guard(&self, guard: &BtcChainGuard) -> mfm_runtime::Result<()>;
}

/// Redaction-safe portfolio adapter error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioAdapterError {
    message: String,
}

impl PortfolioAdapterError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PortfolioAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PortfolioAdapterError {}

/// Extracts certified EVM guards from a portfolio launch config artifact.
pub fn evm_chain_guards_from_launch_config(
    schema_id: &SchemaId,
    bytes: &[u8],
) -> Result<Vec<EvmChainGuard>, PortfolioAdapterError> {
    if schema_id == &config_schema::<PinViewsConfig>()? {
        let config = decode_replay_config::<PinViewsConfig>(bytes)?;
        return pin_view_evm_guards(&config).map_err(portfolio_adapter_error);
    }
    if schema_id == &config_schema::<ObserveBatchConfig>()? {
        let config = decode_replay_config::<ObserveBatchConfig>(bytes)?;
        return observe_batch_evm_guards(&config).map_err(portfolio_adapter_error);
    }
    Ok(Vec::new())
}

/// Runtime capabilities used by portfolio adapter runners.
#[derive(Clone)]
pub struct PortfolioRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    evm: Arc<dyn PortfolioEvmProvider>,
    btc: Option<Arc<dyn PortfolioBtcProvider>>,
    runtime: Arc<dyn PortfolioRuntimeValidator>,
}

impl PortfolioRunnerCapabilities {
    /// Creates portfolio runner capabilities from artifact, EVM, and optional Bitcoin providers.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        evm: Arc<dyn PortfolioEvmProvider>,
        btc: Option<Arc<dyn PortfolioBtcProvider>>,
        runtime: Arc<dyn PortfolioRuntimeValidator>,
    ) -> Self {
        Self {
            artifacts,
            evm,
            btc,
            runtime,
        }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn evm(&self) -> Arc<dyn PortfolioEvmProvider> {
        Arc::clone(&self.evm)
    }

    fn btc(&self) -> Option<Arc<dyn PortfolioBtcProvider>> {
        self.btc.as_ref().map(Arc::clone)
    }

    fn runtime(&self) -> Arc<dyn PortfolioRuntimeValidator> {
        Arc::clone(&self.runtime)
    }
}

/// Registers typed portfolio runners.
pub fn register_portfolio_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: PortfolioRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let artifacts = capabilities.artifacts();
    let evm = capabilities.evm();
    let btc = capabilities.btc();
    let runtime = capabilities.runtime();
    let mut registrations = RunnerRegistrationBuilder::new(registry, implementation_id);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-portfolio",
        "typed-portfolio",
        env!("CARGO_PKG_VERSION"),
    )?;
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let pure_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(PURE_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    registrations.register_adapter_executable_with_factory(
        portfolio_adapter_kind()?,
        portfolio_adapter_version()?,
        &adapter_factory,
    )?;
    registrations.register_state_descriptor_with_factory::<PrepareSourcesState>(
        &read_factory,
        Arc::new(PrepareSourcesRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ResolveSubjectsState>(
        &pure_factory,
        Arc::new(ResolveSubjectsRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<PinViewsState>(
        &read_factory,
        Arc::new(PinViewsRunner {
            artifacts: artifacts.clone(),
            runtime: runtime.clone(),
            evm: evm.clone(),
            btc: btc.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ResolveValuationsState>(
        &pure_factory,
        Arc::new(ResolveValuationsRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ObserveBatchState>(
        &read_factory,
        Arc::new(ObserveBatchRunner {
            artifacts: artifacts.clone(),
            runtime,
            evm,
            btc,
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<MergeObservationsState>(
        &pure_factory,
        Arc::new(MergeObservationsRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<AssembleSnapshotState>(
        &pure_factory,
        Arc::new(AssembleSnapshotRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<ProjectReportState>(
        &pure_factory,
        Arc::new(ProjectReportRunner { artifacts }),
    )?;
    Ok(())
}

struct PrepareSourcesRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for PrepareSourcesRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<PrepareSourcesConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let prepared = prepare_sources_from_config(config);
            state_output(ctx, &prepared)
        })
    }
}

struct ResolveSubjectsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ResolveSubjectsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ResolveSubjectsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let _prepared = load_materialized_input_value::<mfm_state_portfolio::PreparedSources>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = resolve_subjects_from_config(config);
            state_output(ctx, &output)
        })
    }
}

struct PinViewsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime: Arc<dyn PortfolioRuntimeValidator>,
    evm: Arc<dyn PortfolioEvmProvider>,
    btc: Option<Arc<dyn PortfolioBtcProvider>>,
}

impl ErasedNodeRunner for PinViewsRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config::<PinViewsConfig>(&ctx)?;
        for guard in
            pin_view_evm_guards(config.as_ref()).map_err(portfolio_runtime_binding_error)?
        {
            self.runtime.validate_evm_guard(&guard)?;
        }
        for guard in
            pin_view_btc_guards(config.as_ref()).map_err(portfolio_runtime_binding_error)?
        {
            self.runtime.validate_btc_guard(&guard)?;
        }
        Ok(())
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<PinViewsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let _prepared = load_materialized_input_value::<mfm_state_portfolio::PreparedSources>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let backend = CapabilityPortfolioBackend::new(Arc::clone(&self.evm), self.btc.clone());
            let output = pin_views_with_backend(config, &backend)
                .await
                .map_err(portfolio_read_runtime_error)?;
            state_output(ctx, &output)
        })
    }
}

struct ResolveValuationsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ResolveValuationsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ResolveValuationsConfig>(&ctx, self.artifacts.as_ref())
                    .await?;
            let config = config.as_ref();
            let views = load_materialized_input_value::<mfm_state_portfolio::PinnedViews>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = resolve_valuations_from_config(config, &views);
            state_output(ctx, &output)
        })
    }
}

struct ObserveBatchRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime: Arc<dyn PortfolioRuntimeValidator>,
    evm: Arc<dyn PortfolioEvmProvider>,
    btc: Option<Arc<dyn PortfolioBtcProvider>>,
}

impl ErasedNodeRunner for ObserveBatchRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config::<ObserveBatchConfig>(&ctx)?;
        for guard in
            observe_batch_evm_guards(config.as_ref()).map_err(portfolio_runtime_binding_error)?
        {
            self.runtime.validate_evm_guard(&guard)?;
        }
        for guard in
            observe_batch_btc_guards(config.as_ref()).map_err(portfolio_runtime_binding_error)?
        {
            self.runtime.validate_btc_guard(&guard)?;
        }
        Ok(())
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ObserveBatchConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let input = load_materialized_struct_input::<ObserveBatchInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let backend = CapabilityPortfolioBackend::new(Arc::clone(&self.evm), self.btc.clone());
            let output = observe_batch_with_backend(config, &input, &backend)
                .await
                .map_err(portfolio_read_runtime_error)?;
            state_output(ctx, &output)
        })
    }
}

struct MergeObservationsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for MergeObservationsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config =
                load_runner_config::<MergeObservationsConfig>(&ctx, self.artifacts.as_ref())
                    .await?;
            let batches = load_non_empty_materialized_input::<ObservationBatch>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = mfm_state_portfolio::merge_observation_batches(batches);
            state_output(ctx, &output)
        })
    }
}

struct AssembleSnapshotRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleSnapshotRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<AssembleSnapshotConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let input = load_materialized_struct_input::<AssembleSnapshotInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = mfm_state_portfolio::assemble_snapshot(config, input, 0);
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
            let config =
                load_runner_config::<ProjectReportConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let input = load_materialized_struct_input::<ProjectReportInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = mfm_state_portfolio::project_report_from_snapshot(
                input.snapshot,
                config.report_version(),
            )
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
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

fn decode_replay_config<T>(bytes: &[u8]) -> Result<T, PortfolioAdapterError>
where
    T: MfmConfig + DeserializeOwned,
{
    let config = serde_json::from_slice(bytes)
        .map_err(|error| PortfolioAdapterError::new(format!("config decode failed: {error}")))?;
    ValidatedConfig::new(config)
        .map(ValidatedConfig::into_inner)
        .map_err(|error| PortfolioAdapterError::new(format!("config validation failed: {error}")))
}

fn config_schema<T>() -> Result<SchemaId, PortfolioAdapterError>
where
    T: MfmConfig,
{
    T::schema_id()
        .map_err(|error| PortfolioAdapterError::new(format!("config schema failed: {error}")))
}

fn observe_batch_requires_evm(config: &ObserveBatchConfig) -> bool {
    config.network().family() == NetworkFamilyConfig::Evm
        && matches!(
            &config.symbol().balance_reader,
            BalanceReaderConfig::NativeBalance {} | BalanceReaderConfig::Erc20Balance { .. }
        )
}

fn observe_batch_requires_btc(config: &ObserveBatchConfig) -> bool {
    config.network().family() == NetworkFamilyConfig::Bitcoin
        && matches!(
            &config.symbol().balance_reader,
            BalanceReaderConfig::NativeBalance {}
        )
}

fn pin_view_evm_guards(config: &PinViewsConfig) -> Result<Vec<EvmChainGuard>, PortfolioReadError> {
    config
        .networks()
        .iter()
        .filter(|network| network.family() == NetworkFamilyConfig::Evm)
        .map(portfolio_evm_guard)
        .collect()
}

fn pin_view_btc_guards(config: &PinViewsConfig) -> Result<Vec<BtcChainGuard>, PortfolioReadError> {
    config
        .networks()
        .iter()
        .filter(|network| network.family() == NetworkFamilyConfig::Bitcoin)
        .map(portfolio_btc_guard)
        .collect()
}

fn observe_batch_evm_guards(
    config: &ObserveBatchConfig,
) -> Result<Vec<EvmChainGuard>, PortfolioReadError> {
    if observe_batch_requires_evm(config) {
        portfolio_evm_guard(config.network()).map(|guard| vec![guard])
    } else {
        Ok(Vec::new())
    }
}

fn observe_batch_btc_guards(
    config: &ObserveBatchConfig,
) -> Result<Vec<BtcChainGuard>, PortfolioReadError> {
    if observe_batch_requires_btc(config) {
        portfolio_btc_guard(config.network()).map(|guard| vec![guard])
    } else {
        Ok(Vec::new())
    }
}

fn portfolio_evm_guard(network: &NetworkConfig) -> Result<EvmChainGuard, PortfolioReadError> {
    let expected_chain_id = network.chain_id_u64().ok_or_else(|| {
        PortfolioReadError::new(
            "network_not_evm",
            "portfolio EVM read requested a non-EVM network",
        )
    })?;
    let network_id = EvmNetworkId::new(network.network_id().as_str()).map_err(|_| {
        PortfolioReadError::new(
            "network_id_invalid",
            "portfolio network id could not be used as an EVM guard",
        )
    })?;
    EvmChainGuard::new(network_id, expected_chain_id).map_err(|_| {
        PortfolioReadError::new(
            "chain_id_invalid",
            "portfolio EVM chain id could not be used as an EVM guard",
        )
    })
}

fn portfolio_btc_guard(network: &NetworkConfig) -> Result<BtcChainGuard, PortfolioReadError> {
    if network.family() != NetworkFamilyConfig::Bitcoin {
        return Err(PortfolioReadError::new(
            "network_not_bitcoin",
            "portfolio Bitcoin read requested a non-Bitcoin network",
        ));
    }
    let network_id = BtcNetworkId::new(network.network_id().as_str()).map_err(|_| {
        PortfolioReadError::new(
            "network_id_invalid",
            "portfolio network id could not be used as a Bitcoin guard",
        )
    })?;
    let source_identity = BtcSourceIdentity::new(network.network_id().as_str()).map_err(|_| {
        PortfolioReadError::new(
            "network_id_invalid",
            "portfolio network id could not be used as Bitcoin source identity",
        )
    })?;
    Ok(BtcChainGuard::new(
        BtcChain::Bitcoin,
        network_id,
        source_identity,
    ))
}

#[derive(Clone)]
struct CapabilityPortfolioBackend {
    evm: Arc<dyn PortfolioEvmProvider>,
    btc: Option<Arc<dyn PortfolioBtcProvider>>,
}

impl CapabilityPortfolioBackend {
    fn new(evm: Arc<dyn PortfolioEvmProvider>, btc: Option<Arc<dyn PortfolioBtcProvider>>) -> Self {
        Self { evm, btc }
    }

    async fn read_execution_anchor(
        &self,
        network: &NetworkConfig,
    ) -> Result<ExecutionAnchor, PortfolioReadError> {
        match network.family() {
            NetworkFamilyConfig::Evm => self.read_evm_execution_anchor(network).await,
            NetworkFamilyConfig::Bitcoin => self.read_bitcoin_execution_anchor(network).await,
        }
    }

    async fn read_evm_execution_anchor(
        &self,
        network: &NetworkConfig,
    ) -> Result<ExecutionAnchor, PortfolioReadError> {
        let guard = portfolio_evm_guard(network)?;
        let response = self
            .evm
            .read_block(&EvmBlockReadRequest {
                guard: guard.clone(),
                block: EvmBlockSelector::Latest,
            })
            .await
            .map_err(portfolio_evm_capability_error)?;
        response
            .evidence
            .verify_guard(&guard)
            .map_err(portfolio_evm_capability_error)?;
        evm_anchor(network, response.block_number)
    }

    async fn read_bitcoin_execution_anchor(
        &self,
        network: &NetworkConfig,
    ) -> Result<ExecutionAnchor, PortfolioReadError> {
        let guard = portfolio_btc_guard(network)?;
        let request = BtcChainHeadRequest {
            guard,
            selection: BtcHeadSelection::best(),
        };
        let btc = self.btc.as_ref().ok_or_else(missing_btc_provider)?;
        let response = btc
            .read_chain_head(&request)
            .await
            .map_err(portfolio_btc_capability_error)?;
        response
            .verify_request(&request)
            .map_err(portfolio_btc_capability_error)?;
        Ok(ExecutionAnchor::Bitcoin {
            height: response.block_height,
            block_hash: response.block_hash.to_string(),
        })
    }

    async fn read_raw_balance(
        &self,
        config: &ObserveBatchConfig,
        anchor: &ExecutionAnchor,
    ) -> Result<RawBalanceObservation, PortfolioReadError> {
        match &config.symbol().balance_reader {
            mfm_portfolio_model::symbol::BalanceReaderConfig::NativeBalance {} => {
                match config.network().family() {
                    NetworkFamilyConfig::Evm => self.read_evm_balance(config, anchor).await,
                    NetworkFamilyConfig::Bitcoin => self.read_bitcoin_balance(config, anchor).await,
                }
            }
            mfm_portfolio_model::symbol::BalanceReaderConfig::Erc20Balance { token_address } => {
                let block_number = evm_block_number_from_anchor(config.network(), anchor)?;
                let decimals = match config.symbol().decimals {
                    Some(decimals) => decimals,
                    None => {
                        self.erc20_decimals(config.network(), token_address, block_number)
                            .await?
                    }
                };
                let wallet = wallet_evm_address(config)?;
                let token = parse_address(token_address, "token address")?;
                let raw = self
                    .evm_call_u256(
                        config.network(),
                        token,
                        encode_erc20_balance_of(&wallet),
                        block_number,
                    )
                    .await?;
                Ok(RawBalanceObservation::new(
                    raw,
                    decimals,
                    Some(anchor.clone()),
                ))
            }
            mfm_portfolio_model::symbol::BalanceReaderConfig::ProtocolPosition { .. } => {
                Err(PortfolioReadError::new(
                    "unsupported_balance_reader",
                    "protocol position reads are not enabled in the typed portfolio runner",
                ))
            }
        }
    }

    async fn read_evm_balance(
        &self,
        config: &ObserveBatchConfig,
        anchor: &ExecutionAnchor,
    ) -> Result<RawBalanceObservation, PortfolioReadError> {
        let block_number = evm_block_number_from_anchor(config.network(), anchor)?;
        let wallet = wallet_evm_address(config)?;
        let guard = portfolio_evm_guard(config.network())?;
        let response = self
            .evm
            .read_balance(&EvmBalanceReadRequest {
                guard: guard.clone(),
                account: wallet,
                block: EvmBlockSelector::Number(block_number),
            })
            .await
            .map_err(portfolio_evm_capability_error)?;
        response
            .evidence
            .verify_guard(&guard)
            .map_err(portfolio_evm_capability_error)?;
        Ok(RawBalanceObservation::new(
            response.balance_wei,
            config.symbol().decimals.unwrap_or(18),
            Some(anchor.clone()),
        ))
    }

    async fn read_bitcoin_balance(
        &self,
        config: &ObserveBatchConfig,
        anchor: &ExecutionAnchor,
    ) -> Result<RawBalanceObservation, PortfolioReadError> {
        ensure_bitcoin_anchor(anchor)?;
        let guard = portfolio_btc_guard(config.network())?;
        let address = wallet_btc_address(config)?;
        let request = BtcBalanceReadRequest {
            guard,
            address,
            selection: BtcHeadSelection::best(),
        };
        let btc = self.btc.as_ref().ok_or_else(missing_btc_provider)?;
        let response = btc
            .read_balance(&request)
            .await
            .map_err(portfolio_btc_capability_error)?;
        response
            .verify_request(&request)
            .map_err(portfolio_btc_capability_error)?;
        let observed_anchor = ExecutionAnchor::Bitcoin {
            height: response.block_height,
            block_hash: response.block_hash.to_string(),
        };
        if &observed_anchor != anchor {
            return Err(observation_anchor_mismatch(anchor, &observed_anchor));
        }
        Ok(RawBalanceObservation::new(
            U256::from(response.balance_sats),
            config.symbol().decimals.unwrap_or(8),
            Some(observed_anchor),
        ))
    }

    async fn erc20_decimals(
        &self,
        network: &NetworkConfig,
        token_address: &str,
        block_number: u64,
    ) -> Result<u8, PortfolioReadError> {
        let token = parse_address(token_address, "token address")?;
        let raw = self
            .evm_call_u256(network, token, encode_erc20_decimals(), block_number)
            .await?;
        parse_u8_u256(raw).map_err(|_| {
            PortfolioReadError::new(
                "evm_response_invalid",
                "token decimals response was out of range",
            )
        })
    }

    async fn evm_call_u256(
        &self,
        network: &NetworkConfig,
        to: Address,
        calldata_hex: String,
        block_number: u64,
    ) -> Result<U256, PortfolioReadError> {
        let guard = portfolio_evm_guard(network)?;
        let calldata = hex_to_bytes(&calldata_hex).map_err(|_| {
            PortfolioReadError::new("invalid_call_data", "portfolio EVM call data was invalid")
        })?;
        let response = self
            .evm
            .read_call(&EvmCallReadRequest {
                guard: guard.clone(),
                to,
                calldata,
                block: EvmBlockSelector::Number(block_number),
            })
            .await
            .map_err(portfolio_evm_capability_error)?;
        response
            .evidence
            .verify_guard(&guard)
            .map_err(portfolio_evm_capability_error)?;
        decode_u256_return(&response.return_data)
    }
}

fn evm_anchor(
    network: &NetworkConfig,
    block_number: u64,
) -> Result<ExecutionAnchor, PortfolioReadError> {
    Ok(ExecutionAnchor::Evm {
        chain_id: network.chain_id_u64().ok_or_else(|| {
            PortfolioReadError::new(
                "network_not_evm",
                "portfolio EVM read requested a non-EVM network",
            )
        })?,
        block_number,
    })
}

fn evm_block_number_from_anchor(
    network: &NetworkConfig,
    anchor: &ExecutionAnchor,
) -> Result<u64, PortfolioReadError> {
    let expected_chain_id = network.chain_id_u64().ok_or_else(|| {
        PortfolioReadError::new(
            "network_not_evm",
            "portfolio EVM read requested a non-EVM network",
        )
    })?;
    match anchor {
        ExecutionAnchor::Evm {
            chain_id,
            block_number,
        } if *chain_id == expected_chain_id => Ok(*block_number),
        ExecutionAnchor::Evm { .. } => Err(PortfolioReadError::new(
            "network_anchor_mismatch",
            "portfolio EVM read anchor did not match the configured network",
        )),
        ExecutionAnchor::Bitcoin { .. } => Err(PortfolioReadError::new(
            "network_anchor_mismatch",
            "portfolio EVM read received a non-EVM execution anchor",
        )),
    }
}

fn ensure_bitcoin_anchor(anchor: &ExecutionAnchor) -> Result<(), PortfolioReadError> {
    match anchor {
        ExecutionAnchor::Bitcoin { .. } => Ok(()),
        ExecutionAnchor::Evm { .. } => Err(PortfolioReadError::new(
            "network_anchor_mismatch",
            "portfolio Bitcoin read received a non-Bitcoin execution anchor",
        )),
    }
}

fn observation_anchor_mismatch(
    expected: &ExecutionAnchor,
    observed: &ExecutionAnchor,
) -> PortfolioReadError {
    PortfolioReadError::new(
        "observation_anchor_mismatch",
        format!(
            "portfolio balance read did not match the pinned execution anchor: expected {}, observed {}",
            execution_anchor_label(expected),
            execution_anchor_label(observed),
        ),
    )
}

fn execution_anchor_label(anchor: &ExecutionAnchor) -> String {
    match anchor {
        ExecutionAnchor::Evm {
            chain_id,
            block_number,
        } => format!("evm(chain_id={chain_id}, block_number={block_number})"),
        ExecutionAnchor::Bitcoin { height, block_hash } => {
            format!("bitcoin(height={height}, block_hash={block_hash})")
        }
    }
}

impl PortfolioReadBackend for CapabilityPortfolioBackend {
    fn execution_anchor<'a>(
        &'a self,
        network: &'a NetworkConfig,
    ) -> PortfolioReadFuture<'a, ExecutionAnchor> {
        Box::pin(async move { self.read_execution_anchor(network).await })
    }

    fn observe_raw_balance<'a>(
        &'a self,
        config: &'a ObserveBatchConfig,
        anchor: &'a ExecutionAnchor,
    ) -> PortfolioReadFuture<'a, RawBalanceObservation> {
        Box::pin(async move { self.read_raw_balance(config, anchor).await })
    }
}

fn decode_u256_return(data: &[u8]) -> Result<U256, PortfolioReadError> {
    if data.len() != 32 {
        return Err(PortfolioReadError::new(
            "evm_response_invalid",
            "EVM call response was not a single uint256 word",
        ));
    }
    Ok(U256::from_be_slice(data))
}

fn parse_address(value: &str, label: &'static str) -> Result<Address, PortfolioReadError> {
    value.parse().map_err(|_| {
        PortfolioReadError::new(
            "invalid_evm_address",
            format!("{label} was invalid for portfolio EVM read"),
        )
    })
}

fn wallet_evm_address(config: &ObserveBatchConfig) -> Result<Address, PortfolioReadError> {
    let Some(address) = config.wallet().subject.evm_address() else {
        return Err(PortfolioReadError::new(
            "invalid_wallet_subject",
            "wallet subject was not an EVM address for portfolio EVM read",
        ));
    };
    parse_address(address, "wallet address")
}

fn wallet_btc_address(config: &ObserveBatchConfig) -> Result<BtcAddress, PortfolioReadError> {
    if config.wallet().subject.kind()
        != mfm_portfolio_model::wallet::WalletSubjectKind::BitcoinAddress
    {
        return Err(PortfolioReadError::new(
            "invalid_wallet_subject",
            "wallet subject was not a Bitcoin address for portfolio Bitcoin read",
        ));
    }
    BtcAddress::new(config.wallet().subject.address_str()).map_err(|_| {
        PortfolioReadError::new(
            "invalid_bitcoin_address",
            "wallet address was invalid for portfolio Bitcoin read",
        )
    })
}

fn portfolio_evm_capability_error(error: EvmCapabilityError) -> PortfolioReadError {
    let details = match &error {
        EvmCapabilityError::ChainMismatch { evidence } => {
            Some(evidence.chain_mismatch_diagnostic_details())
        }
        _ => None,
    };
    let error = PortfolioReadError::new(
        "evm_capability_failed",
        format!("portfolio EVM read capability failed: {error}"),
    );
    if let Some(details) = details {
        error
            .with_redacted_details(details)
            .with_fatal_attempt_failure()
    } else {
        error
    }
}

fn portfolio_btc_capability_error(
    error: mfm_btc_capabilities::BtcCapabilityError,
) -> PortfolioReadError {
    PortfolioReadError::new(
        "bitcoin_capability_failed",
        format!("portfolio Bitcoin read capability failed: {error}"),
    )
}

fn missing_btc_provider() -> PortfolioReadError {
    PortfolioReadError::new(
        "bitcoin_capability_unavailable",
        "portfolio Bitcoin read capability is unavailable",
    )
    .with_fatal_attempt_failure()
}

fn portfolio_read_runtime_error(error: PortfolioReadError) -> mfm_runtime::RuntimeError {
    let message = error.to_string();
    match error.redacted_details {
        Some(details) => mfm_runtime::RuntimeError::InvalidRunnerOutputDiagnostic {
            message,
            details: mfm_runtime::RuntimeDiagnosticDetails::from_json(details),
        },
        None => mfm_runtime::RuntimeError::InvalidRunnerOutput(message),
    }
}

fn portfolio_runtime_binding_error(error: PortfolioReadError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn portfolio_adapter_error(error: PortfolioReadError) -> PortfolioAdapterError {
    PortfolioAdapterError::new(error.to_string())
}

#[cfg(test)]
mod tests;
