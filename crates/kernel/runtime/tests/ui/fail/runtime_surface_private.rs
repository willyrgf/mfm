fn fact_record_input_fields<T>(input: mfm_runtime::FactRecordInput<T>)
where
    T: mfm_program::MfmFactType,
{
    let _ = input.fact;
    let _ = input.visibility;
    let _ = input.observed_at;
}

fn erased_runner_output_fields() {
    let output = mfm_runtime::ErasedRunnerOutput::new(Vec::new());
    let _ = output.staged_artifacts;
    let _ = output.staged_retention_refs;
    let _ = output.payloads;
}

fn runner_capability_binding_fields(binding: mfm_runtime::RunnerCapabilityBinding) {
    let _ = binding.capability_kind;
    let _ = binding.capability_version;
    let _ = binding.adapter_kind;
    let _ = binding.adapter_version;
}

fn side_effect_artifact_builder_helpers() {
    let _ = mfm_runtime::RunnerArtifactBuilder::side_effect_intent;
    let _ = mfm_runtime::RunnerArtifactBuilder::prepared_invocation;
    let _ = mfm_runtime::RunnerArtifactBuilder::staged_side_effect;
}

fn side_effect_payload_builder_helpers() {
    let _ = mfm_runtime::RunnerPayloadBuilder::side_effect_failed;
    let _ = core::mem::size_of::<mfm_runtime::RunnerSideEffectBinding>();
}

fn side_effect_callback_dto_fields() {
    let _ = mfm_runtime::SideEffectIntentPlan::<(), ()> {};
    let _ = mfm_runtime::SideEffectReplayEvidence {};
    let _ = mfm_runtime::SideEffectObservedEvidence::<()> {};
}

fn removed_public_types() {
    let _ = core::mem::size_of::<mfm_runtime::StagedFactRecord>();
    let _ = mfm_runtime::SideEffectLanePreclaimBuilder::new;
    let _ = core::mem::size_of::<mfm_runtime::SideEffectPreparedInvocationPlan<()>>();
}

fn staged_artifact_metadata() {
    let _ = core::mem::size_of::<mfm_runtime::StagedArtifactHandle>();
    let _ = core::mem::size_of::<mfm_runtime::StagedArtifactBindingKind>();
    let _ = core::mem::size_of::<mfm_runtime::StagedSideEffectArtifactPhase>();
    let _ = mfm_runtime::StagedArtifact::handle;
}

fn side_effect_staged_artifact_helpers() {
    let _ = mfm_runtime::StagedArtifact::inline_side_effect_artifact;
    let _ = mfm_runtime::StagedArtifact::inline_pre_invocation_side_effect_artifact;
}

fn side_effect_attempt_view() {
    let _ = core::mem::size_of::<mfm_runtime::SideEffectAttemptView<'_>>();
}

fn main() {}
