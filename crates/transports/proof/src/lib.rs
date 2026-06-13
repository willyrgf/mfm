#![warn(missing_docs)]
//! Deterministic typed proof implementation.
//!
//! This crate provides the enabled proof implementation used by conformance tests and by typed
//! runner assembly. It exposes typed runtime runners and replay verifiers only; it does not expose
//! legacy live-IO transports or generic request/response namespaces.

use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::CapabilitySpec;
use mfm_collectors_proof::{
    proof_adapter_kind, proof_adapter_version, ProofApplyConfig, ProofApplySideEffectState,
    ProofAssembleConfig, ProofAssembleOutputState, ProofConfirmation, ProofFact, ProofFactRequest,
    ProofFactResponse, ProofIdempotencyInput, ProofIntent, ProofMutationCapability, ProofOutput,
    ProofReadCapability, ProofReadConfig, ProofReadFactState, ProofReceipt, ProofReplayError,
    ProofReplayVerifier, ProofSideEffectResult, ProofSubmission, RecordedProofFacts,
};
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{ArtifactId, ContentDigest, DescriptorId, NodeId, SchemaId};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput,
    ErasedRunnerRegistry, MaterializedCellTerminal, MaterializedInputNode, RunnerEventPayload,
    StagedArtifact, StagedRetentionRefs,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue};
use serde::Serialize;

const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const PURE_FACTORY: &str = "pure";
const REPLAY_VERIFIER_ID: &str = "mfm.proof.replay.deterministic.v1";

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
    let ledger_purpose = events::SideEffectLedgerPurpose::Forward;
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
                ledger_purpose: ledger_purpose.clone(),
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
                prepared_artifact_id: None,
                prepared_hash: None,
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
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
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
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
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
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
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
/// Returns `Ok(false)` when the broker stream contains no deterministic proof side-effect intent.
pub fn verify_deterministic_proof_replay(broker: &replay::ReplayBroker) -> replay::Result<bool> {
    let Some(frames) = proof_side_effect_replay_frames(broker.events())? else {
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
