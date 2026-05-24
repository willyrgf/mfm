#![warn(missing_docs)]
//! Typed EVM deploy/configure/validate workflow runners.
//!
//! This crate binds certified typed EVM DCV state descriptors to concrete live runners. It
//! persists intent, prepared invocation, submission, receipt, confirmation, and state-output
//! artifacts through the typed artifact store and never exposes legacy dynamic IO surfaces.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use mfm_artifact_store_fs::FsTypedArtifactStore;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::CapabilitySpec;
use mfm_core::crypto::EthereumPrivateKey;
use mfm_events::v1::{self as events, side_effect};
use mfm_evm_core::tx::{
    encode_signed_legacy_tx_hex, legacy_signing_hash, parse_address, parse_data_hex,
    parse_u128_quantity, parse_u64_quantity, raw_transaction_hash, LegacyTxToSign,
};
use mfm_evm_deploy_configure_validate_config::{
    DeployConfigureValidateConfigureConfig, DeployConfigureValidateDeployConfig,
    DeployConfigureValidateValidateConfig,
};
use mfm_ids::{ArtifactId, ContentDigest, DescriptorId, NodeId};
use mfm_program::{SideEffectState, StateSpec};
use mfm_runtime::{
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput,
    ErasedRunnerRegistry, MaterializedCellTerminal, MaterializedInputNode, StagedRetentionRefs,
};
use mfm_spec::v1 as spec;
use mfm_state_evm_dcv::{
    configure_intent_from_config, deploy_intent_from_config, evm_dcv_adapter_kind,
    evm_dcv_adapter_version, validate_configured_contract_with_backend, ConfigureContractState,
    ConfiguredContract, DeployContractState, DeployedContract, EvmDcvConfigureConfirmation,
    EvmDcvConfigureIdempotencyInput, EvmDcvConfigureIntent, EvmDcvConfigureReceipt,
    EvmDcvConfigureReceiptEntry, EvmDcvConfigureSubmission, EvmDcvDeployConfirmation,
    EvmDcvDeployIdempotencyInput, EvmDcvDeployIntent, EvmDcvDeployReceipt, EvmDcvDeploySubmission,
    EvmDcvReadBackend, EvmDcvReadError, EvmDcvReadFuture, EvmDcvSubmissionUnknownEvidence,
    EvmDcvTransactionIntent, EvmDcvTransactionSubmitCapability, ValidateContractState,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";

/// Future returned by typed EVM DCV artifact stores.
pub type EvmDcvArtifactStoreFuture<'a, T> =
    Pin<Box<dyn Future<Output = mfm_runtime::Result<T>> + Send + 'a>>;

/// Artifact store boundary used by typed EVM DCV runners.
pub trait EvmDcvArtifactStore: Send + Sync {
    /// Loads verified typed artifact bytes by id.
    fn get_artifact_by_id<'a>(
        &'a self,
        artifact_id: &'a ArtifactId,
    ) -> EvmDcvArtifactStoreFuture<'a, (Vec<u8>, store::ArtifactEvidenceRef)>;

    /// Persists bytes after validating they match the supplied typed evidence.
    fn put_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> EvmDcvArtifactStoreFuture<'a, ()>;
}

impl EvmDcvArtifactStore for FsTypedArtifactStore {
    fn get_artifact_by_id<'a>(
        &'a self,
        artifact_id: &'a ArtifactId,
    ) -> EvmDcvArtifactStoreFuture<'a, (Vec<u8>, store::ArtifactEvidenceRef)> {
        Box::pin(async move {
            self.get_artifact_by_id(artifact_id)
                .await
                .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))
        })
    }

    fn put_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> EvmDcvArtifactStoreFuture<'a, ()> {
        Box::pin(async move {
            self.put_verified_artifact(bytes, evidence)
                .await
                .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
            Ok(())
        })
    }
}

/// Registers typed EVM deploy/configure/validate runners.
pub fn register_evm_dcv_runners(
    registry: &mut ErasedRunnerRegistry,
    artifacts: Arc<dyn EvmDcvArtifactStore>,
) -> mfm_runtime::Result<()> {
    registry.register(binding(
        registered_descriptor::<DeployContractState>()?,
        SIDE_EFFECT_FACTORY,
        Arc::new(DeployRunner {
            artifacts: artifacts.clone(),
            rpc: EvmDcvRpcClient::from_env(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<ConfigureContractState>()?,
        SIDE_EFFECT_FACTORY,
        Arc::new(ConfigureRunner {
            artifacts: artifacts.clone(),
            rpc: EvmDcvRpcClient::from_env(),
        }),
    )?)?;
    registry.register(binding(
        registered_descriptor::<ValidateContractState>()?,
        READ_FACTORY,
        Arc::new(ValidateRunner {
            artifacts,
            rpc: EvmDcvRpcClient::from_env(),
        }),
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
        source_revision: events::SourceRevision::new("mfm-transports-evm-dcv-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-transports-evm-dcv")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: digest_json(serde_json::json!({
            "crate": "mfm-transports-evm-dcv",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        binary_digest: digest_json(serde_json::json!({
            "crate": "mfm-transports-evm-dcv",
            "runner": "typed-evm-dcv",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct DeployRunner {
    artifacts: Arc<dyn EvmDcvArtifactStore>,
    rpc: EvmDcvRpcClient,
}

impl ErasedNodeRunner for DeployRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        let artifacts = self.artifacts.clone();
        let rpc = self.rpc.clone();
        Box::pin(async move { run_deploy(ctx, artifacts.as_ref(), &rpc).await })
    }
}

struct ConfigureRunner {
    artifacts: Arc<dyn EvmDcvArtifactStore>,
    rpc: EvmDcvRpcClient,
}

impl ErasedNodeRunner for ConfigureRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        let artifacts = self.artifacts.clone();
        let rpc = self.rpc.clone();
        Box::pin(async move { run_configure(ctx, artifacts.as_ref(), &rpc).await })
    }
}

struct ValidateRunner {
    artifacts: Arc<dyn EvmDcvArtifactStore>,
    rpc: EvmDcvRpcClient,
}

impl ErasedNodeRunner for ValidateRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        let artifacts = self.artifacts.clone();
        let rpc = self.rpc.clone();
        Box::pin(async move { run_validate(ctx, artifacts.as_ref(), &rpc).await })
    }
}

async fn run_deploy(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let ledger_key = ledger_key(&ctx, "deploy")?;
    let projection = ctx.projections.side_effect(&ledger_key);
    match projection.map(|projection| &projection.phase) {
        None => deploy_persist_intent(ctx, ledger_key, artifacts).await,
        Some(store::SideEffectPhase::IntentPersisted { .. }) => {
            claim_side_effect(ctx, ledger_key, projection.expect("projection"))
        }
        Some(store::SideEffectPhase::Claimed { .. }) => {
            deploy_prepare_invocation(
                ctx,
                ledger_key,
                projection.expect("projection"),
                artifacts,
                rpc,
            )
            .await
        }
        Some(store::SideEffectPhase::InvocationPrepared { .. }) => {
            start_invocation(ctx, ledger_key, projection.expect("projection"))
        }
        Some(store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        })
        | Some(store::SideEffectPhase::SubmissionUnknown { invocation_epoch }) => {
            deploy_submission(ctx, ledger_key, *invocation_epoch, artifacts, rpc).await
        }
        Some(store::SideEffectPhase::SubmissionObserved { invocation_epoch }) => {
            deploy_receipt(ctx, ledger_key, *invocation_epoch, artifacts, rpc).await
        }
        Some(store::SideEffectPhase::ReceiptObserved { invocation_epoch }) => {
            deploy_confirmation(ctx, ledger_key, *invocation_epoch, artifacts, rpc).await
        }
        Some(store::SideEffectPhase::ConfirmationObserved { .. }) => {
            deploy_output(ctx, ledger_key, artifacts).await
        }
        Some(store::SideEffectPhase::Ambiguous { .. }) => Ok(ErasedRunnerOutput::new(Vec::new())),
        Some(other) => Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "unsupported EVM deploy side-effect phase: {other:?}"
        ))),
    }
}

async fn run_configure(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let ledger_key = ledger_key(&ctx, "configure")?;
    let projection = ctx.projections.side_effect(&ledger_key);
    match projection.map(|projection| &projection.phase) {
        None => configure_persist_intent(ctx, ledger_key, artifacts).await,
        Some(store::SideEffectPhase::IntentPersisted { .. }) => {
            claim_side_effect(ctx, ledger_key, projection.expect("projection"))
        }
        Some(store::SideEffectPhase::Claimed { .. }) => {
            configure_prepare_invocation(
                ctx,
                ledger_key,
                projection.expect("projection"),
                artifacts,
                rpc,
            )
            .await
        }
        Some(store::SideEffectPhase::InvocationPrepared { .. }) => {
            start_invocation(ctx, ledger_key, projection.expect("projection"))
        }
        Some(store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        })
        | Some(store::SideEffectPhase::SubmissionUnknown { invocation_epoch }) => {
            configure_submission(ctx, ledger_key, *invocation_epoch, artifacts, rpc).await
        }
        Some(store::SideEffectPhase::SubmissionObserved { invocation_epoch }) => {
            configure_receipt(ctx, ledger_key, *invocation_epoch, artifacts, rpc).await
        }
        Some(store::SideEffectPhase::ReceiptObserved { invocation_epoch }) => {
            configure_confirmation(ctx, ledger_key, *invocation_epoch, artifacts, rpc).await
        }
        Some(store::SideEffectPhase::ConfirmationObserved { .. }) => {
            configure_output(ctx, ledger_key, artifacts).await
        }
        Some(store::SideEffectPhase::Ambiguous { .. }) => Ok(ErasedRunnerOutput::new(Vec::new())),
        Some(other) => Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "unsupported EVM configure side-effect phase: {other:?}"
        ))),
    }
}

async fn deploy_persist_intent(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_config::<DeployConfigureValidateDeployConfig>(&ctx, artifacts).await?;
    let intent = deploy_intent_from_config(&config).map_err(runtime_invalid)?;
    let idem_input = EvmDcvDeployIdempotencyInput {
        transaction: intent.transaction.clone(),
    };
    persist_side_effect_intent::<EvmDcvDeployIntent, EvmDcvDeployIdempotencyInput>(
        ctx,
        ledger_key,
        artifacts,
        intent,
        idem_input,
        EvmDcvTransactionSubmitCapability::kind().map_err(runtime_capability_error)?,
        EvmDcvTransactionSubmitCapability::version().map_err(runtime_capability_error)?,
    )
    .await
}

async fn configure_persist_intent(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_config::<DeployConfigureValidateConfigureConfig>(&ctx, artifacts).await?;
    let deployed = load_input_cell::<DeployedContract>(ctx.inputs, artifacts).await?;
    let intent = configure_intent_from_config(&config, &deployed).map_err(runtime_invalid)?;
    let idem_input = EvmDcvConfigureIdempotencyInput {
        deployed: intent.deployed.clone(),
        transactions: intent.transactions.clone(),
    };
    persist_side_effect_intent::<EvmDcvConfigureIntent, EvmDcvConfigureIdempotencyInput>(
        ctx,
        ledger_key,
        artifacts,
        intent,
        idem_input,
        EvmDcvTransactionSubmitCapability::kind().map_err(runtime_capability_error)?,
        EvmDcvTransactionSubmitCapability::version().map_err(runtime_capability_error)?,
    )
    .await
}

async fn persist_side_effect_intent<I, D>(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    artifacts: &dyn EvmDcvArtifactStore,
    intent: I,
    idem_input: D,
    capability_kind: mfm_ids::CapabilityKind,
    capability_version: mfm_ids::CapabilityVersion,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    I: MfmValue + Serialize,
    D: MfmValue + Serialize,
{
    let intent_artifact = artifact_for_value(
        &intent,
        events::ArtifactRole::SideEffectIntent,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &intent_artifact).await?;
    let idem_hash = digest_value(&idem_input)?;
    let idempotency_key =
        events::IdempotencyKeyRef::new(format!("idem-{}", short_digest(&idem_hash)))?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![intent_artifact.evidence.clone()],
        staged_retention_refs: vec![retention(&intent_artifact.evidence)],
        payloads: vec![events::KernelEventPayload::SideEffectIntentPersisted(
            side_effect::IntentPersisted {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                scope_id: ctx.node.scope_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key: ledger_key.clone(),
                invocation_epoch: 1,
                intent_schema_id: I::schema_id().map_err(runtime_value_error)?,
                intent_hash: intent_artifact.evidence.digest.clone(),
                intent_artifact_id: intent_artifact.evidence.artifact_id.clone(),
                idempotency_input_schema_id: D::schema_id().map_err(runtime_value_error)?,
                idempotency_input_hash: idem_hash,
                idempotency_key,
                capability_kind,
                capability_version,
                adapter_kind: evm_dcv_adapter_kind()?,
                adapter_version: evm_dcv_adapter_version()?,
            },
        )],
    })
}

fn claim_side_effect(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    projection: &store::SideEffectProjection,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let invocation_epoch = match projection.phase {
        store::SideEffectPhase::IntentPersisted { invocation_epoch } => invocation_epoch,
        _ => {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "EVM side-effect claim requires persisted intent".to_owned(),
            ))
        }
    };
    Ok(ErasedRunnerOutput::new(vec![
        events::KernelEventPayload::SideEffectClaimed(side_effect::Claimed {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            ledger_key: ledger_key.clone(),
            claim_owner: events::RunnerInvocationId::new("mfm.evm.dcv.owner.1")?,
            invocation_epoch,
            claim_generation: 1,
            claim_fencing_token: side_effect::ClaimFencingToken::new("mfm.evm.dcv.token.1")?,
        }),
    ]))
}

async fn deploy_prepare_invocation(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    projection: &store::SideEffectProjection,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_config::<DeployConfigureValidateDeployConfig>(&ctx, artifacts).await?;
    let intent = load_intent_from_projection::<EvmDcvDeployIntent>(projection, artifacts).await?;
    let prepared = rpc
        .prepare_transactions(
            &config.network_id,
            config.signing_key_env.as_deref(),
            &intent.transaction.from,
            &[intent.transaction.clone()],
            config.poll_interval_ms,
            config.max_receipt_polls,
        )
        .await?;
    prepare_invocation(ctx, ledger_key, projection, artifacts, prepared).await
}

async fn configure_prepare_invocation(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    projection: &store::SideEffectProjection,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_config::<DeployConfigureValidateConfigureConfig>(&ctx, artifacts).await?;
    let intent =
        load_intent_from_projection::<EvmDcvConfigureIntent>(projection, artifacts).await?;
    let prepared = rpc
        .prepare_transactions(
            &config.network_id,
            config.signing_key_env.as_deref(),
            &config.from,
            &intent.transactions,
            config.poll_interval_ms,
            config.max_receipt_polls,
        )
        .await?;
    prepare_invocation(ctx, ledger_key, projection, artifacts, prepared).await
}

async fn prepare_invocation(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    projection: &store::SideEffectProjection,
    artifacts: &dyn EvmDcvArtifactStore,
    prepared: PreparedTransactions,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let claim = active_claim(projection)?;
    let prepared_artifact = artifact_for_json(
        &prepared,
        events::ArtifactRole::PreparedInvocation,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &prepared_artifact).await?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![prepared_artifact.evidence.clone()],
        staged_retention_refs: vec![retention(&prepared_artifact.evidence)],
        payloads: vec![events::KernelEventPayload::SideEffectInvocationPrepared(
            side_effect::InvocationPrepared {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key,
                invocation_epoch: claim.invocation_epoch,
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token.clone(),
                prepared_artifact_id: Some(prepared_artifact.evidence.artifact_id.clone()),
                prepared_hash: Some(prepared_artifact.evidence.digest.clone()),
            },
        )],
    })
}

fn start_invocation(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    projection: &store::SideEffectProjection,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let claim = active_claim(projection)?;
    Ok(ErasedRunnerOutput::new(vec![
        events::KernelEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
            spec_hash: ctx.spec_hash.clone(),
            node_id: ctx.node.node_id.clone(),
            attempt_id: ctx.attempt_id.clone(),
            ledger_key,
            invocation_epoch: claim.invocation_epoch,
            claim_owner: claim.claim_owner.clone(),
            claim_generation: claim.claim_generation,
            claim_fencing_token: claim.claim_fencing_token.clone(),
        }),
    ]))
}

async fn deploy_submission(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let projection = side_effect_projection(ctx.projections, &ledger_key)?;
    let prepared = load_prepared_transactions(&projection, artifacts).await?;
    match rpc.submit_prepared(&prepared).await? {
        PreparedSubmissionOutcome::Observed => {}
        PreparedSubmissionOutcome::Unknown(evidence) => {
            return observe_submission_unknown(
                ctx,
                ledger_key,
                invocation_epoch,
                artifacts,
                evidence,
            )
            .await;
        }
    }
    let tx = prepared.transactions.first().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput("deploy prepared no transaction".to_owned())
    })?;
    let submission = EvmDcvDeploySubmission {
        transaction_hash: tx.transaction_hash.clone(),
        idempotency_digest: projection.intent.idempotency_input_hash.as_str().to_owned(),
        protected_raw_transaction_artifact_id: projection
            .prepared_invocation
            .as_ref()
            .expect("prepared projection")
            .artifact_id
            .as_str()
            .to_owned(),
    };
    observe_submission::<EvmDcvDeploySubmission>(
        ctx,
        ledger_key,
        invocation_epoch,
        artifacts,
        submission,
    )
    .await
}

async fn configure_submission(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let projection = side_effect_projection(ctx.projections, &ledger_key)?;
    let prepared = load_prepared_transactions(&projection, artifacts).await?;
    match rpc.submit_prepared(&prepared).await? {
        PreparedSubmissionOutcome::Observed => {}
        PreparedSubmissionOutcome::Unknown(evidence) => {
            return observe_submission_unknown(
                ctx,
                ledger_key,
                invocation_epoch,
                artifacts,
                evidence,
            )
            .await;
        }
    }
    let prepared_id = projection
        .prepared_invocation
        .as_ref()
        .expect("prepared projection")
        .artifact_id
        .as_str()
        .to_owned();
    let submission = EvmDcvConfigureSubmission {
        transaction_hashes: prepared
            .transactions
            .iter()
            .map(|tx| tx.transaction_hash.clone())
            .collect(),
        idempotency_digest: projection.intent.idempotency_input_hash.as_str().to_owned(),
        protected_raw_transaction_artifact_ids: vec![prepared_id; prepared.transactions.len()],
    };
    observe_submission::<EvmDcvConfigureSubmission>(
        ctx,
        ledger_key,
        invocation_epoch,
        artifacts,
        submission,
    )
    .await
}

async fn observe_submission<T>(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    submission: T,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        &submission,
        events::ArtifactRole::Submission,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &artifact).await?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![artifact.evidence.clone()],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![events::KernelEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key,
                invocation_epoch,
                submission_schema_id: T::schema_id().map_err(runtime_value_error)?,
                submission_hash: artifact.evidence.digest,
                submission_artifact_id: artifact.evidence.artifact_id,
            },
        )],
    })
}

async fn observe_submission_unknown(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    evidence: EvmDcvSubmissionUnknownEvidence,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let artifact = artifact_for_value(
        &evidence,
        events::ArtifactRole::SubmissionUnknownEvidence,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &artifact).await?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![artifact.evidence.clone()],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![events::KernelEventPayload::SideEffectSubmissionUnknown(
            side_effect::SubmissionUnknown {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key,
                invocation_epoch,
                evidence_schema_id: EvmDcvSubmissionUnknownEvidence::schema_id()
                    .map_err(runtime_value_error)?,
                evidence_hash: artifact.evidence.digest,
                evidence_artifact_id: artifact.evidence.artifact_id,
            },
        )],
    })
}

async fn deploy_receipt(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let projection = side_effect_projection(ctx.projections, &ledger_key)?;
    let prepared = load_prepared_transactions(&projection, artifacts).await?;
    let tx = prepared.transactions.first().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput("deploy prepared no transaction".to_owned())
    })?;
    let receipt = rpc
        .wait_receipt(
            &prepared.network_id,
            &tx.transaction_hash,
            prepared.max_receipt_polls,
            prepared.poll_interval_ms,
        )
        .await?;
    let contract_address = receipt.contract_address.ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "deploy receipt did not include contract address".to_owned(),
        )
    })?;
    let receipt = EvmDcvDeployReceipt {
        transaction_hash: receipt.transaction_hash,
        contract_address,
        block_number: receipt.block_number,
        status: receipt.status,
    };
    observe_receipt::<EvmDcvDeployReceipt>(ctx, ledger_key, invocation_epoch, artifacts, receipt)
        .await
}

async fn configure_receipt(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let projection = side_effect_projection(ctx.projections, &ledger_key)?;
    let prepared = load_prepared_transactions(&projection, artifacts).await?;
    let mut receipts = Vec::with_capacity(prepared.transactions.len());
    for tx in &prepared.transactions {
        let receipt = rpc
            .wait_receipt(
                &prepared.network_id,
                &tx.transaction_hash,
                prepared.max_receipt_polls,
                prepared.poll_interval_ms,
            )
            .await?;
        receipts.push(EvmDcvConfigureReceiptEntry {
            transaction_hash: receipt.transaction_hash,
            block_number: receipt.block_number,
            status: receipt.status,
        });
    }
    observe_receipt::<EvmDcvConfigureReceipt>(
        ctx,
        ledger_key,
        invocation_epoch,
        artifacts,
        EvmDcvConfigureReceipt { receipts },
    )
    .await
}

async fn observe_receipt<T>(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    receipt: T,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        &receipt,
        events::ArtifactRole::Receipt,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &artifact).await?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![artifact.evidence.clone()],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![events::KernelEventPayload::SideEffectReceiptObserved(
            side_effect::ReceiptObserved {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key,
                invocation_epoch,
                receipt_schema_id: T::schema_id().map_err(runtime_value_error)?,
                receipt_hash: artifact.evidence.digest,
                receipt_artifact_id: artifact.evidence.artifact_id,
                replay_verifier_id: events::ReplayVerifierId::new("mfm.evm.dcv.replay.v1")?,
            },
        )],
    })
}

async fn deploy_confirmation(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let projection = side_effect_projection(ctx.projections, &ledger_key)?;
    let receipt_ref = projection.receipt.as_ref().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "deploy receipt projection missing".to_owned(),
        )
    })?;
    let receipt =
        load_artifact_value::<EvmDcvDeployReceipt>(&receipt_ref.artifact_id, artifacts).await?;
    let confirmations = rpc
        .confirmations(
            &load_prepared_transactions(&projection, artifacts)
                .await?
                .network_id,
            receipt.block_number,
        )
        .await?;
    let confirmation = EvmDcvDeployConfirmation {
        transaction_hash: receipt.transaction_hash,
        contract_address: receipt.contract_address,
        receipt_artifact_id: receipt_ref.artifact_id.as_str().to_owned(),
        block_number: receipt.block_number,
        confirmations,
    };
    observe_confirmation::<EvmDcvDeployConfirmation>(
        ctx,
        ledger_key,
        invocation_epoch,
        artifacts,
        confirmation,
    )
    .await
}

async fn configure_confirmation(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let projection = side_effect_projection(ctx.projections, &ledger_key)?;
    let receipt_ref = projection.receipt.as_ref().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "configure receipt projection missing".to_owned(),
        )
    })?;
    let receipt =
        load_artifact_value::<EvmDcvConfigureReceipt>(&receipt_ref.artifact_id, artifacts).await?;
    let configured_block_number = receipt
        .receipts
        .iter()
        .map(|receipt| receipt.block_number)
        .max();
    let network_id = load_prepared_transactions(&projection, artifacts)
        .await?
        .network_id;
    let confirmations = match configured_block_number {
        Some(block) => rpc.confirmations(&network_id, block).await?,
        None => 1,
    };
    let confirmation = EvmDcvConfigureConfirmation {
        transaction_hashes: receipt
            .receipts
            .iter()
            .map(|receipt| receipt.transaction_hash.clone())
            .collect(),
        receipt_artifact_ids: vec![receipt_ref.artifact_id.as_str().to_owned()],
        configured_block_number,
        confirmations,
    };
    observe_confirmation::<EvmDcvConfigureConfirmation>(
        ctx,
        ledger_key,
        invocation_epoch,
        artifacts,
        confirmation,
    )
    .await
}

async fn observe_confirmation<T>(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn EvmDcvArtifactStore,
    confirmation: T,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        &confirmation,
        events::ArtifactRole::Confirmation,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &artifact).await?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![artifact.evidence.clone()],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![events::KernelEventPayload::SideEffectConfirmationObserved(
            side_effect::ConfirmationObserved {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key,
                invocation_epoch,
                confirmation_schema_id: T::schema_id().map_err(runtime_value_error)?,
                confirmation_hash: artifact.evidence.digest,
                confirmation_artifact_id: artifact.evidence.artifact_id,
                replay_verifier_id: events::ReplayVerifierId::new("mfm.evm.dcv.replay.v1")?,
            },
        )],
    })
}

async fn deploy_output(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_config::<DeployConfigureValidateDeployConfig>(&ctx, artifacts).await?;
    let state = DeployContractState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let projection = side_effect_projection(ctx.projections, &ledger_key)?;
    let intent =
        load_artifact_value::<EvmDcvDeployIntent>(&projection.intent.intent_artifact_id, artifacts)
            .await?;
    let confirmation_ref = projection.confirmation.as_ref().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "deploy confirmation projection missing".to_owned(),
        )
    })?;
    let confirmation =
        load_artifact_value::<EvmDcvDeployConfirmation>(&confirmation_ref.artifact_id, artifacts)
            .await?;
    let output = state
        .output_from_confirmation(&(), &intent, &confirmation)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    state_output(ctx, artifacts, &output).await
}

async fn configure_output(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_config::<DeployConfigureValidateConfigureConfig>(&ctx, artifacts).await?;
    let input = load_input_cell::<DeployedContract>(ctx.inputs, artifacts).await?;
    let state = ConfigureContractState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let projection = side_effect_projection(ctx.projections, &ledger_key)?;
    let intent = load_artifact_value::<EvmDcvConfigureIntent>(
        &projection.intent.intent_artifact_id,
        artifacts,
    )
    .await?;
    let confirmation_ref = projection.confirmation.as_ref().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "configure confirmation projection missing".to_owned(),
        )
    })?;
    let confirmation = load_artifact_value::<EvmDcvConfigureConfirmation>(
        &confirmation_ref.artifact_id,
        artifacts,
    )
    .await?;
    let output = state
        .output_from_confirmation(&input, &intent, &confirmation)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    state_output(ctx, artifacts, &output).await
}

async fn run_validate(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn EvmDcvArtifactStore,
    rpc: &EvmDcvRpcClient,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_config::<DeployConfigureValidateValidateConfig>(&ctx, artifacts).await?;
    let input = load_input_cell::<ConfiguredContract>(ctx.inputs, artifacts).await?;
    let output = validate_configured_contract_with_backend(&config, &input, rpc)
        .await
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    state_output(ctx, artifacts, &output).await
}

async fn state_output<T>(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn EvmDcvArtifactStore,
    output: &T,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        output,
        events::ArtifactRole::StateOutput,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &artifact).await?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![artifact.evidence.clone()],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![cell_produced(&ctx, &artifact.evidence), completed(&ctx)],
    })
}

fn side_effect_projection<'a>(
    projections: &'a store::ProjectionSnapshot,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<&'a store::SideEffectProjection> {
    projections.side_effect(ledger_key).ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "missing EVM side-effect projection for {ledger_key}"
        ))
    })
}

fn ledger_key(
    ctx: &ErasedRunCtx<'_>,
    phase: &'static str,
) -> mfm_runtime::Result<events::SideEffectLedgerKey> {
    events::SideEffectLedgerKey::new(format!(
        "mfm.evm.dcv.{}.{}",
        phase,
        short_digest(&ctx.node.node_id)
    ))
    .map_err(Into::into)
}

fn cell_produced(
    ctx: &ErasedRunCtx<'_>,
    artifact: &store::ArtifactEvidenceRef,
) -> events::KernelEventPayload {
    events::KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: ctx.spec_hash.clone(),
        node_id: ctx.node.node_id.clone(),
        cell_id: ctx.node.output_cell.clone(),
        scope_id: ctx.output_cell.scope_id.clone(),
        attempt_id: ctx.attempt_id.clone(),
        semantic_type_id: ctx.output_cell.semantic_type_id.clone(),
        schema_id: ctx.output_cell.schema_id.clone(),
        value_lineage: ctx.output_cell.value_lineage.clone(),
        artifact_id: artifact.artifact_id.clone(),
        content_digest: artifact.digest.clone(),
        producer_state_kind: Some(ctx.node.state_kind.clone()),
        producer_state_version: Some(ctx.node.state_version.clone()),
    })
}

fn completed(ctx: &ErasedRunCtx<'_>) -> events::KernelEventPayload {
    events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: ctx.spec_hash.clone(),
        node_id: ctx.node.node_id.clone(),
        attempt_id: ctx.attempt_id.clone(),
        output_cell_id: ctx.node.output_cell.clone(),
    })
}

struct EvmDcvArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn artifact_for_value<T>(
    value: &T,
    role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
) -> mfm_runtime::Result<EvmDcvArtifact>
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
    Ok(EvmDcvArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

fn artifact_for_json<T>(
    value: &T,
    role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
) -> mfm_runtime::Result<EvmDcvArtifact>
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
    Ok(EvmDcvArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

async fn persist_artifact(
    artifacts: &dyn EvmDcvArtifactStore,
    artifact: &EvmDcvArtifact,
) -> mfm_runtime::Result<()> {
    artifacts
        .put_verified_artifact(artifact.bytes.clone(), artifact.evidence.clone())
        .await
}

async fn load_config<T>(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let (bytes, evidence) = artifacts
        .get_artifact_by_id(&ctx.node.config_ref.artifact_id)
        .await?;
    if evidence.digest != ctx.node.config_ref.digest
        || evidence.byte_len != ctx.node.config_ref.byte_len
        || evidence.media_type != ctx.node.config_ref.media_type
        || evidence.schema_id.as_ref() != Some(&ctx.node.config_ref.schema_id)
        || evidence.artifact_role != events::ArtifactRole::TypedConfig
    {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "EVM DCV config artifact did not match certified config ref for node {}",
            ctx.node.node_id
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn load_input_cell<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::Cell(cell) = &inputs.root else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM DCV input node was not a produced cell".to_owned(),
        ));
    };
    let (artifact_id, content_digest) = match &cell.terminal {
        MaterializedCellTerminal::Produced {
            artifact_id,
            content_digest,
        }
        | MaterializedCellTerminal::Seed {
            artifact_id,
            content_digest,
            ..
        } => (artifact_id, content_digest),
        MaterializedCellTerminal::Skipped { .. } => {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "EVM DCV input cell was skipped".to_owned(),
            ))
        }
    };
    let (bytes, evidence) = artifacts.get_artifact_by_id(artifact_id).await?;
    if &evidence.digest != content_digest {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM DCV input artifact digest did not match cell terminal".to_owned(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn load_artifact_value<T>(
    artifact_id: &ArtifactId,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<T>
where
    T: DeserializeOwned,
{
    let (bytes, _) = artifacts.get_artifact_by_id(artifact_id).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn load_intent_from_projection<T>(
    projection: &store::SideEffectProjection,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let (bytes, evidence) = artifacts
        .get_artifact_by_id(&projection.intent.intent_artifact_id)
        .await?;
    if evidence.digest != projection.intent.intent_hash
        || evidence.schema_id.as_ref() != Some(&T::schema_id().map_err(runtime_value_error)?)
        || evidence.semantic_type_id.as_ref()
            != Some(&T::semantic_id().map_err(runtime_value_error)?)
        || evidence.artifact_role != events::ArtifactRole::SideEffectIntent
    {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM side-effect intent artifact evidence did not match projection".to_owned(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn active_claim(
    projection: &store::SideEffectProjection,
) -> mfm_runtime::Result<&store::SideEffectClaimProjection> {
    projection.claim.as_ref().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM side-effect projection has no active claim".to_owned(),
        )
    })
}

async fn load_prepared_transactions(
    projection: &store::SideEffectProjection,
    artifacts: &dyn EvmDcvArtifactStore,
) -> mfm_runtime::Result<PreparedTransactions> {
    let prepared = projection.prepared_invocation.as_ref().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM side-effect prepared invocation projection missing".to_owned(),
        )
    })?;
    let (bytes, evidence) = artifacts.get_artifact_by_id(&prepared.artifact_id).await?;
    if evidence.digest != prepared.content_digest
        || evidence.artifact_role != events::ArtifactRole::PreparedInvocation
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
    {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "prepared invocation artifact metadata did not match projection".to_owned(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn retention(artifact: &store::ArtifactEvidenceRef) -> StagedRetentionRefs {
    StagedRetentionRefs {
        refs: vec![events::RetentionRef {
            artifact_id: artifact.artifact_id.clone(),
            role: artifact.artifact_role,
            content_digest: artifact.digest.clone(),
        }],
        reason: events::RetentionReason::RuntimeEvidence,
    }
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

fn short_digest(digest: &impl std::fmt::Display) -> String {
    let rendered = digest.to_string();
    rendered
        .rsplit(':')
        .next()
        .unwrap_or(rendered.as_str())
        .chars()
        .take(16)
        .collect()
}

fn runtime_value_error(error: mfm_values::ValueError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_capability_error(error: mfm_capabilities::CapabilityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_invalid(message: String) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(message)
}

#[derive(Clone, Serialize, Deserialize)]
struct PreparedTransactions {
    network_id: String,
    poll_interval_ms: u64,
    max_receipt_polls: u64,
    transactions: Vec<PreparedTransaction>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PreparedTransaction {
    transaction_index: u64,
    raw_transaction_hex: String,
    transaction_hash: String,
}

#[derive(Clone, Deserialize)]
struct EnvRpcSource {
    id: String,
    network_id: Option<String>,
    rpc_url: String,
    authorization: Option<String>,
}

#[derive(Clone)]
struct RpcSource {
    network_id: String,
    rpc_url: String,
    authorization: Option<String>,
}

#[derive(Clone)]
struct EvmDcvRpcClient {
    client: reqwest::Client,
    sources: Vec<RpcSource>,
}

enum PreparedSubmissionOutcome {
    Observed,
    Unknown(EvmDcvSubmissionUnknownEvidence),
}

impl EvmDcvRpcClient {
    fn from_env() -> Self {
        let sources = std::env::var(ENV_EVM_RPC_SOURCES_JSON)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<EnvRpcSource>>(&raw).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|source| {
                let network_id = source.network_id?;
                if source.id.trim().is_empty()
                    || network_id.trim().is_empty()
                    || source.rpc_url.trim().is_empty()
                {
                    return None;
                }
                Some(RpcSource {
                    network_id,
                    rpc_url: source.rpc_url,
                    authorization: source.authorization,
                })
            })
            .collect();
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            sources,
        }
    }

    async fn prepare_transactions(
        &self,
        network_id: &str,
        signing_key_env: Option<&str>,
        expected_from: &str,
        intents: &[EvmDcvTransactionIntent],
        poll_interval_ms: u64,
        max_receipt_polls: u64,
    ) -> mfm_runtime::Result<PreparedTransactions> {
        let signing_key_env = signing_key_env.ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "typed EVM DCV side effects require signing_key_env".to_owned(),
            )
        })?;
        let raw_key = Zeroizing::new(std::env::var(signing_key_env).map_err(|_| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "configured EVM signing key environment variable was not available".to_owned(),
            )
        })?);
        let key = EthereumPrivateKey::from_hex_secret(raw_key.as_str()).map_err(|_| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "configured EVM signing key was invalid".to_owned(),
            )
        })?;
        let derived = format!(
            "{:?}",
            key.address().map_err(|_| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "configured EVM signing key address derivation failed".to_owned(),
                )
            })?
        );
        if !derived.eq_ignore_ascii_case(expected_from) {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "configured EVM signing key did not match from address".to_owned(),
            ));
        }

        let chain_id = self.chain_id(network_id).await?;
        let mut nonce = self.transaction_count(network_id, expected_from).await?;
        let gas_price_wei = self.gas_price(network_id).await?;
        let mut prepared = Vec::with_capacity(intents.len());
        for intent in intents {
            let tx = self
                .legacy_transaction(network_id, intent, chain_id, nonce, gas_price_wei)
                .await?;
            let signing_hash = legacy_signing_hash(&tx);
            let mut hash = [0u8; 32];
            hash.copy_from_slice(signing_hash.as_slice());
            let signature = key.sign_hash_recoverable(&hash).map_err(|_| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "failed to sign EVM transaction".to_owned(),
                )
            })?;
            let raw_transaction_hex = encode_signed_legacy_tx_hex(&tx, signature);
            let transaction_hash = raw_transaction_hash(&raw_transaction_hex).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            prepared.push(PreparedTransaction {
                transaction_index: intent.transaction_index,
                raw_transaction_hex,
                transaction_hash,
            });
            nonce = nonce.saturating_add(1);
        }

        Ok(PreparedTransactions {
            network_id: network_id.to_owned(),
            poll_interval_ms,
            max_receipt_polls,
            transactions: prepared,
        })
    }

    async fn legacy_transaction(
        &self,
        network_id: &str,
        intent: &EvmDcvTransactionIntent,
        chain_id: u64,
        nonce: u64,
        gas_price_wei: u128,
    ) -> mfm_runtime::Result<LegacyTxToSign> {
        let gas_limit = self.estimate_gas(network_id, intent).await?;
        let data = parse_data_hex(&intent.data_hex)
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        let to = intent
            .to
            .as_deref()
            .map(|address| parse_address(address, "to"))
            .transpose()
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        let value_wei = parse_u128_quantity(&intent.value_hex, "value")
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        Ok(LegacyTxToSign {
            to,
            value_wei,
            chain_id,
            nonce,
            gas_price_wei,
            gas_limit,
            data,
        })
    }

    async fn submit_prepared(
        &self,
        prepared: &PreparedTransactions,
    ) -> mfm_runtime::Result<PreparedSubmissionOutcome> {
        for tx in &prepared.transactions {
            match self
                .call(
                    &prepared.network_id,
                    "eth_sendRawTransaction",
                    serde_json::json!([tx.raw_transaction_hex]),
                )
                .await
            {
                Ok(value) => {
                    let observed = value.as_str().ok_or_else(|| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(
                            "eth_sendRawTransaction returned non-string transaction hash"
                                .to_owned(),
                        )
                    })?;
                    if !observed.eq_ignore_ascii_case(&tx.transaction_hash) {
                        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                            "eth_sendRawTransaction returned unexpected transaction hash"
                                .to_owned(),
                        ));
                    }
                }
                Err(_) => {
                    if self
                        .transaction_receipt(&prepared.network_id, &tx.transaction_hash)
                        .await?
                        .is_none()
                    {
                        return Ok(PreparedSubmissionOutcome::Unknown(
                            EvmDcvSubmissionUnknownEvidence {
                                network_id: prepared.network_id.clone(),
                                transaction_hashes: prepared
                                    .transactions
                                    .iter()
                                    .map(|tx| tx.transaction_hash.clone())
                                    .collect(),
                                unknown_transaction_hash: tx.transaction_hash.clone(),
                                reason_code: "send_raw_transaction_unobserved".to_owned(),
                            },
                        ));
                    }
                }
            }
        }
        Ok(PreparedSubmissionOutcome::Observed)
    }

    async fn wait_receipt(
        &self,
        network_id: &str,
        tx_hash: &str,
        max_polls: u64,
        poll_interval_ms: u64,
    ) -> mfm_runtime::Result<ObservedReceipt> {
        for _ in 0..max_polls.max(1) {
            if let Some(receipt) = self.transaction_receipt(network_id, tx_hash).await? {
                if !receipt.status {
                    return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                        "EVM transaction receipt status was failed".to_owned(),
                    ));
                }
                return Ok(receipt);
            }
            if poll_interval_ms > 0 {
                tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
            }
        }
        Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "timed out waiting for EVM transaction receipt".to_owned(),
        ))
    }

    async fn confirmations(&self, network_id: &str, block_number: u64) -> mfm_runtime::Result<u64> {
        let current = self.block_number(network_id).await?;
        Ok(current
            .saturating_sub(block_number)
            .saturating_add(1)
            .max(1))
    }

    async fn block_number(&self, network_id: &str) -> mfm_runtime::Result<u64> {
        let value = self
            .call(network_id, "eth_blockNumber", serde_json::json!([]))
            .await?;
        value
            .as_str()
            .ok_or_else(|| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "eth_blockNumber returned non-string value".to_owned(),
                )
            })
            .and_then(parse_hex_u64_runtime)
    }

    async fn transaction_count(&self, network_id: &str, from: &str) -> mfm_runtime::Result<u64> {
        let value = self
            .call(
                network_id,
                "eth_getTransactionCount",
                serde_json::json!([from, "pending"]),
            )
            .await?;
        let raw = value.as_str().ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "eth_getTransactionCount returned non-string nonce".to_owned(),
            )
        })?;
        parse_u64_quantity(raw, "nonce")
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
    }

    async fn gas_price(&self, network_id: &str) -> mfm_runtime::Result<u128> {
        let value = self
            .call(network_id, "eth_gasPrice", serde_json::json!([]))
            .await?;
        let raw = value.as_str().ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "eth_gasPrice returned non-string gas price".to_owned(),
            )
        })?;
        parse_u128_quantity(raw, "gas_price")
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
    }

    async fn estimate_gas(
        &self,
        network_id: &str,
        intent: &EvmDcvTransactionIntent,
    ) -> mfm_runtime::Result<u64> {
        let mut call = serde_json::Map::new();
        call.insert("from".to_owned(), serde_json::json!(intent.from));
        if let Some(to) = &intent.to {
            call.insert("to".to_owned(), serde_json::json!(to));
        }
        call.insert("data".to_owned(), serde_json::json!(intent.data_hex));
        call.insert("value".to_owned(), serde_json::json!(intent.value_hex));
        let value = self
            .call(
                network_id,
                "eth_estimateGas",
                serde_json::json!([serde_json::Value::Object(call)]),
            )
            .await?;
        let raw = value.as_str().ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "eth_estimateGas returned non-string gas".to_owned(),
            )
        })?;
        let estimate = parse_u64_quantity(raw, "gas")
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        Ok(estimate.saturating_add(estimate / 5).max(21_000))
    }

    async fn transaction_receipt(
        &self,
        network_id: &str,
        tx_hash: &str,
    ) -> mfm_runtime::Result<Option<ObservedReceipt>> {
        let value = self
            .call(
                network_id,
                "eth_getTransactionReceipt",
                serde_json::json!([tx_hash]),
            )
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        let block_number = value
            .get("blockNumber")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "transaction receipt missing blockNumber".to_owned(),
                )
            })
            .and_then(parse_hex_u64_runtime)?;
        let status = value
            .get("status")
            .and_then(|value| value.as_str())
            .map(|status| status == "0x1")
            .unwrap_or(false);
        let transaction_hash = value
            .get("transactionHash")
            .and_then(|value| value.as_str())
            .unwrap_or(tx_hash)
            .to_owned();
        let contract_address = value
            .get("contractAddress")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
        Ok(Some(ObservedReceipt {
            transaction_hash,
            contract_address,
            block_number,
            status,
        }))
    }

    async fn call(
        &self,
        network_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> mfm_runtime::Result<serde_json::Value> {
        let source = self
            .sources
            .iter()
            .find(|source| source.network_id == network_id)
            .ok_or_else(|| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                    "no typed EVM RPC source configured for network `{network_id}`"
                ))
            })?;
        let mut request = self.client.post(&source.rpc_url).json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1u64,
            "method": method,
            "params": params,
        }));
        if let Some(authorization) = &source.authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        let response = request.send().await.map_err(|_| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "EVM JSON-RPC request `{method}` failed"
            ))
        })?;
        if !response.status().is_success() {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "EVM JSON-RPC request `{method}` returned non-success status"
            )));
        }
        let body = response.json::<serde_json::Value>().await.map_err(|_| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "EVM JSON-RPC response for `{method}` was invalid JSON"
            ))
        })?;
        if body.get("error").is_some() {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "EVM JSON-RPC method `{method}` returned an error"
            )));
        }
        body.get("result").cloned().ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "EVM JSON-RPC response for `{method}` had no result"
            ))
        })
    }
}

impl EvmDcvReadBackend for EvmDcvRpcClient {
    fn chain_id<'a>(&'a self, network_id: &'a str) -> EvmDcvReadFuture<'a, u64> {
        Box::pin(async move {
            EvmDcvRpcClient::chain_id(self, network_id)
                .await
                .map_err(runtime_to_read_error)
        })
    }

    fn client_version<'a>(&'a self, network_id: &'a str) -> EvmDcvReadFuture<'a, String> {
        Box::pin(async move {
            let value = self
                .call(network_id, "web3_clientVersion", serde_json::json!([]))
                .await
                .map_err(runtime_to_read_error)?;
            value.as_str().map(str::to_owned).ok_or_else(|| {
                EvmDcvReadError::new(
                    "evm_response_invalid",
                    "web3_clientVersion response was not a string",
                )
            })
        })
    }

    fn eth_call<'a>(
        &'a self,
        network_id: &'a str,
        to: &'a str,
        data_hex: &'a str,
    ) -> EvmDcvReadFuture<'a, String> {
        Box::pin(async move {
            let value = self
                .call(
                    network_id,
                    "eth_call",
                    serde_json::json!([{"to": to, "data": data_hex}, "latest"]),
                )
                .await
                .map_err(runtime_to_read_error)?;
            value.as_str().map(str::to_owned).ok_or_else(|| {
                EvmDcvReadError::new("evm_response_invalid", "eth_call response was not a string")
            })
        })
    }

    fn log_count<'a>(
        &'a self,
        network_id: &'a str,
        address: &'a str,
        topic0_hex: &'a str,
        from_block: &'a serde_json::Value,
        to_block: &'a serde_json::Value,
    ) -> EvmDcvReadFuture<'a, u64> {
        Box::pin(async move {
            let value = self
                .call(
                    network_id,
                    "eth_getLogs",
                    serde_json::json!([{
                        "address": address,
                        "topics": [topic0_hex],
                        "fromBlock": from_block,
                        "toBlock": to_block,
                    }]),
                )
                .await
                .map_err(runtime_to_read_error)?;
            value
                .as_array()
                .map(|logs| logs.len() as u64)
                .ok_or_else(|| {
                    EvmDcvReadError::new(
                        "evm_response_invalid",
                        "eth_getLogs response was not an array",
                    )
                })
        })
    }
}

impl EvmDcvRpcClient {
    async fn chain_id(&self, network_id: &str) -> mfm_runtime::Result<u64> {
        let value = self
            .call(network_id, "eth_chainId", serde_json::json!([]))
            .await?;
        value
            .as_str()
            .ok_or_else(|| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "eth_chainId returned non-string value".to_owned(),
                )
            })
            .and_then(parse_hex_u64_runtime)
    }
}

#[derive(Debug, Clone)]
struct ObservedReceipt {
    transaction_hash: String,
    contract_address: Option<String>,
    block_number: u64,
    status: bool,
}

fn parse_hex_u64_runtime(raw: &str) -> mfm_runtime::Result<u64> {
    let Some(rest) = raw.strip_prefix("0x") else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "hex quantity missing 0x prefix".to_owned(),
        ));
    };
    if rest.is_empty() || rest.len() > 16 || !rest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "hex quantity was not a u64".to_owned(),
        ));
    }
    u64::from_str_radix(rest, 16).map_err(|_| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput("hex quantity was not a u64".to_owned())
    })
}

fn runtime_to_read_error(error: mfm_runtime::RuntimeError) -> EvmDcvReadError {
    EvmDcvReadError::new("evm_rpc_error", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn short_digest_uses_digest_suffix_not_identity_prefix() {
        let content = format!("content:sha256-jcs-v1:{DIGEST_HEX}")
            .parse::<ContentDigest>()
            .expect("content digest");
        let node = format!("node:sha256-jcs-v1:{DIGEST_HEX}")
            .parse::<NodeId>()
            .expect("node id");

        assert_eq!(short_digest(&content), "0123456789abcdef");
        assert_eq!(short_digest(&node), "0123456789abcdef");
    }

    #[test]
    fn prepared_invocation_artifact_is_non_semantic() {
        let prepared = PreparedTransactions {
            network_id: "local".to_owned(),
            poll_interval_ms: 1,
            max_receipt_polls: 1,
            transactions: vec![PreparedTransaction {
                transaction_index: 0,
                raw_transaction_hex: "0xdeadbeef".to_owned(),
                transaction_hash: "0xhash".to_owned(),
            }],
        };

        let artifact = artifact_for_json(&prepared, events::ArtifactRole::PreparedInvocation, None)
            .expect("prepared artifact");
        assert_eq!(
            artifact.evidence.artifact_role,
            events::ArtifactRole::PreparedInvocation
        );
        assert!(artifact.evidence.schema_id.is_none());
        assert!(artifact.evidence.semantic_type_id.is_none());
    }

    #[test]
    fn submission_unknown_evidence_is_typed_without_raw_transaction() {
        let evidence = EvmDcvSubmissionUnknownEvidence {
            network_id: "local".to_owned(),
            transaction_hashes: vec!["0xhash".to_owned()],
            unknown_transaction_hash: "0xhash".to_owned(),
            reason_code: "send_raw_transaction_unobserved".to_owned(),
        };

        let artifact = artifact_for_value(
            &evidence,
            events::ArtifactRole::SubmissionUnknownEvidence,
            None,
        )
        .expect("unknown evidence artifact");
        assert_eq!(
            artifact.evidence.artifact_role,
            events::ArtifactRole::SubmissionUnknownEvidence
        );
        assert_eq!(
            artifact.evidence.schema_id,
            Some(EvmDcvSubmissionUnknownEvidence::schema_id().expect("schema"))
        );
        let json: serde_json::Value =
            serde_json::from_slice(&artifact.bytes).expect("evidence json");
        assert!(json.get("raw_transaction_hex").is_none());
    }
}
