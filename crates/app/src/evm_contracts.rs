use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use mfm_adapter_contracts::evm_contract_lifecycle_adapter_binding;
use mfm_adapters_evm_contracts::{
    deploy_contract_address_from_prepared, ensure_prepared_invocation_public,
    lifecycle_evidence_ref, replay_verifier_id, EvmContractAdapterError,
    EvmContractLifecycleAdapter, EvmContractMutationProviders, EvmContractReadProviders,
    EvmContractRuntimeRoute, PreparedContractInvocation, PreparedContractMutation,
};
use mfm_artifact_capabilities::{ArtifactReadProvider, ArtifactReadRequest};
use mfm_artifact_store_fs::FsTypedArtifactStore;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::CapabilitySpec;
use mfm_events::v1::{self as events, side_effect};
use mfm_evm_capabilities::{
    EvmCallReadCapability, EvmCallReadProvider, EvmCapabilityError, EvmChainIdentityCapability,
    EvmChainIdentityProvider, EvmFeeReadProvider, EvmGasEstimateProvider, EvmLogsReadCapability,
    EvmLogsReadProvider, EvmNonceReadProvider, EvmReceiptReadProvider, EvmSourcePolicyId,
    EvmSourceRef, EvmTransactionSubmitCapability, EvmTransactionSubmitProvider,
};
use mfm_evm_contract_config::{
    ConfigurePhaseConfig, DeployPhaseConfig, ReceiptRetryPolicy, ValidatePhaseConfig,
};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
use mfm_ids::{ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest, DescriptorId, NodeId};
use mfm_program::{SideEffectState, StateSpec};
use mfm_runtime::{
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput,
    ErasedRunnerRegistry, MaterializedCell, MaterializedCellTerminal, MaterializedInputNode,
    RunnerEventPayload, StagedArtifact, StagedRetentionRefs,
};
use mfm_signers_keystore::{KeystoreSignerProvider, KeystoreSignerRegistryEntry};
use mfm_signing::{SignerRef, SigningProvider};
use mfm_spec::v1 as spec;
use mfm_state_evm_contracts::{
    ConfigureContractInput, ConfigureContractState, ContractConfigureConfirmation,
    ContractConfigureIntent, ContractDeployConfirmation, ContractDeployIntent,
    ContractTransactionIdempotency, ContractTransactionReceipts, ContractTransactionSubmissions,
    DeployContractState, ValidateContractInput, ValidateContractState,
};
use mfm_store::v1 as store;
use mfm_transports_evm::EvmJsonRpcClient;
use mfm_values::{MfmConfig, MfmValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use uuid::Uuid;

const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const ENV_CONTRACT_SOURCE_REF: &str = "MFM_EVM_CONTRACT_SOURCE_REF";
const ENV_CONTRACT_SOURCE_POLICY_ID: &str = "MFM_EVM_CONTRACT_SOURCE_POLICY_ID";
const ENV_EVM_SIGNERS_JSON: &str = "MFM_EVM_SIGNERS_JSON";

trait EvmContractProvider:
    EvmChainIdentityProvider
    + EvmNonceReadProvider
    + EvmFeeReadProvider
    + EvmGasEstimateProvider
    + EvmTransactionSubmitProvider
    + EvmReceiptReadProvider
    + EvmCallReadProvider
    + EvmLogsReadProvider
{
}

impl<T> EvmContractProvider for T where
    T: EvmChainIdentityProvider
        + EvmNonceReadProvider
        + EvmFeeReadProvider
        + EvmGasEstimateProvider
        + EvmTransactionSubmitProvider
        + EvmReceiptReadProvider
        + EvmCallReadProvider
        + EvmLogsReadProvider
{
}

trait EvmContractRuntimeFactory: Send + Sync {
    fn artifacts(&self) -> &dyn ArtifactReadProvider;

    fn runtime_for(&self, network_id: &str) -> mfm_runtime::Result<EvmContractRuntime>;
}

#[derive(Clone)]
struct EvmContractRuntime {
    route: EvmContractRuntimeRoute,
    evm: Arc<dyn EvmContractProvider>,
    signer: Arc<dyn SigningProvider>,
}

impl fmt::Debug for EvmContractRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmContractRuntime")
            .field("route", &self.route)
            .finish_non_exhaustive()
    }
}

impl EvmContractRuntime {
    fn adapter(&self) -> EvmContractLifecycleAdapter<'_> {
        let evm = self.evm.as_ref();
        EvmContractLifecycleAdapter::new(
            &self.route,
            EvmContractMutationProviders {
                chain_identity: evm,
                nonce: evm,
                fee: evm,
                gas: evm,
                signer: self.signer.as_ref(),
                submit: evm,
                receipt: evm,
            },
            EvmContractReadProviders {
                chain_identity: evm,
                call: evm,
                logs: evm,
            },
        )
    }
}

#[derive(Clone)]
struct EnvEvmContractRuntimeFactory {
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl EnvEvmContractRuntimeFactory {
    fn new(artifacts: FsTypedArtifactStore) -> Self {
        Self {
            artifacts: Arc::new(artifacts),
        }
    }
}

impl EvmContractRuntimeFactory for EnvEvmContractRuntimeFactory {
    fn artifacts(&self) -> &dyn ArtifactReadProvider {
        self.artifacts.as_ref()
    }

    fn runtime_for(&self, network_id: &str) -> mfm_runtime::Result<EvmContractRuntime> {
        let route = EvmContractRuntimeRoute::new(
            EvmSourceRef::new(
                std::env::var(ENV_CONTRACT_SOURCE_REF).unwrap_or_else(|_| network_id.to_owned()),
            )
            .map_err(runtime_evm_capability_error)?,
            EvmSourcePolicyId::new(
                std::env::var(ENV_CONTRACT_SOURCE_POLICY_ID)
                    .unwrap_or_else(|_| network_id.to_owned()),
            )
            .map_err(runtime_evm_capability_error)?,
        );
        let evm = Arc::new(EvmJsonRpcClient::from_env().map_err(runtime_evm_transport_error)?);
        let signer = Arc::new(keystore_signer_provider_from_env()?);
        Ok(EvmContractRuntime { route, evm, signer })
    }
}

#[derive(Debug, Deserialize)]
struct RuntimeSignerConfig {
    signer_ref: String,
    entry_id: Uuid,
    keystore_env: String,
    unlock_file_env: String,
}

fn keystore_signer_provider_from_env() -> mfm_runtime::Result<KeystoreSignerProvider> {
    let raw = std::env::var(ENV_EVM_SIGNERS_JSON).unwrap_or_else(|_| "[]".to_owned());
    let entries = serde_json::from_str::<Vec<RuntimeSignerConfig>>(&raw)
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?
        .into_iter()
        .map(|entry| {
            let signer_ref = SignerRef::new(entry.signer_ref).map_err(runtime_signing_error)?;
            KeystoreSignerRegistryEntry::from_env_sources(
                signer_ref,
                entry.entry_id,
                entry.keystore_env,
                entry.unlock_file_env,
            )
            .map_err(runtime_signing_error)
        })
        .collect::<mfm_runtime::Result<Vec<_>>>()?;
    Ok(KeystoreSignerProvider::new(entries))
}

/// Registers contract lifecycle runners in the process production runner registry.
pub(crate) fn register_contract_lifecycle_runners(
    registry: &mut ErasedRunnerRegistry,
    artifacts: FsTypedArtifactStore,
) -> mfm_runtime::Result<()> {
    register_contract_lifecycle_runners_with_factory(
        registry,
        Arc::new(EnvEvmContractRuntimeFactory::new(artifacts)),
    )
}

fn register_contract_lifecycle_runners_with_factory(
    registry: &mut ErasedRunnerRegistry,
    factory: Arc<dyn EvmContractRuntimeFactory>,
) -> mfm_runtime::Result<()> {
    registry.register(binding(
        registered_descriptor::<DeployContractState>()?,
        SIDE_EFFECT_FACTORY,
        Arc::new(ContractMutationRunner {
            phase: ContractMutationRunnerPhase::Deploy,
            factory: factory.clone(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<ConfigureContractState>()?,
        SIDE_EFFECT_FACTORY,
        Arc::new(ContractMutationRunner {
            phase: ContractMutationRunnerPhase::Configure,
            factory: factory.clone(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<ValidateContractState>()?,
        READ_FACTORY,
        Arc::new(ContractValidateRunner { factory }),
    )?)?;
    Ok(())
}

fn registered_descriptor<S>() -> mfm_runtime::Result<DescriptorId>
where
    S: StateSpec,
    S::Effect: mfm_program::EffectRunner<S>,
    S::Caps: mfm_capabilities::CapabilitySetFor<S::Effect>,
{
    let mut states = mfm_program::StateRegistryBuilder::new();
    let registered = states
        .register::<S>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    Ok(registered.descriptor().descriptor_id().clone())
}

fn binding(
    descriptor_id: DescriptorId,
    factory: &'static str,
    runner: Arc<dyn ErasedNodeRunner>,
) -> mfm_runtime::Result<ErasedRunnerBinding> {
    let factory_id = events::RunnerFactoryId::new(factory)?;
    ErasedRunnerBinding::new(
        descriptor_id,
        factory_id.clone(),
        executable(factory_id)?,
        runner,
    )
}

fn executable(
    factory_id: events::RunnerFactoryId,
) -> mfm_runtime::Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-app-evm-contract-lifecycle")?,
        cargo_package_name: events::PackageName::new("mfm-app")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: digest_json(serde_json::json!({
            "crate": "mfm-app",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        binary_digest: digest_json(serde_json::json!({
            "crate": "mfm-app",
            "runner": "evm-contract-lifecycle",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct ContractValidateRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
}

impl ErasedNodeRunner for ContractValidateRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_validate(ctx, self.factory.as_ref()).await })
    }
}

#[derive(Clone, Copy)]
enum ContractMutationRunnerPhase {
    Deploy,
    Configure,
}

struct ContractMutationRunner {
    phase: ContractMutationRunnerPhase,
    factory: Arc<dyn EvmContractRuntimeFactory>,
}

impl ErasedNodeRunner for ContractMutationRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_mutation(ctx, self.factory.as_ref(), self.phase).await })
    }
}

async fn run_mutation(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
    phase: ContractMutationRunnerPhase,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    match phase {
        ContractMutationRunnerPhase::Deploy => run_deploy_mutation(ctx, factory).await,
        ContractMutationRunnerPhase::Configure => run_configure_mutation(ctx, factory).await,
    }
}

struct DeployMutationPlan {
    config: DeployPhaseConfig,
    state: DeployContractState,
    intent: ContractDeployIntent,
    idempotency: ContractTransactionIdempotency,
    ledger_key: events::SideEffectLedgerKey,
}

struct ConfigureMutationPlan {
    config: ConfigurePhaseConfig,
    state: ConfigureContractState,
    input: ConfigureContractInput,
    intent: ContractConfigureIntent,
    idempotency: ContractTransactionIdempotency,
    ledger_key: events::SideEffectLedgerKey,
}

async fn run_deploy_mutation(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let plan = deploy_mutation_plan(&ctx, factory.artifacts()).await?;
    match ctx
        .projections()
        .side_effect(&plan.ledger_key)
        .map(|projection| projection.phase.clone())
    {
        None => {
            let runtime = factory.runtime_for(plan.config.network().network_id())?;
            let prepared = runtime
                .adapter()
                .prepare_deploy_invocation(&plan.config, &plan.intent)
                .await
                .map_err(runtime_adapter_error)?;
            side_effect_prepare(
                ctx,
                plan.ledger_key,
                &plan.intent,
                &plan.idempotency,
                prepared.evidence(),
            )
        }
        Some(store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        })
        | Some(store::SideEffectPhase::SubmissionUnknown { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let stored_prepared = load_prepared_invocation(
                &prepared_projection,
                events::ArtifactRole::PreparedInvocation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let runtime = factory.runtime_for(plan.config.network().network_id())?;
            let prepared = runtime
                .adapter()
                .reconstruct_deploy_invocation(&plan.config, &plan.intent, &stored_prepared)
                .map_err(runtime_adapter_error)?;
            side_effect_submission(ctx, &runtime, plan.ledger_key, invocation_epoch, &prepared)
                .await
        }
        Some(store::SideEffectPhase::SubmissionObserved { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let submission_projection = projected_submission_artifact(&ctx, &plan.ledger_key)?;
            let runtime = factory.runtime_for(plan.config.network().network_id())?;
            side_effect_receipt(
                ctx,
                factory.artifacts(),
                &runtime,
                plan.ledger_key,
                invocation_epoch,
                &prepared_projection,
                &submission_projection,
            )
            .await
        }
        Some(store::SideEffectPhase::ReceiptObserved { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let receipt_projection = projected_receipt_artifact(&ctx, &plan.ledger_key)?;
            let prepared = load_prepared_invocation(
                &prepared_projection,
                events::ArtifactRole::PreparedInvocation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let (receipts, receipt_evidence) =
                load_side_effect_artifact::<ContractTransactionReceipts>(
                    &receipt_projection,
                    events::ArtifactRole::Receipt,
                    &ctx,
                    factory.artifacts(),
                )
                .await?;
            let receipts = receipts_with_evidence(receipts, &receipt_evidence);
            let receipt = single_receipt(receipts)?;
            let confirmation = ContractDeployConfirmation {
                confirmation_version: 1,
                contract_address: deploy_contract_address_from_prepared(&prepared)
                    .map_err(runtime_adapter_error)?,
                receipt,
            };
            side_effect_confirmation(ctx, plan.ledger_key, invocation_epoch, &confirmation)
        }
        Some(store::SideEffectPhase::ConfirmationObserved { .. }) => {
            let confirmation_projection = projected_confirmation_artifact(&ctx, &plan.ledger_key)?;
            let confirmation = load_side_effect_value::<ContractDeployConfirmation>(
                &confirmation_projection,
                events::ArtifactRole::Confirmation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let output = plan
                .state
                .output_from_confirmation(&(), &plan.intent, &confirmation)
                .map_err(runtime_state_error)?;
            mutation_output(ctx, &output)
        }
        Some(store::SideEffectPhase::Ambiguous { .. }) => Ok(ErasedRunnerOutput::new(Vec::new())),
        Some(other) => unsupported_side_effect_phase("deploy", other),
    }
}

async fn run_configure_mutation(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let plan = configure_mutation_plan(&ctx, factory.artifacts()).await?;
    match ctx
        .projections()
        .side_effect(&plan.ledger_key)
        .map(|projection| projection.phase.clone())
    {
        None => {
            let runtime = factory.runtime_for(plan.config.network().network_id())?;
            let prepared = runtime
                .adapter()
                .prepare_configure_invocation(&plan.config, &plan.input, &plan.intent)
                .await
                .map_err(runtime_adapter_error)?;
            side_effect_prepare(
                ctx,
                plan.ledger_key,
                &plan.intent,
                &plan.idempotency,
                prepared.evidence(),
            )
        }
        Some(store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        })
        | Some(store::SideEffectPhase::SubmissionUnknown { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let stored_prepared = load_prepared_invocation(
                &prepared_projection,
                events::ArtifactRole::PreparedInvocation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let runtime = factory.runtime_for(plan.config.network().network_id())?;
            let prepared = runtime
                .adapter()
                .reconstruct_configure_invocation(
                    &plan.config,
                    &plan.input,
                    &plan.intent,
                    &stored_prepared,
                )
                .map_err(runtime_adapter_error)?;
            side_effect_submission(ctx, &runtime, plan.ledger_key, invocation_epoch, &prepared)
                .await
        }
        Some(store::SideEffectPhase::SubmissionObserved { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let submission_projection = projected_submission_artifact(&ctx, &plan.ledger_key)?;
            let runtime = factory.runtime_for(plan.config.network().network_id())?;
            side_effect_receipt(
                ctx,
                factory.artifacts(),
                &runtime,
                plan.ledger_key,
                invocation_epoch,
                &prepared_projection,
                &submission_projection,
            )
            .await
        }
        Some(store::SideEffectPhase::ReceiptObserved { invocation_epoch }) => {
            let receipt_projection = projected_receipt_artifact(&ctx, &plan.ledger_key)?;
            let (receipts, receipt_evidence) =
                load_side_effect_artifact::<ContractTransactionReceipts>(
                    &receipt_projection,
                    events::ArtifactRole::Receipt,
                    &ctx,
                    factory.artifacts(),
                )
                .await?;
            let receipts = receipts_with_evidence(receipts, &receipt_evidence);
            let confirmation = ContractConfigureConfirmation {
                confirmation_version: 1,
                configured_block_number: receipts
                    .transactions
                    .iter()
                    .map(|receipt| receipt.block_number)
                    .max(),
                receipts: receipts.transactions,
            };
            side_effect_confirmation(ctx, plan.ledger_key, invocation_epoch, &confirmation)
        }
        Some(store::SideEffectPhase::ConfirmationObserved { .. }) => {
            let confirmation_projection = projected_confirmation_artifact(&ctx, &plan.ledger_key)?;
            let confirmation = load_side_effect_value::<ContractConfigureConfirmation>(
                &confirmation_projection,
                events::ArtifactRole::Confirmation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let output = plan
                .state
                .output_from_confirmation(&plan.input, &plan.intent, &confirmation)
                .map_err(runtime_state_error)?;
            mutation_output(ctx, &output)
        }
        Some(store::SideEffectPhase::Ambiguous { .. }) => Ok(ErasedRunnerOutput::new(Vec::new())),
        Some(other) => unsupported_side_effect_phase("configure", other),
    }
}

async fn deploy_mutation_plan(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<DeployMutationPlan> {
    let config = load_config::<DeployPhaseConfig>(ctx, artifacts).await?;
    let state =
        DeployContractState::new(validated_config(config.clone())?).map_err(runtime_plan_error)?;
    let intent = state.prepare_intent(&()).map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&(), &intent)
        .map_err(runtime_state_error)?;
    let ledger_key = ledger_key_for_idempotency(&idempotency)?;
    Ok(DeployMutationPlan {
        config,
        state,
        intent,
        idempotency,
        ledger_key,
    })
}

async fn configure_mutation_plan(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ConfigureMutationPlan> {
    let config = load_config::<ConfigurePhaseConfig>(ctx, artifacts).await?;
    let deployed =
        load_struct_input_value::<DeployedContract>(ctx.inputs(), "deployed", artifacts).await?;
    let input = ConfigureContractInput { deployed };
    let state = ConfigureContractState::new(validated_config(config.clone())?)
        .map_err(runtime_plan_error)?;
    let intent = state.prepare_intent(&input).map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&input, &intent)
        .map_err(runtime_state_error)?;
    let ledger_key = ledger_key_for_idempotency(&idempotency)?;
    Ok(ConfigureMutationPlan {
        config,
        state,
        input,
        intent,
        idempotency,
        ledger_key,
    })
}

async fn run_validate(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_config::<ValidatePhaseConfig>(&ctx, factory.artifacts()).await?;
    let input = load_validate_input(ctx.inputs(), factory.artifacts()).await?;
    let state = ValidateContractState::new(validated_config(config.clone())?)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let request = state
        .read_request(&input)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let runtime = factory.runtime_for(config.network().network_id())?;
    let response = runtime
        .adapter()
        .validate_contract(&config, &input, &request)
        .await
        .map_err(runtime_adapter_error)?;
    let report = state
        .report_from_response(&input, response.clone())
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let fact_capabilities = validation_fact_capabilities(&response)?;
    read_output(ctx, request, response, report, fact_capabilities).await
}

struct CapabilityFact {
    key_suffix: &'static str,
    kind: CapabilityKind,
    version: CapabilityVersion,
}

fn validation_fact_capabilities(
    response: &mfm_state_evm_contracts::ContractValidationReadResponse,
) -> mfm_runtime::Result<Vec<CapabilityFact>> {
    let mut facts = vec![CapabilityFact {
        key_suffix: "chain_identity",
        kind: EvmChainIdentityCapability::kind().map_err(runtime_capability_error)?,
        version: EvmChainIdentityCapability::version().map_err(runtime_capability_error)?,
    }];
    if !response.configuration_read_results.is_empty() || !response.read_results.is_empty() {
        facts.push(CapabilityFact {
            key_suffix: "call",
            kind: EvmCallReadCapability::kind().map_err(runtime_capability_error)?,
            version: EvmCallReadCapability::version().map_err(runtime_capability_error)?,
        });
    }
    if !response.configuration_event_results.is_empty() || !response.event_results.is_empty() {
        facts.push(CapabilityFact {
            key_suffix: "logs",
            kind: EvmLogsReadCapability::kind().map_err(runtime_capability_error)?,
            version: EvmLogsReadCapability::version().map_err(runtime_capability_error)?,
        });
    }
    Ok(facts)
}

fn side_effect_prepare<Intent>(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    intent: &Intent,
    idempotency: &ContractTransactionIdempotency,
    prepared: &mfm_adapters_evm_contracts::PreparedContractInvocation,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Intent: MfmValue + Serialize,
{
    let intent_artifact = artifact_for_value(
        intent,
        events::ArtifactRole::SideEffectIntent,
        Some(ctx.node().node_id.clone()),
    )?;
    let prepared_artifact = artifact_for_schema_less_json(
        prepared,
        events::ArtifactRole::PreparedInvocation,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_intent = staged_side_effect_artifact(&ctx, &intent_artifact, ledger_key.clone(), 1)?;
    let staged_prepared =
        staged_side_effect_artifact(&ctx, &prepared_artifact, ledger_key.clone(), 1)?;
    let idempotency_hash = digest_value(idempotency)?;
    let owner = runner_invocation_id(&ctx, &ledger_key)?;
    let token = claim_fencing_token(&ctx, &ledger_key)?;
    let ledger_purpose = events::SideEffectLedgerPurpose::Forward;
    let binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_intent, staged_prepared],
        staged_retention_refs: vec![
            retention(&intent_artifact.evidence),
            retention(&prepared_artifact.evidence),
        ],
        payloads: vec![
            RunnerEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                scope_id: ctx.node().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                invocation_epoch: 1,
                intent_schema_id: Intent::schema_id().map_err(runtime_value_error)?,
                intent_hash: intent_artifact.evidence.digest.clone(),
                intent_artifact_id: intent_artifact.evidence.artifact_id.clone(),
                idempotency_input_schema_id: ContractTransactionIdempotency::schema_id()
                    .map_err(runtime_value_error)?,
                idempotency_input_hash: idempotency_hash,
                idempotency_key: idempotency_key_ref(idempotency)?,
                capability_kind: EvmTransactionSubmitCapability::kind()
                    .map_err(runtime_capability_error)?,
                capability_version: EvmTransactionSubmitCapability::version()
                    .map_err(runtime_capability_error)?,
                adapter_kind: binding.adapter_kind().clone(),
                adapter_version: binding.adapter_version().clone(),
            }),
            RunnerEventPayload::SideEffectClaimed(side_effect::Claimed {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                claim_owner: owner.clone(),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
            }),
            RunnerEventPayload::SideEffectInvocationPrepared(side_effect::InvocationPrepared {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
                prepared_artifact_id: Some(prepared_artifact.evidence.artifact_id.clone()),
                prepared_hash: Some(prepared_artifact.evidence.digest.clone()),
                resource_key: None,
            }),
            RunnerEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
                ledger_purpose,
                invocation_epoch: 1,
                claim_owner: owner,
                claim_generation: 1,
                claim_fencing_token: token,
            }),
        ],
    })
}

async fn side_effect_submission(
    ctx: ErasedRunCtx<'_>,
    runtime: &EvmContractRuntime,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    prepared: &PreparedContractMutation,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let submissions = ContractTransactionSubmissions {
        submissions_version: 1,
        transactions: runtime
            .adapter()
            .submit_prepared(prepared)
            .await
            .map_err(runtime_adapter_error)?,
    };
    let artifact = artifact_for_value(
        &submissions,
        events::ArtifactRole::Submission,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                invocation_epoch,
                submission_schema_id: ContractTransactionSubmissions::schema_id()
                    .map_err(runtime_value_error)?,
                submission_hash: artifact.evidence.digest,
                submission_artifact_id: artifact.evidence.artifact_id,
            },
        )],
    })
}

async fn side_effect_receipt(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
    runtime: &EvmContractRuntime,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    prepared_projection: &store::SideEffectArtifactProjection,
    submission_projection: &store::SideEffectArtifactProjection,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let prepared = load_prepared_invocation(
        prepared_projection,
        events::ArtifactRole::PreparedInvocation,
        &ctx,
        artifacts,
    )
    .await?;
    let submissions = load_side_effect_value::<ContractTransactionSubmissions>(
        submission_projection,
        events::ArtifactRole::Submission,
        &ctx,
        artifacts,
    )
    .await?;
    let receipts = ContractTransactionReceipts {
        receipts_version: 1,
        transactions: read_receipts_with_poll(runtime, &prepared, &submissions).await?,
    };
    let artifact = artifact_for_value(
        &receipts,
        events::ArtifactRole::Receipt,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectReceiptObserved(
            side_effect::ReceiptObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                invocation_epoch,
                receipt_schema_id: ContractTransactionReceipts::schema_id()
                    .map_err(runtime_value_error)?,
                receipt_hash: artifact.evidence.digest,
                receipt_artifact_id: artifact.evidence.artifact_id,
                replay_verifier_id: replay_verifier_id().map_err(runtime_adapter_error)?,
                resource_touched_set: None,
            },
        )],
    })
}

async fn read_receipts_with_poll(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractInvocation,
    submissions: &ContractTransactionSubmissions,
) -> mfm_runtime::Result<Vec<mfm_state_evm_contracts::ContractTransactionReceipt>> {
    if prepared.transactions.len() != submissions.transactions.len() {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract lifecycle receipt read transaction count mismatch".to_owned(),
        ));
    }
    let receipt_policy =
        ReceiptRetryPolicy::new(prepared.poll_interval_ms, prepared.max_receipt_polls)
            .map_err(mfm_runtime::RuntimeError::InvalidRunnerOutput)?;

    let max_polls = receipt_policy.max_receipt_polls();
    for attempt in 0..max_polls {
        match runtime
            .adapter()
            .read_receipts(&submissions.transactions)
            .await
        {
            Ok(receipts) => return Ok(receipts),
            Err(EvmContractAdapterError::EvmCapability(EvmCapabilityError::ReceiptPending)) => {
                if attempt + 1 == max_polls {
                    return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                        "contract lifecycle receipt polling exhausted".to_owned(),
                    ));
                }
                sleep(Duration::from_millis(receipt_policy.poll_interval_ms())).await;
            }
            Err(error) => return Err(runtime_adapter_error(error)),
        }
    }

    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
        "contract lifecycle receipt polling exhausted".to_owned(),
    ))
}

fn side_effect_confirmation<Confirmation>(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    confirmation: &Confirmation,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Confirmation: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        confirmation,
        events::ArtifactRole::Confirmation,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectConfirmationObserved(
            side_effect::ConfirmationObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                invocation_epoch,
                confirmation_schema_id: Confirmation::schema_id().map_err(runtime_value_error)?,
                confirmation_hash: artifact.evidence.digest,
                confirmation_artifact_id: artifact.evidence.artifact_id,
                replay_verifier_id: replay_verifier_id().map_err(runtime_adapter_error)?,
                resource_touched_set: None,
            },
        )],
    })
}

fn mutation_output<Output>(
    ctx: ErasedRunCtx<'_>,
    output: &Output,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Output: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        output,
        events::ArtifactRole::StateOutput,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged = staged_attempt_artifact(&ctx, &artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![cell_produced(&ctx, &artifact.evidence)],
    })
}

async fn load_validate_input(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ValidateContractInput> {
    let MaterializedInputNode::Struct(fields) = &inputs.root else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract validate input was not materialized as a struct".to_owned(),
        ));
    };
    let configured = fields
        .iter()
        .find(|field| field.field_path.as_str() == "configured")
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "contract validate input missing configured field".to_owned(),
            )
        })?;
    let configured =
        load_value_from_node::<ConfiguredContract>(&configured.node, artifacts).await?;
    Ok(ValidateContractInput { configured })
}

async fn load_struct_input_value<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    field_path: &str,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::Struct(fields) = &inputs.root else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract lifecycle input was not materialized as a struct".to_owned(),
        ));
    };
    let field = fields
        .iter()
        .find(|field| field.field_path.as_str() == field_path)
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "contract lifecycle input missing {field_path} field"
            ))
        })?;
    load_value_from_node::<T>(&field.node, artifacts).await
}

async fn load_config<T>(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let request = ArtifactReadRequest::from_certified_config_ref(&ctx.node().config_ref);
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    verified.decode_json().map_err(runtime_artifact_read_error)
}

async fn load_prepared_invocation(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<mfm_adapters_evm_contracts::PreparedContractInvocation> {
    let prepared =
        load_side_effect_value::<mfm_adapters_evm_contracts::PreparedContractInvocation>(
            artifact, role, ctx, artifacts,
        )
        .await?;
    ensure_prepared_invocation_public(&prepared).map_err(runtime_adapter_error)?;
    Ok(prepared)
}

async fn load_side_effect_value<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let (value, _) = load_side_effect_artifact(artifact, role, ctx, artifacts).await?;
    Ok(value)
}

async fn load_side_effect_artifact<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<(T, mfm_artifact_capabilities::ArtifactEvidenceRef)>
where
    T: MfmValue + DeserializeOwned,
{
    let request = ArtifactReadRequest::from_side_effect_projection(
        artifact,
        role,
        ctx.node().node_id.clone(),
    );
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    let evidence = verified.evidence().clone();
    let value = verified
        .decode_json()
        .map_err(runtime_artifact_read_error)?;
    Ok((value, evidence))
}

async fn read_output<Request, Response, Output>(
    ctx: ErasedRunCtx<'_>,
    request: Request,
    response: Response,
    output: Output,
    fact_capabilities: Vec<CapabilityFact>,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Request: MfmValue + Serialize,
    Response: MfmValue + Serialize,
    Output: MfmValue + Serialize,
{
    let request_hash = digest_value(&request)?;
    let response_artifact = artifact_for_value(
        &response,
        events::ArtifactRole::FactResponse,
        Some(ctx.node().node_id.clone()),
    )?;
    let output_artifact = artifact_for_value(
        &output,
        events::ArtifactRole::StateOutput,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_response = staged_attempt_artifact(&ctx, &response_artifact)?;
    let staged_output = staged_attempt_artifact(&ctx, &output_artifact)?;
    let binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let mut payloads = Vec::with_capacity(fact_capabilities.len() + 1);
    for fact in fact_capabilities {
        payloads.push(RunnerEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            capability_kind: fact.kind,
            capability_version: fact.version,
            adapter_kind: binding.adapter_kind().clone(),
            adapter_version: binding.adapter_version().clone(),
            request_schema_id: Request::schema_id().map_err(runtime_value_error)?,
            request_hash: request_hash.clone(),
            response_schema_id: Response::schema_id().map_err(runtime_value_error)?,
            response_hash: response_artifact.evidence.digest.clone(),
            fact_key: events::FactKey::new(format!(
                "mfm.evm.contract.fact.{}.{}",
                ctx.node().node_id.as_str(),
                fact.key_suffix
            ))?,
            artifact_id: response_artifact.evidence.artifact_id.clone(),
        }));
    }
    payloads.push(cell_produced(&ctx, &output_artifact.evidence));
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_response, staged_output],
        staged_retention_refs: vec![
            retention(&response_artifact.evidence),
            retention(&output_artifact.evidence),
        ],
        payloads,
    })
}

async fn load_value_from_node<T>(
    node: &MaterializedInputNode,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::Cell(cell) = node else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract lifecycle input was not a materialized cell".to_owned(),
        ));
    };
    load_value_from_cell(cell, artifacts).await
}

async fn load_value_from_cell<T>(
    cell: &MaterializedCell,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let request = match &cell.terminal {
        MaterializedCellTerminal::Seed {
            seed_id,
            artifact_id,
            content_digest,
        } => ArtifactReadRequest::from_materialized_seed_cell(
            artifact_id.clone(),
            content_digest.clone(),
            cell.schema_id.clone(),
            cell.semantic_type_id.clone(),
            seed_id.clone(),
        ),
        MaterializedCellTerminal::Produced {
            artifact_id,
            content_digest,
        } => ArtifactReadRequest::from_materialized_produced_cell(
            artifact_id.clone(),
            content_digest.clone(),
            cell.schema_id.clone(),
            cell.semantic_type_id.clone(),
        ),
        MaterializedCellTerminal::Skipped { .. } => {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "contract lifecycle input cell was skipped".to_owned(),
            ));
        }
    };
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    verified.decode_json().map_err(runtime_artifact_read_error)
}

fn staged_attempt_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &ContractAppArtifact,
) -> mfm_runtime::Result<StagedArtifact> {
    StagedArtifact::inline_attempt_artifact(ctx, artifact.bytes.clone(), artifact.evidence.clone())
}

fn staged_side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &ContractAppArtifact,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> mfm_runtime::Result<StagedArtifact> {
    StagedArtifact::inline_side_effect_artifact(
        ctx,
        artifact.bytes.clone(),
        artifact.evidence.clone(),
        ledger_key,
        invocation_epoch,
    )
}

fn cell_produced(
    ctx: &ErasedRunCtx<'_>,
    artifact: &store::ArtifactEvidenceRef,
) -> RunnerEventPayload {
    RunnerEventPayload::CellProduced(events::CellProduced {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        cell_id: ctx.node().output_cell.clone(),
        scope_id: ctx.output_cell().scope_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
        schema_id: ctx.output_cell().schema_id.clone(),
        value_lineage: ctx.output_cell().value_lineage.clone(),
        artifact_id: artifact.artifact_id.clone(),
        content_digest: artifact.digest.clone(),
        producer_state_kind: Some(ctx.node().state_kind.clone()),
        producer_state_version: Some(ctx.node().state_version.clone()),
    })
}

struct ContractAppArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn artifact_for_value<T>(
    value: &T,
    role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
) -> mfm_runtime::Result<ContractAppArtifact>
where
    T: MfmValue + Serialize,
{
    let bytes = canonical_value(value)?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(T::schema_id().map_err(runtime_value_error)?),
        semantic_type_id: Some(T::semantic_id().map_err(runtime_value_error)?),
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    };
    Ok(ContractAppArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

fn artifact_for_schema_less_json<T>(
    value: &T,
    role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
) -> mfm_runtime::Result<ContractAppArtifact>
where
    T: Serialize,
{
    let bytes = canonical_value(value)?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    };
    Ok(ContractAppArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

fn retention(artifact: &store::ArtifactEvidenceRef) -> StagedRetentionRefs {
    StagedRetentionRefs::runtime_evidence(vec![events::RetentionRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        content_digest: artifact.digest.clone(),
    }])
}

fn projected_side_effect<'a>(
    ctx: &'a ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<&'a store::SideEffectProjection> {
    ctx.projections().side_effect(ledger_key).ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "contract lifecycle side-effect projection missing for ledger {ledger_key}"
        ))
    })
}

fn projected_prepared_artifact(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect(ctx, ledger_key)?
        .prepared_invocation
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("prepared invocation"))
}

fn projected_submission_artifact(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect(ctx, ledger_key)?
        .submission
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("submission"))
}

fn projected_receipt_artifact(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect(ctx, ledger_key)?
        .receipt
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("receipt"))
}

fn projected_confirmation_artifact(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect(ctx, ledger_key)?
        .confirmation
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("confirmation"))
}

fn missing_side_effect_artifact(label: &str) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "contract lifecycle side-effect {label} artifact is missing"
    ))
}

fn single_receipt(
    receipts: ContractTransactionReceipts,
) -> mfm_runtime::Result<mfm_state_evm_contracts::ContractTransactionReceipt> {
    let mut transactions = receipts.transactions;
    if transactions.len() != 1 {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract deploy confirmation requires exactly one receipt".to_owned(),
        ));
    }
    Ok(transactions.remove(0))
}

fn receipts_with_evidence(
    mut receipts: ContractTransactionReceipts,
    evidence: &mfm_artifact_capabilities::ArtifactEvidenceRef,
) -> ContractTransactionReceipts {
    let evidence = lifecycle_evidence_ref(evidence);
    for receipt in &mut receipts.transactions {
        receipt.receipt_evidence = Some(evidence.clone());
    }
    receipts
}

fn ledger_key_for_idempotency(
    idempotency: &ContractTransactionIdempotency,
) -> mfm_runtime::Result<events::SideEffectLedgerKey> {
    events::SideEffectLedgerKey::new(format!(
        "mfm.evm.contract.ledger.{}",
        short_stable_key(&idempotency.key)
    ))
    .map_err(Into::into)
}

fn idempotency_key_ref(
    idempotency: &ContractTransactionIdempotency,
) -> mfm_runtime::Result<events::IdempotencyKeyRef> {
    events::IdempotencyKeyRef::new(format!(
        "mfm.evm.contract.idem.{}",
        short_stable_key(&idempotency.key)
    ))
    .map_err(Into::into)
}

fn runner_invocation_id(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<events::RunnerInvocationId> {
    let digest = digest_json(serde_json::json!({
        "attempt_id": ctx.attempt_id().as_str(),
        "ledger_key": ledger_key.as_str(),
        "node_id": ctx.node().node_id.as_str(),
        "run_id": ctx.run_id().as_str(),
    }))?;
    events::RunnerInvocationId::new(format!("mfm.evm.contract.owner.{}", short_digest(&digest)))
        .map_err(Into::into)
}

fn claim_fencing_token(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<side_effect::ClaimFencingToken> {
    let digest = digest_json(serde_json::json!({
        "attempt_id": ctx.attempt_id().as_str(),
        "ledger_key": ledger_key.as_str(),
        "node_id": ctx.node().node_id.as_str(),
        "token": "contract-lifecycle",
    }))?;
    side_effect::ClaimFencingToken::new(format!("mfm.evm.contract.token.{}", short_digest(&digest)))
        .map_err(Into::into)
}

fn short_stable_key(value: &str) -> String {
    value
        .rsplit(':')
        .next()
        .unwrap_or(value)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(32)
        .collect()
}

fn short_digest(digest: &ContentDigest) -> String {
    short_stable_key(digest.as_str())
}

fn unsupported_side_effect_phase(
    phase: &'static str,
    current: store::SideEffectPhase,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "unsupported contract lifecycle {phase} side-effect phase: {current:?}"
    )))
}

fn canonical_value<T: Serialize>(value: &T) -> mfm_runtime::Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))
}

fn digest_value<T: Serialize>(value: &T) -> mfm_runtime::Result<ContentDigest> {
    Ok(canonical_value(value)?.content_digest())
}

fn digest_json(value: serde_json::Value) -> mfm_runtime::Result<ContentDigest> {
    let json = serde_json::to_string(&value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    Ok(PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?
        .content_digest())
}

fn runtime_value_error(error: mfm_values::ValueError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_capability_error(error: mfm_capabilities::CapabilityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_artifact_read_error(
    error: mfm_artifact_capabilities::ArtifactReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_evm_capability_error(
    error: mfm_evm_capabilities::EvmCapabilityError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_evm_transport_error(
    error: mfm_transports_evm::EvmTransportError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_signing_error(error: mfm_signing::SigningError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_adapter_error(
    error: mfm_adapters_evm_contracts::EvmContractAdapterError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_plan_error(error: mfm_program::PlanError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn validated_config<T: MfmConfig>(
    config: T,
) -> mfm_runtime::Result<mfm_program::ValidatedConfig<T>> {
    mfm_program::ValidatedConfig::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn runtime_state_error(error: mfm_program::StateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        make_in_memory_typed_services_with_certification_registry, new_run_id,
        prepare_certified_run_launch, CertifiedRunLaunchInput, DriveMode, RunLaunchConfigArtifact,
        RunLaunchSeedArtifact, TypedRunMode,
    };
    use mfm_capabilities::CapabilitySpec;
    use mfm_certify::CertificationRegistry;
    use mfm_core::crypto::EthereumPrivateKey;
    use mfm_evm_capabilities::{
        EvmCallReadCapability, EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityError,
        EvmCapabilityFuture, EvmChainIdentityCapability, EvmChainIdentityRequest,
        EvmChainIdentityResponse, EvmFeeReadRequest, EvmFeeReadResponse, EvmGasEstimateRequest,
        EvmGasEstimateResponse, EvmLogEntry, EvmLogsReadCapability, EvmLogsReadRequest,
        EvmLogsReadResponse, EvmNonceReadRequest, EvmNonceReadResponse, EvmReceiptReadRequest,
        EvmReceiptReadResponse, EvmTransactionSubmitRequest, EvmTransactionSubmitResponse,
        RedactedEvmSourceEvidence,
    };
    use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
    use mfm_op_evm_contract_lifecycle::{
        compile_contract_deploy_program, compile_contract_lifecycle_program,
        compile_contract_validate_program, ContractLifecycleConfig,
    };
    use mfm_signing::{
        PublicSigningIdentity, SignatureBytes, SignerRef, SigningError, SigningFuture,
        SigningRequest, SigningResult,
    };
    use mfm_store::v1::TypedRunEventStore;
    use serde_json::json;
    use std::sync::Mutex;

    const TEST_SIGNER_HEX: &str =
        "4c0883a69102937d6231471b5dbb6204fe512961708279c2f802d6a8ebf2d3a4";

    #[tokio::test]
    async fn app_runner_resumes_replays_and_renders_validate_only_lifecycle_run() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-evm-contract-validate-{}",
            uuid::Uuid::new_v4()
        ));
        let artifacts = FsTypedArtifactStore::new(&root);
        let config = validate_config();
        let configured = configured_contract();
        let seed_bytes = canonical_value(&configured)
            .expect("canonical configured")
            .to_vec();
        let compiled = compile_contract_validate_program(config, configured)
            .expect("compiled validate program");
        let mut certification = CertificationRegistry::new();
        mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
            &mut certification,
        )
        .expect("contract certification descriptors");
        let mut runners = ErasedRunnerRegistry::new();
        register_contract_lifecycle_runners_with_factory(
            &mut runners,
            Arc::new(TestRuntimeFactory::new(artifacts.clone())),
        )
        .expect("contract runners");
        let services = make_in_memory_typed_services_with_certification_registry(
            runners,
            &root,
            certification,
        );
        let run_id = new_run_id();
        let seed_id = compiled
            .certified_spec
            .envelope()
            .spec
            .seeds
            .first()
            .expect("validate spec has configured seed")
            .seed_id
            .clone();
        let request = prepare_certified_run_launch(
            CertifiedRunLaunchInput {
                certified_spec: compiled.certified_spec.clone(),
                registry: services.certification_registry(),
                run_id: run_id.clone(),
                framework_version: "mfm.test.contract",
                source_revision: "test-source",
                drive: DriveMode::AppendOnly,
            },
            compiled
                .config_artifacts
                .iter()
                .map(|artifact| RunLaunchConfigArtifact {
                    schema_id: artifact.schema_id.clone(),
                    bytes: artifact.bytes.clone(),
                    media_type: artifact.media_type.clone(),
                })
                .collect(),
            vec![RunLaunchSeedArtifact {
                seed_id,
                bytes: seed_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            }],
        )
        .expect("launch request");

        let started = services.launch_run(request).await.expect("append start");
        assert_eq!(started.run_mode, TypedRunMode::Forward);
        let resumed = services
            .resume_stored_run(&run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume validate lifecycle");
        assert_eq!(resumed.run_mode, TypedRunMode::Completed);
        let replay = services
            .replay_broker(&run_id)
            .await
            .expect("replay validate lifecycle");
        assert_eq!(
            replay.projection_snapshot().run_state(&run_id),
            store::RunState::Completed
        );
        let public_output = services
            .typed_public_output(&run_id, &compiled.public_schema_id)
            .await
            .expect("public output");
        let rendered = public_output.json.expect("json");
        assert!(
            rendered.to_string().contains("\"valid\":true"),
            "rendered validation output must contain a valid report: {rendered}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn app_runner_records_distinct_validation_capability_facts() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-evm-contract-validation-facts-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("root dir");
        let artifacts = FsTypedArtifactStore::new(&root);
        let configured = configured_contract();
        let seed_bytes = canonical_value(&configured)
            .expect("canonical configured")
            .to_vec();
        let compiled =
            compile_contract_validate_program(validate_config_with_assertions(), configured)
                .expect("compiled contract validation program");
        let mut certification = CertificationRegistry::new();
        mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
            &mut certification,
        )
        .expect("contract certification descriptors");
        let mut runners = ErasedRunnerRegistry::new();
        register_contract_lifecycle_runners_with_factory(
            &mut runners,
            Arc::new(TestRuntimeFactory::new(artifacts.clone())),
        )
        .expect("contract runners");
        let services = make_in_memory_typed_services_with_certification_registry(
            runners,
            &root,
            certification,
        );
        let run_id = new_run_id();
        let seed_id = compiled
            .certified_spec
            .envelope()
            .spec
            .seeds
            .first()
            .expect("validate spec has configured seed")
            .seed_id
            .clone();
        let request = prepare_certified_run_launch(
            CertifiedRunLaunchInput {
                certified_spec: compiled.certified_spec.clone(),
                registry: services.certification_registry(),
                run_id: run_id.clone(),
                framework_version: "mfm.test.contract",
                source_revision: "test-source",
                drive: DriveMode::AppendOnly,
            },
            compiled
                .config_artifacts
                .iter()
                .map(|artifact| RunLaunchConfigArtifact {
                    schema_id: artifact.schema_id.clone(),
                    bytes: artifact.bytes.clone(),
                    media_type: artifact.media_type.clone(),
                })
                .collect(),
            vec![RunLaunchSeedArtifact {
                seed_id,
                bytes: seed_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            }],
        )
        .expect("launch request");

        let started = services.launch_run(request).await.expect("append start");
        assert_eq!(started.run_mode, TypedRunMode::Forward);
        let resumed = services
            .resume_stored_run(&run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume validate lifecycle");
        assert_eq!(resumed.run_mode, TypedRunMode::Completed);
        let stream = {
            let store = services.store();
            let store = store.lock().await;
            store.load_run_stream(&run_id)
        };
        let fact_kinds = stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::FactRecorded(fact) => {
                    Some(fact.capability_kind.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            fact_kinds.len(),
            3,
            "validation with chain, call, and logs must record separate facts"
        );
        assert!(fact_kinds
            .contains(&EvmChainIdentityCapability::kind().expect("chain identity capability")));
        assert!(fact_kinds.contains(&EvmCallReadCapability::kind().expect("call capability")));
        assert!(fact_kinds.contains(&EvmLogsReadCapability::kind().expect("logs capability")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn app_runner_resumes_replays_and_renders_deploy_lifecycle_run() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-evm-contract-deploy-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("root dir");
        let artifacts = FsTypedArtifactStore::new(&root);
        let signer = test_contract_signer();
        let config = deploy_config(&signer.address);
        let compiled = compile_contract_deploy_program(config).expect("compiled deploy program");
        let mut certification = CertificationRegistry::new();
        mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
            &mut certification,
        )
        .expect("contract certification descriptors");
        let mut runners = ErasedRunnerRegistry::new();
        register_contract_lifecycle_runners_with_factory(
            &mut runners,
            Arc::new(TestRuntimeFactory::with_signer(
                artifacts.clone(),
                Arc::new(signer.provider),
                true,
            )),
        )
        .expect("contract runners");
        let services = make_in_memory_typed_services_with_certification_registry(
            runners,
            &root,
            certification,
        );
        let run_id = new_run_id();
        let request = prepare_certified_run_launch(
            CertifiedRunLaunchInput {
                certified_spec: compiled.certified_spec.clone(),
                registry: services.certification_registry(),
                run_id: run_id.clone(),
                framework_version: "mfm.test.contract",
                source_revision: "test-source",
                drive: DriveMode::AppendOnly,
            },
            compiled
                .config_artifacts
                .iter()
                .map(|artifact| RunLaunchConfigArtifact {
                    schema_id: artifact.schema_id.clone(),
                    bytes: artifact.bytes.clone(),
                    media_type: artifact.media_type.clone(),
                })
                .collect(),
            Vec::new(),
        )
        .expect("launch request");

        let started = services.launch_run(request).await.expect("append start");
        assert_eq!(started.run_mode, TypedRunMode::Forward);
        let resumed = services
            .resume_stored_run(&run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume deploy lifecycle");
        assert_eq!(resumed.run_mode, TypedRunMode::Completed);
        let replay = services
            .replay_broker(&run_id)
            .await
            .expect("replay deploy lifecycle");
        assert_eq!(
            replay.projection_snapshot().run_state(&run_id),
            store::RunState::Completed
        );
        assert!(
            mfm_adapters_evm_contracts::verify_contract_lifecycle_replay(&replay)
                .expect("contract replay verifier")
        );
        let public_output = services
            .typed_public_output(&run_id, &compiled.public_schema_id)
            .await
            .expect("public output");
        let rendered = public_output.json.expect("json");
        assert!(
            rendered.to_string().contains("contract_address"),
            "rendered deploy output must contain deployed contract evidence: {rendered}"
        );
        assert!(
            rendered.to_string().contains("deploy_receipt_evidence"),
            "rendered deploy output must contain receipt evidence refs: {rendered}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn app_runner_resumes_replays_and_renders_full_lifecycle_run() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-evm-contract-lifecycle-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("root dir");
        let artifacts = FsTypedArtifactStore::new(&root);
        let signer = test_contract_signer();
        let config = ContractLifecycleConfig::new(
            deploy_config(&signer.address),
            configure_config(&signer.address),
            validate_config(),
        );
        let compiled =
            compile_contract_lifecycle_program(config).expect("compiled lifecycle program");
        let mut certification = CertificationRegistry::new();
        mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
            &mut certification,
        )
        .expect("contract certification descriptors");
        let mut runners = ErasedRunnerRegistry::new();
        register_contract_lifecycle_runners_with_factory(
            &mut runners,
            Arc::new(TestRuntimeFactory::with_signer(
                artifacts.clone(),
                Arc::new(signer.provider),
                false,
            )),
        )
        .expect("contract runners");
        let services = make_in_memory_typed_services_with_certification_registry(
            runners,
            &root,
            certification,
        );
        let run_id = new_run_id();
        let request = prepare_certified_run_launch(
            CertifiedRunLaunchInput {
                certified_spec: compiled.certified_spec.clone(),
                registry: services.certification_registry(),
                run_id: run_id.clone(),
                framework_version: "mfm.test.contract",
                source_revision: "test-source",
                drive: DriveMode::AppendOnly,
            },
            compiled
                .config_artifacts
                .iter()
                .map(|artifact| RunLaunchConfigArtifact {
                    schema_id: artifact.schema_id.clone(),
                    bytes: artifact.bytes.clone(),
                    media_type: artifact.media_type.clone(),
                })
                .collect(),
            Vec::new(),
        )
        .expect("launch request");

        let started = services.launch_run(request).await.expect("append start");
        assert_eq!(started.run_mode, TypedRunMode::Forward);
        let resumed = services
            .resume_stored_run(&run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume full lifecycle");
        assert_eq!(resumed.run_mode, TypedRunMode::Completed);
        let replay = services
            .replay_broker(&run_id)
            .await
            .expect("replay full lifecycle");
        assert!(
            mfm_adapters_evm_contracts::verify_contract_lifecycle_replay(&replay)
                .expect("contract replay verifier")
        );
        let public_output = services
            .typed_public_output(&run_id, &compiled.public_schema_id)
            .await
            .expect("public output");
        let rendered = public_output.json.expect("json");
        assert!(
            rendered.to_string().contains("\"valid\":true"),
            "rendered lifecycle output must contain a valid validation report: {rendered}"
        );
        assert!(
            rendered.to_string().contains("configure_tx_hashes"),
            "rendered lifecycle output must contain configure transaction hashes: {rendered}"
        );
        assert!(
            rendered.to_string().contains("configure_receipt_evidence"),
            "rendered lifecycle output must contain configure receipt evidence refs: {rendered}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn receipt_polling_does_not_retry_permanent_capability_failures() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-evm-contract-receipt-failure-{}",
            uuid::Uuid::new_v4()
        ));
        let factory = TestRuntimeFactory::new(FsTypedArtifactStore::new(&root));
        let transaction_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let prepared = mfm_adapters_evm_contracts::PreparedContractInvocation {
            prepared_version: 1,
            phase: mfm_adapters_evm_contracts::ContractMutationPhase::Deploy,
            network_id: "reth-dev".to_owned(),
            expected_chain_id: 31337,
            signer_ref: "deployer".to_owned(),
            expected_signer_address: "0x0000000000000000000000000000000000000001".to_owned(),
            transactions: vec![mfm_adapters_evm_contracts::PreparedContractTransactionEvidence {
                index: 0,
                style: mfm_adapters_evm_contracts::PreparedContractTransactionStyle::Eip1559,
                chain_id: 31337,
                nonce: 7,
                to_address: None,
                value_wei: "0".to_owned(),
                gas_limit: 21_000,
                max_fee_per_gas: Some("11".to_owned()),
                max_priority_fee_per_gas: Some("3".to_owned()),
                gas_price: None,
                data_digest: "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
                data_len: 0,
                signing_digest: transaction_hash.to_owned(),
            }],
            poll_interval_ms: 25,
            max_receipt_polls: 3,
        };
        let submissions = ContractTransactionSubmissions {
            submissions_version: 1,
            transactions: vec![mfm_state_evm_contracts::ContractTransactionSubmission {
                submission_version: 1,
                transaction_hash: transaction_hash.to_owned(),
                signer_public_key: None,
            }],
        };

        let error = read_receipts_with_poll(&factory.runtime, &prepared, &submissions)
            .await
            .expect_err("permanent capability failure");

        assert!(error.to_string().contains("EVM capability provider failed"));
        assert_eq!(factory.reads.lock().expect("reads").receipt, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[derive(Clone)]
    struct TestRuntimeFactory {
        artifacts: Arc<dyn ArtifactReadProvider>,
        runtime: EvmContractRuntime,
        reads: Arc<Mutex<TestEvmReads>>,
    }

    impl TestRuntimeFactory {
        fn new(artifacts: FsTypedArtifactStore) -> Self {
            let source_ref = EvmSourceRef::new("reth-dev").expect("source ref");
            let policy_id = EvmSourcePolicyId::new("reth-dev").expect("policy id");
            let reads = Arc::new(Mutex::new(TestEvmReads::default()));
            let evm = Arc::new(TestEvmProvider {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                chain_id: 31337,
                mutation: false,
                fail_repeated_prepare_reads: false,
                reads: Arc::clone(&reads),
            });
            Self {
                artifacts: Arc::new(artifacts),
                runtime: EvmContractRuntime {
                    route: EvmContractRuntimeRoute::new(source_ref, policy_id),
                    evm,
                    signer: Arc::new(TestSigner),
                },
                reads,
            }
        }

        fn with_signer(
            artifacts: FsTypedArtifactStore,
            signer: Arc<dyn SigningProvider>,
            fail_repeated_prepare_reads: bool,
        ) -> Self {
            let source_ref = EvmSourceRef::new("reth-dev").expect("source ref");
            let policy_id = EvmSourcePolicyId::new("reth-dev").expect("policy id");
            let reads = Arc::new(Mutex::new(TestEvmReads::default()));
            let evm = Arc::new(TestEvmProvider {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                chain_id: 31337,
                mutation: true,
                fail_repeated_prepare_reads,
                reads: Arc::clone(&reads),
            });
            Self {
                artifacts: Arc::new(artifacts),
                runtime: EvmContractRuntime {
                    route: EvmContractRuntimeRoute::new(source_ref, policy_id),
                    evm,
                    signer,
                },
                reads,
            }
        }
    }

    impl EvmContractRuntimeFactory for TestRuntimeFactory {
        fn artifacts(&self) -> &dyn ArtifactReadProvider {
            self.artifacts.as_ref()
        }

        fn runtime_for(&self, _network_id: &str) -> mfm_runtime::Result<EvmContractRuntime> {
            Ok(self.runtime.clone())
        }
    }

    struct TestSigner;

    impl SigningProvider for TestSigner {
        fn sign<'a>(&'a self, _request: &'a SigningRequest) -> SigningFuture<'a> {
            Box::pin(async { Err(SigningError::redacted_provider_failure("test signer")) })
        }
    }

    #[derive(Clone)]
    struct TestEvmProvider {
        source_ref: EvmSourceRef,
        policy_id: EvmSourcePolicyId,
        chain_id: u64,
        mutation: bool,
        fail_repeated_prepare_reads: bool,
        reads: Arc<Mutex<TestEvmReads>>,
    }

    #[derive(Default)]
    struct TestEvmReads {
        nonce: u32,
        fee: u32,
        gas: u32,
        receipt: u32,
    }

    impl TestEvmProvider {
        fn evidence(&self) -> RedactedEvmSourceEvidence {
            RedactedEvmSourceEvidence {
                source_ref: self.source_ref.clone(),
                policy_id: self.policy_id.clone(),
                chain_id: self.chain_id,
            }
        }

        fn record_prepare_read(&self, kind: TestPrepareReadKind) -> Result<(), EvmCapabilityError> {
            if !self.fail_repeated_prepare_reads {
                return Ok(());
            }
            let mut reads = self
                .reads
                .lock()
                .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
            let count = match kind {
                TestPrepareReadKind::Nonce => &mut reads.nonce,
                TestPrepareReadKind::Fee => &mut reads.fee,
                TestPrepareReadKind::Gas => &mut reads.gas,
            };
            *count += 1;
            if *count > 1 {
                Err(EvmCapabilityError::redacted_provider_failure("test evm"))
            } else {
                Ok(())
            }
        }
    }

    enum TestPrepareReadKind {
        Nonce,
        Fee,
        Gas,
    }

    impl EvmChainIdentityProvider for TestEvmProvider {
        fn chain_identity<'a>(
            &'a self,
            _request: &'a EvmChainIdentityRequest,
        ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
            Box::pin(async move {
                Ok(EvmChainIdentityResponse {
                    evidence: self.evidence(),
                    chain_id: self.chain_id,
                    client_version: Some("mfm-test-evm".to_owned()),
                })
            })
        }
    }

    impl EvmNonceReadProvider for TestEvmProvider {
        fn read_nonce<'a>(
            &'a self,
            _request: &'a EvmNonceReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
            if !self.mutation {
                return failed_evm();
            }
            Box::pin(async move {
                self.record_prepare_read(TestPrepareReadKind::Nonce)?;
                Ok(EvmNonceReadResponse {
                    evidence: self.evidence(),
                    nonce: 7,
                })
            })
        }
    }

    impl EvmFeeReadProvider for TestEvmProvider {
        fn read_fee<'a>(
            &'a self,
            _request: &'a EvmFeeReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
            if !self.mutation {
                return failed_evm();
            }
            Box::pin(async move {
                self.record_prepare_read(TestPrepareReadKind::Fee)?;
                Ok(EvmFeeReadResponse {
                    evidence: self.evidence(),
                    base_fee_per_gas: Some(5),
                    priority_fee_per_gas: Some(3),
                    max_fee_per_gas: Some(11),
                    legacy_gas_price: Some(7),
                })
            })
        }
    }

    impl EvmGasEstimateProvider for TestEvmProvider {
        fn estimate_gas<'a>(
            &'a self,
            _request: &'a EvmGasEstimateRequest,
        ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
            if !self.mutation {
                return failed_evm();
            }
            Box::pin(async move {
                self.record_prepare_read(TestPrepareReadKind::Gas)?;
                Ok(EvmGasEstimateResponse {
                    evidence: self.evidence(),
                    gas_limit: 21_000,
                })
            })
        }
    }

    impl EvmTransactionSubmitProvider for TestEvmProvider {
        fn submit_transaction<'a>(
            &'a self,
            request: &'a EvmTransactionSubmitRequest,
        ) -> EvmCapabilityFuture<'a, EvmTransactionSubmitResponse> {
            if !self.mutation {
                return failed_evm();
            }
            Box::pin(async move {
                Ok(EvmTransactionSubmitResponse {
                    evidence: self.evidence(),
                    transaction_hash: request.signed_payload.transaction_hash(),
                })
            })
        }
    }

    impl EvmReceiptReadProvider for TestEvmProvider {
        fn read_receipt<'a>(
            &'a self,
            request: &'a EvmReceiptReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
            let reads = Arc::clone(&self.reads);
            if !self.mutation {
                return Box::pin(async move {
                    let mut reads = reads
                        .lock()
                        .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
                    reads.receipt += 1;
                    Err(EvmCapabilityError::redacted_provider_failure("test evm"))
                });
            }
            Box::pin(async move {
                let mut reads = self
                    .reads
                    .lock()
                    .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
                reads.receipt += 1;
                Ok(EvmReceiptReadResponse {
                    evidence: self.evidence(),
                    transaction_hash: request.transaction_hash,
                    block_number: 42,
                    status: true,
                })
            })
        }
    }

    impl EvmCallReadProvider for TestEvmProvider {
        fn read_call<'a>(
            &'a self,
            _request: &'a EvmCallReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
            Box::pin(async move {
                let mut return_data = vec![0_u8; 32];
                return_data[31] = 1;
                Ok(EvmCallReadResponse {
                    evidence: self.evidence(),
                    return_data,
                })
            })
        }
    }

    impl EvmLogsReadProvider for TestEvmProvider {
        fn read_logs<'a>(
            &'a self,
            request: &'a EvmLogsReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
            Box::pin(async move {
                Ok(EvmLogsReadResponse {
                    evidence: self.evidence(),
                    logs: vec![EvmLogEntry {
                        address: request.address.expect("validation log address"),
                        topics: request.topics.clone(),
                        data: Vec::new(),
                        block_number: Some(42),
                        transaction_hash: None,
                        log_index: Some(0),
                    }],
                })
            })
        }
    }

    fn failed_evm<'a, T>() -> EvmCapabilityFuture<'a, T> {
        Box::pin(async { Err(EvmCapabilityError::redacted_provider_failure("test evm")) })
    }

    struct TestSignerRuntime {
        provider: LocalTestSigner,
        address: String,
    }

    fn test_contract_signer() -> TestSignerRuntime {
        let signer_ref = SignerRef::new("deployer").expect("signer ref");
        let material = EthereumPrivateKey::from_hex_secret(TEST_SIGNER_HEX).expect("signer");
        let address = format!("{:?}", material.address().expect("address"));
        let provider = LocalTestSigner {
            signer_ref,
            address: address.clone(),
            material,
        };
        TestSignerRuntime { provider, address }
    }

    struct LocalTestSigner {
        signer_ref: SignerRef,
        address: String,
        material: EthereumPrivateKey,
    }

    impl SigningProvider for LocalTestSigner {
        fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
            let result = self.sign_request(request);
            Box::pin(async move { result })
        }
    }

    impl LocalTestSigner {
        fn sign_request(&self, request: &SigningRequest) -> mfm_signing::Result<SigningResult> {
            if request.signer_ref() != &self.signer_ref {
                return Err(SigningError::redacted_provider_failure("test signer"));
            }
            let digest = request.digest().as_bytes();
            let signature = self
                .material
                .sign_hash_recoverable(digest)
                .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
            let identity = PublicSigningIdentity::new(
                request.algorithm().clone(),
                None,
                Some(self.address.clone()),
            )?;
            let signature = SignatureBytes::new(signature.as_bytes().to_vec())?;
            SigningResult::for_request(request, identity, signature)
        }
    }

    fn validate_config() -> ValidatePhaseConfig {
        serde_json::from_value(json!({
            "network": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337
            }
        }))
        .expect("validate config")
    }

    fn validate_config_with_assertions() -> ValidatePhaseConfig {
        serde_json::from_value(json!({
            "artifact": artifact_json(),
            "network": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337
            },
            "validation": {
                "read_assertions": [
                    {
                        "function": "ready",
                        "args": [],
                        "expected": {
                            "json_text": "true"
                        }
                    }
                ],
                "event_assertions": [
                    {
                        "event": "Configured",
                        "min_count": 1
                    }
                ]
            }
        }))
        .expect("validate config with assertions")
    }

    fn deploy_config(expected_signer_address: &str) -> DeployPhaseConfig {
        serde_json::from_value(json!({
            "artifact": artifact_json(),
            "network": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337
            },
            "signer": {
                "signer_ref": "deployer",
                "expected_signer_address": expected_signer_address
            },
            "transaction": {
                "style": "eip1559",
                "max_fee_per_gas": "11",
                "max_priority_fee_per_gas": "3"
            }
        }))
        .expect("deploy config")
    }

    fn configure_config(expected_signer_address: &str) -> ConfigurePhaseConfig {
        serde_json::from_value(json!({
            "artifact": artifact_json(),
            "network": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337
            },
            "signer": {
                "signer_ref": "deployer",
                "expected_signer_address": expected_signer_address
            },
            "calls": [
                {
                    "function": "configure",
                    "args": []
                },
                {
                    "function": "configure",
                    "args": []
                }
            ]
        }))
        .expect("configure config")
    }

    fn artifact_json() -> serde_json::Value {
        json!({
            "abi": {
                "json_text": json!([
                    {
                        "type": "constructor",
                        "inputs": []
                    },
                    {
                        "type": "function",
                        "name": "configure",
                        "inputs": [],
                        "outputs": [],
                        "stateMutability": "nonpayable"
                    },
                    {
                        "type": "function",
                        "name": "ready",
                        "inputs": [],
                        "outputs": [
                            {
                                "name": "",
                                "type": "bool"
                            }
                        ],
                        "stateMutability": "view"
                    },
                    {
                        "type": "event",
                        "name": "Configured",
                        "inputs": [],
                        "anonymous": false
                    }
                ]).to_string()
            },
            "bytecode": {
                "json_text": json!({"object": "0x6000"}).to_string()
            }
        })
    }

    fn configured_contract() -> ConfiguredContract {
        ConfiguredContract {
            lifecycle_version: 1,
            deployed: DeployedContract {
                lifecycle_version: 1,
                network_id: "reth-dev".to_owned(),
                expected_chain_id: 31337,
                contract_address: "0x000000000000000000000000000000000000dead".to_owned(),
                deploy_tx_hash: "0x01".to_owned(),
                deploy_receipt_evidence: None,
                deployed_block_number: Some(1),
            },
            configure_calls: Vec::new(),
            confirmation_read_assertions: Vec::new(),
            confirmation_event_assertions: Vec::new(),
            configure_tx_hashes: Vec::new(),
            configure_receipt_evidence: Vec::new(),
            configured_block_number: Some(1),
        }
    }
}
