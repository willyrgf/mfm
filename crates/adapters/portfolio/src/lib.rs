#![warn(missing_docs)]
//! Portfolio adapter runners.
//!
//! This crate binds certified portfolio state descriptors to typed runners over explicit artifact,
//! EVM, and Bitcoin capability contracts. Concrete artifact stores and live transports are supplied
//! by app assembly.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use alloy_primitives::{Address, B256, U256};
use mfm_btc_capabilities::{
    BitcoinNetworkTag, BtcAddress, BtcBalanceReadProvider, BtcBalanceReadRequest, BtcBlockHash,
    BtcCapabilityError, BtcChainHeadReadProvider, BtcChainHeadRequest, BtcHeadSelection,
    BtcNetworkId, BtcSourceBinding, BtcSourceIdentity,
};
use mfm_capabilities::{ProviderDiagnosticCode, RedactedProviderDiagnostic};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBlockReadProvider, EvmBlockReadRequest,
    EvmBlockSelector, EvmCallReadProvider, EvmCallReadRequest, EvmCapabilityError,
    EvmNetworkBinding, EvmNetworkId,
};
use mfm_evm_core::encoding::{encode_erc20_balance_of, encode_erc20_decimals, parse_u8_u256};
use mfm_evm_core::hex::hex_to_bytes;
use mfm_portfolio_model::portfolio::ExecutionAnchor;
use mfm_runtime::{
    load_launch_config, load_materialized_input_value, load_materialized_struct_input,
    load_non_empty_materialized_input, load_runner_config, CapabilityImplementationId,
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerRegistrationBuilder,
};
use mfm_state_portfolio::{
    network_read_intent_for_network, observation_batch_error, observation_batch_from_raw_balance,
    observation_batch_missing_pinned_view, observe_batch_network_read_intent,
    observe_batch_read_intent, pin_view_read_intents, pinned_anchor_for, pinned_view_for_network,
    pinned_views_from_views, portfolio_adapter_kind, portfolio_adapter_version,
    resolve_subjects_from_config, resolve_valuations_from_config, AssembleSnapshotConfig,
    AssembleSnapshotInput, AssembleSnapshotState, MergeObservationsConfig, MergeObservationsState,
    ObservationBatch, ObserveBatchConfig, ObserveBatchInput, ObserveBatchState, PinViewsConfig,
    PinViewsState, PortfolioBalanceReadIntent, PortfolioNetworkReadIntent, PortfolioReadError,
    ProjectReportConfig, ProjectReportInput, ProjectReportState, RawBalanceObservation,
    ResolveSubjectsConfig, ResolveSubjectsState, ResolveValuationsConfig, ResolveValuationsState,
};
use mfm_store::v1 as store;
use mfm_values::MfmValue;

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

/// Factory for portfolio transport providers bound to certified source intent.
pub trait PortfolioTransportFactory: Send + Sync {
    /// Validates that the EVM binding can resolve without live network IO.
    fn validate_evm_network_binding(&self, binding: &EvmNetworkBinding) -> mfm_runtime::Result<()>;

    /// Binds a checked network binding to EVM read capabilities.
    fn bind_evm_network(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn PortfolioEvmProvider>>;

    /// Validates that the Bitcoin binding can resolve without live network IO.
    fn validate_btc_source_binding(&self, binding: &BtcSourceBinding) -> mfm_runtime::Result<()>;

    /// Binds a checked source binding to Bitcoin read capabilities.
    fn bind_btc_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn PortfolioBtcProvider>>;
}

/// Runtime capabilities used by portfolio adapter runners.
#[derive(Clone)]
pub struct PortfolioRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    transport: Arc<dyn PortfolioTransportFactory>,
}

impl PortfolioRunnerCapabilities {
    /// Creates portfolio runner capabilities from artifact and transport providers.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        transport: Arc<dyn PortfolioTransportFactory>,
    ) -> Self {
        Self {
            artifacts,
            transport,
        }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn transport(&self) -> Arc<dyn PortfolioTransportFactory> {
        Arc::clone(&self.transport)
    }
}

/// Registers typed portfolio runners.
pub fn register_portfolio_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: PortfolioRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let artifacts = capabilities.artifacts();
    let transport = capabilities.transport();
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
            transport: transport.clone(),
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
            transport,
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

struct ResolveSubjectsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ResolveSubjectsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<ResolveSubjectsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let output = resolve_subjects_from_config(config);
            state_output(ctx, &output)
        })
    }
}

struct PinViewsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    transport: Arc<dyn PortfolioTransportFactory>,
}

impl ErasedNodeRunner for PinViewsRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config::<PinViewsConfig>(&ctx)?;
        validate_network_read_intents(
            self.transport.as_ref(),
            pin_view_read_intents(config.as_ref()),
        )
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config::<PinViewsConfig>(&ctx, self.artifacts.as_ref()).await?;
            let config = config.as_ref();
            let backend = CapabilityPortfolioBackend::new(Arc::clone(&self.transport));
            let mut views = Vec::new();
            for network in config.networks() {
                let intent = network_read_intent_for_network(network);
                let anchor = backend
                    .read_execution_anchor(&intent)
                    .await
                    .map_err(portfolio_read_runtime_error)?;
                views.push(pinned_view_for_network(network, anchor));
            }
            let output = pinned_views_from_views(views);
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
    transport: Arc<dyn PortfolioTransportFactory>,
}

impl ErasedNodeRunner for ObserveBatchRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config::<ObserveBatchConfig>(&ctx)?;
        if let Some(intent) = observe_batch_network_read_intent(config.as_ref()) {
            validate_network_read_intents(self.transport.as_ref(), [intent])?;
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
            let backend = CapabilityPortfolioBackend::new(Arc::clone(&self.transport));
            let output = observe_batch_with_capabilities(config, &input, &backend)
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

async fn observe_batch_with_capabilities(
    config: &ObserveBatchConfig,
    input: &ObserveBatchInput,
    backend: &CapabilityPortfolioBackend,
) -> Result<ObservationBatch, PortfolioReadError> {
    let Some(anchor) = pinned_anchor_for(&input.views, config.network().network_id().as_str())
    else {
        return Ok(observation_batch_missing_pinned_view(
            config,
            input.valuations.errors.clone(),
        ));
    };
    let intent = match observe_batch_read_intent(config, anchor) {
        Ok(intent) => intent,
        Err(error) => return Ok(observation_batch_error(config, error.code, error.message)),
    };
    match backend.read_raw_balance(&intent).await {
        Ok(balance) => Ok(observation_batch_from_raw_balance(config, input, balance)),
        Err(error) if error.fatal_attempt_failure => Err(error),
        Err(error) => Ok(observation_batch_error(config, error.code, error.message)),
    }
}

fn state_output<T>(ctx: ErasedRunCtx<'_>, value: &T) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue,
{
    ErasedRunnerOutput::state_output(&ctx, value)
}

fn validate_network_read_intents(
    transport: &dyn PortfolioTransportFactory,
    intents: impl IntoIterator<Item = PortfolioNetworkReadIntent>,
) -> mfm_runtime::Result<()> {
    for intent in intents {
        match intent {
            PortfolioNetworkReadIntent::Evm {
                network_id,
                chain_id,
            } => {
                let binding = portfolio_evm_network_binding(&network_id, chain_id)
                    .map_err(portfolio_runtime_binding_error)?;
                transport.validate_evm_network_binding(&binding)?;
            }
            PortfolioNetworkReadIntent::Bitcoin {
                network_id,
                source_identity,
                bitcoin_network,
            } => {
                let binding =
                    portfolio_btc_source_binding(&network_id, &source_identity, &bitcoin_network)
                        .map_err(portfolio_runtime_binding_error)?;
                transport.validate_btc_source_binding(&binding)?;
            }
        }
    }
    Ok(())
}

fn portfolio_evm_network_id(network_id: &str) -> Result<EvmNetworkId, PortfolioReadError> {
    EvmNetworkId::new(network_id).map_err(|_| {
        PortfolioReadError::new(
            "network_id_invalid",
            "portfolio network id could not be used as an EVM request",
        )
    })
}

fn portfolio_evm_network_binding(
    network_id: &str,
    expected_chain_id: u64,
) -> Result<EvmNetworkBinding, PortfolioReadError> {
    let network_id = portfolio_evm_network_id(network_id)?;
    EvmNetworkBinding::new(network_id, expected_chain_id).map_err(|_| {
        PortfolioReadError::new(
            "chain_id_invalid",
            "portfolio expected EVM chain id could not be used as an EVM source binding",
        )
    })
}

fn portfolio_btc_source_identity(
    source_identity: &str,
) -> Result<BtcSourceIdentity, PortfolioReadError> {
    BtcSourceIdentity::new(source_identity).map_err(|_| {
        PortfolioReadError::new(
            "source_identity_invalid",
            "portfolio source identity could not be used as Bitcoin source identity",
        )
    })
}

fn portfolio_btc_source_binding(
    network_id: &str,
    source_identity: &str,
    bitcoin_network: &str,
) -> Result<BtcSourceBinding, PortfolioReadError> {
    let network_id = BtcNetworkId::new(network_id).map_err(|_| {
        PortfolioReadError::new(
            "network_id_invalid",
            "portfolio network id could not be used as a Bitcoin source binding",
        )
    })?;
    let source_identity = portfolio_btc_source_identity(source_identity)?;
    let bitcoin_network = BitcoinNetworkTag::new(bitcoin_network).map_err(|_| {
        PortfolioReadError::new(
            "bitcoin_network_invalid",
            "portfolio Bitcoin network tag could not be used as a Bitcoin source binding",
        )
    })?;
    BtcSourceBinding::new(network_id, source_identity, bitcoin_network).map_err(|_| {
        PortfolioReadError::new(
            "bitcoin_source_binding_invalid",
            "portfolio Bitcoin source binding was invalid",
        )
    })
}

type PortfolioEvmProviderCache = Mutex<BTreeMap<(String, u64), Arc<dyn PortfolioEvmProvider>>>;
type PortfolioBtcProviderCache =
    Mutex<BTreeMap<(String, String, String), Arc<dyn PortfolioBtcProvider>>>;

struct CapabilityPortfolioBackend {
    transport: Arc<dyn PortfolioTransportFactory>,
    evm_providers: PortfolioEvmProviderCache,
    btc_providers: PortfolioBtcProviderCache,
}

impl CapabilityPortfolioBackend {
    fn new(transport: Arc<dyn PortfolioTransportFactory>) -> Self {
        Self {
            transport,
            evm_providers: Mutex::new(BTreeMap::new()),
            btc_providers: Mutex::new(BTreeMap::new()),
        }
    }

    async fn read_execution_anchor(
        &self,
        intent: &PortfolioNetworkReadIntent,
    ) -> Result<ExecutionAnchor, PortfolioReadError> {
        match intent {
            PortfolioNetworkReadIntent::Evm {
                network_id,
                chain_id,
            } => self.read_evm_execution_anchor(network_id, *chain_id).await,
            PortfolioNetworkReadIntent::Bitcoin {
                network_id,
                source_identity,
                bitcoin_network,
            } => {
                self.read_bitcoin_execution_anchor(network_id, source_identity, bitcoin_network)
                    .await
            }
        }
    }

    async fn read_evm_execution_anchor(
        &self,
        network_id: &str,
        chain_id: u64,
    ) -> Result<ExecutionAnchor, PortfolioReadError> {
        let provider = self.evm_provider(network_id, chain_id)?;
        let response = provider
            .read_block(&EvmBlockReadRequest::new(EvmBlockSelector::Latest))
            .await
            .map_err(portfolio_evm_capability_error)?;
        Ok(ExecutionAnchor::Evm {
            chain_id,
            block_number: response.block_number,
            block_hash: format!("{:?}", response.block_hash),
        })
    }

    async fn read_bitcoin_execution_anchor(
        &self,
        network_id: &str,
        source_identity: &str,
        bitcoin_network: &str,
    ) -> Result<ExecutionAnchor, PortfolioReadError> {
        let provider = self.btc_provider(network_id, source_identity, bitcoin_network)?;
        let request = BtcChainHeadRequest::new(BtcHeadSelection::best());
        let response = provider
            .read_chain_head(&request)
            .await
            .map_err(portfolio_btc_capability_error)?;
        Ok(ExecutionAnchor::Bitcoin {
            height: response.block_height,
            block_hash: response.block_hash.to_string(),
        })
    }

    async fn read_raw_balance(
        &self,
        intent: &PortfolioBalanceReadIntent,
    ) -> Result<RawBalanceObservation, PortfolioReadError> {
        match intent {
            PortfolioBalanceReadIntent::EvmNativeBalance {
                network_id,
                chain_id,
                account,
                block_number: _,
                block_hash,
                decimals,
                anchor,
            } => {
                self.read_evm_balance(
                    network_id, *chain_id, account, block_hash, *decimals, anchor,
                )
                .await
            }
            PortfolioBalanceReadIntent::Erc20Balance {
                network_id,
                chain_id,
                account,
                token_address,
                block_number: _,
                block_hash,
                decimals,
                anchor,
            } => {
                let account = parse_address(account, "wallet address")?;
                let token = parse_address(token_address, "token address")?;
                let block_hash = parse_evm_block_hash(block_hash)?;
                let decimals = match decimals {
                    Some(decimals) => *decimals,
                    None => {
                        self.erc20_decimals(network_id, *chain_id, token, block_hash)
                            .await?
                    }
                };
                let raw = self
                    .evm_call_u256(
                        network_id,
                        *chain_id,
                        token,
                        encode_erc20_balance_of(&account),
                        block_hash,
                    )
                    .await?;
                Ok(RawBalanceObservation::new(
                    raw,
                    decimals,
                    Some(anchor.clone()),
                ))
            }
            PortfolioBalanceReadIntent::BitcoinNativeBalance {
                network_id,
                source_identity,
                bitcoin_network,
                address,
                anchor,
                decimals,
            } => {
                self.read_bitcoin_balance(
                    network_id,
                    source_identity,
                    bitcoin_network,
                    address,
                    anchor,
                    *decimals,
                )
                .await
            }
        }
    }

    async fn read_evm_balance(
        &self,
        network_id: &str,
        chain_id: u64,
        account: &str,
        block_hash: &str,
        decimals: u8,
        anchor: &ExecutionAnchor,
    ) -> Result<RawBalanceObservation, PortfolioReadError> {
        let account = parse_address(account, "wallet address")?;
        let block_hash = parse_evm_block_hash(block_hash)?;
        let provider = self.evm_provider(network_id, chain_id)?;
        let response = provider
            .read_balance(&EvmBalanceReadRequest::new(
                account,
                EvmBlockSelector::Hash(block_hash),
            ))
            .await
            .map_err(portfolio_evm_capability_error)?;
        Ok(RawBalanceObservation::new(
            response.balance_wei,
            decimals,
            Some(anchor.clone()),
        ))
    }

    async fn read_bitcoin_balance(
        &self,
        network_id: &str,
        source_identity: &str,
        bitcoin_network: &str,
        address: &str,
        anchor: &ExecutionAnchor,
        decimals: u8,
    ) -> Result<RawBalanceObservation, PortfolioReadError> {
        let address = parse_btc_address(address)?;
        let (block_height, block_hash) = btc_anchor_parts(anchor)?;
        let request = BtcBalanceReadRequest::new(address, block_height, block_hash);
        let provider = self.btc_provider(network_id, source_identity, bitcoin_network)?;
        let response = provider
            .read_balance(&request)
            .await
            .map_err(portfolio_btc_capability_error)?;
        Ok(RawBalanceObservation::new(
            U256::from(response.balance_sats),
            decimals,
            Some(anchor.clone()),
        ))
    }

    async fn erc20_decimals(
        &self,
        network_id: &str,
        chain_id: u64,
        token: Address,
        block_hash: B256,
    ) -> Result<u8, PortfolioReadError> {
        let raw = self
            .evm_call_u256(
                network_id,
                chain_id,
                token,
                encode_erc20_decimals(),
                block_hash,
            )
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
        network_id: &str,
        chain_id: u64,
        to: Address,
        calldata_hex: String,
        block_hash: B256,
    ) -> Result<U256, PortfolioReadError> {
        let calldata = hex_to_bytes(&calldata_hex).map_err(|_| {
            PortfolioReadError::new("invalid_call_data", "portfolio EVM call data was invalid")
        })?;
        let provider = self.evm_provider(network_id, chain_id)?;
        let response = provider
            .read_call(&EvmCallReadRequest::new(
                to,
                calldata,
                EvmBlockSelector::Hash(block_hash),
            ))
            .await
            .map_err(portfolio_evm_capability_error)?;
        decode_u256_return(&response.return_data)
    }

    fn evm_provider(
        &self,
        network_id: &str,
        chain_id: u64,
    ) -> Result<Arc<dyn PortfolioEvmProvider>, PortfolioReadError> {
        let binding = portfolio_evm_network_binding(network_id, chain_id)?;
        let key = (network_id.to_owned(), chain_id);
        let mut providers = self
            .evm_providers
            .lock()
            .map_err(|_| portfolio_provider_cache_error())?;
        if let Some(provider) = providers.get(&key).cloned() {
            return Ok(provider);
        }
        let provider = self
            .transport
            .bind_evm_network(binding)
            .map_err(portfolio_evm_capability_error)?;
        providers.insert(key, Arc::clone(&provider));
        Ok(provider)
    }

    fn btc_provider(
        &self,
        network_id: &str,
        source_identity: &str,
        bitcoin_network: &str,
    ) -> Result<Arc<dyn PortfolioBtcProvider>, PortfolioReadError> {
        let binding = portfolio_btc_source_binding(network_id, source_identity, bitcoin_network)?;
        let key = (
            network_id.to_owned(),
            source_identity.to_owned(),
            bitcoin_network.to_owned(),
        );
        let mut providers = self
            .btc_providers
            .lock()
            .map_err(|_| portfolio_provider_cache_error())?;
        if let Some(provider) = providers.get(&key).cloned() {
            return Ok(provider);
        }
        let provider = self
            .transport
            .bind_btc_source(binding)
            .map_err(portfolio_btc_capability_error)?;
        providers.insert(key, Arc::clone(&provider));
        Ok(provider)
    }
}

fn portfolio_provider_cache_error() -> PortfolioReadError {
    PortfolioReadError::new(
        "portfolio_provider_cache_unavailable",
        "portfolio provider cache was unavailable",
    )
}

fn btc_anchor_parts(anchor: &ExecutionAnchor) -> Result<(u64, BtcBlockHash), PortfolioReadError> {
    match anchor {
        ExecutionAnchor::Bitcoin { height, block_hash } => {
            let block_hash = BtcBlockHash::new(block_hash).map_err(|_| {
                PortfolioReadError::new(
                    "bitcoin_anchor_invalid",
                    "portfolio Bitcoin execution anchor block hash was invalid",
                )
            })?;
            Ok((*height, block_hash))
        }
        ExecutionAnchor::Evm { .. } => Err(PortfolioReadError::new(
            "bitcoin_anchor_invalid",
            "portfolio Bitcoin read required a Bitcoin execution anchor",
        )),
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

fn parse_evm_block_hash(value: &str) -> Result<B256, PortfolioReadError> {
    value.parse().map_err(|_| {
        PortfolioReadError::new(
            "evm_anchor_invalid",
            "portfolio EVM execution anchor block hash was invalid",
        )
    })
}

fn parse_btc_address(value: &str) -> Result<BtcAddress, PortfolioReadError> {
    BtcAddress::new(value).map_err(|_| {
        PortfolioReadError::new(
            "invalid_bitcoin_address",
            "wallet address was invalid for portfolio Bitcoin read",
        )
    })
}

fn portfolio_evm_capability_error(error: EvmCapabilityError) -> PortfolioReadError {
    match error {
        EvmCapabilityError::InvalidRequest { .. } => PortfolioReadError::new(
            "evm_invalid_request",
            "portfolio EVM read capability request was invalid",
        ),
        EvmCapabilityError::Provider { diagnostic }
        | EvmCapabilityError::SourceMismatch { diagnostic } => {
            portfolio_provider_diagnostic_error("EVM", diagnostic)
        }
        EvmCapabilityError::ReceiptPending => PortfolioReadError::new(
            "evm_receipt_pending",
            "portfolio EVM read transaction receipt was pending",
        ),
    }
}

fn portfolio_btc_capability_error(error: BtcCapabilityError) -> PortfolioReadError {
    match error {
        BtcCapabilityError::InvalidRequest { .. } => PortfolioReadError::new(
            "bitcoin_invalid_request",
            "portfolio Bitcoin read capability request was invalid",
        ),
        BtcCapabilityError::Provider { diagnostic }
        | BtcCapabilityError::SourceMismatch { diagnostic } => {
            portfolio_provider_diagnostic_error("Bitcoin", diagnostic)
        }
    }
}

fn portfolio_provider_diagnostic_error(
    provider_label: &str,
    diagnostic: RedactedProviderDiagnostic,
) -> PortfolioReadError {
    let fatal_attempt_failure = matches!(
        diagnostic.code(),
        ProviderDiagnosticCode::SourceMismatch | ProviderDiagnosticCode::UnsupportedOperation
    );
    let error = PortfolioReadError::new(
        diagnostic.stable_error_code(),
        format!("portfolio {provider_label} read capability failed: {diagnostic}"),
    )
    .with_redacted_details(diagnostic.to_public_details_json());
    if fatal_attempt_failure {
        error.with_fatal_attempt_failure()
    } else {
        error
    }
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

#[cfg(test)]
mod tests;
