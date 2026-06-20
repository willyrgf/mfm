#![warn(missing_docs)]
//! Deterministic typed proof implementation.
//!
//! This crate provides the enabled proof implementation used by conformance tests and by typed
//! runner assembly. It exposes typed runtime runners and replay verifiers only; it does not expose
//! legacy live-IO transports or generic request/response namespaces.

use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{CapabilitySetDescriptor, CapabilitySpec};
use mfm_collectors_proof::{
    proof_adapter_kind, proof_adapter_version, ProofApplyConfig, ProofApplySideEffectState,
    ProofAssembleConfig, ProofAssembleOutputState, ProofConfirmation, ProofFact, ProofFactRequest,
    ProofFactResponse, ProofIdempotencyInput, ProofIntent, ProofMutationCapability, ProofOutput,
    ProofReadCapability, ProofReadConfig, ProofReadFactState, ProofReceipt, ProofReplayError,
    ProofReplayVerifier, ProofSideEffectResult, ProofSubmission, RecordedProofFacts,
};
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{ContentDigest, DescriptorId};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCellTerminal, MaterializedInputNode,
    RunnerArtifactBuilder, RunnerCapabilityBinding, RunnerOutputBuilder, RunnerPayloadBuilder,
    RunnerRegistrationBuilder, RunnerSideEffectBinding, SideEffectClaimAuthority, SideEffectDriver,
    SideEffectDriverCallbacks, SideEffectDriverFuture, SideEffectIntentPlan,
    SideEffectObservedEvidence, SideEffectProtocolAction, SideEffectReplayEvidence,
    SideEffectSubmissionDecision, SideEffectSubmissionDecisionFuture,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::MfmConfig;
use serde::Serialize;

const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const PURE_FACTORY: &str = "pure";
const REPLAY_VERIFIER_ID: &str = "mfm.proof.replay.deterministic.v1";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.proof.runtime.deterministic.v1";

/// Registers deterministic typed proof runners.
pub fn register_deterministic_proof_runners(
    registry: &mut ErasedRunnerRegistry,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let read = registered_descriptor::<ProofReadFactState>()?;
    let side_effect = registered_descriptor::<ProofApplySideEffectState>()?;
    let assemble = registered_descriptor::<ProofAssembleOutputState>()?;
    let mut registrations = RunnerRegistrationBuilder::new(registry, implementation_id);

    register_runner(
        &mut registrations,
        read.descriptor_id,
        &read.capabilities,
        READ_FACTORY,
        Arc::new(ProofReadRunner),
    )?;
    register_runner(
        &mut registrations,
        side_effect.descriptor_id,
        &side_effect.capabilities,
        SIDE_EFFECT_FACTORY,
        Arc::new(ProofSideEffectRunner),
    )?;
    register_runner(
        &mut registrations,
        assemble.descriptor_id,
        &assemble.capabilities,
        PURE_FACTORY,
        Arc::new(ProofAssembleRunner),
    )?;
    Ok(())
}

struct RegisteredRuntimeDescriptor {
    descriptor_id: DescriptorId,
    capabilities: CapabilitySetDescriptor,
}

fn registered_descriptor<S>() -> mfm_runtime::Result<RegisteredRuntimeDescriptor>
where
    S: mfm_program::StateSpec,
    S::Effect: mfm_program::EffectRunner<S>,
    S::Caps: mfm_capabilities::CapabilitySetFor<S::Effect>,
{
    let mut states = mfm_program::StateRegistryBuilder::new();
    let registered = states
        .register::<S>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    Ok(RegisteredRuntimeDescriptor {
        descriptor_id: registered.descriptor().descriptor_id().clone(),
        capabilities: registered.descriptor().capabilities().clone(),
    })
}

fn register_runner(
    registrations: &mut RunnerRegistrationBuilder<'_>,
    descriptor_id: DescriptorId,
    capabilities: &CapabilitySetDescriptor,
    factory: &'static str,
    runner: Arc<dyn ErasedNodeRunner>,
) -> mfm_runtime::Result<()> {
    let factory_id = events::RunnerFactoryId::new(factory)?;
    registrations.register_descriptor(
        descriptor_id,
        capabilities,
        factory_id.clone(),
        executable(factory_id)?,
        runner,
    )?;
    Ok(())
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
    let artifacts = RunnerArtifactBuilder::new(&ctx);
    let payloads = RunnerPayloadBuilder::new(&ctx);
    let fact = ProofFact { n: 1 };
    let request = ProofFactRequest {
        source: "deterministic-proof".to_owned(),
    };
    let response = ProofFactResponse { fact: fact.clone() };
    let response_artifact = artifacts.fact_response(&response)?;
    let output_artifact = artifacts.state_output(&fact)?;
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.stage_attempt_artifact(&response_artifact)?;
    output.retain_runtime_evidence(&response_artifact);
    output.stage_attempt_artifact(&output_artifact)?;
    output.retain_runtime_evidence(&output_artifact);
    output.payload(payloads.fact_recorded(
        events::FactKey::new("mfm.proof.fact.default")?,
        &request,
        &response_artifact,
        proof_read_binding()?,
    )?);
    output.payload(payloads.cell_produced(&output_artifact)?);
    Ok(output.finish())
}

async fn run_side_effect(ctx: ErasedRunCtx<'_>) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config =
        ProofApplyConfig::new("accept").map_err(mfm_runtime::RuntimeError::InvalidRunnerOutput)?;
    ensure_config::<ProofApplyConfig>(&ctx.node().config_ref, &config)?;
    SideEffectDriver::drive(ctx, &ProofSideEffectCallbacks).await
}

struct ProofSideEffectCallbacks;

impl SideEffectDriverCallbacks for ProofSideEffectCallbacks {
    type Intent = ProofIntent;
    type Idempotency = ProofIdempotencyInput;
    type PreparedInvocation = serde_json::Value;
    type Submission = ProofSubmission;
    type SubmissionUnknownEvidence = ProofSideEffectResult;
    type NotSubmittedProof = ProofSideEffectResult;
    type Receipt = ProofReceipt;
    type Confirmation = ProofConfirmation;
    type AmbiguityEvidence = ProofSideEffectResult;
    type Output = ProofSideEffectResult;

    fn intent_and_idempotency<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
    ) -> SideEffectDriverFuture<'a, SideEffectIntentPlan<Self::Intent, Self::Idempotency>> {
        Box::pin(async {
            let idempotency = proof_idempotency_input();
            let idem_hash = digest_value(&idempotency)?;
            Ok(SideEffectIntentPlan {
                side_effect: RunnerSideEffectBinding {
                    ledger_key: events::SideEffectLedgerKey::new("mfm.proof.ledger.default")?,
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    invocation_epoch: 1,
                },
                claim: SideEffectClaimAuthority {
                    claim_owner: events::RunnerInvocationId::new("mfm.proof.owner.1")?,
                    claim_generation: 1,
                    claim_fencing_token: side_effect::ClaimFencingToken::new("mfm.proof.token.1")?,
                    resource_key: None,
                },
                intent: proof_intent(),
                idempotency,
                idempotency_key: events::IdempotencyKeyRef::new(format!(
                    "idem-{}",
                    short_digest(&idem_hash)
                ))?,
                capability_binding: proof_mutation_binding()?,
            })
        })
    }

    fn prepare_invocation<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _plan: &'a SideEffectIntentPlan<Self::Intent, Self::Idempotency>,
    ) -> SideEffectDriverFuture<'a, Option<Self::PreparedInvocation>> {
        Box::pin(async { Ok(None) })
    }

    fn reconstruct_prepared_invocation<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation> {
        Box::pin(async {
            Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "deterministic proof side effect does not use prepared invocation evidence"
                    .to_owned(),
            ))
        })
    }

    fn submit_or_recover_submission<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _action: SideEffectProtocolAction,
        _prepared: Option<Self::PreparedInvocation>,
    ) -> SideEffectSubmissionDecisionFuture<
        'a,
        Self::Submission,
        Self::SubmissionUnknownEvidence,
        Self::NotSubmittedProof,
        Self::AmbiguityEvidence,
    > {
        Box::pin(async { Ok(SideEffectSubmissionDecision::Observed(proof_submission()?)) })
    }

    fn read_receipt<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        Box::pin(async {
            Ok(SideEffectObservedEvidence {
                evidence: proof_receipt()?,
                replay: proof_replay_evidence()?,
            })
        })
    }

    fn build_confirmation<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        Box::pin(async {
            Ok(SideEffectObservedEvidence {
                evidence: proof_confirmation()?,
                replay: proof_replay_evidence()?,
            })
        })
    }

    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async { proof_side_effect_result() })
    }
}

fn proof_replay_evidence() -> mfm_runtime::Result<SideEffectReplayEvidence> {
    Ok(SideEffectReplayEvidence {
        replay_verifier_id: replay_verifier_id()?,
        resource_touched_set: None,
    })
}

async fn run_assemble(ctx: ErasedRunCtx<'_>) -> mfm_runtime::Result<ErasedRunnerOutput> {
    ensure_config::<ProofAssembleConfig>(&ctx.node().config_ref, &ProofAssembleConfig {})?;
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
    let artifacts = RunnerArtifactBuilder::new(&ctx);
    let payloads = RunnerPayloadBuilder::new(&ctx);
    let artifact = artifacts.state_output(&output)?;
    let mut runner_output = RunnerOutputBuilder::new(&ctx);
    runner_output.stage_attempt_artifact(&artifact)?;
    runner_output.retain_runtime_evidence(&artifact);
    runner_output.payload(payloads.cell_produced(&artifact)?);
    Ok(runner_output.finish())
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

fn proof_read_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    Ok(RunnerCapabilityBinding {
        capability_kind: ProofReadCapability::kind().map_err(runtime_capability_error)?,
        capability_version: ProofReadCapability::version().map_err(runtime_capability_error)?,
        adapter_kind: proof_adapter_kind()?,
        adapter_version: proof_adapter_version()?,
    })
}

fn proof_mutation_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    Ok(RunnerCapabilityBinding {
        capability_kind: ProofMutationCapability::kind().map_err(runtime_capability_error)?,
        capability_version: ProofMutationCapability::version().map_err(runtime_capability_error)?,
        adapter_kind: proof_adapter_kind()?,
        adapter_version: proof_adapter_version()?,
    })
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
    let frames = broker.side_effect_replay_frames_matching(is_deterministic_proof_intent)?;
    if frames.is_empty() {
        return Ok(false);
    }
    if frames.len() > 1 {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "multiple deterministic proof side-effect intents in one run",
        ));
    }
    let frames = frames[0];
    let verifier = DeterministicProofReplayVerifier::new().map_err(replay_runtime_error)?;
    let Some(submission_request) = frames.submission_request() else {
        return Err(proof_side_effect_missing("submission"));
    };
    let Some(receipt_request) = frames.receipt_request() else {
        return Err(proof_side_effect_missing("receipt"));
    };
    let Some(confirmation_request) = frames.confirmation_request() else {
        return Err(proof_side_effect_missing("confirmation"));
    };

    broker.verify_side_effect_submission(&submission_request, &verifier)?;
    broker.verify_side_effect_receipt(&receipt_request, &verifier)?;
    broker.verify_side_effect_confirmation(&confirmation_request, &verifier)?;
    Ok(true)
}

fn is_deterministic_proof_intent(intent: &side_effect::IntentPersisted) -> replay::Result<bool> {
    let expected_capability = ProofMutationCapability::kind().map_err(replay_capability_error)?;
    let expected_adapter = proof_adapter_kind().map_err(replay_identity_error)?;
    Ok(intent.capability_kind == expected_capability && intent.adapter_kind == expected_adapter)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_identity_summary_matches_golden() {
        assert_eq!(
            executable_identity_summary([READ_FACTORY, SIDE_EFFECT_FACTORY, PURE_FACTORY]),
            [
                "factory=read_external;source=mfm-transports-proof-built-in;package=mfm-transports-proof;version=0.1.0;cargo_digest=content:sha256-jcs-v1:8e5756e097f23f2a6d5fe8c69846ba7ca609c5a7816528a85d7718cd760357f2;binary_digest=content:sha256-jcs-v1:67de41659eff846aa936862dfb86e7bb6acc9405aeb40f46bddfc4e287cb7f1b;nix_derivation=false;nix_output=false",
                "factory=apply_side_effect;source=mfm-transports-proof-built-in;package=mfm-transports-proof;version=0.1.0;cargo_digest=content:sha256-jcs-v1:8e5756e097f23f2a6d5fe8c69846ba7ca609c5a7816528a85d7718cd760357f2;binary_digest=content:sha256-jcs-v1:67de41659eff846aa936862dfb86e7bb6acc9405aeb40f46bddfc4e287cb7f1b;nix_derivation=false;nix_output=false",
                "factory=pure;source=mfm-transports-proof-built-in;package=mfm-transports-proof;version=0.1.0;cargo_digest=content:sha256-jcs-v1:8e5756e097f23f2a6d5fe8c69846ba7ca609c5a7816528a85d7718cd760357f2;binary_digest=content:sha256-jcs-v1:67de41659eff846aa936862dfb86e7bb6acc9405aeb40f46bddfc4e287cb7f1b;nix_derivation=false;nix_output=false",
            ]
        );
    }

    fn executable_identity_summary(factories: [&str; 3]) -> Vec<String> {
        factories
            .into_iter()
            .map(|factory| {
                let identity =
                    executable(events::RunnerFactoryId::new(factory).expect("factory id"))
                        .expect("executable identity");
                format!(
                    "factory={};source={};package={};version={};cargo_digest={};binary_digest={};nix_derivation={};nix_output={}",
                    identity.factory_id,
                    identity.source_revision,
                    identity.cargo_package_name,
                    identity.cargo_package_version,
                    identity.cargo_package_digest,
                    identity.binary_digest,
                    identity.nix_derivation_hash.is_some(),
                    identity.nix_output_hash.is_some()
                )
            })
            .collect()
    }
}
