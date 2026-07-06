#![warn(missing_docs)]
//! Serial typed scheduler for certified MFM execution specs.
//!
//! This crate owns the first certified runtime boundary. It derives runnable nodes, materialized
//! input evidence, runner bindings, and runtime capabilities only from a verified
//! [`mfm_certify::CertifiedTypedSpec`] plus store-owned typed projections.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, AttemptId, CapabilityKind, CapabilityVersion, ContentDigest,
    DescriptorId, DigestAlgorithm, NodeId, RunId, SchemaId, SpecHash,
};
use mfm_spec::v1 as spec;

#[cfg(test)]
use mfm_ids::CellId;

mod admission;
mod artifacts;
mod attempt;
mod binding;
mod commit;
mod error;
mod framework;
mod framework_lifecycle;
mod frontier;
mod history;
mod invocation;
mod manual_resolution;
mod recovery;
mod runner_kit;
mod runners;
mod scheduler;
mod side_effect_driver;
mod side_effect_lifecycle;
mod side_effects;
mod spec_authority;
mod transition;

pub use admission::RunAdmissionAuthority;
pub use artifacts::{RuntimeArtifactStore, StagedArtifact, StagedRetentionRefs};
pub use binding::{
    BoundCapabilityAuthority, BoundFrameworkHandlerAuthority, BoundFrameworkHandlerKind,
    BoundRuntimeContext, BoundRuntimeContextLoader,
};
pub use commit::{PreparedRunLaunch, RunLaunchArtifact, RunLaunchEvidence, RunLaunchSeedCell};
pub use error::{RuntimeDiagnosticDetails, RuntimeError};
pub use history::{VerifiedRunContext, VerifiedRunContextLoader, VerifiedRunHistoryView};
pub use invocation::{
    CertifiedInvocationContext, CertifiedRuntimeCapabilities, ErasedRunCtx, MaterializedCell,
    MaterializedCellTerminal, MaterializedInputNode, MaterializedInputs, NamedMaterializedInput,
    PreInvocationRunCtx, PreparedRunnerInvocation, RecordedFact, RecordedFacts,
};
pub use manual_resolution::{
    manual_resolution_block_reason, manual_resolution_stream_prefix_digest,
    unresolved_manual_obligations_digest, ManualResolutionEvidenceArtifact,
};
pub use runner_kit::{
    load_launch_config, load_launch_config_for_node, load_materialized_input_value,
    load_materialized_node_value, load_materialized_struct_field_value,
    load_materialized_struct_input, load_non_empty_materialized_input, load_runner_config,
    load_runner_config_for_node, load_side_effect_artifact, load_side_effect_artifact_for_node,
    load_side_effect_value, load_side_effect_value_for_node, materialized_input_node_json,
    FactRecordInput, RunnerArtifactBuilder, RunnerCapabilityBinding,
    RunnerExecutableIdentityTemplate, RunnerFactoryBinding, RunnerJsonArtifact,
    RunnerOutputBuilder, RunnerPayloadBuilder, RunnerRegistrationBuilder,
    TypedContextOutputExtractor,
};
pub use runners::{
    AdapterExecutableBinding, CapabilityImplementationBinding, CapabilityImplementationId,
    ContextOutputExtractor, ErasedNodeRunner, ErasedRunnerBinding, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, PreInvocationRunnerFuture, RunnerEventPayload,
    RunnerFactRecorded, RunnerIngressContext,
};
pub use scheduler::{ManualResolutionRequest, SchedulerStatus, SerialTypedScheduler};
pub use side_effect_driver::{
    preclaim_side_effect_resource_lane, SideEffectDriver, SideEffectDriverCallbacks,
    SideEffectDriverFuture, SideEffectIntentPlan, SideEffectObservedEvidence,
    SideEffectProtocolAction, SideEffectReplayEvidence, SideEffectSubmissionDecision,
    SideEffectUnknownSubmissionDecision, SideEffectVerifyCallbacks, SideEffectVerifyDriver,
};
pub use spec_authority::CertifiedRuntimeSpec;

#[cfg(test)]
use artifacts::{
    staged_artifact_binding_kind, staged_side_effect_artifact_phase, StagedArtifactBindingKind,
};

#[cfg(test)]
use commit::{retention_manifest_payloads, runner_payloads_with_derived_lifecycle};
#[cfg(test)]
use framework::{
    build_retention_manifest_artifact, certified_complete_run_node,
    certified_retention_manifest_node,
};
#[cfg(test)]
use history::RuntimeRunView;
#[cfg(test)]
use runner_kit::{RunnerClaimBinding, RunnerPreparedInvocationBinding, RunnerSideEffectBinding};
#[cfg(test)]
use side_effect_driver::{
    RuntimeSideEffectClaimAuthority, SideEffectEvidenceBuilder, SideEffectPrepareEvidence,
};
#[cfg(test)]
use side_effect_lifecycle::{side_effect_projection_for_attempt, SideEffectAttemptView};

/// Result type for typed runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

fn validate_public_output(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    validate_public_output_render_node(
        runtime_spec,
        node,
        payload.public_schema_id.clone(),
        &payload.renderer_descriptor_id,
    )?;
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not certified as a public-output render node",
            node.node_id
        )));
    };
    if payload.output_spec_digest != render.output_spec_digest
        || payload.receipt_cell_id != node.output_cell
        || payload.cells.len() != render.required_cells.len()
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "public-output payload for node {} does not match certified render contract",
            node.node_id
        )));
    }
    for (actual, expected) in payload.cells.iter().zip(render.required_cells.iter()) {
        if actual.public_field_path != expected.public_field_path
            || actual.cell_id != expected.cell_id
            || actual.producer != expected.producer
            || actual.scope_id != expected.scope_id
            || actual.semantic_type_id != expected.semantic_type_id
            || actual.schema_id != expected.schema_id
            || actual.value_lineage != expected.value_lineage
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "public-output cell evidence for node {} does not match certified output",
                node.node_id
            )));
        }
    }
    Ok(())
}

fn validate_public_output_render_node(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    public_schema_id: SchemaId,
    renderer_descriptor_id: &DescriptorId,
) -> Result<()> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not certified as a public-output render node",
            node.node_id
        )));
    };
    if render.public_schema_id != public_schema_id
        || runtime_spec.spec().public_outputs.public_schema_id != public_schema_id
        || render.renderer_descriptor.descriptor_id != *renderer_descriptor_id
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "public-output render metadata for node {} does not match certified spec",
            node.node_id
        )));
    }
    Ok(())
}

fn require_attempt(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    actual_node_id: &NodeId,
    actual_attempt_id: &AttemptId,
) -> Result<()> {
    if actual_node_id == &node.node_id && actual_attempt_id == attempt_id {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "payload for node {} attempt {} was bound to node {} attempt {}",
            node.node_id, attempt_id, actual_node_id, actual_attempt_id
        )))
    }
}

fn require_capability(
    caps: &CertifiedRuntimeCapabilities,
    kind: &CapabilityKind,
    version: &CapabilityVersion,
    node_id: &NodeId,
) -> Result<()> {
    if caps.contains(kind, version) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {node_id} used uncertified capability {kind}:{version}"
        )))
    }
}

fn require_adapter(
    node: &spec::NodeSpec,
    kind: &AdapterKind,
    version: &AdapterVersion,
) -> Result<()> {
    if node
        .adapter_bindings
        .iter()
        .any(|adapter| adapter.adapter_kind == *kind && adapter.adapter_version == *version)
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} used uncertified adapter {}:{}",
            node.node_id, kind, version
        )))
    }
}

fn attempt_id(
    run_id: &RunId,
    spec_hash: &SpecHash,
    node_id: &NodeId,
    attempt_no: u32,
) -> Result<AttemptId> {
    let canonical = canonical_json(serde_json::json!({
        "attempt_no": attempt_no,
        "node_id": node_id.as_str(),
        "run_id": run_id.as_str(),
        "spec_hash": spec_hash.as_str(),
    }))?;
    Ok(AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *canonical.content_digest().digest(),
    ))
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(&value)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))
}

fn content_digest_json(value: serde_json::Value) -> Result<ContentDigest> {
    Ok(canonical_json(value)?.content_digest())
}

fn executable_identity_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
    serde_json::json!({
        "binary_digest": identity.binary_digest.as_str(),
        "cargo_package_digest": identity.cargo_package_digest.as_str(),
        "factory_id": identity.factory_id.as_str(),
        "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(events::NixDerivationHash::as_str),
        "nix_output_hash": identity.nix_output_hash.as_ref().map(events::NixOutputHash::as_str),
    })
}

fn config_ref_key(config_ref: &spec::ConfigRef) -> String {
    format!("{}:{}", config_ref.schema_id, config_ref.digest)
}

#[cfg(test)]
mod tests;
