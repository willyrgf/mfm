use std::collections::{BTreeMap, BTreeSet};

use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, AttemptId, CellId, ContentDigest, NodeId, RunId, SchemaId, SemanticTypeId, SpecHash,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::{
    artifact_role_name, staged_artifact_binding_kind, staged_artifact_binding_role,
    verify_artifact_bytes, StagedArtifact, StagedArtifactBindingKind, StagedRetentionRefs,
    StagedSideEffectArtifactPhase,
};
use crate::framework::{
    bootstrap_run_receipt_artifact, build_retention_manifest_artifact_with_producer,
    certified_bootstrap_run_node, framework_run_completed_payload, projected_retention_manifest,
    retention_reason_str, run_completion_evidence, GenesisContext, RetentionManifestArtifact,
};
use crate::history::{
    event_artifact_ref_from_store, payload_spec_hash, store_seed_artifact,
    validate_certificate_artifact, validate_config_artifacts, validate_seed_cells,
    validate_spec_artifact, RuntimeRunView,
};
use crate::runners::{ErasedRunnerOutput, ErasedRunnerRegistry, RunnerEventPayload};
use crate::side_effects::{
    side_effect_artifact_binding, side_effect_projection_for_attempt,
    validate_runner_side_effect_payload, validate_side_effect_resume_output,
    validate_side_effect_terminal_evidence,
};
use crate::{
    attempt_id, config_artifact_reference_payloads, content_digest_json, require_adapter,
    require_attempt, require_capability, retention_ref_for_artifact, validate_public_output,
    validate_public_output_render_node, CertifiedRuntimeCapabilities, CertifiedRuntimeSpec,
    RecordedFacts, Result, RuntimeError,
};

/// Launch evidence needed to prepare a typed run genesis commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchEvidence {
    /// Staged certified spec bytes and the evidence to admit with `RunStarted`.
    pub spec_artifact: RunLaunchArtifact,
    /// Staged certified spec certificate bytes and the evidence to admit with `RunStarted`.
    pub certificate_artifact: RunLaunchArtifact,
    /// Staged config artifacts for every certified config reference.
    pub config_artifacts: Vec<RunLaunchArtifact>,
    /// Framework build/version identity.
    pub framework_version: events::FrameworkVersion,
    /// Source revision identity.
    pub source_revision: events::SourceRevision,
    /// Adapter executable identities bound to the run.
    pub adapter_executables: Vec<events::ExecutableIdentity>,
    /// Seed cells materialized at run start.
    pub seed_cells: Vec<RunLaunchSeedCell>,
}

/// Staged launch artifact bytes plus typed evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchArtifact {
    /// Artifact bytes to stage through runtime middleware before the genesis commit.
    pub bytes: Vec<u8>,
    /// Typed artifact evidence to admit atomically with the genesis commit.
    pub evidence: store::ArtifactEvidenceRef,
}

/// Staged launch seed bytes plus the seed cell authority bound into `RunStarted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchSeedCell {
    /// Seed artifact bytes to stage through runtime middleware before the genesis commit.
    pub bytes: Vec<u8>,
    /// Seed cell reference to persist in `RunStarted`.
    pub cell: events::SeedCellRef,
}

/// Prepared genesis launch authority accepted by runtime-owned start middleware.
pub struct PreparedRunLaunch {
    pub(crate) commit: store::PreparedTypedCommit,
    pub(crate) artifacts_to_stage: Vec<PreparedStagedArtifact>,
}

pub(crate) struct RunnerOutputCommitInput<'a> {
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    pub(crate) run_id: &'a RunId,
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) caps: &'a CertifiedRuntimeCapabilities,
    pub(crate) recorded_facts: &'a RecordedFacts,
    pub(crate) view: &'a RuntimeRunView,
    pub(crate) output: ErasedRunnerOutput,
}

pub(crate) struct CompleteRunCommitValidation<'a> {
    pub(crate) run_id: &'a RunId,
    pub(crate) spec_hash: &'a SpecHash,
    pub(crate) completion: &'a events::PublicOutputCompletionEvidence,
    pub(crate) completion_node_id: &'a NodeId,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) receipt_cell_id: &'a CellId,
    pub(crate) receipt_artifact_id: &'a ArtifactId,
    pub(crate) receipt_digest: &'a ContentDigest,
    pub(crate) expected_receipt_ref: &'a events::ArtifactEvidenceRef,
}

pub(crate) struct PreparedRunnerOutput {
    pub(crate) commit: store::PreparedTypedCommit,
    pub(crate) artifacts_to_stage: Vec<PreparedStagedArtifact>,
}

pub(crate) struct PreparedStagedArtifact {
    pub(crate) bytes: Vec<u8>,
    pub(crate) evidence: store::ArtifactEvidenceRef,
}

pub(crate) struct CommitPlanner;

impl CommitPlanner {
    pub(crate) fn prepare_run_launch(
        runners: &ErasedRunnerRegistry,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
    ) -> Result<PreparedRunLaunch> {
        let spec_input = evidence.spec_artifact;
        verify_artifact_bytes(&spec_input.bytes, &spec_input.evidence)?;
        let spec_artifact = validate_spec_artifact(runtime_spec, spec_input.evidence.clone())?;
        let certificate_input = evidence.certificate_artifact;
        verify_artifact_bytes(&certificate_input.bytes, &certificate_input.evidence)?;
        let certificate_artifact =
            validate_certificate_artifact(runtime_spec, certificate_input.evidence.clone())?;
        let config_inputs = evidence.config_artifacts;
        let config_artifacts = validate_config_artifacts(
            runtime_spec,
            config_inputs
                .iter()
                .map(|artifact| {
                    verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
                    Ok(artifact.evidence.clone())
                })
                .collect::<Result<Vec<_>>>()?,
        )?;
        let mut config_staged_artifacts = launch_artifacts_by_id(config_inputs, "config")?;
        let config_reference_payloads =
            config_artifact_reference_payloads(runtime_spec.spec_hash(), &config_artifacts)?;
        let seed_inputs = evidence.seed_cells;
        let seed_cell_refs = seed_inputs
            .iter()
            .map(|seed| seed.cell.clone())
            .collect::<Vec<_>>();
        let seed_cells = validate_seed_cells(runtime_spec, &seed_cell_refs)?;
        let seed_staged_artifacts =
            validate_launch_seed_artifacts(seed_inputs, seed_cells.values())?;
        let runner_executables = runners.executables_for_spec(runtime_spec)?;
        let bootstrap_node = certified_bootstrap_run_node(runtime_spec)?;
        let bootstrap_output_cell =
            runtime_spec
                .cell(&bootstrap_node.output_cell)
                .ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "bootstrap lifecycle node {} output cell {} is missing",
                        bootstrap_node.node_id, bootstrap_node.output_cell
                    ))
                })?;
        let bootstrap_attempt_id = attempt_id(
            &run_id,
            runtime_spec.spec_hash(),
            &bootstrap_node.node_id,
            1,
        )?;
        let mut required_artifacts =
            Vec::with_capacity(2 + config_artifacts.len() + seed_cells.len());
        required_artifacts.push(spec_artifact.clone());
        required_artifacts.push(certificate_artifact.clone());
        required_artifacts.extend(config_artifacts.iter().cloned());
        required_artifacts.extend(seed_cells.values().map(store_seed_artifact));
        let run_started = events::RunStarted {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            spec_artifact_id: spec_artifact.artifact_id.clone(),
            certificate_artifact_id: certificate_artifact.artifact_id.clone(),
            certificate_artifact_digest: certificate_artifact.digest.clone(),
            certificate_media_type: certificate_artifact.media_type.clone(),
            spec_media_type: runtime_spec.spec().media_type.clone(),
            spec_version: runtime_spec.spec().spec_version.clone(),
            lowering_version: runtime_spec.spec().lowering_version.clone(),
            public_output_schema_id: runtime_spec.spec().public_outputs.public_schema_id.clone(),
            descriptor_identities: runtime_spec.spec().descriptor_identities.clone(),
            runner_executables,
            adapter_executables: evidence.adapter_executables,
            canonicalizer_identity: runtime_spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
                .clone(),
            framework_version: evidence.framework_version,
            source_revision: evidence.source_revision,
            seed_cells: seed_cell_refs,
        };
        let genesis = GenesisContext {
            runtime_spec,
            run_id: &run_id,
            node: bootstrap_node,
            output_cell: bootstrap_output_cell,
            attempt_id: &bootstrap_attempt_id,
            run_started: &run_started,
            config_artifacts: &config_artifacts,
        };
        let (bootstrap_receipt_bytes, bootstrap_receipt_artifact) =
            bootstrap_run_receipt_artifact(&genesis)?;
        let mut run_started_retention_refs = required_artifacts
            .iter()
            .map(retention_ref_for_artifact)
            .collect::<Vec<_>>();
        run_started_retention_refs.push(retention_ref_for_artifact(&bootstrap_receipt_artifact));
        required_artifacts.push(bootstrap_receipt_artifact.clone());
        let admitted_artifacts = required_artifacts.clone();
        let start_payload = events::KernelEventPayload::RunStarted(run_started);
        let bootstrap_start =
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: bootstrap_node.node_id.clone(),
                attempt_id: bootstrap_attempt_id.clone(),
                attempt_no: 1,
                state_kind: bootstrap_node.state_kind.clone(),
                state_version: bootstrap_node.state_version.clone(),
            });
        let bootstrap_cell = events::KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: runtime_spec.spec_hash().clone(),
            node_id: bootstrap_node.node_id.clone(),
            cell_id: bootstrap_node.output_cell.clone(),
            scope_id: bootstrap_output_cell.scope_id.clone(),
            attempt_id: bootstrap_attempt_id.clone(),
            semantic_type_id: bootstrap_output_cell.semantic_type_id.clone(),
            schema_id: bootstrap_output_cell.schema_id.clone(),
            value_lineage: bootstrap_output_cell.value_lineage.clone(),
            artifact_id: bootstrap_receipt_artifact.artifact_id.clone(),
            content_digest: bootstrap_receipt_artifact.digest.clone(),
            producer_state_kind: Some(bootstrap_node.state_kind.clone()),
            producer_state_version: Some(bootstrap_node.state_version.clone()),
        });
        let bootstrap_completed =
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: bootstrap_node.node_id.clone(),
                attempt_id: bootstrap_attempt_id.clone(),
                output_cell_id: bootstrap_node.output_cell.clone(),
            });
        let bootstrap_ref =
            events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: Some(bootstrap_node.node_id.clone()),
                attempt_id: Some(bootstrap_attempt_id.clone()),
                artifact_ref: event_artifact_ref_from_store(&bootstrap_receipt_artifact)?,
            });
        let retention_payload =
            events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                refs: run_started_retention_refs,
                reason: events::RetentionReason::RunStarted,
            });
        let mut payloads = Vec::with_capacity(6 + config_reference_payloads.len());
        payloads.push(start_payload);
        payloads.extend(config_reference_payloads);
        payloads.push(bootstrap_start);
        payloads.push(bootstrap_cell);
        payloads.push(bootstrap_completed);
        payloads.push(bootstrap_ref);
        payloads.push(retention_payload);
        let request = store::TypedCommitRequest {
            run_id,
            expected_next_seq,
            commit_key: store::CommitKey::new(format!(
                "run-start:{}",
                runtime_spec.spec_hash().as_str()
            ))?,
            payloads,
            required_artifacts,
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::Absent,
                ..store::CommitPreconditions::default()
            },
        };
        let commit = store::PreparedTypedCommit::new(request, admitted_artifacts)?;
        let mut artifacts_to_stage =
            Vec::with_capacity(3 + config_artifacts.len() + seed_staged_artifacts.len());
        artifacts_to_stage.push(PreparedStagedArtifact {
            bytes: spec_input.bytes,
            evidence: spec_artifact,
        });
        artifacts_to_stage.push(PreparedStagedArtifact {
            bytes: certificate_input.bytes,
            evidence: certificate_artifact,
        });
        for artifact in &config_artifacts {
            let staged = config_staged_artifacts
                .remove(&artifact.artifact_id)
                .ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "missing staged config artifact bytes for {}",
                        artifact.artifact_id
                    ))
                })?;
            artifacts_to_stage.push(PreparedStagedArtifact {
                bytes: staged.bytes,
                evidence: artifact.clone(),
            });
        }
        artifacts_to_stage.extend(seed_staged_artifacts);
        artifacts_to_stage.push(PreparedStagedArtifact {
            bytes: bootstrap_receipt_bytes.to_vec(),
            evidence: bootstrap_receipt_artifact,
        });
        Ok(PreparedRunLaunch {
            commit,
            artifacts_to_stage,
        })
    }

    pub(crate) fn prepare_attempt_start(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        attempt_no: u32,
        view: &RuntimeRunView,
    ) -> Result<store::PreparedTypedCommit> {
        let start_payload =
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            });
        let mut preconditions = store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        };
        preconditions
            .required_cell_states
            .extend(node_cell_preconditions(runtime_spec, node)?);
        let request = store::TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: view.next_seq,
            commit_key: store::CommitKey::new(format!(
                "attempt-start:{}:{}",
                node.node_id, attempt_id
            ))?,
            payloads: vec![start_payload],
            required_artifacts: Vec::new(),
            preconditions,
        };
        store::PreparedTypedCommit::new(request, Vec::new()).map_err(RuntimeError::from)
    }

    pub(crate) fn prepare_runner_output(
        input: RunnerOutputCommitInput<'_>,
    ) -> Result<PreparedRunnerOutput> {
        Self::prepare_runner_output_with_start(input, None)
    }

    pub(crate) fn prepare_started_runner_output(
        input: RunnerOutputCommitInput<'_>,
        attempt_no: u32,
    ) -> Result<PreparedRunnerOutput> {
        Self::prepare_runner_output_with_start(input, Some(attempt_no))
    }

    fn prepare_runner_output_with_start(
        input: RunnerOutputCommitInput<'_>,
        started_in_same_commit: Option<u32>,
    ) -> Result<PreparedRunnerOutput> {
        let ErasedRunnerOutput {
            staged_artifacts,
            staged_retention_refs,
            payloads: runner_payloads,
        } = input.output;
        let runner_payloads = runner_payloads_with_derived_lifecycle(
            input.runtime_spec,
            input.node,
            input.attempt_id,
            runner_payloads,
        )?;
        validate_runner_output(
            input.runtime_spec,
            input.node,
            input.attempt_id,
            input.caps,
            input.recorded_facts,
            &input.view.projections,
            &runner_payloads,
        )?;
        if matches!(
            &input.node.framework,
            Some(
                spec::FrameworkNodeSpec::CompleteRun(_)
                    | spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
            )
        ) && !staged_retention_refs.is_empty()
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "terminal framework node {} cannot stage retention refs",
                input.node.node_id
            )));
        }
        let staged_artifacts = validate_staged_artifacts(
            input.run_id,
            input.node,
            input.attempt_id,
            &staged_artifacts,
        )?;
        let retention_manifest = framework_retention_manifest_artifact(
            input.runtime_spec,
            input.run_id,
            input.node,
            &input.view.stream,
            &staged_artifacts,
        )?;
        let payload_bound_artifacts = staged_artifacts
            .iter()
            .filter(|artifact| artifact.binding != StagedArtifactBindingKind::RetentionManifest)
            .cloned()
            .collect::<Vec<_>>();
        validate_staged_artifact_payload_bindings(
            input.node,
            input.attempt_id,
            &runner_payloads,
            &payload_bound_artifacts,
        )?;
        let mut payloads = if let Some(attempt_no) = started_in_same_commit {
            vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: input.runtime_spec.spec_hash().clone(),
                    node_id: input.node.node_id.clone(),
                    attempt_id: input.attempt_id.clone(),
                    attempt_no,
                    state_kind: input.node.state_kind.clone(),
                    state_version: input.node.state_version.clone(),
                },
            )]
        } else {
            Vec::new()
        };
        payloads.extend(runner_payloads);
        if let Some(manifest) = retention_manifest {
            payloads.extend(retention_manifest_payloads(
                input.runtime_spec,
                input.run_id,
                manifest,
            ));
        }
        payloads.extend(staged_artifact_reference_payloads(
            input.runtime_spec.spec_hash(),
            input.node,
            input.attempt_id,
            &payloads,
            &payload_bound_artifacts,
        ));
        let artifacts_to_stage = staged_artifacts
            .iter()
            .filter_map(|artifact| {
                artifact.bytes.as_ref().map(|bytes| PreparedStagedArtifact {
                    bytes: bytes.clone(),
                    evidence: artifact.evidence.clone(),
                })
            })
            .collect::<Vec<_>>();
        let required_artifacts = staged_artifacts
            .iter()
            .map(|artifact| artifact.evidence.clone())
            .collect::<Vec<_>>();
        payloads.extend(bind_staged_retention_refs(
            input.runtime_spec,
            input.run_id,
            input.node,
            &required_artifacts,
            staged_retention_refs,
        )?);
        if let Some(run_completed) = framework_run_completed_payload(
            input.runtime_spec,
            input.run_id,
            input.node,
            &input.view.projections,
        )? {
            payloads.push(run_completed);
        }
        let admitted_artifacts = required_artifacts.clone();
        let preconditions = runner_output_preconditions(
            input.runtime_spec,
            input.run_id,
            input.node,
            input.attempt_id,
            &input.view.projections,
            &payloads,
            started_in_same_commit.is_none(),
        )?;
        let request = store::TypedCommitRequest {
            run_id: input.run_id.clone(),
            expected_next_seq: input.view.next_seq,
            commit_key: runner_output_commit_key(input.node, input.attempt_id, &payloads)?,
            payloads,
            required_artifacts,
            preconditions,
        };
        let commit = store::PreparedTypedCommit::new(request, admitted_artifacts)?;
        Ok(PreparedRunnerOutput {
            commit,
            artifacts_to_stage,
        })
    }
}

fn launch_artifacts_by_id(
    artifacts: Vec<RunLaunchArtifact>,
    kind: &'static str,
) -> Result<BTreeMap<ArtifactId, RunLaunchArtifact>> {
    let mut by_artifact = BTreeMap::new();
    for artifact in artifacts {
        verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
        if by_artifact
            .insert(artifact.evidence.artifact_id.clone(), artifact)
            .is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate staged {kind} launch artifact"
            )));
        }
    }
    Ok(by_artifact)
}

fn validate_launch_seed_artifacts<'a>(
    seeds: Vec<RunLaunchSeedCell>,
    validated_cells: impl IntoIterator<Item = &'a events::SeedCellRef>,
) -> Result<Vec<PreparedStagedArtifact>> {
    let mut by_cell = BTreeMap::new();
    for seed in seeds {
        if by_cell.insert(seed.cell.cell_id.clone(), seed).is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "duplicate staged seed launch artifact".to_owned(),
            ));
        }
    }
    let mut staged = Vec::with_capacity(by_cell.len());
    for cell in validated_cells {
        let seed = by_cell.remove(&cell.cell_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing staged seed bytes for cell {}",
                cell.cell_id
            ))
        })?;
        let evidence = store_seed_artifact(cell);
        verify_artifact_bytes(&seed.bytes, &evidence)?;
        staged.push(PreparedStagedArtifact {
            bytes: seed.bytes,
            evidence,
        });
    }
    if !by_cell.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "staged seed launch artifacts contain entries not certified by the spec".to_owned(),
        ));
    }
    Ok(staged)
}

fn runner_output_commit_key(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<store::CommitKey> {
    let mut fragments = BTreeSet::new();
    for payload in payloads {
        fragments.insert(runner_output_commit_fragment(payload));
    }
    if fragments.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let suffix = content_digest_json(serde_json::json!({
        "fragments": fragments.into_iter().collect::<Vec<_>>(),
    }))?;
    Ok(store::CommitKey::new(format!(
        "attempt-output:{}:{}:{}",
        node.node_id, attempt_id, suffix
    ))?)
}

fn runner_output_commit_fragment(payload: &events::KernelEventPayload) -> String {
    match payload {
        events::KernelEventPayload::StateAttemptCompleted(payload) => {
            format!("completed:{}", payload.output_cell_id)
        }
        events::KernelEventPayload::StateAttemptFailed(_) => "failed".to_owned(),
        events::KernelEventPayload::CellProduced(payload) => {
            format!("cell-produced:{}", payload.cell_id)
        }
        events::KernelEventPayload::CellSkipped(payload) => {
            format!("cell-skipped:{}", payload.cell_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => format!("fact:{}", payload.fact_key),
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            format!("artifact:{}", payload.artifact_ref.artifact_id)
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            format!("public-output:{}", payload.public_schema_id)
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            format!("public-output-failed:{}", payload.public_schema_id)
        }
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            format!("sidefx-intent:{}", payload.ledger_key)
        }
        events::KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx-claim:{}:{}:{}",
            payload.ledger_key, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx-claim-takeover:{}:{}:{}",
            payload.ledger_key, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx-prepared:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx-started:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx-not-submitted:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx-submission:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx-submission-unknown:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx-receipt:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx-confirmation:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!("sidefx-ambiguous:{}", payload.ledger_key)
        }
        events::KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx-failed:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::RetentionManifestProjected(payload) => {
            format!(
                "retention-manifest:{}:{}",
                payload.manifest_seq, payload.manifest_digest
            )
        }
        events::KernelEventPayload::RetentionRefsAppended(payload) => format!(
            "retention-refs:{}:{}",
            retention_reason_str(payload.reason),
            payload.refs.len()
        ),
        events::KernelEventPayload::RunStarted(_)
        | events::KernelEventPayload::ManualResolutionRecorded(_)
        | events::KernelEventPayload::RunCompleted(_)
        | events::KernelEventPayload::StateAttemptStarted(_) => "scheduler-owned".to_owned(),
    }
}

fn validate_staged_artifacts(
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    staged_artifacts: &[StagedArtifact],
) -> Result<Vec<ValidatedStagedArtifact>> {
    let mut by_artifact = BTreeMap::<ArtifactId, ValidatedStagedArtifact>::new();
    for staged in staged_artifacts {
        let handle = staged.handle();
        if handle.run_id() != run_id
            || handle.node_id() != &node.node_id
            || handle.attempt_id() != attempt_id
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} outside its sealed attempt",
                node.node_id,
                handle.evidence().artifact_id
            )));
        }
        if let Some(bytes) = staged.bytes() {
            verify_artifact_bytes(bytes, handle.evidence())?;
        }
        if let Some(existing) = by_artifact.get(&handle.evidence().artifact_id) {
            if existing.evidence != *handle.evidence() || existing.binding != *handle.binding() {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged conflicting evidence for artifact {}",
                    node.node_id,
                    handle.evidence().artifact_id
                )));
            }
            continue;
        }
        by_artifact.insert(
            handle.evidence().artifact_id.clone(),
            ValidatedStagedArtifact {
                evidence: handle.evidence().clone(),
                binding: handle.binding().clone(),
                bytes: staged.bytes().map(ToOwned::to_owned),
            },
        );
    }
    Ok(by_artifact.into_values().collect())
}

fn framework_retention_manifest_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    pre_projection_stream: &[store::KernelEventEnvelope],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Result<Option<RetentionManifestArtifact>> {
    let manifests = staged_artifacts
        .iter()
        .filter(|artifact| artifact.binding == StagedArtifactBindingKind::RetentionManifest)
        .collect::<Vec<_>>();
    if manifests.is_empty() {
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        ) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "retention framework node {} did not stage a retention manifest",
                node.node_id
            )));
        }
        return Ok(None);
    }
    if !matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    ) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} staged retention manifest outside framework retention authority",
            node.node_id
        )));
    }
    if manifests.len() != 1 {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged multiple retention manifests",
            node.node_id
        )));
    }
    let staged = manifests[0];
    let Some(bytes) = staged.bytes.as_deref() else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged manifest without bytes",
            node.node_id
        )));
    };
    let expected = build_retention_manifest_artifact_with_producer(
        runtime_spec,
        run_id,
        pre_projection_stream,
        Some(node.node_id.clone()),
    )?;
    if staged.evidence != expected.evidence || bytes != expected.bytes.as_bytes() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged manifest outside authoritative stream",
            node.node_id
        )));
    }
    Ok(Some(expected))
}

pub(crate) fn retention_manifest_payloads(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    manifest: RetentionManifestArtifact,
) -> Vec<events::KernelEventPayload> {
    let manifest_ref = retention_ref_for_artifact(&manifest.evidence);
    vec![
        events::KernelEventPayload::RetentionManifestProjected(
            events::RetentionManifestProjected {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                manifest_seq: manifest.manifest_seq,
                manifest_digest: manifest.evidence.digest.clone(),
                previous_manifest_digest: manifest.previous_manifest_digest,
                manifest_artifact_id: manifest.evidence.artifact_id.clone(),
            },
        ),
        events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            refs: vec![manifest_ref],
            reason: events::RetentionReason::ManifestProjection,
        }),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedStagedArtifact {
    evidence: store::ArtifactEvidenceRef,
    binding: StagedArtifactBindingKind,
    bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StagedArtifactRequirement {
    artifact_id: ArtifactId,
    digest: ContentDigest,
    byte_len: Option<u64>,
    media_type: Option<spec::MediaType>,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<SemanticTypeId>,
    role: events::ArtifactRole,
    binding: StagedArtifactBindingKind,
}

fn validate_staged_artifact_payload_bindings(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Result<()> {
    let requirements = staged_payload_artifact_requirements(node, attempt_id, payloads)?;
    let mut requirements_by_artifact: BTreeMap<ArtifactId, &StagedArtifactRequirement> =
        BTreeMap::new();
    for requirement in &requirements {
        if let Some(existing) = requirements_by_artifact.get(&requirement.artifact_id) {
            if !staged_artifact_requirements_are_compatible(existing, requirement) {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} returned conflicting typed payload requirements for artifact {}",
                    node.node_id, requirement.artifact_id
                )));
            }
        }
        requirements_by_artifact.insert(requirement.artifact_id.clone(), requirement);
    }
    let staged_by_artifact = staged_artifacts
        .iter()
        .map(|artifact| (artifact.evidence.artifact_id.clone(), artifact))
        .collect::<BTreeMap<_, _>>();

    for staged in staged_artifacts {
        let Some(requirement) = requirements_by_artifact.get(&staged.evidence.artifact_id) else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} without typed payload reference",
                node.node_id, staged.evidence.artifact_id
            )));
        };
        validate_staged_artifact_requirement(node, &staged.evidence, &staged.binding, requirement)?;
    }

    for requirement in requirements {
        let Some(staged) = staged_by_artifact.get(&requirement.artifact_id) else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} referenced artifact {} without staged artifact",
                node.node_id, requirement.artifact_id
            )));
        };
        validate_staged_artifact_requirement(
            node,
            &staged.evidence,
            &staged.binding,
            &requirement,
        )?;
    }
    Ok(())
}

fn validate_staged_artifact_requirement(
    node: &spec::NodeSpec,
    evidence: &store::ArtifactEvidenceRef,
    binding: &StagedArtifactBindingKind,
    requirement: &StagedArtifactRequirement,
) -> Result<()> {
    if evidence.artifact_id != requirement.artifact_id
        || evidence.digest != requirement.digest
        || requirement
            .byte_len
            .is_some_and(|byte_len| evidence.byte_len != byte_len)
        || requirement
            .media_type
            .as_ref()
            .is_some_and(|media_type| &evidence.media_type != media_type)
        || requirement
            .schema_id
            .as_ref()
            .is_some_and(|schema_id| evidence.schema_id.as_ref() != Some(schema_id))
        || requirement
            .semantic_type_id
            .as_ref()
            .is_some_and(|semantic_type_id| {
                evidence.semantic_type_id.as_ref() != Some(semantic_type_id)
            })
        || evidence.producer_node_id.as_ref() != Some(&node.node_id)
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != requirement.role
        || binding != &requirement.binding
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} staged artifact {} does not match typed payload binding",
            node.node_id, evidence.artifact_id
        )));
    }
    Ok(())
}

fn staged_artifact_requirements_are_compatible(
    left: &StagedArtifactRequirement,
    right: &StagedArtifactRequirement,
) -> bool {
    left.artifact_id == right.artifact_id
        && left.digest == right.digest
        && optional_requirements_are_compatible(left.byte_len.as_ref(), right.byte_len.as_ref())
        && optional_requirements_are_compatible(left.media_type.as_ref(), right.media_type.as_ref())
        && optional_requirements_are_compatible(left.schema_id.as_ref(), right.schema_id.as_ref())
        && optional_requirements_are_compatible(
            left.semantic_type_id.as_ref(),
            right.semantic_type_id.as_ref(),
        )
        && left.role == right.role
        && left.binding == right.binding
}

fn optional_requirements_are_compatible<T: Eq>(left: Option<&T>, right: Option<&T>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn staged_payload_artifact_requirements(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<Vec<StagedArtifactRequirement>> {
    let mut requirements = Vec::new();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::FactRecorded(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(StagedArtifactRequirement {
                    artifact_id: payload.artifact_id.clone(),
                    digest: payload.response_hash.clone(),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.response_schema_id.clone()),
                    semantic_type_id: None,
                    role: events::ArtifactRole::FactResponse,
                    binding: StagedArtifactBindingKind::FactResponse,
                });
            }
            events::KernelEventPayload::CellProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(StagedArtifactRequirement {
                    artifact_id: payload.artifact_id.clone(),
                    digest: payload.content_digest.clone(),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.schema_id.clone()),
                    semantic_type_id: Some(payload.semantic_type_id.clone()),
                    role: events::ArtifactRole::StateOutput,
                    binding: StagedArtifactBindingKind::StateOutput,
                });
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(artifact_id) = &payload.rendered_artifact_id {
                    requirements.push(StagedArtifactRequirement {
                        artifact_id: artifact_id.clone(),
                        digest: payload.rendered_digest.clone(),
                        byte_len: None,
                        media_type: None,
                        schema_id: Some(payload.public_schema_id.clone()),
                        semantic_type_id: None,
                        role: events::ArtifactRole::PublicOutput,
                        binding: StagedArtifactBindingKind::PublicOutput,
                    });
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(ref diagnostic) = payload.error.diagnostic_ref {
                    push_staged_event_artifact_requirement(
                        node,
                        diagnostic,
                        StagedArtifactBindingKind::RedactedDiagnostic,
                        &mut requirements,
                    )?;
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(ref diagnostic) = payload.error.diagnostic_ref {
                    push_staged_event_artifact_requirement(
                        node,
                        diagnostic,
                        StagedArtifactBindingKind::RedactedDiagnostic,
                        &mut requirements,
                    )?;
                }
            }
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.intent_artifact_id.clone(),
                    payload.intent_hash.clone(),
                    Some(payload.intent_schema_id.clone()),
                    events::ArtifactRole::SideEffectIntent,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Intent,
                ));
            }
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let (Some(artifact_id), Some(hash)) =
                    (&payload.prepared_artifact_id, &payload.prepared_hash)
                {
                    requirements.push(side_effect_artifact_requirement(
                        artifact_id.clone(),
                        hash.clone(),
                        None,
                        events::ArtifactRole::PreparedInvocation,
                        payload.ledger_key.clone(),
                        payload.invocation_epoch,
                        StagedSideEffectArtifactPhase::PreparedInvocation,
                    ));
                }
            }
            events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.proof_artifact_id.clone(),
                    payload.proof_hash.clone(),
                    Some(payload.proof_schema_id.clone()),
                    events::ArtifactRole::NotSubmittedProof,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::NotSubmittedProof,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.submission_artifact_id.clone(),
                    payload.submission_hash.clone(),
                    Some(payload.submission_schema_id.clone()),
                    events::ArtifactRole::Submission,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Submission,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    Some(payload.evidence_schema_id.clone()),
                    events::ArtifactRole::SubmissionUnknownEvidence,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::SubmissionUnknownEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.receipt_artifact_id.clone(),
                    payload.receipt_hash.clone(),
                    Some(payload.receipt_schema_id.clone()),
                    events::ArtifactRole::Receipt,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Receipt,
                ));
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.confirmation_artifact_id.clone(),
                    payload.confirmation_hash.clone(),
                    Some(payload.confirmation_schema_id.clone()),
                    events::ArtifactRole::Confirmation,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Confirmation,
                ));
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    Some(payload.evidence_schema_id.clone()),
                    events::ArtifactRole::AmbiguityEvidence,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::AmbiguityEvidence,
                ));
            }
            events::KernelEventPayload::RunStarted(_)
            | events::KernelEventPayload::ManualResolutionRecorded(_)
            | events::KernelEventPayload::ArtifactReferenced(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_)
            | events::KernelEventPayload::StateAttemptCompleted(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {}
        }
    }
    Ok(requirements)
}

fn staged_artifact_reference_payloads(
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Vec<events::KernelEventPayload> {
    let existing_refs = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(&node.node_id)
                    && payload.attempt_id.as_ref() == Some(attempt_id) =>
            {
                Some(payload.artifact_ref.artifact_id.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut refs = Vec::new();
    for artifact in staged_artifacts {
        if staged_artifact_binding_kind(artifact.evidence.artifact_role).is_none() {
            continue;
        }
        if existing_refs.contains(&artifact.evidence.artifact_id) {
            continue;
        }
        let Some(schema_id) = artifact.evidence.schema_id.clone() else {
            continue;
        };
        refs.push(events::KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: spec_hash.clone(),
                node_id: Some(node.node_id.clone()),
                attempt_id: Some(attempt_id.clone()),
                artifact_ref: events::ArtifactEvidenceRef {
                    artifact_id: artifact.evidence.artifact_id.clone(),
                    role: artifact.evidence.artifact_role,
                    schema_id,
                    semantic_type_id: artifact.evidence.semantic_type_id.clone(),
                    content_digest: artifact.evidence.digest.clone(),
                    byte_len: artifact.evidence.byte_len,
                    media_type: artifact.evidence.media_type.clone(),
                },
            },
        ));
    }
    refs
}

fn push_staged_event_artifact_requirement(
    node: &spec::NodeSpec,
    artifact: &events::ArtifactEvidenceRef,
    binding: StagedArtifactBindingKind,
    requirements: &mut Vec<StagedArtifactRequirement>,
) -> Result<()> {
    let role = staged_artifact_binding_role(&binding);
    if artifact.role != role {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} diagnostic artifact role {} does not match staged binding",
            node.node_id,
            artifact_role_name(artifact.role)
        )));
    }
    requirements.push(StagedArtifactRequirement {
        artifact_id: artifact.artifact_id.clone(),
        digest: artifact.content_digest.clone(),
        byte_len: Some(artifact.byte_len),
        media_type: Some(artifact.media_type.clone()),
        schema_id: Some(artifact.schema_id.clone()),
        semantic_type_id: artifact.semantic_type_id.clone(),
        role: artifact.role,
        binding,
    });
    Ok(())
}

fn side_effect_artifact_requirement(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: Option<SchemaId>,
    role: events::ArtifactRole,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    phase: StagedSideEffectArtifactPhase,
) -> StagedArtifactRequirement {
    StagedArtifactRequirement {
        artifact_id,
        digest,
        byte_len: None,
        media_type: None,
        schema_id,
        semantic_type_id: None,
        role,
        binding: side_effect_artifact_binding(ledger_key, invocation_epoch, phase),
    }
}

fn bind_staged_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    required_artifacts: &[store::ArtifactEvidenceRef],
    staged: Vec<StagedRetentionRefs>,
) -> Result<Vec<events::KernelEventPayload>> {
    let artifact_evidence = required_artifacts
        .iter()
        .map(|artifact| (artifact.artifact_id.clone(), artifact))
        .collect::<BTreeMap<_, _>>();
    let mut payloads = Vec::with_capacity(staged.len());
    for staged_refs in staged {
        let reason = staged_refs.reason;
        validate_staged_retention_reason(runtime_spec, node, &artifact_evidence, &staged_refs)?;
        if staged_refs.refs.is_empty() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged empty retention refs",
                node.node_id
            )));
        }
        for retention_ref in &staged_refs.refs {
            let Some(artifact) = artifact_evidence.get(&retention_ref.artifact_id) else {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged retention for artifact {} without staged artifact evidence",
                    node.node_id, retention_ref.artifact_id
                )));
            };
            if artifact.digest != retention_ref.content_digest
                || artifact.artifact_role != retention_ref.role
            {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged retention evidence for artifact {} does not match artifact evidence",
                    node.node_id, retention_ref.artifact_id
                )));
            }
        }
        payloads.push(events::KernelEventPayload::RetentionRefsAppended(
            events::RetentionRefsAppended {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                refs: staged_refs.refs,
                reason,
            },
        ));
    }
    Ok(payloads)
}

fn validate_staged_retention_reason(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    artifact_evidence: &BTreeMap<ArtifactId, &store::ArtifactEvidenceRef>,
    staged_refs: &StagedRetentionRefs,
) -> Result<()> {
    match staged_refs.reason {
        events::RetentionReason::RunStarted | events::RetentionReason::ManifestProjection => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged middleware-owned retention reason {}",
                node.node_id,
                retention_reason_str(staged_refs.reason)
            )));
        }
        events::RetentionReason::PublicOutput => {
            if !matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            ) {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged public-output retention outside sealed framework renderer",
                    node.node_id
                )));
            }
            let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "public-output framework node {} references missing output cell {}",
                    node.node_id, node.output_cell
                ))
            })?;
            for retention_ref in &staged_refs.refs {
                let artifact = artifact_evidence
                    .get(&retention_ref.artifact_id)
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunnerOutput(format!(
                            "node {} staged public-output retention for artifact {} without staged artifact evidence",
                            node.node_id, retention_ref.artifact_id
                        ))
                    })?;
                let framework_artifact = artifact.producer_node_id.as_ref() == Some(&node.node_id)
                    && matches!(
                        artifact.artifact_role,
                        events::ArtifactRole::StateOutput | events::ArtifactRole::PublicOutput
                    )
                    && (artifact.artifact_role != events::ArtifactRole::StateOutput
                        || artifact.schema_id.as_ref() == Some(&output_cell.schema_id));
                if !framework_artifact {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} staged public-output retention for non-framework artifact {}",
                        node.node_id, retention_ref.artifact_id
                    )));
                }
            }
        }
        events::RetentionReason::RuntimeEvidence => {}
    }
    Ok(())
}

fn runner_output_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    projections: &store::ProjectionSnapshot,
    payloads: &[events::KernelEventPayload],
    require_existing_attempt: bool,
) -> Result<store::CommitPreconditions> {
    let mut preconditions = store::CommitPreconditions {
        required_run_state: store::RequiredRunState::NotCompleted,
        required_cell_states: vec![store::CellStatePrecondition {
            cell_id: node.output_cell.clone(),
            required: store::RequiredCellState::Absent,
        }],
        required_public_output_absent: matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        ),
        ..store::CommitPreconditions::default()
    };
    if require_existing_attempt {
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))?);
    } else {
        preconditions
            .required_cell_states
            .extend(node_cell_preconditions(runtime_spec, node)?);
    }
    if matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    ) {
        let completion = run_completion_evidence(runtime_spec, projections)?;
        let retention_manifest = projected_retention_manifest(run_id, projections)?;
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "public_output:{}",
                completion.public_output_schema_id
            ))?);
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "retention:{}:manifest:{}",
                run_id, retention_manifest.manifest_seq
            ))?);
    }

    if node.side_effect.is_none() {
        return Ok(preconditions);
    }

    let mut requires_terminal_confirmation = false;
    let prepared_in_batch = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                Some(payload.ledger_key.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::Absent,
                    },
                );
            }
            events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
                if !prepared_in_batch.contains(&payload.ledger_key) {
                    preconditions.required_side_effect_states.push(
                        store::SideEffectStatePrecondition {
                            ledger_key: payload.ledger_key.clone(),
                            required: store::RequiredSideEffectState::InvocationPrepared,
                        },
                    );
                }
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::SubmissionResult,
                    },
                );
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::ReceiptObserved,
                    },
                );
            }
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::StateAttemptCompleted(_) => {
                requires_terminal_confirmation = true;
            }
            _ => {}
        }
    }

    if requires_terminal_confirmation {
        let projection = side_effect_projection_for_attempt(projections, node, attempt_id)?
            .ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} attempted output without ledger evidence",
                    node.node_id
                ))
            })?;
        preconditions
            .required_side_effect_states
            .push(store::SideEffectStatePrecondition {
                ledger_key: projection.ledger_key.clone(),
                required: store::RequiredSideEffectState::ConfirmationObserved,
            });
    }

    Ok(preconditions)
}

fn node_cell_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
) -> Result<Vec<store::CellStatePrecondition>> {
    let mut preconditions = Vec::new();
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        let cell = runtime_spec
            .cell(&cell_id)
            .expect("validated input binding cell exists");
        if matches!(cell.producer, spec::CellProducer::Node(_)) {
            preconditions.push(store::CellStatePrecondition {
                cell_id,
                required: store::RequiredCellState::Terminal,
            });
        }
    }
    Ok(preconditions)
}

pub(crate) fn runner_payloads_with_derived_lifecycle(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    runner_payloads: Vec<RunnerEventPayload>,
) -> Result<Vec<events::KernelEventPayload>> {
    let mut payloads = runner_payloads
        .into_iter()
        .map(events::KernelEventPayload::from)
        .collect::<Vec<_>>();
    let mut terminal_cell = false;
    let mut failure: Option<(bool, events::MfmErrorInfo)> = None;
    for payload in &payloads {
        match payload {
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::CellSkipped(_) => {
                terminal_cell = true;
            }
            events::KernelEventPayload::SideEffectFailed(payload) => {
                if failure
                    .replace((payload.retryable, payload.error.clone()))
                    .is_some()
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                if failure
                    .replace((false, side_effect_ambiguity_error(payload)?))
                    .is_some()
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload)
                if failure
                    .replace((payload.error.retryable, payload.error.clone()))
                    .is_some() =>
            {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned multiple failure payloads",
                    node.node_id
                )));
            }
            _ => {}
        }
    }

    if let Some((retryable, error)) = failure {
        payloads.push(events::KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                retryable,
                error,
            },
        ));
    } else if terminal_cell {
        payloads.push(events::KernelEventPayload::StateAttemptCompleted(
            events::StateAttemptCompleted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: node.output_cell.clone(),
            },
        ));
    }

    Ok(payloads)
}

fn side_effect_ambiguity_error(
    payload: &events::side_effect::Ambiguous,
) -> Result<events::MfmErrorInfo> {
    Ok(events::MfmErrorInfo {
        code: events::ErrorCode::new("side_effect_ambiguous")?,
        category: events::ErrorCategory::SideEffect,
        retryable: false,
        safe_message: format!(
            "side-effect outcome is ambiguous: {}",
            payload.ambiguity_code
        ),
        public_details: None,
        diagnostic_ref: None,
    })
}

fn validate_runner_output(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    recorded_facts: &RecordedFacts,
    projections: &store::ProjectionSnapshot,
    payloads: &[events::KernelEventPayload],
) -> Result<()> {
    if payloads.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let mut completed = false;
    let mut failed = false;
    let mut terminal_cell = false;
    let mut public_output_produced = false;
    let mut public_output_failed = false;
    let mut side_effect_payload = false;
    let mut side_effect_terminal_failure = false;
    let mut attempt_failure_retryable = None;
    let mut side_effect_terminal_failure_retryable = None;
    for payload in payloads {
        if payload_spec_hash(payload) != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "runner for node {} returned payload with mismatched spec hash",
                node.node_id
            )));
        }
        match payload {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} completed output cell {} instead of certified {}",
                        node.node_id, payload.output_cell_id, node.output_cell
                    )));
                }
                completed = true;
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if failed {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
                attempt_failure_retryable = Some(payload.retryable);
                failed = true;
            }
            events::KernelEventPayload::CellProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced cell metadata outside certified spec",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                    || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped a cell outside certified skip policy",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if !recorded_facts.is_empty() {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} attempted to record fact {} after committed facts existed for the same attempt",
                        node.node_id, payload.fact_key
                    )));
                }
                require_capability(
                    caps,
                    &payload.capability_kind,
                    &payload.capability_version,
                    &node.node_id,
                )?;
                require_adapter(node, &payload.adapter_kind, &payload.adapter_version)?;
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned artifact reference payload for {}",
                    node.node_id, payload.artifact_ref.artifact_id
                )));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output(runtime_spec, node, payload)?;
                public_output_produced = true;
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )?;
                public_output_failed = true;
            }
            events::KernelEventPayload::RunStarted(_)
            | events::KernelEventPayload::ManualResolutionRecorded(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned scheduler-owned payload",
                    node.node_id
                )));
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {
                validate_runner_side_effect_payload(
                    runtime_spec,
                    node,
                    attempt_id,
                    caps,
                    projections,
                    payload,
                )?;
                side_effect_payload = true;
                match payload {
                    events::KernelEventPayload::SideEffectFailed(payload) => {
                        side_effect_terminal_failure = true;
                        side_effect_terminal_failure_retryable = Some(payload.retryable);
                    }
                    events::KernelEventPayload::SideEffectAmbiguous(_) => {
                        side_effect_terminal_failure = true;
                        side_effect_terminal_failure_retryable = Some(false);
                    }
                    _ => {}
                }
            }
        }
    }
    if node.side_effect.is_some() {
        validate_side_effect_resume_output(projections, node, attempt_id, payloads)?;
        if failed {
            if !side_effect_terminal_failure {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned StateAttemptFailed without terminal side-effect evidence",
                    node.node_id
                )));
            }
            if side_effect_terminal_failure_retryable != attempt_failure_retryable {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned inconsistent failure retryability",
                    node.node_id
                )));
            }
            if completed || terminal_cell || public_output_produced {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} mixed side-effect failure with successful terminal evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if side_effect_terminal_failure {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} returned terminal side-effect evidence without StateAttemptFailed",
                node.node_id
            )));
        }
        if side_effect_payload {
            if completed || terminal_cell || public_output_produced || public_output_failed {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} mixed ledger phase events with terminal output evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if !completed || !terminal_cell {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} output commit must pair StateAttemptCompleted with terminal cell evidence",
                node.node_id
            )));
        }
        validate_side_effect_terminal_evidence(projections, node, attempt_id)
            .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        if public_output_produced && !completed {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} projected public output without completing its certified output cell",
                node.node_id
            )));
        }
        return Ok(());
    }
    if failed && (completed || terminal_cell || public_output_produced) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} mixed failure with successful terminal evidence",
            node.node_id
        )));
    }
    if public_output_failed && !failed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned public-output failure without StateAttemptFailed",
            node.node_id
        )));
    }
    if !failed && (!completed || !terminal_cell) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} successful terminal commit must pair StateAttemptCompleted with terminal cell evidence",
            node.node_id
        )));
    }
    if public_output_produced && !completed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} projected public output without completing its certified output cell",
            node.node_id
        )));
    }
    Ok(())
}
