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
    ProofIdempotencyInput, ProofIntent, ProofMutationCapability, ProofOutput, ProofReadEvidence,
    ProofReadFactState, ProofReadPlan, ProofReceipt, ProofReplayError, ProofReplayVerifier,
    ProofSideEffectResult, ProofSubmission, RecordedProofFacts, MANUAL_RESOLUTION_PROOF_ACTION,
};
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::ContentDigest;
use mfm_program::{SideEffectState, StateSpec};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config_for_node, load_side_effect_artifact,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, ExternalReadExecution, ExternalReadExecutionFuture,
    ExternalReadPlanExecutor, ExternalReadRunner, MaterializedCellTerminal, MaterializedInputNode,
    MaterializedInputs, RunnerArtifactBuilder, RunnerCapabilityBinding, RunnerOutputBuilder,
    RunnerPayloadBuilder, RunnerRegistrationBuilder, SideEffectAdapter, SideEffectDriver,
    SideEffectDriverFuture, SideEffectObservedEvidence, SideEffectPreparedInvocation,
    SideEffectReplayEvidence, SideEffectSubmissionDecision, SideEffectUnknownSubmissionDecision,
    SideEffectVerifyDriver,
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
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let adapter_factory = events::RunnerFactoryId::new(ADAPTER_FACTORY)?;
    let read = mfm_program::state_descriptor::<ProofReadFactState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let side_effect = mfm_program::state_descriptor::<ProofApplySideEffectState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let assemble = mfm_program::state_descriptor::<ProofAssembleOutputState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registry.register_capability_set(read.capabilities(), implementation_id.clone())?;
    registry.register_capability_set(side_effect.capabilities(), implementation_id)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    registrations.register_adapter_executable(
        proof_adapter_kind()?,
        proof_adapter_version()?,
        executable(adapter_factory)?,
    )?;

    let read_factory = events::RunnerFactoryId::new(READ_FACTORY)?;
    registrations.register_runner(
        read.descriptor_id().clone(),
        read_factory.clone(),
        executable(read_factory.clone())?,
        Arc::new(ExternalReadRunner::<ProofReadFactState, _>::new(
            artifacts.clone(),
            ProofReadExecutor,
        )),
    )?;
    let side_effect_factory = events::RunnerFactoryId::new(SIDE_EFFECT_FACTORY)?;
    registrations.register_runner(
        side_effect.descriptor_id().clone(),
        side_effect_factory.clone(),
        executable(side_effect_factory)?,
        Arc::new(ProofSideEffectRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_side_effect_verify_runner(
        side_effect.descriptor_id().clone(),
        read_factory.clone(),
        executable(read_factory)?,
        Arc::new(ProofSideEffectRunner { artifacts }),
    )?;
    let assemble_factory = events::RunnerFactoryId::new(PURE_FACTORY)?;
    registrations.register_runner(
        assemble.descriptor_id().clone(),
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

struct ProofReadExecutor;

impl ExternalReadPlanExecutor<ProofReadFactState> for ProofReadExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: mfm_runtime::RunnerIngressContext<'a>,
        _state: &'a ProofReadFactState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a ProofReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, ProofReadEvidence> {
        Box::pin(async move {
            Ok(ExternalReadExecution::primary(ProofReadEvidence {
                fact_n: plan.fact_n,
            }))
        })
    }
}

struct ProofSideEffectRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ProofSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_side_effect(ctx, self.artifacts.clone()).await })
    }
}

struct ProofAssembleRunner;

impl ErasedNodeRunner for ProofAssembleRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_assemble(ctx).await })
    }
}

async fn run_side_effect(
    ctx: ErasedRunCtx<'_>,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
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
    let callbacks = ProofSideEffectCallbacks {
        action: action.to_owned(),
        ambiguous,
        artifacts,
    };
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
    action: String,
    ambiguous: bool,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

async fn proof_state_material(
    ctx: &ErasedRunCtx<'_>,
    submit_node: &spec::NodeSpec,
    submit_inputs: &MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<(
    ProofApplySideEffectState,
    ProofFact,
    mfm_program::CertifiedContext<mfm_program::NoContext>,
)> {
    let config = load_runner_config_for_node::<ProofApplyConfig>(submit_node, artifacts).await?;
    let state = ProofApplySideEffectState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let input = load_materialized_struct_input::<ProofFact>(submit_inputs, artifacts).await?;
    let context = ctx
        .invocation_context_for_node(submit_node)?
        .certified_context::<mfm_program::NoContext>()?;
    Ok((state, input, context))
}

async fn load_proof_side_effect_artifact<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    _producer_node: &spec::NodeSpec,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: mfm_values::MfmValue + serde::de::DeserializeOwned,
{
    load_side_effect_artifact::<T>(artifact, role, artifacts)
        .await
        .map(|(value, _)| value)
}

impl SideEffectAdapter for ProofSideEffectCallbacks {
    type Intent = ProofIntent;
    type Idempotency = ProofIdempotencyInput;
    type PreparedInvocation = ProofIntent;
    type Submission = ProofSubmission;
    type RecoveryEvidence = ProofSideEffectResult;
    type Receipt = ProofReceipt;
    type Confirmation = ProofConfirmation;
    type Output = ProofSideEffectResult;

    fn capability_binding(&self) -> mfm_runtime::Result<RunnerCapabilityBinding> {
        proof_mutation_binding()
    }

    fn authored_intent<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
    ) -> SideEffectDriverFuture<'a, mfm_program::SideEffectIntent<Self::Intent, Self::Idempotency>>
    {
        let artifacts = self.artifacts.clone();
        Box::pin(async move {
            let (state, input, context) =
                proof_state_material(ctx, submit_node, submit_inputs, artifacts.as_ref()).await?;
            state
                .intent(&input, &context)
                .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
        })
    }

    fn prepare<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        intent: &'a Self::Intent,
        _idempotency: &'a Self::Idempotency,
    ) -> SideEffectDriverFuture<'a, SideEffectPreparedInvocation<Self::PreparedInvocation>> {
        let prepared = intent.clone();
        let expected_action = self.action.clone();
        Box::pin(async move {
            if prepared.action != expected_action {
                return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "proof prepared action differs from configured intent".to_owned(),
                ));
            }
            Ok(SideEffectPreparedInvocation::new(prepared))
        })
    }

    fn load_prepared<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation> {
        let artifacts = self.artifacts.clone();
        let expected_action = self.action.clone();
        Box::pin(async move {
            let prepared = load_proof_side_effect_artifact::<ProofIntent>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            if prepared.action != expected_action {
                return Err(mfm_runtime::RuntimeError::InvalidRunStream(
                    "retained proof prepared action differs from configured intent".to_owned(),
                ));
            }
            Ok(prepared)
        })
    }

    fn submit_prepared<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        prepared: Self::PreparedInvocation,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectSubmissionDecision<Self::Submission, Self::RecoveryEvidence>,
    > {
        Box::pin(async move {
            if self.ambiguous {
                Ok(SideEffectSubmissionDecision::Unknown(
                    proof_side_effect_result_for(&prepared, 0, "submission_unknown"),
                ))
            } else {
                Ok(SideEffectSubmissionDecision::Observed(
                    proof_submission_for(&prepared)?,
                ))
            }
        })
    }
    fn recover_unknown<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        prepared: Self::PreparedInvocation,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectUnknownSubmissionDecision<Self::Submission, Self::RecoveryEvidence>,
    > {
        Box::pin(async move {
            if self.ambiguous {
                Ok(SideEffectUnknownSubmissionDecision::Ambiguous {
                    ambiguity_code: events::AmbiguityCode::new("mfm.proof.manual_resolution")
                        .map_err(|error| {
                            mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                        })?,
                    evidence: proof_side_effect_result_for(&prepared, 1, "confirmed"),
                })
            } else {
                Ok(SideEffectUnknownSubmissionDecision::Observed(
                    proof_submission_for(&prepared)?,
                ))
            }
        })
    }

    fn observe_receipt<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        let artifacts = self.artifacts.clone();
        Box::pin(async move {
            let prepared = load_proof_side_effect_artifact::<ProofIntent>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            let submission = load_proof_side_effect_artifact::<ProofSubmission>(
                submission,
                events::ArtifactRole::Submission,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_submission(&prepared, &submission)?;
            Ok(SideEffectObservedEvidence::new(
                proof_receipt_for(&submission),
                proof_replay_evidence()?,
            ))
        })
    }

    fn observe_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        let artifacts = self.artifacts.clone();
        Box::pin(async move {
            let prepared = load_proof_side_effect_artifact::<ProofIntent>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            let submission = load_proof_side_effect_artifact::<ProofSubmission>(
                submission,
                events::ArtifactRole::Submission,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_submission(&prepared, &submission)?;
            let receipt = load_proof_side_effect_artifact::<ProofReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx.node(),
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_receipt(&submission, &receipt)?;
            Ok(SideEffectObservedEvidence::new(
                proof_confirmation_for(&receipt),
                proof_replay_evidence()?,
            ))
        })
    }

    fn output_from_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let artifacts = self.artifacts.clone();
        Box::pin(async move {
            let submit_node = proof_side_effect_config_node(ctx)?;
            let (state, input, context) =
                proof_state_material(ctx, submit_node, submit_inputs, artifacts.as_ref()).await?;
            let authored = state.intent(&input, &context).map_err(proof_state_error)?;
            let prepared = load_proof_side_effect_artifact::<ProofIntent>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_prepared(authored.intent(), &prepared)?;
            let submission = load_proof_side_effect_artifact::<ProofSubmission>(
                submission,
                events::ArtifactRole::Submission,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_submission(&prepared, &submission)?;
            let receipt = load_proof_side_effect_artifact::<ProofReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx.node(),
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_receipt(&submission, &receipt)?;
            state
                .output_from_receipt(&input, &prepared, &submission, &receipt, &context)
                .map_err(proof_state_error)
        })
    }

    fn output_from_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let artifacts = self.artifacts.clone();
        Box::pin(async move {
            let submit_node = proof_side_effect_config_node(ctx)?;
            let (state, input, context) =
                proof_state_material(ctx, submit_node, submit_inputs, artifacts.as_ref()).await?;
            let authored = state.intent(&input, &context).map_err(proof_state_error)?;
            let prepared = load_proof_side_effect_artifact::<ProofIntent>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_prepared(authored.intent(), &prepared)?;
            let submission = load_proof_side_effect_artifact::<ProofSubmission>(
                submission,
                events::ArtifactRole::Submission,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_submission(&prepared, &submission)?;
            let receipt = load_proof_side_effect_artifact::<ProofReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx.node(),
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_receipt(&submission, &receipt)?;
            let confirmation = load_proof_side_effect_artifact::<ProofConfirmation>(
                confirmation,
                events::ArtifactRole::Confirmation,
                ctx.node(),
                artifacts.as_ref(),
            )
            .await?;
            ensure_proof_confirmation(&receipt, &confirmation)?;
            state
                .output_from_confirmation(
                    &input,
                    &prepared,
                    &submission,
                    &receipt,
                    &confirmation,
                    &context,
                )
                .map_err(proof_state_error)
        })
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
    runner_output.retain_runtime_evidence(&artifact)?;
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

fn proof_submission_for(intent: &ProofIntent) -> mfm_runtime::Result<ProofSubmission> {
    let idempotency = ProofIdempotencyInput {
        fact_n: intent.fact_n,
        action: intent.action.clone(),
    };
    let idempotency_digest = digest_value(&idempotency)?.as_str().to_owned();
    Ok(ProofSubmission {
        submission_id: format!("proof-submission-{}-{}", intent.action, intent.fact_n),
        idempotency_digest,
    })
}

fn proof_submission() -> mfm_runtime::Result<ProofSubmission> {
    proof_submission_for(&proof_intent(ACCEPT_PROOF_ACTION))
}

fn proof_receipt_for(submission: &ProofSubmission) -> ProofReceipt {
    ProofReceipt {
        tx_hash: submission
            .submission_id
            .replacen("proof-submission-", "0xproof", 1)
            .replace('-', ""),
        submission_id: submission.submission_id.clone(),
    }
}

fn proof_receipt() -> mfm_runtime::Result<ProofReceipt> {
    Ok(proof_receipt_for(&proof_submission()?))
}

fn proof_confirmation_for(receipt: &ProofReceipt) -> ProofConfirmation {
    ProofConfirmation {
        tx_hash: receipt.tx_hash.clone(),
        confirmations: 1,
    }
}

fn proof_confirmation() -> mfm_runtime::Result<ProofConfirmation> {
    Ok(proof_confirmation_for(&proof_receipt()?))
}

fn proof_side_effect_result_for(
    intent: &ProofIntent,
    confirmations: u64,
    status: &str,
) -> ProofSideEffectResult {
    let submission_id = format!("proof-submission-{}-{}", intent.action, intent.fact_n);
    ProofSideEffectResult {
        tx_hash: submission_id
            .replacen("proof-submission-", "0xproof", 1)
            .replace('-', ""),
        confirmations,
        status: status.to_owned(),
    }
}

fn proof_side_effect_result() -> mfm_runtime::Result<ProofSideEffectResult> {
    Ok(proof_side_effect_result_for(
        &proof_intent(ACCEPT_PROOF_ACTION),
        1,
        "confirmed",
    ))
}

fn ensure_proof_prepared(intent: &ProofIntent, prepared: &ProofIntent) -> mfm_runtime::Result<()> {
    if intent == prepared {
        Ok(())
    } else {
        Err(mfm_runtime::RuntimeError::InvalidRunStream(
            "proof prepared invocation differs from authored intent".to_owned(),
        ))
    }
}

fn ensure_proof_submission(
    prepared: &ProofIntent,
    submission: &ProofSubmission,
) -> mfm_runtime::Result<()> {
    if submission == &proof_submission_for(prepared)? {
        Ok(())
    } else {
        Err(mfm_runtime::RuntimeError::InvalidRunStream(
            "proof submission differs from prepared invocation".to_owned(),
        ))
    }
}

fn ensure_proof_receipt(
    submission: &ProofSubmission,
    receipt: &ProofReceipt,
) -> mfm_runtime::Result<()> {
    if receipt == &proof_receipt_for(submission) {
        Ok(())
    } else {
        Err(mfm_runtime::RuntimeError::InvalidRunStream(
            "proof receipt differs from submission evidence".to_owned(),
        ))
    }
}

fn ensure_proof_confirmation(
    receipt: &ProofReceipt,
    confirmation: &ProofConfirmation,
) -> mfm_runtime::Result<()> {
    if confirmation == &proof_confirmation_for(receipt) {
        Ok(())
    } else {
        Err(mfm_runtime::RuntimeError::InvalidRunStream(
            "proof confirmation differs from receipt evidence".to_owned(),
        ))
    }
}

fn proof_state_error(error: mfm_program::StateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
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
/// The application dispatches this verifier only for proof-owned side-effect intents.
pub fn verify_deterministic_proof_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    replay::verify_external_read_state::<ProofReadFactState>(broker)?;
    let frames = broker.side_effect_replay_frames_matching(is_deterministic_proof_replay_intent)?;
    if frames.is_empty() {
        return Ok(());
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
    Ok(())
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

/// Identifies side-effect intents owned by the deterministic proof replay verifier.
pub fn is_deterministic_proof_replay_intent(
    intent: &side_effect::IntentPersisted,
) -> replay::Result<bool> {
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
