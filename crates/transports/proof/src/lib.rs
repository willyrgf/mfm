#![warn(missing_docs)]
//! Deterministic typed proof implementation.
//!
//! This crate provides the enabled proof implementation used by conformance tests and by typed
//! runner assembly. It exposes typed runtime runners and replay verifiers.

use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::CapabilitySpec;
use mfm_collectors_proof::{
    proof_adapter_kind, proof_adapter_version, ProofApplyConfig, ProofApplySideEffectState,
    ProofAssembleConfig, ProofAssembleOutputState, ProofConfirmation, ProofFact,
    ProofIdempotencyInput, ProofIntent, ProofMutationCapability, ProofOutput, ProofReadConfig,
    ProofReadFactState, ProofReceipt, ProofReplayError, ProofReplayVerifier, ProofSideEffectResult,
    ProofSubmission, RecordedProofFacts, MANUAL_RESOLUTION_PROOF_ACTION,
};
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{short_stable_id_fragment, ContentDigest};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCellTerminal, MaterializedInputNode,
    MaterializedInputs, RunnerArtifactBuilder, RunnerCapabilityBinding, RunnerOutputBuilder,
    RunnerPayloadBuilder, RunnerRegistrationBuilder, SideEffectDriver, SideEffectDriverCallbacks,
    SideEffectDriverFuture, SideEffectIntentPlan, SideEffectObservedEvidence,
    SideEffectProtocolAction, SideEffectReplayEvidence, SideEffectSubmissionDecision,
    SideEffectUnknownSubmissionDecision, SideEffectVerifyCallbacks, SideEffectVerifyDriver,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::MfmConfig;
use serde::Serialize;

const ACCEPT_PROOF_ACTION: &str = "accept";
const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const PURE_FACTORY: &str = "pure";
const ADAPTER_FACTORY: &str = "proof_adapter";
const REPLAY_VERIFIER_ID: &str = "mfm.proof.replay.deterministic.v1";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.proof.runtime.deterministic.v1";

/// Registers deterministic typed proof runners.
pub fn register_deterministic_proof_runners(
    registry: &mut ErasedRunnerRegistry,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry, implementation_id);
    let adapter_factory = events::RunnerFactoryId::new(ADAPTER_FACTORY)?;
    registrations.register_adapter_executable(
        proof_adapter_kind()?,
        proof_adapter_version()?,
        executable(adapter_factory)?,
    )?;

    let read = mfm_program::registered_state_descriptor::<ProofReadFactState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let read_factory = events::RunnerFactoryId::new(READ_FACTORY)?;
    registrations.register_descriptor(
        read.descriptor_id().clone(),
        read.capabilities(),
        read_factory.clone(),
        executable(read_factory.clone())?,
        Arc::new(ProofReadRunner),
    )?;
    let side_effect = mfm_program::registered_state_descriptor::<ProofApplySideEffectState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let side_effect_factory = events::RunnerFactoryId::new(SIDE_EFFECT_FACTORY)?;
    registrations.register_descriptor(
        side_effect.descriptor_id().clone(),
        side_effect.capabilities(),
        side_effect_factory.clone(),
        executable(side_effect_factory)?,
        Arc::new(ProofSideEffectRunner),
    )?;
    registrations.register_side_effect_verify_runner(
        side_effect.descriptor_id().clone(),
        read_factory.clone(),
        executable(read_factory)?,
        Arc::new(ProofSideEffectRunner),
    )?;
    let assemble = mfm_program::registered_state_descriptor::<ProofAssembleOutputState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let assemble_factory = events::RunnerFactoryId::new(PURE_FACTORY)?;
    registrations.register_descriptor(
        assemble.descriptor_id().clone(),
        assemble.capabilities(),
        assemble_factory.clone(),
        executable(assemble_factory)?,
        Arc::new(ProofAssembleRunner),
    )?;
    Ok(())
}

fn executable(
    factory_id: events::RunnerFactoryId,
) -> mfm_runtime::Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id,
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
    let output_artifact = artifacts.state_output(&fact)?;
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.stage_attempt_artifact(&output_artifact)?;
    output.retain_runtime_evidence(&output_artifact);
    output.payload(payloads.cell_produced(&output_artifact)?);
    Ok(output.finish())
}

async fn run_side_effect(ctx: ErasedRunCtx<'_>) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config_node = proof_side_effect_config_node(&ctx)?;
    let accept = ProofApplyConfig::new(ACCEPT_PROOF_ACTION)
        .map_err(mfm_runtime::RuntimeError::InvalidRunnerOutput)?;
    let (action, ambiguous) =
        if ensure_config::<ProofApplyConfig>(&config_node.config_ref, &accept).is_ok() {
            (ACCEPT_PROOF_ACTION, false)
        } else {
            let manual_resolution = ProofApplyConfig::new(MANUAL_RESOLUTION_PROOF_ACTION)
                .map_err(mfm_runtime::RuntimeError::InvalidRunnerOutput)?;
            ensure_config::<ProofApplyConfig>(&config_node.config_ref, &manual_resolution)?;
            (MANUAL_RESOLUTION_PROOF_ACTION, true)
        };
    let callbacks = ProofSideEffectCallbacks { action, ambiguous };
    if matches!(
        &ctx.node().framework,
        Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
    ) {
        return SideEffectVerifyDriver::drive(ctx, &callbacks).await;
    }
    SideEffectDriver::drive(ctx, &callbacks).await
}

fn proof_side_effect_config_node<'a>(
    ctx: &'a ErasedRunCtx<'a>,
) -> mfm_runtime::Result<&'a spec::NodeSpec> {
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &ctx.node().framework else {
        return Ok(ctx.node());
    };
    let submit_node = ctx.certified_node(&verify.submit_node_id).ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "proof side-effect verify node {} references missing submit node {}",
            ctx.node().node_id,
            verify.submit_node_id
        ))
    })?;
    if submit_node.side_effect.is_none() {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "proof side-effect verify node {} references non-side-effect submit node {}",
            ctx.node().node_id,
            submit_node.node_id
        )));
    }
    Ok(submit_node)
}

struct ProofSideEffectCallbacks {
    action: &'static str,
    ambiguous: bool,
}

impl SideEffectDriverCallbacks for ProofSideEffectCallbacks {
    type Intent = ProofIntent;
    type Idempotency = ProofIdempotencyInput;
    type PreparedInvocation = serde_json::Value;
    type Submission = ProofSubmission;
    type SubmissionUnknownEvidence = ProofSideEffectResult;
    type NotSubmittedProof = ProofSideEffectResult;
    type AmbiguityEvidence = ProofSideEffectResult;

    fn intent_and_idempotency<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
    ) -> SideEffectDriverFuture<'a, SideEffectIntentPlan<Self::Intent, Self::Idempotency>> {
        let action = self.action;
        Box::pin(async move {
            let idempotency = proof_idempotency_input(action);
            let idem_hash = digest_value(&idempotency)?;
            Ok(SideEffectIntentPlan::new(
                proof_intent(action),
                idempotency,
                events::IdempotencyKeyRef::new(format!(
                    "idem-{}",
                    short_stable_id_fragment(idem_hash.as_str(), 16)
                ))?,
                proof_mutation_binding()?,
            ))
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
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectSubmissionDecision<
            Self::Submission,
            Self::SubmissionUnknownEvidence,
            Self::NotSubmittedProof,
            Self::AmbiguityEvidence,
        >,
    > {
        Box::pin(async move {
            if self.ambiguous {
                Ok(SideEffectSubmissionDecision::Unknown(
                    proof_side_effect_result()?,
                ))
            } else {
                Ok(SideEffectSubmissionDecision::Observed(proof_submission()?))
            }
        })
    }
}

impl SideEffectVerifyCallbacks for ProofSideEffectCallbacks {
    type Submission = ProofSubmission;
    type Receipt = ProofReceipt;
    type Confirmation = ProofConfirmation;
    type Output = ProofSideEffectResult;
    type NotSubmittedProof = ProofSideEffectResult;
    type AmbiguityEvidence = ProofSideEffectResult;

    fn recover_unknown_submission<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _prepared_invocation: Option<&'a store::SideEffectArtifactProjection>,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectUnknownSubmissionDecision<
            Self::Submission,
            Self::NotSubmittedProof,
            Self::AmbiguityEvidence,
        >,
    > {
        Box::pin(async move {
            if self.ambiguous {
                Ok(SideEffectUnknownSubmissionDecision::Ambiguous {
                    ambiguity_code: events::AmbiguityCode::new("mfm.proof.manual_resolution")
                        .map_err(|error| {
                            mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                        })?,
                    evidence: proof_side_effect_result()?,
                })
            } else {
                Ok(SideEffectUnknownSubmissionDecision::Observed(
                    proof_submission()?,
                ))
            }
        })
    }

    fn read_receipt<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        Box::pin(async {
            Ok(SideEffectObservedEvidence::new(
                proof_receipt()?,
                proof_replay_evidence()?,
            ))
        })
    }

    fn build_confirmation<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        Box::pin(async {
            Ok(SideEffectObservedEvidence::new(
                proof_confirmation()?,
                proof_replay_evidence()?,
            ))
        })
    }

    fn map_receipt_to_output<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async { proof_side_effect_result() })
    }

    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async { proof_side_effect_result() })
    }
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

fn proof_mutation_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    RunnerCapabilityBinding::for_capability::<ProofMutationCapability>(
        proof_adapter_kind()?,
        proof_adapter_version()?,
    )
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

fn proof_fact() -> ProofFact {
    ProofFact { n: 1 }
}

fn proof_intent(action: &str) -> ProofIntent {
    ProofIntent {
        fact_n: 1,
        action: action.to_owned(),
    }
}

fn proof_idempotency_input(action: &str) -> ProofIdempotencyInput {
    ProofIdempotencyInput {
        fact_n: 1,
        action: action.to_owned(),
    }
}

fn proof_submission() -> mfm_runtime::Result<ProofSubmission> {
    let idempotency_digest = digest_value(&proof_idempotency_input(ACCEPT_PROOF_ACTION))?
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

fn replay_verifier_id() -> mfm_runtime::Result<events::ReplayVerifierId> {
    Ok(events::ReplayVerifierId::new(REPLAY_VERIFIER_ID)?)
}

fn proof_replay_evidence() -> mfm_runtime::Result<SideEffectReplayEvidence> {
    Ok(SideEffectReplayEvidence::new(replay_verifier_id()?, None))
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
            &proof_intent(ACCEPT_PROOF_ACTION),
        )?;
        ensure_digest(
            "proof idempotency",
            &input.intent.intent.idempotency_input_hash,
            &proof_idempotency_input(ACCEPT_PROOF_ACTION),
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
            &proof_intent(ACCEPT_PROOF_ACTION),
            &submission,
            &RecordedProofFacts { fact: proof_fact() },
        )
        .map_err(proof_replay_error)
    }

    fn verify_receipt(&self, input: &replay::SideEffectReceiptReplayInput) -> replay::Result<()> {
        ensure_digest(
            "proof intent",
            &input.intent.intent.intent_hash,
            &proof_intent(ACCEPT_PROOF_ACTION),
        )?;
        ensure_digest(
            "proof idempotency",
            &input.intent.intent.idempotency_input_hash,
            &proof_idempotency_input(ACCEPT_PROOF_ACTION),
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
            &proof_intent(ACCEPT_PROOF_ACTION),
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
            &proof_intent(ACCEPT_PROOF_ACTION),
        )?;
        ensure_digest(
            "proof idempotency",
            &input.intent.intent.idempotency_input_hash,
            &proof_idempotency_input(ACCEPT_PROOF_ACTION),
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
    let frame = &frames[0];
    match deterministic_proof_action(frame.intent)? {
        DeterministicProofAction::Accept => verify_accepted_proof_replay(broker, frame)?,
        DeterministicProofAction::ManualResolution => {
            verify_manual_resolution_proof_replay(broker, frame)?
        }
    }
    Ok(true)
}

fn verify_accepted_proof_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::SideEffectReplayFrame<'_>,
) -> replay::Result<()> {
    if frame.ambiguity.is_some() {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "accepted deterministic proof side effect recorded ambiguity evidence",
        ));
    }
    let verifier = DeterministicProofReplayVerifier::new().map_err(replay_runtime_error)?;
    let Some(submission_request) = frame.submission_request() else {
        return Err(proof_side_effect_missing("submission"));
    };
    let Some(receipt_request) = frame.receipt_request() else {
        return Err(proof_side_effect_missing("receipt"));
    };
    let Some(confirmation_request) = frame.confirmation_request() else {
        return Err(proof_side_effect_missing("confirmation"));
    };

    broker.verify_side_effect_submission(&submission_request, &verifier)?;
    broker.verify_side_effect_receipt(&receipt_request, &verifier)?;
    broker.verify_side_effect_confirmation(&confirmation_request, &verifier)?;
    Ok(())
}

fn verify_manual_resolution_proof_replay(
    broker: &replay::ReplayBroker,
    frame: &replay::SideEffectReplayFrame<'_>,
) -> replay::Result<()> {
    if frame.submission.is_some() || frame.receipt.is_some() || frame.confirmation.is_some() {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "manual-resolution deterministic proof side effect recorded non-ambiguity evidence",
        ));
    }
    let Some(ambiguity_request) = frame.ambiguity_request() else {
        return Err(proof_side_effect_missing("ambiguity"));
    };
    let ambiguity = broker.side_effect_ambiguity(&ambiguity_request)?;
    let expected_code =
        events::AmbiguityCode::new("mfm.proof.manual_resolution").map_err(|error| {
            replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                error.to_string(),
            )
        })?;
    if ambiguity.ambiguity.ambiguity_code != expected_code {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "manual-resolution deterministic proof ambiguity code mismatch",
        ));
    }
    let expected = proof_side_effect_result().map_err(replay_runtime_error)?;
    ensure_digest(
        "proof ambiguity",
        &ambiguity.ambiguity.evidence_hash,
        &expected,
    )?;
    ensure_digest(
        "proof ambiguity artifact",
        &ambiguity.artifact.digest,
        &expected,
    )?;
    Ok(())
}

fn is_deterministic_proof_intent(intent: &side_effect::IntentPersisted) -> replay::Result<bool> {
    let expected_capability = ProofMutationCapability::kind().map_err(replay_capability_error)?;
    let expected_adapter = proof_adapter_kind().map_err(replay_identity_error)?;
    Ok(intent.capability_kind == expected_capability && intent.adapter_kind == expected_adapter)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeterministicProofAction {
    Accept,
    ManualResolution,
}

fn deterministic_proof_action(
    intent: &side_effect::IntentPersisted,
) -> replay::Result<DeterministicProofAction> {
    if proof_intent_matches_action(intent, ACCEPT_PROOF_ACTION)? {
        return Ok(DeterministicProofAction::Accept);
    }
    if proof_intent_matches_action(intent, MANUAL_RESOLUTION_PROOF_ACTION)? {
        return Ok(DeterministicProofAction::ManualResolution);
    }
    Err(replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        "deterministic proof intent evidence did not match a supported action",
    ))
}

fn proof_intent_matches_action(
    intent: &side_effect::IntentPersisted,
    action: &str,
) -> replay::Result<bool> {
    Ok(
        intent.intent_hash == digest_value(&proof_intent(action)).map_err(replay_runtime_error)?
            && intent.idempotency_input_hash
                == digest_value(&proof_idempotency_input(action)).map_err(replay_runtime_error)?,
    )
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
mod tests;
