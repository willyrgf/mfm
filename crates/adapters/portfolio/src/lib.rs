#![warn(missing_docs)]
//! Portfolio adapter runners for certified portfolio snapshots.
//!
//! This crate binds one checked source-stable EVM read session per demanded network, executes
//! exact-hash balance reads with bounded concurrency, and atomically records each unified fact
//! batch with its direct network snapshot. It also retains the Platform fact-index binding used
//! only by the Bitcoin receipt-pinned selection path.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use alloy_primitives::{Address, Bytes, U256};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBlockSelector, EvmCall, EvmCapabilityError, EvmNetworkBinding, EvmReadCapability,
    EvmReadSession, ProviderDiagnosticCode, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_fact_capabilities::{FactIndexReadProvider, FactRecordCapability};
use mfm_portfolio_model::evm::EvmBlockAnchor;
use mfm_portfolio_model::symbol::HoldingSourceConfig;
use mfm_program::{ManagedWriteState, StateSpec};
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config_for_node, CapabilityImplementationId,
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    ExternalReadExecution, ExternalReadExecutionFuture, ExternalReadPlanExecutor,
    ExternalReadRunner, FactRecordInput, RunnerCapabilityBinding, RunnerExecutableIdentityTemplate,
    RunnerIngressContext, RunnerOutputBuilder, RunnerRegistrationBuilder,
};
use mfm_state_portfolio::{
    assemble_snapshot, portfolio_adapter_kind, portfolio_adapter_version,
    validate_receipt_against_portfolio, AssembleSnapshotConfig, AssembleSnapshotInput,
    AssembleSnapshotState, CollectEvmNetworkEvidence, CollectEvmNetworkPlan,
    CollectEvmNetworkState, EvmBalanceReadEvidence, EvmNetworkCollectionConfig,
    EvmTokenDecimalsEvidence, PortfolioCollectionReceipt, ProjectReportConfig, ProjectReportInput,
    ProjectReportState, PublishEvmHoldingsInput, PublishEvmHoldingsState, SelectHoldingsConfig,
    SelectHoldingsInput, SelectHoldingsReadEvidence, SelectHoldingsReadPlan, SelectHoldingsState,
    SelectedHoldings,
};
use mfm_store::v1 as store;
use mfm_values::MfmValue;

#[path = "replay.rs"]
mod replay;
#[path = "selection.rs"]
mod selection;
pub use self::replay::verify_portfolio_replay;
#[cfg(test)]
pub(crate) use self::selection::select_holdings;

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const MANAGED_WRITE_FACTORY: &str = "managed_platform_write";
const ADAPTER_FACTORY: &str = "portfolio_adapter";

/// Future returned by the application-owned EVM read-session binder.
pub type PortfolioEvmReadSessionBindFuture = Pin<
    Box<
        dyn Future<Output = mfm_evm_capabilities::Result<Arc<dyn EvmReadSession>>> + Send + 'static,
    >,
>;

type ValidateEvmBinding =
    dyn Fn(&EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> + Send + Sync;
type BindEvmReadSession =
    dyn Fn(EvmNetworkBinding) -> PortfolioEvmReadSessionBindFuture + Send + Sync;

/// Runtime capabilities used by portfolio adapter runners.
#[derive(Clone)]
pub struct PortfolioRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
    validate_evm_binding: Arc<ValidateEvmBinding>,
    bind_evm_read_session: Arc<BindEvmReadSession>,
}

impl PortfolioRunnerCapabilities {
    /// Creates portfolio runner capabilities from artifact and Platform fact-index providers.
    pub fn new<V, B>(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        fact_index: Arc<dyn FactIndexReadProvider>,
        validate_evm_binding: V,
        bind_evm_read_session: B,
    ) -> Self
    where
        V: Fn(&EvmNetworkBinding) -> mfm_evm_capabilities::Result<()> + Send + Sync + 'static,
        B: Fn(EvmNetworkBinding) -> PortfolioEvmReadSessionBindFuture + Send + Sync + 'static,
    {
        Self {
            artifacts,
            fact_index,
            validate_evm_binding: Arc::new(validate_evm_binding),
            bind_evm_read_session: Arc::new(bind_evm_read_session),
        }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn fact_index(&self) -> Arc<dyn FactIndexReadProvider> {
        Arc::clone(&self.fact_index)
    }

    fn validate_evm_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()> {
        (self.validate_evm_binding)(binding)
    }

    async fn bind_evm_read_session(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn EvmReadSession>> {
        let session = (self.bind_evm_read_session)(binding.clone()).await?;
        if !session.evidence().matches_binding(&binding)
            || session.evidence().implementation_id() != EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
        {
            return Err(EvmCapabilityError::provider_failure(
                mfm_evm_capabilities::evm_diagnostic(
                    ProviderDiagnosticCode::ProviderConfigurationInvalid,
                ),
            ));
        }
        Ok(session)
    }
}

/// Registers EVM collection/publication, Bitcoin selection, and snapshot projection runners.
pub fn register_portfolio_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: PortfolioRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let artifacts = capabilities.artifacts();
    let fact_index = capabilities.fact_index();
    registry.register_capability_spec::<EvmReadCapability>(CapabilityImplementationId::new(
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
    )?)?;
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
    let managed_write_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(MANAGED_WRITE_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    registrations.register_adapter_executable_with_factory(
        portfolio_adapter_kind()?,
        portfolio_adapter_version()?,
        &adapter_factory,
    )?;
    registrations.register_state_runner_with_factory::<SelectHoldingsState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<SelectHoldingsState, _>::new(
            artifacts.clone(),
            SelectHoldingsExecutor {
                artifacts: artifacts.clone(),
                fact_index,
            },
        )),
    )?;
    registrations.register_state_runner_with_factory::<CollectEvmNetworkState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<CollectEvmNetworkState, _>::new(
            artifacts.clone(),
            CollectEvmNetworkExecutor {
                capabilities: capabilities.clone(),
            },
        )),
    )?;
    registrations.register_state_runner_with_factory::<PublishEvmHoldingsState>(
        &managed_write_factory,
        Arc::new(PublishEvmHoldingsRunner {
            artifacts: artifacts.clone(),
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

struct SelectHoldingsExecutor {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl ExternalReadPlanExecutor<SelectHoldingsState> for SelectHoldingsExecutor {
    fn execute<'a>(
        &'a self,
        plan: &'a SelectHoldingsReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, SelectHoldingsReadEvidence> {
        Box::pin(async move {
            let requests = plan.requests().map_err(portfolio_state_runtime_error)?;
            let responses = if requests.is_empty() {
                Vec::new()
            } else {
                self.fact_index
                    .read_fact_index_batch(&requests)
                    .await
                    .map_err(fact_index_runtime_error)?
            };
            plan.validate_query_results(&responses)
                .map_err(portfolio_state_runtime_error)?;
            let hydrated =
                selection::hydrate_holding_responses(plan, &responses, self.artifacts.as_ref())
                    .await?;
            let queries = plan
                .query_evidence(&responses, &hydrated)
                .map_err(portfolio_state_runtime_error)?;
            let primary = SelectHoldingsReadEvidence::new(&queries, hydrated)
                .map_err(portfolio_state_runtime_error)?;
            Ok(ExternalReadExecution::new(primary, queries))
        })
    }
}

const EVM_READ_CONCURRENCY_LIMIT: usize = 16;
const ERC20_DECIMALS_SELECTOR: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];
const ERC20_BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];

struct CollectEvmNetworkExecutor {
    capabilities: PortfolioRunnerCapabilities,
}

impl ExternalReadPlanExecutor<CollectEvmNetworkState> for CollectEvmNetworkExecutor {
    fn validate_ingress(
        &self,
        _ctx: RunnerIngressContext<'_>,
        state: &CollectEvmNetworkState,
    ) -> mfm_runtime::Result<()> {
        let binding = state
            .config()
            .binding()
            .map_err(portfolio_evm_state_runtime_error)?;
        self.capabilities
            .validate_evm_binding(&binding)
            .map_err(evm_capability_runtime_error)
    }

    fn execute<'a>(
        &'a self,
        plan: &'a CollectEvmNetworkPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, CollectEvmNetworkEvidence> {
        Box::pin(async move {
            let evidence = collect_evm_network(plan, &self.capabilities).await?;
            Ok(ExternalReadExecution::primary(evidence))
        })
    }
}

async fn collect_evm_network(
    plan: &CollectEvmNetworkPlan,
    capabilities: &PortfolioRunnerCapabilities,
) -> mfm_runtime::Result<CollectEvmNetworkEvidence> {
    let binding = plan.binding().map_err(portfolio_evm_state_runtime_error)?;
    let session = capabilities
        .bind_evm_read_session(binding)
        .await
        .map_err(evm_capability_runtime_error)?;
    let latest = session
        .read_block(&EvmBlockSelector::Latest)
        .await
        .map_err(evm_capability_runtime_error)?;
    let anchor = EvmBlockAnchor::new(latest.number, latest.hash);
    let exact = EvmBlockSelector::ExactHash(latest.hash);

    let token_decimals = read_token_decimals(plan, Arc::clone(&session), exact.clone()).await?;
    let balances = read_balances(plan, Arc::clone(&session), exact).await?;
    let final_block = session
        .read_block(&EvmBlockSelector::Number(latest.number))
        .await
        .map_err(evm_capability_runtime_error)?;
    let final_canonical_block = EvmBlockAnchor::new(final_block.number, final_block.hash);
    Ok(CollectEvmNetworkEvidence::new(
        session.evidence(),
        anchor,
        token_decimals,
        balances,
        final_canonical_block,
    ))
}

async fn read_token_decimals(
    plan: &CollectEvmNetworkPlan,
    session: Arc<dyn EvmReadSession>,
    selector: EvmBlockSelector,
) -> mfm_runtime::Result<Vec<EvmTokenDecimalsEvidence>> {
    let contracts = plan.token_contracts();
    let mut evidence = Vec::with_capacity(contracts.len());
    for chunk in contracts.chunks(EVM_READ_CONCURRENCY_LIMIT) {
        let reads = chunk
            .iter()
            .map(|contract| {
                let contract = contract.clone();
                let session = Arc::clone(&session);
                let selector = selector.clone();
                async move {
                    let contract_address = contract.as_str().parse::<Address>().map_err(|_| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(
                            "certified ERC-20 contract address was invalid".to_owned(),
                        )
                    })?;
                    let call = EvmCall::new(
                        Address::ZERO,
                        contract_address,
                        U256::ZERO,
                        Bytes::copy_from_slice(&ERC20_DECIMALS_SELECTOR),
                        U256::from(100_000_u64),
                        Default::default(),
                        selector,
                    )
                    .map_err(evm_capability_runtime_error)?;
                    let result = session
                        .call(&call)
                        .await
                        .map_err(evm_capability_runtime_error)?;
                    let decimals = u8::try_from(decode_abi_word(&result)?).map_err(|_| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(
                            "ERC-20 decimals result exceeded u8".to_owned(),
                        )
                    })?;
                    EvmTokenDecimalsEvidence::new(contract, decimals)
                        .map_err(portfolio_evm_state_runtime_error)
                }
            })
            .collect::<Vec<_>>();
        evidence.extend(try_join_ordered(reads).await?);
    }
    evidence.sort();
    Ok(evidence)
}

async fn read_balances(
    plan: &CollectEvmNetworkPlan,
    session: Arc<dyn EvmReadSession>,
    selector: EvmBlockSelector,
) -> mfm_runtime::Result<Vec<EvmBalanceReadEvidence>> {
    let mut evidence = Vec::with_capacity(plan.sources().len());
    for chunk in plan.sources().chunks(EVM_READ_CONCURRENCY_LIMIT) {
        let reads = chunk
            .iter()
            .map(|source| {
                let source = source.clone();
                let session = Arc::clone(&session);
                let selector = selector.clone();
                async move {
                    let account = source
                        .account_address()
                        .map_err(portfolio_evm_state_runtime_error)?;
                    let raw_units = match source.asset() {
                        HoldingSourceConfig::Native => session
                            .read_balance(account, &selector)
                            .await
                            .map_err(evm_capability_runtime_error)?,
                        HoldingSourceConfig::Erc20 { contract_address } => {
                            let contract =
                                contract_address.as_str().parse::<Address>().map_err(|_| {
                                    mfm_runtime::RuntimeError::InvalidRunnerOutput(
                                        "certified ERC-20 contract address was invalid".to_owned(),
                                    )
                                })?;
                            let call = EvmCall::new(
                                Address::ZERO,
                                contract,
                                U256::ZERO,
                                erc20_balance_of_calldata(account),
                                U256::from(100_000_u64),
                                Default::default(),
                                selector,
                            )
                            .map_err(evm_capability_runtime_error)?;
                            let result = session
                                .call(&call)
                                .await
                                .map_err(evm_capability_runtime_error)?;
                            decode_abi_word(&result)?
                        }
                    };
                    Ok::<_, mfm_runtime::RuntimeError>(EvmBalanceReadEvidence::new(
                        source, raw_units,
                    ))
                }
            })
            .collect::<Vec<_>>();
        evidence.extend(try_join_ordered(reads).await?);
    }
    evidence.sort();
    Ok(evidence)
}

fn erc20_balance_of_calldata(account: Address) -> Bytes {
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&ERC20_BALANCE_OF_SELECTOR);
    calldata.extend_from_slice(&[0_u8; 12]);
    calldata.extend_from_slice(account.as_slice());
    Bytes::from(calldata)
}

fn decode_abi_word(bytes: &Bytes) -> mfm_runtime::Result<U256> {
    if bytes.len() != 32 {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "ERC-20 call result was not one ABI word".to_owned(),
        ));
    }
    Ok(U256::from_be_slice(bytes.as_ref()))
}

async fn try_join_ordered<T, F>(reads: Vec<F>) -> mfm_runtime::Result<Vec<T>>
where
    F: Future<Output = mfm_runtime::Result<T>>,
{
    let mut reads = reads
        .into_iter()
        .map(|read| Some(Box::pin(read)))
        .collect::<Vec<_>>();
    let mut outputs = (0..reads.len()).map(|_| None).collect::<Vec<_>>();
    std::future::poll_fn(|context| {
        for (read, output) in reads.iter_mut().zip(outputs.iter_mut()) {
            let Some(future) = read.as_mut() else {
                continue;
            };
            match future.as_mut().poll(context) {
                Poll::Ready(Ok(value)) => {
                    *output = Some(value);
                    *read = None;
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => {}
            }
        }
        if reads.iter().any(Option::is_some) {
            return Poll::Pending;
        }
        let mut ordered = Vec::with_capacity(outputs.len());
        for output in std::mem::take(&mut outputs) {
            let Some(output) = output else {
                return Poll::Ready(Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "ordered EVM read completed without output".to_owned(),
                )));
            };
            ordered.push(output);
        }
        Poll::Ready(Ok(ordered))
    })
    .await
}

struct PublishEvmHoldingsRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for PublishEvmHoldingsRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<EvmNetworkCollectionConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let state = PublishEvmHoldingsState::new(config).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            let input = load_materialized_struct_input::<PublishEvmHoldingsInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let context = ctx.certified_context::<mfm_program::NoContext>()?;
            let snapshot = state
                .run(input, &(FactRecordCapability,), &context)
                .await
                .map_err(|error| {
                    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                })?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            let producer = portfolio_fact_record_binding()?;
            for fact in snapshot.facts() {
                output.record_fact(
                    FactRecordInput::new(fact, mfm_state_portfolio::evm_balance_fact_visibility()),
                    producer.clone(),
                )?;
            }
            output.state_output(&snapshot)?;
            Ok(output.finish())
        })
    }
}

fn portfolio_fact_record_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    RunnerCapabilityBinding::for_capability::<FactRecordCapability>(
        portfolio_adapter_kind()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?,
        portfolio_adapter_version()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?,
    )
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

fn fact_index_runtime_error(
    error: mfm_fact_capabilities::FactIndexReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn evm_capability_runtime_error(error: EvmCapabilityError) -> mfm_runtime::RuntimeError {
    let Some(diagnostic) = error.redacted_diagnostic().cloned() else {
        return mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM capability request failed without provider diagnostics".to_owned(),
        );
    };
    let (code, message) = match diagnostic.code() {
        ProviderDiagnosticCode::ProviderConfigurationMissing
        | ProviderDiagnosticCode::RouteUnavailable => (
            "RuntimeConfigRequired",
            "EVM runtime configuration is required",
        ),
        ProviderDiagnosticCode::ProviderConfigurationInvalid => (
            "RuntimeConfigInvalid",
            "EVM runtime configuration is invalid",
        ),
        _ => ("EvmProviderFailure", "EVM provider capability failed"),
    };
    let failure = mfm_runtime::RuntimeFailure::new(
        events::ErrorCode::new(code).expect("EVM runtime failure code is checked public text"),
        events::ErrorCategory::Capability,
        message,
        vec![diagnostic],
    )
    .expect("EVM runtime failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::Failure(failure)
}

fn portfolio_evm_state_runtime_error(
    error: mfm_state_portfolio::PortfolioEvmError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn portfolio_state_runtime_error(
    error: mfm_state_portfolio::PortfolioHoldingSelectionError,
) -> mfm_runtime::RuntimeError {
    let code = error.code.as_str();
    let failure = mfm_runtime::RuntimeFailure::new(
        events::ErrorCode::new(code).expect("portfolio error code is a checked public code"),
        events::ErrorCategory::Validation,
        format!("{code}: portfolio holding selection failed"),
        Vec::new(),
    )
    .expect("portfolio failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::Failure(failure)
}

#[cfg(test)]
mod tests;
