#![warn(missing_docs)]
//! Deterministic typed proof implementation.
//!
//! This crate provides the enabled proof implementation used by conformance tests and by typed
//! runner assembly. It exposes typed runtime runners and replay verifiers only; it does not expose
//! legacy live-IO transports or generic request/response namespaces.

use std::collections::BTreeMap;
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
    CertifiedRuntimeSpec, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCellTerminal, MaterializedInputNode,
    RunLaunchEvidence, RunnerEventPayload, RuntimeArtifactStageFuture, RuntimeArtifactStager,
    SchedulerStatus, SerialTypedScheduler, StagedArtifact, StagedRetentionRefs,
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

#[derive(Clone, Default)]
struct InMemoryProofArtifacts {
    artifacts: Arc<Mutex<ProofArtifactMap>>,
}

impl InMemoryProofArtifacts {
    fn evidence_for(&self, artifact_id: &ArtifactId) -> Option<store::ArtifactEvidenceRef> {
        self.artifacts.lock().ok().and_then(|artifacts| {
            artifacts
                .get(artifact_id)
                .map(|(_, evidence)| evidence.clone())
        })
    }

    async fn store_verified_artifact(
        &self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> mfm_runtime::Result<()> {
        let digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
        if evidence.digest != digest
            || evidence.byte_len != bytes.len() as u64
            || evidence.artifact_id != ArtifactId::from_digest(digest.algorithm(), *digest.digest())
        {
            return Err(mfm_runtime::RuntimeError::Store(
                "proof artifact bytes do not match typed evidence".to_owned(),
            ));
        }
        let mut artifacts = self.artifacts.lock().map_err(|_| {
            mfm_runtime::RuntimeError::Store("proof artifact store lock was poisoned".to_owned())
        })?;
        if let Some((existing_bytes, existing_evidence)) = artifacts.get(&evidence.artifact_id) {
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
    }
}

impl RuntimeArtifactStager for InMemoryProofArtifacts {
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a> {
        Box::pin(async move { self.store_verified_artifact(bytes, evidence).await })
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
) -> mfm_runtime::Result<()> {
    let read = registered_descriptor::<ProofReadFactState>()?;
    let side_effect = registered_descriptor::<ProofApplySideEffectState>()?;
    let assemble = registered_descriptor::<ProofAssembleOutputState>()?;

    registry.register(binding(read, READ_FACTORY, Arc::new(ProofReadRunner))?)?;
    registry.register(binding(
        side_effect,
        SIDE_EFFECT_FACTORY,
        Arc::new(ProofSideEffectRunner),
    )?)?;
    registry.register(binding(
        assemble,
        PURE_FACTORY,
        Arc::new(ProofAssembleRunner),
    )?)?;
    Ok(())
}

/// Builds a runner registry containing only the deterministic proof implementation.
pub fn deterministic_proof_runner_registry() -> mfm_runtime::Result<ErasedRunnerRegistry> {
    let mut registry = ErasedRunnerRegistry::new();
    register_deterministic_proof_runners(&mut registry)?;
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

struct ProofReadRunner;

impl ErasedNodeRunner for ProofReadRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_read(ctx).await })
    }
}

struct ProofSideEffectRunner;

impl ErasedNodeRunner for ProofSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_side_effect(ctx).await })
    }
}

struct ProofAssembleRunner;

impl ErasedNodeRunner for ProofAssembleRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_assemble(ctx).await })
    }
}

async fn run_read(ctx: ErasedRunCtx<'_>) -> mfm_runtime::Result<ErasedRunnerOutput> {
    ensure_config::<ProofReadConfig>(&ctx.node().config_ref, &ProofReadConfig { fact_n: 1 })?;
    let fact = ProofFact { n: 1 };
    let request = ProofFactRequest {
        source: "deterministic-proof".to_owned(),
    };
    let response = ProofFactResponse { fact: fact.clone() };
    let request_hash = digest_value(&request)?;
    let response_artifact = artifact_for_value(
        &response,
        events::ArtifactRole::FactResponse,
        Some(ctx.node().node_id.clone()),
    )?;
    let output_artifact = artifact_for_value(
        &fact,
        events::ArtifactRole::StateOutput,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_response = staged_attempt_artifact(&ctx, &response_artifact)?;
    let staged_output = staged_attempt_artifact(&ctx, &output_artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_response, staged_output],
        staged_retention_refs: vec![
            retention(&response_artifact.evidence),
            retention(&output_artifact.evidence),
        ],
        payloads: vec![
            RunnerEventPayload::FactRecorded(events::FactRecorded {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
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
        ],
    })
}

async fn run_side_effect(ctx: ErasedRunCtx<'_>) -> mfm_runtime::Result<ErasedRunnerOutput> {
    ensure_config::<ProofApplyConfig>(
        &ctx.node().config_ref,
        &ProofApplyConfig {
            action: "accept".to_owned(),
        },
    )?;
    let ledger_key = events::SideEffectLedgerKey::new("mfm.proof.ledger.default")?;
    let phase = ctx
        .projections()
        .side_effect(&ledger_key)
        .map(|projection| projection.phase.clone());
    match phase {
        None => side_effect_prepare(ctx, ledger_key).await,
        Some(store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        })
        | Some(store::SideEffectPhase::SubmissionUnknown { invocation_epoch }) => {
            side_effect_submission(ctx, ledger_key, invocation_epoch).await
        }
        Some(store::SideEffectPhase::SubmissionObserved { invocation_epoch }) => {
            side_effect_receipt(ctx, ledger_key, invocation_epoch).await
        }
        Some(store::SideEffectPhase::ReceiptObserved { invocation_epoch }) => {
            side_effect_confirmation(ctx, ledger_key, invocation_epoch).await
        }
        Some(store::SideEffectPhase::ConfirmationObserved { .. }) => side_effect_output(ctx).await,
        Some(store::SideEffectPhase::Ambiguous { .. }) => Ok(ErasedRunnerOutput::new(Vec::new())),
        Some(other) => Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "unsupported proof side-effect phase: {other:?}"
        ))),
    }
}

async fn side_effect_prepare(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let intent = proof_intent();
    let idem_input = proof_idempotency_input();
    let intent_artifact = artifact_for_value(
        &intent,
        events::ArtifactRole::SideEffectIntent,
        Some(ctx.node().node_id.clone()),
    )?;
    let idem_hash = digest_value(&idem_input)?;
    let idempotency_key =
        events::IdempotencyKeyRef::new(format!("idem-{}", short_digest(&idem_hash)))?;
    let owner = events::RunnerInvocationId::new("mfm.proof.owner.1")?;
    let token = side_effect::ClaimFencingToken::new("mfm.proof.token.1")?;
    let staged_artifact =
        staged_side_effect_artifact(&ctx, &intent_artifact, ledger_key.clone(), 1)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: vec![retention(&intent_artifact.evidence)],
        payloads: vec![
            RunnerEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                scope_id: ctx.node().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
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
            RunnerEventPayload::SideEffectClaimed(side_effect::Claimed {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger_key.clone(),
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
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
                prepared_artifact_id: None,
                prepared_hash: None,
            }),
            RunnerEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
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
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let submission = proof_submission()?;
    let artifact = artifact_for_value(
        &submission,
        events::ArtifactRole::Submission,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_artifact =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
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
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let receipt = proof_receipt()?;
    let artifact = artifact_for_value(
        &receipt,
        events::ArtifactRole::Receipt,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_artifact =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectReceiptObserved(
            side_effect::ReceiptObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
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
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let confirmation = proof_confirmation()?;
    let artifact = artifact_for_value(
        &confirmation,
        events::ArtifactRole::Confirmation,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_artifact =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectConfirmationObserved(
            side_effect::ConfirmationObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
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

async fn side_effect_output(ctx: ErasedRunCtx<'_>) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let output = proof_side_effect_result()?;
    let artifact = artifact_for_value(
        &output,
        events::ArtifactRole::StateOutput,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_artifact = staged_attempt_artifact(&ctx, &artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![cell_produced(&ctx, &artifact.evidence)],
    })
}

async fn run_assemble(ctx: ErasedRunCtx<'_>) -> mfm_runtime::Result<ErasedRunnerOutput> {
    ensure_config::<ProofAssembleConfig>(
        &ctx.node().config_ref,
        &ProofAssembleConfig { output_version: 1 },
    )?;
    ensure_struct_input_digest(ctx.inputs(), "fact", &digest_value(&proof_fact())?)?;
    ensure_struct_input_digest(
        ctx.inputs(),
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
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_artifact = staged_attempt_artifact(&ctx, &artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![cell_produced(&ctx, &artifact.evidence)],
    })
}

fn staged_attempt_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &ProofArtifact,
) -> mfm_runtime::Result<StagedArtifact> {
    StagedArtifact::inline_attempt_artifact(ctx, artifact.bytes.clone(), artifact.evidence.clone())
}

fn staged_side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &ProofArtifact,
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

fn retention(artifact: &store::ArtifactEvidenceRef) -> StagedRetentionRefs {
    StagedRetentionRefs::runtime_evidence(vec![events::RetentionRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        content_digest: artifact.digest.clone(),
    }])
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
        CertifiedRuntimeSpec::new(certified.clone()).map_err(|error| error.to_string())?;
    let artifacts = InMemoryProofArtifacts::default();
    persist_conformance_config_artifacts(&artifacts, &draft, runtime_spec.spec())
        .await
        .map_err(|error| error.to_string())?;
    persist_conformance_spec_artifact(&artifacts, &runtime_spec)
        .await
        .map_err(|error| error.to_string())?;
    let mut store = store::InMemoryTypedRunStore::new();
    let artifact_stager: Arc<dyn RuntimeArtifactStager> = Arc::new(artifacts.clone());
    let scheduler = SerialTypedScheduler::new(
        deterministic_proof_runner_registry().map_err(|error| error.to_string())?,
        artifact_stager,
    );
    let run_id = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x34; 32]),
    );
    start_proof_run(&scheduler, &mut store, &runtime_spec, run_id.clone())
        .await
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
    let replay_valid = verify_conformance_replay(&runtime_spec, &stream, &artifacts)
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
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    artifacts: &InMemoryProofArtifacts,
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
    mfm_runtime::validate_run_stream(runtime_spec, &run_started.run_id, stream).map_err(
        |error| {
            replay::ReplayError::new(replay::ReplayErrorKind::InvalidRunStream, error.to_string())
        },
    )?;
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
        runtime_spec.envelope(),
        run_started.runner_executables.clone(),
        run_started.adapter_executables.clone(),
        artifact_evidence,
    );
    let broker =
        replay::ReplayBroker::from_runtime_validated_stream(runtime_spec, stream, authority)?;
    let proof_verified = verify_deterministic_proof_replay(&broker, stream)?;
    Ok(proof_verified
        && broker.projection_snapshot().run_state(&run_started.run_id)
            == store::RunState::Completed)
}

async fn persist_conformance_spec_artifact(
    artifacts: &InMemoryProofArtifacts,
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
        .store_verified_artifact(spec_bytes.to_vec(), evidence)
        .await
        .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    let certificate_bytes = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    let certificate_digest = certificate_bytes.content_digest();
    let certificate_media_type = mfm_spec::v1::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)?;
    let certificate_evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(
            certificate_digest.algorithm(),
            *certificate_digest.digest(),
        ),
        digest: certificate_digest,
        byte_len: certificate_bytes.as_bytes().len() as u64,
        media_type: certificate_media_type,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedSpecCertificate,
    };
    artifacts
        .store_verified_artifact(certificate_bytes.to_vec(), certificate_evidence)
        .await
        .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    Ok(())
}

async fn persist_conformance_config_artifacts(
    artifacts: &InMemoryProofArtifacts,
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
            .store_verified_artifact(config.canonical_json.to_vec(), evidence)
            .await
            .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    }

    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes = spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
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
            .store_verified_artifact(bytes.to_vec(), evidence)
            .await
            .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
    }
    Ok(())
}

fn replay_store_error(error: store::StoreError) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::InvalidRunStream, error.to_string())
}

async fn start_proof_run(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: RunId,
) -> mfm_runtime::Result<()> {
    let launch = scheduler.prepare_run_launch(
        runtime_spec,
        run_id.clone(),
        run_start_evidence(runtime_spec)?,
        store.expected_next_seq(&run_id),
    )?;
    scheduler.start_run(store, launch).await?;
    Ok(())
}

fn run_start_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
) -> mfm_runtime::Result<RunLaunchEvidence> {
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
    let certificate_bytes = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    let certificate_digest = certificate_bytes.content_digest();
    let certificate_artifact = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(
            certificate_digest.algorithm(),
            *certificate_digest.digest(),
        ),
        digest: certificate_digest,
        byte_len: certificate_bytes.as_bytes().len() as u64,
        media_type: mfm_spec::v1::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)?,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedSpecCertificate,
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
    Ok(RunLaunchEvidence {
        spec_artifact,
        certificate_artifact,
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
    use mfm_ids::EventId;

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

    #[tokio::test]
    async fn conformance_replay_rejects_standalone_retention_projection_history() {
        let (runtime_spec, stream, artifacts) = conformance_stream().await;
        let corrupt = standalone_retention_projection_history(&stream);

        let error = verify_conformance_replay(&runtime_spec, &corrupt, &artifacts)
            .await
            .expect_err("standalone retention projection history must reject");

        assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
    }

    #[tokio::test]
    async fn conformance_replay_rejects_completed_history_without_retention_projection() {
        let (runtime_spec, stream, artifacts) = conformance_stream().await;
        let corrupt = remove_retention_projection_commit(&stream);

        let error = verify_conformance_replay(&runtime_spec, &corrupt, &artifacts)
            .await
            .expect_err("completed history without retention projection must reject");

        assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
    }

    #[tokio::test]
    async fn conformance_replay_rejects_failed_completion_without_framework_authority() {
        let (runtime_spec, stream, artifacts) = conformance_stream().await;
        let corrupt = failed_completion_after_run_start(&stream);

        let error = verify_conformance_replay(&runtime_spec, &corrupt, &artifacts)
            .await
            .expect_err("failed completion without framework authority must reject");

        assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
    }

    #[tokio::test]
    async fn conformance_replay_rejects_post_completion_retention_refs() {
        let (runtime_spec, stream, artifacts) = conformance_stream().await;
        let corrupt = append_post_completion_retention_refs(&stream);

        let error = verify_conformance_replay(&runtime_spec, &corrupt, &artifacts)
            .await
            .expect_err("post-completion retention refs must reject");

        assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
    }

    #[tokio::test]
    async fn conformance_replay_rejects_post_projection_retention_refs_before_completion() {
        let (runtime_spec, stream, artifacts) = conformance_stream().await;
        let corrupt = append_post_projection_retention_refs_before_completion(&stream);

        let error = verify_conformance_replay(&runtime_spec, &corrupt, &artifacts)
            .await
            .expect_err("post-projection retention refs before completion must reject");

        assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
    }

    #[tokio::test]
    async fn conformance_replay_rejects_extra_payload_in_retention_projection_commit() {
        let (runtime_spec, stream, artifacts) = conformance_stream().await;
        let corrupt = append_extra_retention_ref_to_projection_commit(&stream);

        let error = verify_conformance_replay(&runtime_spec, &corrupt, &artifacts)
            .await
            .expect_err("extra retention projection commit payload must reject");

        assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
    }

    #[tokio::test]
    async fn conformance_replay_rejects_same_sequence_sidecar_retention_projection_payload() {
        let (runtime_spec, stream, artifacts) = conformance_stream().await;
        let corrupt = append_same_sequence_sidecar_to_retention_projection(&stream);

        let error = verify_conformance_replay(&runtime_spec, &corrupt, &artifacts)
            .await
            .expect_err("same-sequence sidecar retention payload must reject");

        assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
    }

    async fn conformance_stream() -> (
        CertifiedRuntimeSpec,
        Vec<store::KernelEventEnvelope>,
        InMemoryProofArtifacts,
    ) {
        let config = ProofWorkflowConfig::default();
        let draft = proof_program_draft(config.clone()).expect("proof draft");
        let certified = certified_proof_spec(config).expect("certified proof spec");
        let runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
        let artifacts = InMemoryProofArtifacts::default();
        persist_conformance_config_artifacts(&artifacts, &draft, runtime_spec.spec())
            .await
            .expect("persist config artifacts");
        persist_conformance_spec_artifact(&artifacts, &runtime_spec)
            .await
            .expect("persist spec artifact");

        let mut store = store::InMemoryTypedRunStore::new();
        let artifact_stager: Arc<dyn RuntimeArtifactStager> = Arc::new(artifacts.clone());
        let scheduler = SerialTypedScheduler::new(
            deterministic_proof_runner_registry().expect("runner registry"),
            artifact_stager,
        );
        let run_id = RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x34; 32]),
        );
        start_proof_run(&scheduler, &mut store, &runtime_spec, run_id.clone())
            .await
            .expect("start run");

        for _ in 0..16 {
            match scheduler
                .drive_until_blocked(&mut store, &runtime_spec, &run_id)
                .await
                .expect("drive conformance run")
            {
                SchedulerStatus::PublicOutputProjected | SchedulerStatus::Blocked => break,
                SchedulerStatus::Advanced => continue,
            }
        }

        let stream = store.load_run_stream(&run_id);
        (runtime_spec, stream, artifacts)
    }

    fn standalone_retention_projection_history(
        valid_stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let run_id = valid_stream
            .first()
            .expect("valid stream is non-empty")
            .run_id()
            .clone();
        let retention_seq = valid_stream
            .iter()
            .find_map(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                )
                .then_some(event.seq())
            })
            .expect("retention projection event");
        let mut rewritten = Vec::new();
        let mut index = 0;
        while index < valid_stream.len() {
            let seq = valid_stream[index].seq();
            let commit_key = valid_stream[index].commit_key().clone();
            let mut end = index + 1;
            while end < valid_stream.len()
                && valid_stream[end].seq() == seq
                && valid_stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let payloads = valid_stream[index..end]
                .iter()
                .filter(|event| {
                    if event.seq() != retention_seq {
                        return true;
                    }
                    matches!(
                        event.payload(),
                        events::KernelEventPayload::RetentionManifestProjected(_)
                            | events::KernelEventPayload::RetentionRefsAppended(
                                events::RetentionRefsAppended {
                                    reason: events::RetentionReason::ManifestProjection,
                                    ..
                                }
                            )
                    )
                })
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            if !payloads.is_empty() {
                let request = store::TypedCommitRequest {
                    run_id: run_id.clone(),
                    expected_next_seq: seq,
                    commit_key,
                    payloads,
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let batch =
                    store::build_committed_batch(&request, seq).expect("rewritten commit batch");
                rewritten.extend(batch.events().iter().cloned());
            }
            index = end;
        }
        rewritten
    }

    fn remove_retention_projection_commit(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let mut rewritten = Vec::with_capacity(stream.len());
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let original_seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == original_seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            if commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                )
            }) {
                index = end;
                continue;
            }
            let seq = rewritten
                .last()
                .map(|event: &store::KernelEventEnvelope| {
                    store::StreamSeq::new(event.seq().as_u64() + 1).expect("next stream seq")
                })
                .unwrap_or(store::StreamSeq::FIRST);
            let request = store::TypedCommitRequest {
                run_id: first.run_id().clone(),
                expected_next_seq: seq,
                commit_key,
                payloads: commit
                    .iter()
                    .map(|event| event.payload().clone())
                    .collect::<Vec<_>>(),
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions::default(),
            };
            let batch =
                store::build_committed_batch(&request, seq).expect("rewritten commit batch");
            rewritten.extend(batch.events().iter().cloned());
            index = end;
        }
        rewritten
    }

    fn failed_completion_after_run_start(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let first = stream.first().expect("stream is non-empty");
        let start_seq = first.seq();
        let start_key = first.commit_key().clone();
        let mut rewritten = stream
            .iter()
            .take_while(|event| event.seq() == start_seq && event.commit_key() == &start_key)
            .cloned()
            .collect::<Vec<_>>();
        let run_started = rewritten
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => Some(payload),
                _ => None,
            })
            .expect("run started");
        let seq = store::StreamSeq::new(start_seq.as_u64() + 1).expect("next stream seq");
        let request = store::TypedCommitRequest {
            run_id: run_started.run_id.clone(),
            expected_next_seq: seq,
            commit_key: store::CommitKey::new("failed-completion-without-framework")
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunCompleted(
                events::RunCompleted {
                    run_id: run_started.run_id.clone(),
                    spec_hash: run_started.spec_hash.clone(),
                    outcome: events::RunCompletionOutcome::Failed(test_error()),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let batch = store::build_committed_batch(&request, seq).expect("failed completion batch");
        rewritten.extend(batch.events().iter().cloned());
        rewritten
    }

    fn test_error() -> events::MfmErrorInfo {
        events::MfmErrorInfo {
            code: events::ErrorCode::new("test_failure").expect("error code"),
            category: events::ErrorCategory::Runtime,
            retryable: false,
            safe_message: "test failure".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        }
    }

    fn append_post_completion_retention_refs(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let run_started = stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => Some(payload),
                _ => None,
            })
            .expect("run started");
        let retention = store::ProjectionSnapshot::rebuild_from_run_stream(stream)
            .expect("valid projection")
            .retention(&run_started.run_id)
            .and_then(|projection| projection.refs.values().next().cloned())
            .expect("retention ref");
        let seq = stream
            .last()
            .map(|event| store::StreamSeq::new(event.seq().as_u64() + 1).expect("next stream seq"))
            .unwrap_or(store::StreamSeq::FIRST);
        let request = store::TypedCommitRequest {
            run_id: run_started.run_id.clone(),
            expected_next_seq: seq,
            commit_key: store::CommitKey::new("post-completion-retention-ref").expect("commit key"),
            payloads: vec![events::KernelEventPayload::RetentionRefsAppended(
                events::RetentionRefsAppended {
                    run_id: run_started.run_id.clone(),
                    spec_hash: run_started.spec_hash.clone(),
                    refs: vec![retention],
                    reason: events::RetentionReason::RuntimeEvidence,
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let batch = store::build_committed_batch(&request, seq).expect("retention refs batch");
        let mut rewritten = stream.to_vec();
        rewritten.extend(batch.events().iter().cloned());
        rewritten
    }

    fn append_post_projection_retention_refs_before_completion(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let run_started = stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => Some(payload),
                _ => None,
            })
            .expect("run started");
        let completion = stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RunCompleted(payload) => Some(payload.clone()),
                _ => None,
            })
            .expect("run completed");
        let retention = store::ProjectionSnapshot::rebuild_from_run_stream(stream)
            .expect("valid projection")
            .retention(&run_started.run_id)
            .and_then(|projection| projection.refs.values().next().cloned())
            .expect("retention ref");
        let mut rewritten = remove_completion_commit(stream);
        let retention_seq = next_seq(&rewritten);
        let retention_request = store::TypedCommitRequest {
            run_id: run_started.run_id.clone(),
            expected_next_seq: retention_seq,
            commit_key: store::CommitKey::new("post-projection-retention-ref").expect("commit key"),
            payloads: vec![events::KernelEventPayload::RetentionRefsAppended(
                events::RetentionRefsAppended {
                    run_id: run_started.run_id.clone(),
                    spec_hash: run_started.spec_hash.clone(),
                    refs: vec![retention],
                    reason: events::RetentionReason::RuntimeEvidence,
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let retention_batch = store::build_committed_batch(&retention_request, retention_seq)
            .expect("retention refs batch");
        rewritten.extend(retention_batch.events().iter().cloned());

        let completion_seq = next_seq(&rewritten);
        let completion_request = store::TypedCommitRequest {
            run_id: run_started.run_id.clone(),
            expected_next_seq: completion_seq,
            commit_key: store::CommitKey::new("completion-after-post-projection-retention")
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunCompleted(completion)],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let completion_batch = store::build_committed_batch(&completion_request, completion_seq)
            .expect("completion batch");
        rewritten.extend(completion_batch.events().iter().cloned());
        rewritten
    }

    fn append_extra_retention_ref_to_projection_commit(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let mut rewritten = Vec::with_capacity(stream.len() + 1);
        let mut inserted = false;
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            if commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                )
            }) {
                let runtime_evidence = commit
                    .iter()
                    .find_map(|event| match event.payload() {
                        events::KernelEventPayload::RetentionRefsAppended(payload)
                            if payload.reason == events::RetentionReason::RuntimeEvidence =>
                        {
                            Some(payload)
                        }
                        _ => None,
                    })
                    .expect("projection commit runtime evidence retention refs");
                let mut payloads = commit
                    .iter()
                    .map(|event| event.payload().clone())
                    .collect::<Vec<_>>();
                payloads.push(events::KernelEventPayload::RetentionRefsAppended(
                    events::RetentionRefsAppended {
                        run_id: runtime_evidence.run_id.clone(),
                        spec_hash: runtime_evidence.spec_hash.clone(),
                        refs: runtime_evidence.refs.clone(),
                        reason: events::RetentionReason::RuntimeEvidence,
                    },
                ));
                let request = store::TypedCommitRequest {
                    run_id: first.run_id().clone(),
                    expected_next_seq: seq,
                    commit_key,
                    payloads,
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let batch = store::build_committed_batch(&request, seq).expect("rewritten commit");
                rewritten.extend(batch.events().iter().cloned());
                inserted = true;
            } else {
                rewritten.extend(commit.iter().cloned());
            }
            index = end;
        }
        assert!(inserted, "retention projection commit exists");
        rewritten
    }

    fn append_same_sequence_sidecar_to_retention_projection(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let mut rewritten = Vec::with_capacity(stream.len() + 1);
        let mut inserted = false;
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            rewritten.extend(commit.iter().cloned());
            if commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                )
            }) {
                let runtime_evidence = commit
                    .iter()
                    .find_map(|event| match event.payload() {
                        events::KernelEventPayload::RetentionRefsAppended(payload)
                            if payload.reason == events::RetentionReason::RuntimeEvidence =>
                        {
                            Some(payload)
                        }
                        _ => None,
                    })
                    .expect("projection runtime evidence retention refs");
                let sidecar_key =
                    store::CommitKey::new("forged-same-seq-retention-sidecar").expect("commit key");
                let request = store::TypedCommitRequest {
                    run_id: first.run_id().clone(),
                    expected_next_seq: seq,
                    commit_key: sidecar_key.clone(),
                    payloads: vec![events::KernelEventPayload::RetentionRefsAppended(
                        events::RetentionRefsAppended {
                            run_id: runtime_evidence.run_id.clone(),
                            spec_hash: runtime_evidence.spec_hash.clone(),
                            refs: runtime_evidence.refs.clone(),
                            reason: events::RetentionReason::RuntimeEvidence,
                        },
                    )],
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let batch =
                    store::build_committed_batch(&request, seq).expect("sidecar commit batch");
                let next_ordinal =
                    u32::try_from(commit.len()).expect("retention commit ordinal count");
                for (offset, event) in batch.events().iter().enumerate() {
                    rewritten.push(rewrite_envelope(
                        event,
                        seq,
                        store::CommitOrdinal::new(
                            next_ordinal
                                .checked_add(u32::try_from(offset).expect("sidecar ordinal offset"))
                                .expect("sidecar ordinal"),
                        ),
                        sidecar_key.clone(),
                    ));
                }
                inserted = true;
            }
            index = end;
        }
        assert!(inserted, "retention projection commit exists");
        rewritten
    }

    fn rewrite_envelope(
        event: &store::KernelEventEnvelope,
        seq: store::StreamSeq,
        ordinal: store::CommitOrdinal,
        commit_key: store::CommitKey,
    ) -> store::KernelEventEnvelope {
        store::KernelEventEnvelope::from_persisted_record(store::PersistedKernelEventRecord {
            event_id: event_id_for(event, seq, ordinal),
            event_schema_id: event.event_schema_id().clone(),
            run_id: event.run_id().clone(),
            seq,
            ordinal,
            spec_hash: event.spec_hash().clone(),
            commit_key,
            logical_key: event.logical_key().clone(),
            payload_hash: event.payload_hash().clone(),
            payload: event.payload().clone(),
            payload_canonical_byte_len: event.audit().payload_canonical_byte_len(),
        })
        .expect("rewritten envelope")
    }

    fn event_id_for(
        event: &store::KernelEventEnvelope,
        seq: store::StreamSeq,
        ordinal: store::CommitOrdinal,
    ) -> EventId {
        let canonical = PlainCanonicalJsonBytes::from_json_str(
            &serde_json::json!({
                "event_schema_id": event.event_schema_id().as_str(),
                "ordinal": ordinal.as_u32(),
                "payload_hash": event.payload_hash().as_str(),
                "run_id": event.run_id().as_str(),
                "seq": seq.as_u64(),
            })
            .to_string(),
        )
        .expect("event id canonical");
        EventId::from_digest(DigestAlgorithm::Sha256JcsV1, canonical.digest_bytes())
    }

    fn remove_completion_commit(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let mut rewritten = Vec::with_capacity(stream.len());
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let original_seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == original_seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            if commit
                .iter()
                .any(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
            {
                index = end;
                continue;
            }
            let seq = next_seq(&rewritten);
            let request = store::TypedCommitRequest {
                run_id: first.run_id().clone(),
                expected_next_seq: seq,
                commit_key,
                payloads: commit.iter().map(|event| event.payload().clone()).collect(),
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions::default(),
            };
            let batch = store::build_committed_batch(&request, seq).expect("rewritten commit");
            rewritten.extend(batch.events().iter().cloned());
            index = end;
        }
        rewritten
    }

    fn next_seq(stream: &[store::KernelEventEnvelope]) -> store::StreamSeq {
        stream
            .last()
            .map(|event| store::StreamSeq::new(event.seq().as_u64() + 1).expect("next stream seq"))
            .unwrap_or(store::StreamSeq::FIRST)
    }
}
