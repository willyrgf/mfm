#![warn(missing_docs)]
//! Serial typed scheduler for certified MFM execution specs.
//!
//! This crate owns the first certified runtime boundary. It derives runnable nodes, materialized
//! input evidence, runner bindings, and runtime capabilities only from a verified
//! [`mfm_certify::CertifiedTypedSpec`] plus store-owned typed projections.

use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, AttemptId, CapabilityKind, CapabilityVersion, ContentDigest,
    DescriptorId, DigestAlgorithm, NodeId, RunId, SchemaId, SpecHash,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

#[cfg(test)]
use mfm_ids::{ArtifactId, CellId};
#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};

mod artifacts;
mod commit;
mod error;
mod framework;
mod frontier;
mod history;
mod invocation;
mod runners;
mod side_effects;
mod spec_authority;

pub use artifacts::{
    RuntimeArtifactStageFuture, RuntimeArtifactStager, StagedArtifact, StagedArtifactBindingKind,
    StagedArtifactHandle, StagedRetentionRefs, StagedSideEffectArtifactPhase,
};
pub use commit::{PreparedRunLaunch, RunLaunchArtifact, RunLaunchEvidence, RunLaunchSeedCell};
pub use error::RuntimeError;
pub use framework::build_public_output_receipt_artifact;
pub use history::{validate_run_stream, VerifiedRunStream};
pub use invocation::{
    CertifiedRuntimeCapabilities, ErasedRunCtx, MaterializedCell, MaterializedCellTerminal,
    MaterializedInputNode, MaterializedInputs, NamedMaterializedInput, PreparedRunnerInvocation,
    RecordedFact, RecordedFacts,
};
pub use runners::{
    ErasedNodeRunner, ErasedRunnerBinding, ErasedRunnerFuture, ErasedRunnerOutput,
    ErasedRunnerRegistry, RunnerEventPayload,
};
pub use spec_authority::CertifiedRuntimeSpec;

use artifacts::verify_artifact_bytes;
use commit::{CommitPlanner, PreparedStagedArtifact, RunnerOutputCommitInput};
use error::async_store_error;
use frontier::{scheduler_decision, AttemptPlan, RunnableNode, SchedulerDecision};
use history::{
    committed_config_artifact, materialize_inputs, recorded_facts_for_attempt, RuntimeRunView,
};

#[cfg(test)]
use artifacts::{staged_artifact_binding_kind, staged_side_effect_artifact_phase};

#[cfg(test)]
use commit::{retention_manifest_payloads, runner_payloads_with_derived_lifecycle};
#[cfg(test)]
use framework::{
    build_retention_manifest_artifact_with_producer, certified_bootstrap_run_node,
    certified_complete_run_node, certified_retention_manifest_node,
};
#[cfg(test)]
use history::{next_seq_after_stream, validate_historical_bootstrap_run_batch};
#[cfg(test)]
use side_effects::side_effect_projection_for_attempt;

/// Result type for typed runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Result of one serial scheduler drive call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerStatus {
    /// At least one node attempt ran and committed.
    Advanced,
    /// No node is currently runnable.
    Blocked,
    /// Public output has already been projected.
    PublicOutputProjected,
}

struct RunnerInvocationInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    attempt_id: &'a AttemptId,
    attempt_no: u32,
    view: &'a RuntimeRunView,
}

struct FrameworkNodeAttemptInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    view: &'a RuntimeRunView,
    runnable: RunnableNode<'a>,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    binding: ErasedRunnerBinding,
}

fn prepare_runner_invocation<'a>(
    input: RunnerInvocationInput<'a>,
) -> Result<PreparedRunnerInvocation<'a>> {
    let RunnerInvocationInput {
        runtime_spec,
        run_id,
        node,
        descriptor,
        output_cell,
        attempt_id,
        attempt_no,
        view,
    } = input;
    let config_artifact = committed_config_artifact(node, view)?;
    let inputs = materialize_inputs(runtime_spec, node, view)?;
    let caps =
        CertifiedRuntimeCapabilities::new(node.node_id.clone(), node.capability_bindings.clone());
    let recorded_facts = recorded_facts_for_attempt(&view.projections, &node.node_id, attempt_id)?;
    Ok(PreparedRunnerInvocation {
        runtime_spec,
        run_id,
        spec_hash: runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id,
        attempt_no,
        config_artifact,
        inputs,
        caps,
        recorded_facts,
        projections: &view.projections,
        run_stream: &view.stream,
    })
}

/// Serial typed scheduler.
#[derive(Clone)]
pub struct SerialTypedScheduler {
    runners: ErasedRunnerRegistry,
    artifact_stager: Arc<dyn RuntimeArtifactStager>,
}

impl SerialTypedScheduler {
    /// Creates a scheduler using a certified runner registry and runtime artifact stager.
    pub fn new(
        runners: ErasedRunnerRegistry,
        artifact_stager: Arc<dyn RuntimeArtifactStager>,
    ) -> Self {
        Self {
            runners,
            artifact_stager,
        }
    }

    /// Prepares sealed genesis launch authority for a certified run.
    pub fn prepare_run_launch(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
    ) -> Result<PreparedRunLaunch> {
        CommitPlanner::prepare_run_launch(
            &self.runners,
            runtime_spec,
            run_id,
            evidence,
            expected_next_seq,
        )
    }

    /// Appends the prepared typed genesis commit after staging middleware-owned artifacts.
    pub async fn start_run<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        Ok(store.append_prepared_typed_commit(launch.commit)?)
    }

    /// Appends the prepared typed genesis commit through an async typed store.
    pub async fn start_run_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(launch.commit)
            .await
            .map_err(async_store_error)
    }

    /// Runs one deterministic runnable node, if any.
    pub async fn drive_once<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let view = RuntimeRunView::from_store(runtime_spec, run_id, store)?;
        match scheduler_decision(runtime_spec, run_id, &view)? {
            SchedulerDecision::Run(runnable) => {
                self.run_node_attempt(store, runtime_spec, run_id, &view, runnable)
                    .await?;
                Ok(SchedulerStatus::Advanced)
            }
            SchedulerDecision::Blocked => Ok(SchedulerStatus::Blocked),
            SchedulerDecision::Completed => Ok(SchedulerStatus::PublicOutputProjected),
        }
    }

    /// Runs deterministic runnable nodes until no node is runnable or public output is projected.
    pub async fn drive_until_blocked<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        loop {
            match self.drive_once(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    /// Runs one deterministic runnable node against an async durable typed store, if any.
    pub async fn drive_once_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let view = RuntimeRunView::from_stream(runtime_spec, run_id, &stream)?;
        match scheduler_decision(runtime_spec, run_id, &view)? {
            SchedulerDecision::Run(runnable) => {
                self.run_node_attempt_async(store, runtime_spec, run_id, &view, runnable)
                    .await?;
                Ok(SchedulerStatus::Advanced)
            }
            SchedulerDecision::Blocked => Ok(SchedulerStatus::Blocked),
            SchedulerDecision::Completed => Ok(SchedulerStatus::PublicOutputProjected),
        }
    }

    /// Runs deterministic runnable nodes against an async durable typed store until blocked.
    pub async fn drive_until_blocked_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        loop {
            match self.drive_once_async(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    async fn run_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
            )
        ) {
            self.run_started_framework_node_attempt(
                store,
                FrameworkNodeAttemptInput {
                    runtime_spec,
                    run_id,
                    view,
                    runnable,
                    descriptor,
                    output_cell,
                    binding,
                },
            )
            .await?;
            return Ok(());
        }
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew { attempt_no } => {
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                prepare_runner_invocation(RunnerInvocationInput {
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    attempt_id: &attempt_id,
                    attempt_no,
                    view,
                })?;
                let start_commit = CommitPlanner::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                store.append_prepared_typed_commit(start_commit)?;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_stream = store.load_run_stream(run_id);
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view: &latest_view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
            runtime_spec,
            run_id,
            node,
            attempt_id: &attempt_id,
            caps: invocation.caps(),
            recorded_facts: invocation.recorded_facts(),
            view: &latest_view,
            output,
        })?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store.append_prepared_typed_commit(terminal_output.commit)?;
        Ok(())
    }

    async fn run_started_framework_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        input: FrameworkNodeAttemptInput<'_>,
    ) -> Result<()> {
        let FrameworkNodeAttemptInput {
            runtime_spec,
            run_id,
            view,
            runnable,
            descriptor,
            output_cell,
            binding,
        } = input;
        let node = runnable.node;
        let AttemptPlan::StartNew { attempt_no } = runnable.attempt else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "framework lifecycle node {} attempt was split across commits",
                node.node_id
            )));
        };
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = CommitPlanner::prepare_started_runner_output(
            RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view,
                output,
            },
            attempt_no,
        )?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store.append_prepared_typed_commit(terminal_output.commit)?;
        Ok(())
    }

    async fn run_node_attempt_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
            )
        ) {
            self.run_started_framework_node_attempt_async(
                store,
                FrameworkNodeAttemptInput {
                    runtime_spec,
                    run_id,
                    view,
                    runnable,
                    descriptor,
                    output_cell,
                    binding,
                },
            )
            .await?;
            return Ok(());
        }
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew { attempt_no } => {
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                prepare_runner_invocation(RunnerInvocationInput {
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    attempt_id: &attempt_id,
                    attempt_no,
                    view,
                })?;
                let start_commit = CommitPlanner::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                store
                    .append_prepared_typed_commit(start_commit)
                    .await
                    .map_err(async_store_error)?;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view: &latest_view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
            runtime_spec,
            run_id,
            node,
            attempt_id: &attempt_id,
            caps: invocation.caps(),
            recorded_facts: invocation.recorded_facts(),
            view: &latest_view,
            output,
        })?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
            .map_err(async_store_error)?;
        Ok(())
    }

    async fn run_started_framework_node_attempt_async<
        S: store::AsyncTypedRunEventStore + ?Sized,
    >(
        &self,
        store: &S,
        input: FrameworkNodeAttemptInput<'_>,
    ) -> Result<()> {
        let FrameworkNodeAttemptInput {
            runtime_spec,
            run_id,
            view,
            runnable,
            descriptor,
            output_cell,
            binding,
        } = input;
        let node = runnable.node;
        let AttemptPlan::StartNew { attempt_no } = runnable.attempt else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "framework lifecycle node {} attempt was split across commits",
                node.node_id
            )));
        };
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = CommitPlanner::prepare_started_runner_output(
            RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view,
                output,
            },
            attempt_no,
        )?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
            .map_err(async_store_error)?;
        Ok(())
    }

    async fn stage_prepared_artifacts(&self, artifacts: &[PreparedStagedArtifact]) -> Result<()> {
        for artifact in artifacts {
            self.artifact_stager
                .stage_verified_artifact(artifact.bytes.clone(), artifact.evidence.clone())
                .await?;
        }
        Ok(())
    }
}

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

fn config_artifact_reference_payloads(
    spec_hash: &SpecHash,
    artifacts: &[store::ArtifactEvidenceRef],
) -> Result<Vec<events::KernelEventPayload>> {
    artifacts
        .iter()
        .map(|artifact| {
            let schema_id = artifact.schema_id.clone().ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "config artifact {} is missing schema id",
                    artifact.artifact_id
                ))
            })?;
            Ok(events::KernelEventPayload::ArtifactReferenced(
                events::ArtifactReferenced {
                    spec_hash: spec_hash.clone(),
                    node_id: None,
                    attempt_id: None,
                    artifact_ref: events::ArtifactEvidenceRef {
                        artifact_id: artifact.artifact_id.clone(),
                        role: artifact.artifact_role,
                        schema_id,
                        semantic_type_id: artifact.semantic_type_id.clone(),
                        content_digest: artifact.digest.clone(),
                        byte_len: artifact.byte_len,
                        media_type: artifact.media_type.clone(),
                    },
                },
            ))
        })
        .collect()
}

fn retention_ref_for_artifact(artifact: &store::ArtifactEvidenceRef) -> events::RetentionRef {
    events::RetentionRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        content_digest: artifact.digest.clone(),
    }
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

fn config_ref_key(config_ref: &spec::ConfigRef) -> String {
    format!("{}:{}", config_ref.schema_id, config_ref.digest)
}

#[cfg(test)]
mod tests;
