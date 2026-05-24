#![warn(missing_docs)]
//! Deterministic typed proof implementation.
//!
//! This crate provides the enabled proof implementation used by conformance tests and by typed
//! runner assembly. It exposes typed runtime runners and replay verifiers only; it does not expose
//! legacy live-IO transports or generic request/response namespaces.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::CapabilitySpec;
use mfm_collectors_proof::{
    proof_adapter_kind, proof_adapter_version, ProofApplyConfig, ProofApplySideEffectState,
    ProofAssembleConfig, ProofAssembleOutputState, ProofConfirmation, ProofFact, ProofFactRequest,
    ProofFactResponse, ProofIdempotencyInput, ProofIntent, ProofMutationCapability, ProofOutput,
    ProofReadCapability, ProofReadConfig, ProofReadFactState, ProofReceipt, ProofReplayError,
    ProofReplayVerifier, ProofSideEffectResult, ProofSubmission, ProofWorkflowConfig,
    RecordedProofFacts,
};
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{
    ArtifactId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, NodeId, RunId, SchemaId,
};
use mfm_op_proof::{certified_proof_spec, proof_program_draft};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    build_public_output_receipt_artifact, CertifiedRuntimeSpec, ErasedNodeRunner, ErasedRunCtx,
    ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    MaterializedCellTerminal, MaterializedInputNode, RunStartEvidence, SchedulerStatus,
    SerialTypedScheduler, StagedRetentionRefs,
};
use mfm_spec::v1 as spec;
use mfm_store::v1::{self as store, TypedRunEventStore};
use mfm_values::{MfmConfig, MfmValue};
use serde::Serialize;

const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const PURE_FACTORY: &str = "pure";
const REPLAY_VERIFIER_ID: &str = "mfm.proof.replay.deterministic.v1";

type ProofArtifactRecord = (Vec<u8>, store::ArtifactEvidenceRef);
type ProofArtifactMap = BTreeMap<ArtifactId, ProofArtifactRecord>;

/// Future returned by proof artifact sinks.
pub type ProofArtifactSinkFuture<'a> =
    Pin<Box<dyn Future<Output = mfm_runtime::Result<()>> + Send + 'a>>;

/// Stores artifact bytes before proof runners commit typed evidence.
pub trait ProofArtifactSink: Send + Sync {
    /// Persists bytes after validating they match the supplied typed evidence.
    fn put_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> ProofArtifactSinkFuture<'a>;
}

#[derive(Clone, Default)]
struct InMemoryProofArtifactSink {
    artifacts: Arc<Mutex<ProofArtifactMap>>,
}

impl InMemoryProofArtifactSink {
    fn evidence_for(&self, artifact_id: &ArtifactId) -> Option<store::ArtifactEvidenceRef> {
        self.artifacts.lock().ok().and_then(|artifacts| {
            artifacts
                .get(artifact_id)
                .map(|(_, evidence)| evidence.clone())
        })
    }
}

impl ProofArtifactSink for InMemoryProofArtifactSink {
    fn put_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> ProofArtifactSinkFuture<'a> {
        Box::pin(async move {
            let digest = ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(&bytes),
            );
            if evidence.digest != digest
                || evidence.byte_len != bytes.len() as u64
                || evidence.artifact_id
                    != ArtifactId::from_digest(digest.algorithm(), *digest.digest())
            {
                return Err(mfm_runtime::RuntimeError::Store(
                    "proof artifact bytes do not match typed evidence".to_owned(),
                ));
            }
            let mut artifacts = self.artifacts.lock().map_err(|_| {
                mfm_runtime::RuntimeError::Store("proof artifact sink lock was poisoned".to_owned())
            })?;
            if let Some((existing_bytes, existing_evidence)) = artifacts.get(&evidence.artifact_id)
            {
                if existing_bytes != &bytes || existing_evidence != &evidence {
                    return Err(mfm_runtime::RuntimeError::Store(format!(
                        "conflicting proof artifact evidence for {}",
                        evidence.artifact_id
                    )));
                }
                return Ok(());
            }
            artifacts.insert(evidence.artifact_id.clone(), (bytes, evidence));
            Ok(())
        })
    }
}

/// Summary emitted by the proof implementation conformance fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofImplementationConformanceSummary {
    /// Implementation name.
    pub implementation: String,
    /// Typed facts were recorded and replay-verified.
    pub facts_valid: bool,
    /// Typed submission evidence was recorded and replay-verified.
    pub submission_valid: bool,
    /// Typed receipt evidence was recorded and replay-verified.
    pub receipt_valid: bool,
    /// Typed confirmation evidence was recorded and replay-verified.
    pub confirmation_valid: bool,
    /// Replay verifier used only recorded proof evidence.
    pub replay_valid: bool,
}

impl ProofImplementationConformanceSummary {
    /// Converts the conformance result into the RFC summary shape.
    pub fn to_summary_document(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": "proof-implementation-conformance",
            "version": 1,
            "implementation": self.implementation,
            "payload": {
                "facts_valid": self.facts_valid,
                "submission_valid": self.submission_valid,
                "receipt_valid": self.receipt_valid,
                "confirmation_valid": self.confirmation_valid,
                "replay_valid": self.replay_valid,
            }
        })
    }

    fn validate(&self) -> Result<(), String> {
        for (key, value) in [
            ("facts_valid", self.facts_valid),
            ("submission_valid", self.submission_valid),
            ("receipt_valid", self.receipt_valid),
            ("confirmation_valid", self.confirmation_valid),
            ("replay_valid", self.replay_valid),
        ] {
            if !value {
                return Err(format!(
                    "proof implementation conformance key is false: {key}"
                ));
            }
        }
        Ok(())
    }
}

/// Registers deterministic typed proof runners.
pub fn register_deterministic_proof_runners(
    registry: &mut ErasedRunnerRegistry,
    artifacts: Arc<dyn ProofArtifactSink>,
) -> mfm_runtime::Result<()> {
    let read = registered_descriptor::<ProofReadFactState>()?;
    let side_effect = registered_descriptor::<ProofApplySideEffectState>()?;
    let assemble = registered_descriptor::<ProofAssembleOutputState>()?;

    registry.register(binding(
        read,
        READ_FACTORY,
        Arc::new(ProofReadRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    registry.register(binding(
        side_effect,
        SIDE_EFFECT_FACTORY,
        Arc::new(ProofSideEffectRunner {
            artifacts: artifacts.clone(),
        }),
    )?)?;
    registry.register(binding(
        assemble,
        PURE_FACTORY,
        Arc::new(ProofAssembleRunner { artifacts }),
    )?)?;
    Ok(())
}

/// Builds a runner registry containing only the deterministic proof implementation.
pub fn deterministic_proof_runner_registry(
    artifacts: Arc<dyn ProofArtifactSink>,
) -> mfm_runtime::Result<ErasedRunnerRegistry> {
    let mut registry = ErasedRunnerRegistry::new();
    register_deterministic_proof_runners(&mut registry, artifacts)?;
    Ok(registry)
}

fn registered_descriptor<S>() -> mfm_runtime::Result<DescriptorId>
where
    S: mfm_program::StateSpec,
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
        source_revision: events::SourceRevision::new("mfm-transports-proof-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-transports-proof")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: digest_json(serde_json::json!({
            "crate": "mfm-transports-proof",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        binary_digest: digest_json(serde_json::json!({
            "crate": "mfm-transports-proof",
            "runner": "deterministic-proof",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct ProofReadRunner {
    artifacts: Arc<dyn ProofArtifactSink>,
}

impl ErasedNodeRunner for ProofReadRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        let artifacts = self.artifacts.clone();
        Box::pin(async move { run_read(ctx, artifacts.as_ref()).await })
    }
}

struct ProofSideEffectRunner {
    artifacts: Arc<dyn ProofArtifactSink>,
}

impl ErasedNodeRunner for ProofSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        let artifacts = self.artifacts.clone();
        Box::pin(async move { run_side_effect(ctx, artifacts.as_ref()).await })
    }
}

struct ProofAssembleRunner {
    artifacts: Arc<dyn ProofArtifactSink>,
}

impl ErasedNodeRunner for ProofAssembleRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        let artifacts = self.artifacts.clone();
        Box::pin(async move { run_assemble(ctx, artifacts.as_ref()).await })
    }
}

async fn run_read(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn ProofArtifactSink,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    ensure_config::<ProofReadConfig>(&ctx.node.config_ref, &ProofReadConfig { fact_n: 1 })?;
    let fact = ProofFact { n: 1 };
    let request = ProofFactRequest {
        source: "deterministic-proof".to_owned(),
    };
    let response = ProofFactResponse { fact: fact.clone() };
    let request_hash = digest_value(&request)?;
    let response_artifact = artifact_for_value(
        &response,
        events::ArtifactRole::FactResponse,
        Some(ctx.node.node_id.clone()),
    )?;
    let output_artifact = artifact_for_value(
        &fact,
        events::ArtifactRole::StateOutput,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &response_artifact).await?;
    persist_artifact(artifacts, &output_artifact).await?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![
            response_artifact.evidence.clone(),
            output_artifact.evidence.clone(),
        ],
        staged_retention_refs: vec![
            retention(&response_artifact.evidence),
            retention(&output_artifact.evidence),
        ],
        payloads: vec![
            events::KernelEventPayload::FactRecorded(events::FactRecorded {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                capability_kind: ProofReadCapability::kind().map_err(runtime_capability_error)?,
                capability_version: ProofReadCapability::version()
                    .map_err(runtime_capability_error)?,
                adapter_kind: proof_adapter_kind()?,
                adapter_version: proof_adapter_version()?,
                request_schema_id: ProofFactRequest::schema_id().map_err(runtime_value_error)?,
                request_hash,
                response_schema_id: ProofFactResponse::schema_id().map_err(runtime_value_error)?,
                response_hash: response_artifact.evidence.digest.clone(),
                fact_key: events::FactKey::new("mfm.proof.fact.default")?,
                artifact_id: response_artifact.evidence.artifact_id.clone(),
            }),
            cell_produced(&ctx, &output_artifact.evidence),
            completed(&ctx),
        ],
    })
}

async fn run_side_effect(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn ProofArtifactSink,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    ensure_config::<ProofApplyConfig>(
        &ctx.node.config_ref,
        &ProofApplyConfig {
            action: "accept".to_owned(),
        },
    )?;
    let ledger_key = events::SideEffectLedgerKey::new("mfm.proof.ledger.default")?;
    let projection = ctx.projections.side_effect(&ledger_key);
    match projection.map(|projection| &projection.phase) {
        None => side_effect_prepare(ctx, ledger_key, artifacts).await,
        Some(store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        })
        | Some(store::SideEffectPhase::SubmissionUnknown { invocation_epoch }) => {
            side_effect_submission(ctx, ledger_key, *invocation_epoch, artifacts).await
        }
        Some(store::SideEffectPhase::SubmissionObserved { invocation_epoch }) => {
            side_effect_receipt(ctx, ledger_key, *invocation_epoch, artifacts).await
        }
        Some(store::SideEffectPhase::ReceiptObserved { invocation_epoch }) => {
            side_effect_confirmation(ctx, ledger_key, *invocation_epoch, artifacts).await
        }
        Some(store::SideEffectPhase::ConfirmationObserved { .. }) => {
            side_effect_output(ctx, artifacts).await
        }
        Some(store::SideEffectPhase::Ambiguous { .. }) => Ok(ErasedRunnerOutput::new(Vec::new())),
        Some(other) => Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "unsupported proof side-effect phase: {other:?}"
        ))),
    }
}

async fn side_effect_prepare(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    artifacts: &dyn ProofArtifactSink,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let intent = proof_intent();
    let idem_input = proof_idempotency_input();
    let intent_artifact = artifact_for_value(
        &intent,
        events::ArtifactRole::SideEffectIntent,
        Some(ctx.node.node_id.clone()),
    )?;
    persist_artifact(artifacts, &intent_artifact).await?;
    let idem_hash = digest_value(&idem_input)?;
    let idempotency_key =
        events::IdempotencyKeyRef::new(format!("idem-{}", short_digest(&idem_hash)))?;
    let owner = events::RunnerInvocationId::new("mfm.proof.owner.1")?;
    let token = side_effect::ClaimFencingToken::new("mfm.proof.token.1")?;
    Ok(ErasedRunnerOutput {
        required_artifacts: vec![intent_artifact.evidence.clone()],
        staged_retention_refs: vec![retention(&intent_artifact.evidence)],
        payloads: vec![
            events::KernelEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                scope_id: ctx.node.scope_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key: ledger_key.clone(),
                invocation_epoch: 1,
                intent_schema_id: ProofIntent::schema_id().map_err(runtime_value_error)?,
                intent_hash: intent_artifact.evidence.digest.clone(),
                intent_artifact_id: intent_artifact.evidence.artifact_id.clone(),
                idempotency_input_schema_id: ProofIdempotencyInput::schema_id()
                    .map_err(runtime_value_error)?,
                idempotency_input_hash: idem_hash,
                idempotency_key,
                capability_kind: ProofMutationCapability::kind()
                    .map_err(runtime_capability_error)?,
                capability_version: ProofMutationCapability::version()
                    .map_err(runtime_capability_error)?,
                adapter_kind: proof_adapter_kind()?,
                adapter_version: proof_adapter_version()?,
            }),
            events::KernelEventPayload::SideEffectClaimed(side_effect::Claimed {
                spec_hash: ctx.spec_hash.clone(),
                node_id: ctx.node.node_id.clone(),
                attempt_id: ctx.attempt_id.clone(),
                ledger_key: ledger_key.clone(),
                claim_owner: owner.clone(),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
            }),
            events::KernelEventPayload::SideEffectInvocationPrepared(
                side_effect::InvocationPrepared {
                    spec_hash: ctx.spec_hash.clone(),
                    node_id: ctx.node.node_id.clone(),
                    attempt_id: ctx.attempt_id.clone(),
                    ledger_key: ledger_key.clone(),
                    invocation_epoch: 1,
                    claim_generation: 1,
                    claim_fencing_token: token.clone(),
                    prepared_artifact_id: None,
                    prepared_hash: None,
                },
            ),
            events::KernelEventPayload::SideEffectInvocationStarted(
                side_effect::InvocationStarted {
                    spec_hash: ctx.spec_hash.clone(),
                    node_id: ctx.node.node_id.clone(),
                    attempt_id: ctx.attempt_id.clone(),
                    ledger_key,
                    invocation_epoch: 1,
                    claim_owner: owner,
                    claim_generation: 1,
                    claim_fencing_token: token,
                },
            ),
        ],
    })
}

async fn side_effect_submission(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn ProofArtifactSink,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let submission = proof_submission()?;
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
                submission_schema_id: ProofSubmission::schema_id().map_err(runtime_value_error)?,
                submission_hash: artifact.evidence.digest,
                submission_artifact_id: artifact.evidence.artifact_id,
            },
        )],
    })
}

async fn side_effect_receipt(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn ProofArtifactSink,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let receipt = proof_receipt()?;
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
                receipt_schema_id: ProofReceipt::schema_id().map_err(runtime_value_error)?,
                receipt_hash: artifact.evidence.digest,
                receipt_artifact_id: artifact.evidence.artifact_id,
                replay_verifier_id: replay_verifier_id()?,
            },
        )],
    })
}

async fn side_effect_confirmation(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    artifacts: &dyn ProofArtifactSink,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let confirmation = proof_confirmation()?;
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
                confirmation_schema_id: ProofConfirmation::schema_id()
                    .map_err(runtime_value_error)?,
                confirmation_hash: artifact.evidence.digest,
                confirmation_artifact_id: artifact.evidence.artifact_id,
                replay_verifier_id: replay_verifier_id()?,
            },
        )],
    })
}

async fn side_effect_output(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn ProofArtifactSink,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let output = proof_side_effect_result()?;
    let artifact = artifact_for_value(
        &output,
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

async fn run_assemble(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn ProofArtifactSink,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    ensure_config::<ProofAssembleConfig>(
        &ctx.node.config_ref,
        &ProofAssembleConfig { output_version: 1 },
    )?;
    ensure_struct_input_digest(ctx.inputs, "fact", &digest_value(&proof_fact())?)?;
    ensure_struct_input_digest(
        ctx.inputs,
        "side_effect",
        &digest_value(&proof_side_effect_result()?)?,
    )?;
    let output = ProofOutput {
        fact: proof_fact(),
        side_effect: proof_side_effect_result()?,
    };
    let artifact = artifact_for_value(
        &output,
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

fn ensure_struct_input_digest(
    inputs: &mfm_runtime::MaterializedInputs,
    field: &str,
    expected: &ContentDigest,
) -> mfm_runtime::Result<()> {
    let MaterializedInputNode::Struct(fields) = &inputs.root else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "proof assemble input was not materialized as a struct".to_owned(),
        ));
    };
    let Some(found) = fields
        .iter()
        .find(|named| named.field_path.as_str() == field)
    else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "missing proof assemble input field {field}"
        )));
    };
    let MaterializedInputNode::Cell(cell) = &found.node else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "proof assemble input field {field} was not a cell"
        )));
    };
    match &cell.terminal {
        MaterializedCellTerminal::Produced { content_digest, .. } if content_digest == expected => {
            Ok(())
        }
        MaterializedCellTerminal::Produced { content_digest, .. } => {
            Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "proof assemble input field {field} digest {content_digest} did not match expected {expected}"
            )))
        }
        _ => Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "proof assemble input field {field} was not produced"
        ))),
    }
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

struct ProofArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn artifact_for_value<T>(
    value: &T,
    role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
) -> mfm_runtime::Result<ProofArtifact>
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
    Ok(ProofArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

async fn persist_artifact(
    artifacts: &dyn ProofArtifactSink,
    artifact: &ProofArtifact,
) -> mfm_runtime::Result<()> {
    artifacts
        .put_verified_artifact(artifact.bytes.clone(), artifact.evidence.clone())
        .await
        .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    Ok(())
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

fn ensure_config<T>(config: &spec::ConfigRef, expected: &T) -> mfm_runtime::Result<()>
where
    T: MfmConfig + Serialize,
{
    let bytes = canonical_value(expected)?;
    let digest = bytes.content_digest();
    let schema_id = T::schema_id().map_err(runtime_value_error)?;
    if config.schema_id == schema_id
        && config.digest == digest
        && config.byte_len == bytes.as_bytes().len() as u64
    {
        Ok(())
    } else {
        Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "proof runner config mismatch for schema {}",
            config.schema_id
        )))
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

fn runtime_value_error(error: mfm_values::ValueError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_capability_error(error: mfm_capabilities::CapabilityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn proof_fact() -> ProofFact {
    ProofFact { n: 1 }
}

fn proof_intent() -> ProofIntent {
    ProofIntent {
        fact_n: 1,
        action: "accept".to_owned(),
    }
}

fn proof_idempotency_input() -> ProofIdempotencyInput {
    ProofIdempotencyInput {
        fact_n: 1,
        action: "accept".to_owned(),
    }
}

fn proof_submission() -> mfm_runtime::Result<ProofSubmission> {
    let idempotency_digest = digest_value(&proof_idempotency_input())?
        .as_str()
        .to_owned();
    Ok(ProofSubmission {
        submission_id: "proof-submission-accept-1".to_owned(),
        idempotency_digest,
    })
}

fn proof_receipt() -> mfm_runtime::Result<ProofReceipt> {
    Ok(ProofReceipt {
        tx_hash: "0xproofaccept1".to_owned(),
        submission_id: proof_submission()?.submission_id,
    })
}

fn proof_confirmation() -> mfm_runtime::Result<ProofConfirmation> {
    Ok(ProofConfirmation {
        tx_hash: proof_receipt()?.tx_hash,
        confirmations: 1,
    })
}

fn proof_side_effect_result() -> mfm_runtime::Result<ProofSideEffectResult> {
    let confirmation = proof_confirmation()?;
    Ok(ProofSideEffectResult {
        tx_hash: confirmation.tx_hash,
        confirmations: confirmation.confirmations,
        status: "confirmed".to_owned(),
    })
}

fn short_digest(digest: &ContentDigest) -> String {
    digest
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or(digest.as_str())
        .chars()
        .take(16)
        .collect()
}

fn replay_verifier_id() -> mfm_runtime::Result<events::ReplayVerifierId> {
    Ok(events::ReplayVerifierId::new(REPLAY_VERIFIER_ID)?)
}

fn ensure_digest<T>(label: &str, actual: &ContentDigest, expected: &T) -> replay::Result<()>
where
    T: Serialize,
{
    let expected = digest_value(expected).map_err(replay_runtime_error)?;
    if actual == &expected {
        Ok(())
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            format!("{label} digest does not match deterministic proof evidence"),
        ))
    }
}

fn replay_runtime_error(error: mfm_runtime::RuntimeError) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn proof_replay_error(error: ProofReplayError) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::SideEffectMismatch, error.message)
}

/// Deterministic proof replay verifier.
pub struct DeterministicProofReplayVerifier {
    verifier_id: events::ReplayVerifierId,
}

impl DeterministicProofReplayVerifier {
    /// Creates the deterministic proof replay verifier.
    pub fn new() -> mfm_runtime::Result<Self> {
        Ok(Self {
            verifier_id: replay_verifier_id()?,
        })
    }
}

impl ProofReplayVerifier for DeterministicProofReplayVerifier {
    fn verify_submission(
        &self,
        intent: &ProofIntent,
        submission: &ProofSubmission,
        facts: &RecordedProofFacts,
    ) -> Result<(), ProofReplayError> {
        if intent.fact_n == facts.fact.n && submission.submission_id == "proof-submission-accept-1"
        {
            Ok(())
        } else {
            Err(ProofReplayError::new("proof submission mismatch"))
        }
    }

    fn verify_receipt(
        &self,
        intent: &ProofIntent,
        receipt: &ProofReceipt,
        facts: &RecordedProofFacts,
    ) -> Result<(), ProofReplayError> {
        if intent.fact_n == facts.fact.n && receipt.tx_hash == "0xproofaccept1" {
            Ok(())
        } else {
            Err(ProofReplayError::new("proof receipt mismatch"))
        }
    }

    fn verify_confirmation(
        &self,
        receipt: &ProofReceipt,
        confirmation: &ProofConfirmation,
        _facts: &RecordedProofFacts,
    ) -> Result<(), ProofReplayError> {
        if confirmation.tx_hash == receipt.tx_hash && confirmation.confirmations >= 1 {
            Ok(())
        } else {
            Err(ProofReplayError::new("proof confirmation mismatch"))
        }
    }
}

impl replay::SideEffectReplayVerifier for DeterministicProofReplayVerifier {
    fn verifier_id(&self) -> &events::ReplayVerifierId {
        &self.verifier_id
    }

    fn verify_submission(
        &self,
        input: &replay::SideEffectSubmissionReplayInput,
    ) -> replay::Result<()> {
        ensure_digest(
            "proof intent",
            &input.intent.intent.intent_hash,
            &proof_intent(),
        )?;
        let submission = proof_submission().map_err(replay_runtime_error)?;
        ensure_digest(
            "proof submission",
            &input.submission.submission.submission_hash,
            &submission,
        )?;
        ensure_digest(
            "proof submission artifact",
            &input.submission.artifact.digest,
            &submission,
        )?;
        ProofReplayVerifier::verify_submission(
            self,
            &proof_intent(),
            &submission,
            &RecordedProofFacts { fact: proof_fact() },
        )
        .map_err(proof_replay_error)
    }

    fn verify_receipt(&self, input: &replay::SideEffectReceiptReplayInput) -> replay::Result<()> {
        ensure_digest(
            "proof intent",
            &input.intent.intent.intent_hash,
            &proof_intent(),
        )?;
        if let Some(submission) = &input.submission {
            let expected_submission = proof_submission().map_err(replay_runtime_error)?;
            ensure_digest(
                "proof submission",
                &submission.submission.submission_hash,
                &expected_submission,
            )?;
        }
        let receipt = proof_receipt().map_err(replay_runtime_error)?;
        ensure_digest(
            "proof receipt",
            &input.receipt.receipt.receipt_hash,
            &receipt,
        )?;
        ensure_digest(
            "proof receipt artifact",
            &input.receipt.artifact.digest,
            &receipt,
        )?;
        ProofReplayVerifier::verify_receipt(
            self,
            &proof_intent(),
            &receipt,
            &RecordedProofFacts { fact: proof_fact() },
        )
        .map_err(proof_replay_error)
    }

    fn verify_confirmation(
        &self,
        input: &replay::SideEffectConfirmationReplayInput,
    ) -> replay::Result<()> {
        ensure_digest(
            "proof intent",
            &input.intent.intent.intent_hash,
            &proof_intent(),
        )?;
        let receipt = proof_receipt().map_err(replay_runtime_error)?;
        if let Some(observed_receipt) = &input.receipt {
            ensure_digest(
                "proof receipt",
                &observed_receipt.receipt.receipt_hash,
                &receipt,
            )?;
        }
        let confirmation = proof_confirmation().map_err(replay_runtime_error)?;
        ensure_digest(
            "proof confirmation",
            &input.confirmation.confirmation.confirmation_hash,
            &confirmation,
        )?;
        ensure_digest(
            "proof confirmation artifact",
            &input.confirmation.artifact.digest,
            &confirmation,
        )?;
        ProofReplayVerifier::verify_confirmation(
            self,
            &receipt,
            &confirmation,
            &RecordedProofFacts { fact: proof_fact() },
        )
        .map_err(proof_replay_error)
    }
}

/// Verifies deterministic proof side-effect replay evidence when a run stream contains proof
/// events.
///
/// Returns `Ok(false)` when the stream contains no deterministic proof side-effect intent.
pub fn verify_deterministic_proof_replay(
    broker: &replay::ReplayBroker,
    stream: &[store::KernelEventEnvelope],
) -> replay::Result<bool> {
    let Some(frames) = proof_side_effect_replay_frames(stream)? else {
        return Ok(false);
    };
    let verifier = DeterministicProofReplayVerifier::new().map_err(replay_runtime_error)?;
    let Some(submission) = &frames.submission else {
        return Err(proof_side_effect_missing("submission"));
    };
    let Some(receipt) = &frames.receipt else {
        return Err(proof_side_effect_missing("receipt"));
    };
    let Some(confirmation) = &frames.confirmation else {
        return Err(proof_side_effect_missing("confirmation"));
    };

    let submission_request = side_effect_replay_request(
        &frames.intent,
        submission.submission_schema_id.clone(),
        submission.submission_hash.clone(),
        None,
    );
    broker.verify_side_effect_submission(&submission_request, &verifier)?;

    let receipt_request = side_effect_replay_request(
        &frames.intent,
        receipt.receipt_schema_id.clone(),
        receipt.receipt_hash.clone(),
        Some(receipt.replay_verifier_id.clone()),
    );
    broker.verify_side_effect_receipt(&receipt_request, &verifier)?;

    let confirmation_request = side_effect_replay_request(
        &frames.intent,
        confirmation.confirmation_schema_id.clone(),
        confirmation.confirmation_hash.clone(),
        Some(confirmation.replay_verifier_id.clone()),
    );
    broker.verify_side_effect_confirmation(&confirmation_request, &verifier)?;
    Ok(true)
}

#[derive(Debug, Clone)]
struct ProofSideEffectReplayFrames {
    intent: side_effect::IntentPersisted,
    submission: Option<side_effect::SubmissionObserved>,
    receipt: Option<side_effect::ReceiptObserved>,
    confirmation: Option<side_effect::ConfirmationObserved>,
}

fn proof_side_effect_replay_frames(
    stream: &[store::KernelEventEnvelope],
) -> replay::Result<Option<ProofSideEffectReplayFrames>> {
    let mut frames = None::<ProofSideEffectReplayFrames>;
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::SideEffectIntentPersisted(payload)
                if is_deterministic_proof_intent(payload)? =>
            {
                if frames.is_some() {
                    return Err(replay::ReplayError::new(
                        replay::ReplayErrorKind::SideEffectMismatch,
                        "multiple deterministic proof side-effect intents in one run",
                    ));
                }
                frames = Some(ProofSideEffectReplayFrames {
                    intent: payload.clone(),
                    submission: None,
                    receipt: None,
                    confirmation: None,
                });
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                if let Some(frames) = frames.as_mut() {
                    if payload.ledger_key == frames.intent.ledger_key
                        && payload.invocation_epoch == frames.intent.invocation_epoch
                    {
                        frames.submission = Some(payload.clone());
                    }
                }
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                if let Some(frames) = frames.as_mut() {
                    if payload.ledger_key == frames.intent.ledger_key
                        && payload.invocation_epoch == frames.intent.invocation_epoch
                    {
                        frames.receipt = Some(payload.clone());
                    }
                }
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                if let Some(frames) = frames.as_mut() {
                    if payload.ledger_key == frames.intent.ledger_key
                        && payload.invocation_epoch == frames.intent.invocation_epoch
                    {
                        frames.confirmation = Some(payload.clone());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(frames)
}

fn is_deterministic_proof_intent(intent: &side_effect::IntentPersisted) -> replay::Result<bool> {
    let expected_capability = ProofMutationCapability::kind().map_err(replay_capability_error)?;
    let expected_adapter = proof_adapter_kind().map_err(replay_identity_error)?;
    Ok(intent.capability_kind == expected_capability && intent.adapter_kind == expected_adapter)
}

fn side_effect_replay_request(
    intent: &side_effect::IntentPersisted,
    evidence_schema_id: SchemaId,
    evidence_hash: ContentDigest,
    replay_verifier_id: Option<events::ReplayVerifierId>,
) -> replay::SideEffectEvidenceReplayRequest {
    replay::SideEffectEvidenceReplayRequest {
        ledger_key: intent.ledger_key.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        invocation_epoch: intent.invocation_epoch,
        intent_schema_id: intent.intent_schema_id.clone(),
        intent_hash: intent.intent_hash.clone(),
        idempotency_input_schema_id: intent.idempotency_input_schema_id.clone(),
        idempotency_input_hash: intent.idempotency_input_hash.clone(),
        capability_kind: intent.capability_kind.clone(),
        capability_version: intent.capability_version.clone(),
        adapter_kind: intent.adapter_kind.clone(),
        adapter_version: intent.adapter_version.clone(),
        evidence_schema_id,
        evidence_hash,
        replay_verifier_id,
    }
}

fn proof_side_effect_missing(phase: &str) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMissing,
        format!("missing deterministic proof {phase} evidence"),
    )
}

fn replay_identity_error(error: mfm_ids::IdentityError) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn replay_capability_error(error: mfm_capabilities::CapabilityError) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

/// Runs the deterministic proof implementation conformance fixture.
pub async fn proof_implementation_conformance_summary(
) -> Result<ProofImplementationConformanceSummary, String> {
    let config = ProofWorkflowConfig::default();
    let draft = proof_program_draft(config.clone()).map_err(|error| error.to_string())?;
    let certified = certified_proof_spec(config).map_err(|error| error.to_string())?;
    let runtime_spec =
        CertifiedRuntimeSpec::new(certified.envelope.clone()).map_err(|error| error.to_string())?;
    let artifacts = InMemoryProofArtifactSink::default();
    persist_conformance_config_artifacts(&artifacts, &draft, runtime_spec.spec())
        .await
        .map_err(|error| error.to_string())?;
    persist_conformance_spec_artifact(&artifacts, &runtime_spec)
        .await
        .map_err(|error| error.to_string())?;
    let mut store = store::InMemoryTypedRunStore::new();
    let artifact_sink: Arc<dyn ProofArtifactSink> = Arc::new(artifacts.clone());
    let scheduler = SerialTypedScheduler::new(
        deterministic_proof_runner_registry(artifact_sink).map_err(|error| error.to_string())?,
    );
    let run_id = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x34; 32]),
    );
    scheduler
        .start_run(
            &mut store,
            &runtime_spec,
            run_id.clone(),
            run_start_evidence(&runtime_spec).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;

    for _ in 0..16 {
        match scheduler
            .drive_until_blocked(&mut store, &runtime_spec, &run_id)
            .await
            .map_err(|error| error.to_string())?
        {
            SchedulerStatus::PublicOutputProjected => break,
            SchedulerStatus::Blocked => break,
            SchedulerStatus::Advanced => continue,
        }
    }

    let stream = store.load_run_stream(&run_id);
    persist_conformance_public_output_receipts(&artifacts, &runtime_spec, &stream)
        .await
        .map_err(|error| error.to_string())?;
    let verifier = DeterministicProofReplayVerifier::new().map_err(|error| error.to_string())?;
    let facts = RecordedProofFacts { fact: proof_fact() };
    let mut facts_valid = false;
    let mut submission_valid = false;
    let mut receipt_valid = false;
    let mut confirmation_valid = false;
    for event in &stream {
        match event.payload() {
            events::KernelEventPayload::FactRecorded(payload) => {
                facts_valid = payload.response_hash
                    == digest_value(&ProofFactResponse { fact: proof_fact() })
                        .map_err(|error| error.to_string())?;
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                let submission = proof_submission().map_err(|error| error.to_string())?;
                submission_valid = payload.submission_hash
                    == digest_value(&submission).map_err(|error| error.to_string())?
                    && verifier
                        .verify_submission(&proof_intent(), &submission, &facts)
                        .is_ok();
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                let receipt = proof_receipt().map_err(|error| error.to_string())?;
                receipt_valid = payload.receipt_hash
                    == digest_value(&receipt).map_err(|error| error.to_string())?
                    && payload.replay_verifier_id
                        == replay_verifier_id().map_err(|error| error.to_string())?
                    && verifier
                        .verify_receipt(&proof_intent(), &receipt, &facts)
                        .is_ok();
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                let receipt = proof_receipt().map_err(|error| error.to_string())?;
                let confirmation = proof_confirmation().map_err(|error| error.to_string())?;
                confirmation_valid = payload.confirmation_hash
                    == digest_value(&confirmation).map_err(|error| error.to_string())?
                    && payload.replay_verifier_id
                        == replay_verifier_id().map_err(|error| error.to_string())?
                    && verifier
                        .verify_confirmation(&receipt, &confirmation, &facts)
                        .is_ok();
            }
            _ => {}
        }
    }
    let replay_valid = verify_conformance_replay(&certified.envelope, &stream, &artifacts)
        .await
        .map_err(|error| error.to_string())?;
    let summary = ProofImplementationConformanceSummary {
        implementation: "deterministic-proof".to_owned(),
        facts_valid,
        submission_valid,
        receipt_valid,
        confirmation_valid,
        replay_valid,
    };
    summary.validate()?;
    Ok(summary)
}

async fn verify_conformance_replay(
    envelope: &spec::CertifiedSpecEnvelope,
    stream: &[store::KernelEventEnvelope],
    artifacts: &InMemoryProofArtifactSink,
) -> replay::Result<bool> {
    let run_started = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .ok_or_else(|| {
            replay::ReplayError::new(
                replay::ReplayErrorKind::RunStartedMissing,
                "proof conformance stream has no run-start event",
            )
        })?;
    let projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(stream).map_err(replay_store_error)?;
    let retention = projection.retention(&run_started.run_id).ok_or_else(|| {
        replay::ReplayError::new(
            replay::ReplayErrorKind::ArtifactMissing,
            "proof conformance stream has no retained artifact evidence",
        )
    })?;
    let mut artifact_evidence = Vec::with_capacity(retention.refs.len());
    for retained in retention.refs.values() {
        let evidence = artifacts
            .evidence_for(&retained.artifact_id)
            .ok_or_else(|| {
                replay::ReplayError::new(
                    replay::ReplayErrorKind::ArtifactMissing,
                    format!("missing proof artifact {}", retained.artifact_id),
                )
            })?;
        artifact_evidence.push(evidence);
    }
    let authority = replay::ReplayAuthority::from_certified_spec(
        envelope,
        run_started.runner_executables.clone(),
        run_started.adapter_executables.clone(),
        artifact_evidence,
    );
    let broker = replay::ReplayBroker::from_run_stream(envelope.clone(), stream, authority)?;
    let proof_verified = verify_deterministic_proof_replay(&broker, stream)?;
    Ok(proof_verified
        && broker.projection_snapshot().run_state(&run_started.run_id)
            == store::RunState::Completed)
}

async fn persist_conformance_spec_artifact(
    artifacts: &dyn ProofArtifactSink,
    runtime_spec: &CertifiedRuntimeSpec,
) -> mfm_runtime::Result<()> {
    let spec_bytes = runtime_spec.spec().canonical_json()?;
    let spec_digest = spec_bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(spec_digest.algorithm(), *spec_digest.digest()),
        digest: spec_digest,
        byte_len: spec_bytes.as_bytes().len() as u64,
        media_type: runtime_spec.spec().media_type.clone(),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedExecutionSpec,
    };
    artifacts
        .put_verified_artifact(spec_bytes.to_vec(), evidence)
        .await
        .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    Ok(())
}

async fn persist_conformance_config_artifacts(
    artifacts: &dyn ProofArtifactSink,
    draft: &mfm_program::TypedProgramDraft,
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_runtime::Result<()> {
    for config in draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
    {
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(
                config.content_digest.algorithm(),
                *config.content_digest.digest(),
            ),
            digest: config.content_digest.clone(),
            byte_len: config.byte_len as u64,
            media_type: spec::MediaType::new("application/json")?,
            schema_id: Some(config.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedConfig,
        };
        artifacts
            .put_verified_artifact(config.canonical_json.to_vec(), evidence)
            .await
            .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    }

    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let framework_kind = match framework {
            spec::FrameworkNodeSpec::Bridge(_) => "bridge_same_value",
            spec::FrameworkNodeSpec::PublicOutputRender(_) => "public_output_render",
        };
        let payload = serde_json::json!({
            "framework": framework_kind,
            "node_id": node.node_id.as_str(),
        });
        let bytes = PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&payload)
                .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?,
        )
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: node.config_ref.artifact_id.clone(),
            digest: node.config_ref.digest.clone(),
            byte_len: node.config_ref.byte_len,
            media_type: node.config_ref.media_type.clone(),
            schema_id: Some(node.config_ref.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedConfig,
        };
        artifacts
            .put_verified_artifact(bytes.to_vec(), evidence)
            .await
            .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    }
    Ok(())
}

async fn persist_conformance_public_output_receipts(
    artifacts: &dyn ProofArtifactSink,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> mfm_runtime::Result<()> {
    for event in stream {
        let events::KernelEventPayload::PublicOutputProduced(payload) = event.payload() else {
            continue;
        };
        let (bytes, evidence) = build_public_output_receipt_artifact(runtime_spec, payload)?;
        artifacts
            .put_verified_artifact(bytes.to_vec(), evidence)
            .await
            .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    }
    Ok(())
}

fn replay_store_error(error: store::StoreError) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::InvalidRunStream, error.to_string())
}

fn run_start_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
) -> mfm_runtime::Result<RunStartEvidence> {
    let spec_bytes = runtime_spec.spec().canonical_json()?;
    let spec_digest = spec_bytes.content_digest();
    let spec_artifact = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(spec_digest.algorithm(), *spec_digest.digest()),
        digest: spec_digest,
        byte_len: spec_bytes.as_bytes().len() as u64,
        media_type: runtime_spec.spec().media_type.clone(),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedExecutionSpec,
    };
    let config_artifacts = runtime_spec
        .spec()
        .config_refs
        .iter()
        .map(|config| store::ArtifactEvidenceRef {
            artifact_id: config.artifact_id.clone(),
            digest: config.digest.clone(),
            byte_len: config.byte_len,
            media_type: config.media_type.clone(),
            schema_id: Some(config.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedConfig,
        })
        .collect();
    Ok(RunStartEvidence {
        spec_artifact,
        config_artifacts,
        framework_version: events::FrameworkVersion::new("mfm.proof.typed.v1")?,
        source_revision: events::SourceRevision::new("mfm-transports-proof")?,
        adapter_executables: vec![executable(events::RunnerFactoryId::new(
            "deterministic-proof-adapter",
        )?)?],
        seed_cells: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deterministic_proof_implementation_conforms() {
        let summary = proof_implementation_conformance_summary()
            .await
            .expect("proof conformance");
        assert_eq!(summary.implementation, "deterministic-proof");
        assert_eq!(
            summary.to_summary_document()["kind"],
            "proof-implementation-conformance"
        );
    }
}
