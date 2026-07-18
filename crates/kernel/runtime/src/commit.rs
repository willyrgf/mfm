use std::collections::{BTreeMap, BTreeSet};

use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, AttemptId, CellId, ContentDigest, NodeId, RunId, SchemaId, SemanticTypeId, SpecHash,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::{
    artifact_role_name, fact_query_returned_ref_retention_refs, staged_artifact_binding_kind,
    staged_artifact_binding_role, staged_side_effect_artifact_phase, verify_artifact_bytes,
    StagedArtifact, StagedArtifactBindingKind, StagedRetentionRefAuthority, StagedRetentionRefs,
    StagedSideEffectArtifactPhase,
};
use crate::binding::BoundRuntimeContext;
use crate::framework::{
    build_retention_manifest_artifact, framework_run_completed_payload,
    projected_retention_manifest, retention_reason_str, run_completion_evidence,
    RetentionManifestArtifact,
};
use crate::history::{
    event_artifact_ref_from_store, payload_spec_hash, run_artifact_ref_from_store,
    store_seed_artifact, validate_certificate_artifact, validate_config_artifacts,
    validate_seed_cells, validate_spec_artifact, RuntimeRunView,
};
use crate::runners::{ContextOutputExtractor, ErasedRunnerOutput, RunnerEventPayload};
use crate::side_effect_lifecycle::{
    side_effect_projection_for_attempt, standalone_interruption_allowed, validate_resume_output,
    validate_terminal_batch_evidence,
};
use crate::side_effects::validate_runner_side_effect_payload;
use crate::{
    content_digest_json, require_adapter, require_attempt, require_capability,
    validate_public_output, validate_public_output_render_node, CertifiedRuntimeCapabilities,
    CertifiedRuntimeSpec, RecordedFacts, Result, RuntimeError,
};

#[path = "commit_validation.rs"]
mod commit_validation;

pub(crate) use self::commit_validation::runner_payloads_with_derived_lifecycle;
use self::commit_validation::{runtime_fact_error, validate_runner_output, RunnerOutputValidation};

#[path = "commit_artifacts.rs"]
mod commit_artifacts;

pub(crate) use self::commit_artifacts::retention_manifest_payloads;
use self::commit_artifacts::{
    attempt_failure_commit_fragment, bind_staged_retention_refs,
    framework_retention_manifest_artifact, launch_artifacts_by_evidence,
    required_artifacts_for_payloads, runner_output_commit_key, staged_artifact_reference_payloads,
    validate_attempt_failure_diagnostic_artifact, validate_context_bound_output_artifacts,
    validate_fact_descriptor_launch_artifacts, validate_launch_seed_artifacts,
    validate_staged_artifact_payload_bindings, validate_staged_artifacts,
};

#[path = "commit_preconditions.rs"]
mod commit_preconditions;

use self::commit_preconditions::{
    attempt_commit_preconditions, certified_run_authority, runner_output_preconditions,
    side_effect_verify_spec, side_effect_verify_terminal_required_state,
};

/// Launch evidence needed to prepare a typed run admission commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchEvidence {
    /// Public entry-point operation evidence selected by app assembly.
    pub entry_point: events::EntryPointLaunchEvidence,
    /// Staged certified spec bytes and the evidence to admit with `RunAdmitted`.
    pub spec_artifact: RunLaunchArtifact,
    /// Staged certified spec certificate bytes and the evidence to admit with `RunAdmitted`.
    pub certificate_artifact: RunLaunchArtifact,
    /// Staged config artifacts for every certified config reference.
    pub config_artifacts: Vec<RunLaunchArtifact>,
    /// Staged fact descriptor artifacts for every certified fact descriptor reference.
    pub fact_descriptor_artifacts: Vec<RunLaunchArtifact>,
    /// Seed cells materialized at run start.
    pub seed_cells: Vec<RunLaunchSeedCell>,
}

/// Staged launch artifact bytes plus typed evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchArtifact {
    /// Artifact bytes to stage through runtime middleware before the admission commit.
    pub bytes: Vec<u8>,
    /// Typed artifact evidence to admit atomically with the admission commit.
    pub evidence: store::ArtifactEvidenceRef,
}

/// Staged launch seed bytes plus the seed cell authority bound into `RunAdmitted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLaunchSeedCell {
    /// Seed artifact bytes to stage through runtime middleware before the admission commit.
    pub bytes: Vec<u8>,
    /// Seed cell reference to persist in `RunAdmitted`.
    pub cell: events::SeedCellRef,
}

/// Prepared run admission authority accepted by runtime-owned start middleware.
pub struct PreparedRunLaunch {
    commit: store::PreparedCommit<store::RunAdmission>,
    artifacts_to_stage: Vec<PreparedStagedArtifact>,
}

impl PreparedRunLaunch {
    pub(crate) fn run_id(&self) -> &RunId {
        self.commit.request().run_id()
    }

    pub(crate) fn into_prepared_commit_bundle(self) -> Result<store::PreparedCommitBundle> {
        prepared_commit_bundle(self.commit.into(), self.artifacts_to_stage)
    }
}

pub(crate) struct RunnerOutputCommitInput<'a> {
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    pub(crate) run_id: &'a RunId,
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) caps: &'a CertifiedRuntimeCapabilities,
    pub(crate) recorded_facts: &'a RecordedFacts,
    pub(crate) view: &'a RuntimeRunView,
    pub(crate) context_output_extractor: Option<&'a dyn ContextOutputExtractor>,
    pub(crate) saga_terminal_proof: Option<store::SagaTerminalProof>,
    pub(crate) output: ErasedRunnerOutput,
}

pub(crate) struct AttemptFailureCommitInput<'a> {
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    pub(crate) run_id: &'a RunId,
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) view: &'a RuntimeRunView,
    pub(crate) error: events::MfmErrorInfo,
    pub(crate) diagnostic_artifact: Option<PreparedStagedArtifact>,
}

pub(crate) struct AttemptInterruptionCommitInput<'a> {
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    pub(crate) run_id: &'a RunId,
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) view: &'a RuntimeRunView,
}

/// Validation input for a sealed terminal lifecycle commit batch.
///
/// `CompleteRun` and `ResolveSagaTerminal` both append `StateAttemptStarted` before running and
/// then emit the same four-event terminal batch (`CellProduced`, `StateAttemptCompleted`,
/// `ArtifactReferenced`, `RunCompleted`); they differ only in the committed completion outcome and
/// the diagnostic label. A single validator over this input keeps the two terminal paths from
/// drifting.
pub(crate) struct SealedTerminalCommitValidation<'a> {
    pub(crate) label: &'static str,
    pub(crate) run_id: &'a RunId,
    pub(crate) spec_hash: &'a SpecHash,
    pub(crate) outcome: &'a events::RunCompletionOutcome,
    pub(crate) node_id: &'a NodeId,
    pub(crate) attempt_id: &'a AttemptId,
    pub(crate) receipt_cell_id: &'a CellId,
    pub(crate) receipt_artifact_id: &'a ArtifactId,
    pub(crate) receipt_digest: &'a ContentDigest,
    pub(crate) expected_receipt_ref: &'a events::ArtifactEvidenceRef,
}

pub(crate) struct PreparedRunnerOutput {
    commit: store::PreparedCommitPlan,
    artifact_admissions: Vec<PreparedArtifactAdmission>,
}

impl PreparedRunnerOutput {
    pub(crate) fn request(&self) -> &store::CommitRequest {
        self.commit.request()
    }

    pub(crate) fn into_prepared_commit_bundle(self) -> Result<store::PreparedCommitBundle> {
        prepared_commit_bundle_with_admissions(self.commit, self.artifact_admissions)
    }
}

pub(crate) struct PreparedStagedArtifact {
    pub(crate) bytes: Vec<u8>,
    pub(crate) evidence: store::ArtifactEvidenceRef,
}

enum PreparedArtifactAdmission {
    Bytes(Box<PreparedStagedArtifact>),
    Existing(store::ExistingArtifactAdmission),
}

impl From<PreparedStagedArtifact> for PreparedArtifactAdmission {
    fn from(artifact: PreparedStagedArtifact) -> Self {
        Self::Bytes(Box::new(artifact))
    }
}

pub(crate) fn prepared_commit_bundle(
    commit: store::PreparedCommitPlan,
    artifacts: Vec<PreparedStagedArtifact>,
) -> Result<store::PreparedCommitBundle> {
    prepared_commit_bundle_with_admissions(
        commit,
        artifacts
            .into_iter()
            .map(PreparedArtifactAdmission::from)
            .collect(),
    )
}

fn prepared_commit_bundle_with_admissions(
    commit: store::PreparedCommitPlan,
    artifacts: Vec<PreparedArtifactAdmission>,
) -> Result<store::PreparedCommitBundle> {
    let mut artifact_bytes = Vec::new();
    let mut existing_artifacts = Vec::new();
    for artifact in artifacts {
        match artifact {
            PreparedArtifactAdmission::Bytes(artifact) => {
                let artifact = *artifact;
                artifact_bytes.push(
                    store::PreparedArtifactBytes::new(artifact.bytes, artifact.evidence)
                        .map_err(RuntimeError::from)?,
                );
            }
            PreparedArtifactAdmission::Existing(existing) => {
                existing_artifacts.push(existing);
            }
        }
    }
    store::PreparedCommitBundle::new(commit, artifact_bytes, existing_artifacts)
        .map_err(RuntimeError::from)
}

pub(crate) struct CommitPlanner;

impl CommitPlanner {
    pub(crate) fn prepare_run_launch(
        runtime_spec: &CertifiedRuntimeSpec,
        identity_material: events::RunIdentityMaterialV1,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
        bound_context: &BoundRuntimeContext,
    ) -> Result<PreparedRunLaunch> {
        if identity_material.certified_spec_hash != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunnerOutput(
                "run identity material spec hash does not match certified spec".to_owned(),
            ));
        }
        let run_id = identity_material.derive_run_id()?;
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
        // Content-addressed configs can share bytes/artifact_id while differing by schema_id
        // (for example two empty `{}` configs). Stage by exact evidence identity.
        let mut config_staged_artifacts = launch_artifacts_by_evidence(config_inputs, "config")?;
        let fact_descriptor_artifacts = validate_fact_descriptor_launch_artifacts(
            runtime_spec,
            evidence.fact_descriptor_artifacts,
        )?;
        let fact_descriptor_evidence = fact_descriptor_artifacts
            .iter()
            .map(|artifact| artifact.evidence.clone())
            .collect::<Vec<_>>();
        let seed_inputs = evidence.seed_cells;
        let seed_cell_refs = seed_inputs
            .iter()
            .map(|seed| seed.cell.clone())
            .collect::<Vec<_>>();
        let seed_cells = validate_seed_cells(runtime_spec, &seed_cell_refs)?;
        let seed_staged_artifacts =
            validate_launch_seed_artifacts(seed_inputs, seed_cells.values())?;
        let mut required_artifacts = Vec::with_capacity(
            2 + config_artifacts.len() + fact_descriptor_evidence.len() + seed_cells.len(),
        );
        required_artifacts.push(spec_artifact.clone());
        required_artifacts.push(certificate_artifact.clone());
        required_artifacts.extend(config_artifacts.iter().cloned());
        required_artifacts.extend(fact_descriptor_evidence.iter().cloned());
        required_artifacts.extend(seed_cells.values().map(store_seed_artifact));
        let adapter_executables = bound_context.adapter_executables().to_vec();
        let admitted_binding_digest = bound_context.admitted_binding_digest()?;
        let run_admitted = events::RunAdmitted {
            run_id: run_id.clone(),
            identity_material,
            entry_point: evidence.entry_point,
            spec_hash: runtime_spec.spec_hash().clone(),
            spec_artifact: run_artifact_ref_from_store(&spec_artifact)?,
            certificate_artifact: run_artifact_ref_from_store(&certificate_artifact)?,
            config_artifacts: config_artifacts
                .iter()
                .map(run_artifact_ref_from_store)
                .collect::<Result<_>>()?,
            fact_descriptor_artifacts: fact_descriptor_evidence
                .iter()
                .map(run_artifact_ref_from_store)
                .collect::<Result<_>>()?,
            spec_version: runtime_spec.spec().spec_version.clone(),
            lowering_version: runtime_spec.spec().lowering_version.clone(),
            public_output_schema_id: runtime_spec.spec().public_outputs.public_schema_id.clone(),
            saga_policy_digest: runtime_spec.spec().saga.saga_policy_digest()?,
            descriptor_identities: runtime_spec.spec().descriptor_identities.clone(),
            runner_executables: bound_context.runner_executables().to_vec(),
            adapter_executables,
            capability_implementations: bound_context.capability_implementations().to_vec(),
            admitted_binding_digest,
            canonicalizer_identity: runtime_spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
                .clone(),
            seed_cells: seed_cell_refs,
        };
        let admitted_artifacts = required_artifacts.clone();
        let start_payload = events::KernelEventPayload::RunAdmitted(Box::new(run_admitted));
        let request = store::CommitRequest::from_payloads(
            run_id.clone(),
            expected_next_seq,
            store::CommitKey::new(format!(
                "run-admission:{}",
                runtime_spec.spec_hash().as_str()
            ))?,
            vec![start_payload],
            required_artifacts.clone(),
            store::CommitPreconditions {
                required_run_state: store::RequiredRunState::Absent,
                certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                    run_id.clone(),
                    runtime_spec.spec(),
                )?),
                ..store::CommitPreconditions::default()
            },
        )?;
        let commit = store::PreparedCommit::<store::RunAdmission>::new(
            request,
            store::CommitArtifactEvidenceSet::new(required_artifacts, admitted_artifacts)?,
        )?;
        let mut artifacts_to_stage = Vec::with_capacity(
            3 + config_artifacts.len()
                + fact_descriptor_artifacts.len()
                + seed_staged_artifacts.len(),
        );
        artifacts_to_stage.push(PreparedStagedArtifact {
            bytes: spec_input.bytes,
            evidence: spec_artifact,
        });
        artifacts_to_stage.push(PreparedStagedArtifact {
            bytes: certificate_input.bytes,
            evidence: certificate_artifact,
        });
        for artifact in &config_artifacts {
            let evidence_hash = artifact.evidence_hash()?;
            let staged = config_staged_artifacts
                .remove(&(artifact.artifact_id.clone(), evidence_hash))
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
        for artifact in fact_descriptor_artifacts {
            artifacts_to_stage.push(PreparedStagedArtifact {
                bytes: artifact.bytes,
                evidence: artifact.evidence,
            });
        }
        artifacts_to_stage.extend(seed_staged_artifacts);
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
    ) -> Result<store::PreparedCommit<store::StateAttemptStarted>> {
        let start_payload =
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            });
        let preconditions = attempt_commit_preconditions(runtime_spec, node, None)?;
        let request = store::CommitRequest::from_payloads(
            run_id.clone(),
            view.next_seq,
            store::CommitKey::new(format!("attempt-start:{}:{}", node.node_id, attempt_id))?,
            vec![start_payload],
            Vec::new(),
            preconditions,
        )?;
        Ok(store::PreparedCommit::<store::StateAttemptStarted>::new(
            request,
            store::CommitArtifactEvidenceSet::empty(),
        )?)
    }

    pub(crate) fn prepare_runner_output(
        input: RunnerOutputCommitInput<'_>,
    ) -> Result<PreparedRunnerOutput> {
        let (staged_artifacts, staged_retention_refs, runner_payloads) = input.output.into_parts();
        let runner_payloads = runner_payloads_with_derived_lifecycle(
            input.runtime_spec,
            input.node,
            input.attempt_id,
            runner_payloads,
        )?;
        validate_runner_output(RunnerOutputValidation {
            runtime_spec: input.runtime_spec,
            run_id: input.run_id,
            node: input.node,
            attempt_id: input.attempt_id,
            caps: input.caps,
            recorded_facts: input.recorded_facts,
            projections: &input.view.projections,
            payloads: &runner_payloads,
        })?;
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
            &input.view.artifact_byte_authority,
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
        validate_context_bound_output_artifacts(
            input.node,
            &runner_payloads,
            &payload_bound_artifacts,
            input.context_output_extractor,
        )?;
        let mut payloads = Vec::new();
        payloads.extend(runner_payloads);
        if let Some(manifest) = retention_manifest {
            payloads.extend(retention_manifest_payloads(
                input.runtime_spec,
                input.run_id,
                manifest,
            )?);
        }
        payloads.extend(staged_artifact_reference_payloads(
            input.runtime_spec.spec_hash(),
            input.node,
            input.attempt_id,
            &payloads,
            &payload_bound_artifacts,
        )?);
        let artifact_admissions = staged_artifacts
            .iter()
            .map(|artifact| {
                Ok(match artifact.bytes.as_ref() {
                    Some(bytes) => {
                        PreparedArtifactAdmission::Bytes(Box::new(PreparedStagedArtifact {
                            bytes: bytes.clone(),
                            evidence: artifact.evidence.clone(),
                        }))
                    }
                    None => {
                        PreparedArtifactAdmission::Existing(store::ExistingArtifactAdmission::new(
                            artifact.evidence.artifact_id.clone(),
                            artifact.evidence.evidence_hash()?,
                        ))
                    }
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let admitted_artifacts = staged_artifacts
            .iter()
            .map(|artifact| artifact.evidence.clone())
            .collect::<Vec<_>>();
        let required_artifacts = admitted_artifacts.clone();
        payloads.extend(bind_staged_retention_refs(
            input.runtime_spec,
            input.run_id,
            input.node,
            &input.view.projections,
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
        let required_artifacts =
            required_artifacts_for_payloads(input.view, required_artifacts, &payloads)?;
        let preconditions = runner_output_preconditions(
            input.runtime_spec,
            input.run_id,
            input.node,
            input.attempt_id,
            &input.view.projections,
            &payloads,
            true,
        )?;
        let request = store::CommitRequest::from_payloads(
            input.run_id.clone(),
            input.view.next_seq,
            runner_output_commit_key(input.node, input.attempt_id, &payloads)?,
            payloads,
            required_artifacts.clone(),
            preconditions,
        )?;
        let commit = prepare_runner_output_commit_plan(
            request,
            store::CommitArtifactEvidenceSet::new(required_artifacts, admitted_artifacts)?,
            input.saga_terminal_proof,
        )?;
        Ok(PreparedRunnerOutput {
            commit,
            artifact_admissions,
        })
    }

    pub(crate) fn prepare_attempt_failure(
        input: AttemptFailureCommitInput<'_>,
    ) -> Result<PreparedRunnerOutput> {
        let diagnostic_artifact = input
            .diagnostic_artifact
            .map(|artifact| validate_attempt_failure_diagnostic_artifact(input.node, artifact))
            .transpose()?;
        let mut error = input.error.clone();
        match &diagnostic_artifact {
            Some(artifact) => {
                error.diagnostic_ref = Some(event_artifact_ref_from_store(&artifact.evidence)?);
            }
            None if error.diagnostic_ref.is_some() => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} failure diagnostic ref lacks staged diagnostic artifact",
                    input.node.node_id
                )));
            }
            None => {}
        }
        let terminal_side_effect_payloads = if input.node.side_effect.is_some() {
            side_effect_projection_for_attempt(
                input.runtime_spec,
                input.run_id,
                &input.view.projections,
                input.node,
                input.attempt_id,
            )?
            .map(|projection| match &projection.phase {
                store::SideEffectPhase::Claimed {
                    invocation_epoch, ..
                } => {
                    let mut payloads = Vec::new();
                    if let Some(release) = resource_lane_release_intent_for_failure(
                        input.runtime_spec,
                        input.run_id,
                        input.node,
                        input.attempt_id,
                        &input.view.projections,
                        projection,
                        *invocation_epoch,
                    )? {
                        payloads.push(release);
                    }
                    payloads.push(events::KernelEventPayload::SideEffectFailed(
                        events::side_effect::Failed {
                            spec_hash: input.runtime_spec.spec_hash().clone(),
                            node_id: input.node.node_id.clone(),
                            attempt_id: input.attempt_id.clone(),
                            ledger_key: projection.ledger_key.clone(),
                            ledger_purpose: projection.ledger_purpose.clone(),
                            pair_id: projection.pair_id.clone(),
                            pair_role: events::SideEffectPairRole::Submit,
                            invocation_epoch: *invocation_epoch,
                            failure_phase:
                                events::side_effect::FailurePhase::BeforeInvocationStarted,
                            retryable: error.retryable,
                            error: error.clone(),
                        },
                    ));
                    Ok(payloads)
                }
                _ => Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} attempt {} has acquired side-effect authority for ledger {}",
                    input.node.node_id, input.attempt_id, projection.ledger_key
                ))),
            })
            .transpose()?
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        let failure = events::StateAttemptFailed {
            spec_hash: input.runtime_spec.spec_hash().clone(),
            node_id: input.node.node_id.clone(),
            attempt_id: input.attempt_id.clone(),
            retryable: error.retryable,
            error,
        };
        let commit_fragment = attempt_failure_commit_fragment(&failure)?;
        let mut payloads = Vec::new();
        payloads.extend(terminal_side_effect_payloads);
        payloads.push(events::KernelEventPayload::StateAttemptFailed(failure));
        let (required_artifacts, admitted_artifacts, artifacts_to_stage) =
            if let Some(artifact) = diagnostic_artifact {
                let event_ref = event_artifact_ref_from_store(&artifact.evidence)?;
                payloads.push(events::KernelEventPayload::ArtifactReferenced(
                    events::ArtifactReferenced {
                        spec_hash: input.runtime_spec.spec_hash().clone(),
                        node_id: Some(input.node.node_id.clone()),
                        attempt_id: Some(input.attempt_id.clone()),
                        artifact_ref: event_ref,
                    },
                ));
                payloads.push(events::KernelEventPayload::RetentionRefsAppended(
                    events::RetentionRefsAppended {
                        run_id: input.run_id.clone(),
                        spec_hash: input.runtime_spec.spec_hash().clone(),
                        refs: vec![artifact.evidence.retention_ref()?],
                        reason: events::RetentionReason::RuntimeEvidence,
                    },
                ));
                let evidence = artifact.evidence.clone();
                (
                    vec![evidence.clone()],
                    vec![evidence],
                    vec![PreparedArtifactAdmission::Bytes(Box::new(artifact))],
                )
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
        let mut preconditions =
            attempt_commit_preconditions(input.runtime_spec, input.node, Some(input.attempt_id))?;
        if payloads
            .iter()
            .any(events::KernelEventPayload::is_side_effect_terminal)
        {
            preconditions.certified_run_authority =
                Some(certified_run_authority(input.runtime_spec, input.run_id)?);
        }
        let required_artifacts =
            required_artifacts_for_payloads(input.view, required_artifacts, &payloads)?;
        let request = store::CommitRequest::from_payloads(
            input.run_id.clone(),
            input.view.next_seq,
            store::CommitKey::new(format!(
                "attempt-failure:{}:{}:{}",
                input.node.node_id, input.attempt_id, commit_fragment
            ))?,
            payloads,
            required_artifacts.clone(),
            preconditions,
        )?;
        let commit = prepare_runner_output_commit_plan(
            request,
            store::CommitArtifactEvidenceSet::new(required_artifacts, admitted_artifacts)?,
            None,
        )?;
        Ok(PreparedRunnerOutput {
            commit,
            artifact_admissions: artifacts_to_stage,
        })
    }

    pub(crate) fn prepare_attempt_interruption(
        input: AttemptInterruptionCommitInput<'_>,
    ) -> Result<store::PreparedCommitPlan> {
        let Some(attempt) = input
            .view
            .projections
            .attempt(&input.node.node_id, input.attempt_id)
        else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "cannot interrupt missing attempt {} for node {}",
                input.attempt_id, input.node.node_id
            )));
        };
        if !matches!(attempt.status, store::AttemptStatus::Started { .. }) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "cannot interrupt terminal attempt {} for node {}",
                input.attempt_id, input.node.node_id
            )));
        }
        if input.node.side_effect.is_some()
            && !standalone_interruption_allowed(
                input.runtime_spec,
                input.run_id,
                &input.view.projections,
                input.node,
                input.attempt_id,
            )?
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has prepared side-effect invocation authority and cannot be interrupted generically",
                input.node.node_id, input.attempt_id
            )));
        }

        let payload =
            events::KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
                spec_hash: input.runtime_spec.spec_hash().clone(),
                node_id: input.node.node_id.clone(),
                attempt_id: input.attempt_id.clone(),
            });
        let preconditions =
            attempt_commit_preconditions(input.runtime_spec, input.node, Some(input.attempt_id))?;
        let request = store::CommitRequest::from_payloads(
            input.run_id.clone(),
            input.view.next_seq,
            store::CommitKey::new(format!(
                "attempt-interruption:{}:{}",
                input.node.node_id, input.attempt_id
            ))?,
            vec![payload],
            Vec::new(),
            preconditions,
        )?;
        prepare_runner_output_commit_plan(request, store::CommitArtifactEvidenceSet::empty(), None)
    }
}

fn resource_lane_release_intent_for_failure(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    projections: &store::ProjectionSnapshot,
    projection: &store::SideEffectProjection,
    invocation_epoch: u32,
) -> Result<Option<events::KernelEventPayload>> {
    let holder = store::SideEffectPairLedgerRef::new(run_id.clone(), projection.pair_id.clone());
    let Some((_, lane)) = projections
        .resource_lanes()
        .find(|(_, lane)| lane.holder == holder)
    else {
        return Ok(None);
    };
    if lane.node_id != node.node_id
        || lane.attempt_id != *attempt_id
        || lane.ledger_purpose != projection.ledger_purpose
        || lane.invocation_epoch != invocation_epoch
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "active resource lane for ledger {} does not match side-effect failure context",
            projection.ledger_key
        )));
    }
    Ok(Some(events::KernelEventPayload::ResourceLaneReleaseIntent(
        events::ResourceLaneReleaseIntent {
            spec_hash: runtime_spec.spec_hash().clone(),
            ledger_key: projection.ledger_key.clone(),
            ledger_purpose: projection.ledger_purpose.clone(),
            pair_id: projection.pair_id.clone(),
            pair_role: events::SideEffectPairRole::Verify,
            invocation_epoch,
            claim_id: lane.claim_id.clone(),
            release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
            release_reason: events::ResourceLaneReleaseReason::new("side_effect.failed")?,
        },
    )))
}

fn prepare_runner_output_commit_plan(
    request: store::CommitRequest,
    artifacts: store::CommitArtifactEvidenceSet,
    saga_terminal_proof: Option<store::SagaTerminalProof>,
) -> Result<store::PreparedCommitPlan> {
    let payloads = request.payloads();
    let has_run_completed = payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::RunCompleted(_)));
    let has_retention_manifest_projection = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::RetentionManifestProjected(_)
        )
    });
    if has_run_completed && request.preconditions().certified_run_authority.is_some() {
        let proof = saga_terminal_proof.ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(
                "saga terminal resolution requires SagaTerminalProof".to_owned(),
            )
        })?;
        return Ok(
            store::PreparedCommit::<store::SagaTerminal>::new(request, artifacts, &proof)?.into(),
        );
    }
    if has_run_completed {
        return Ok(
            store::PreparedCommit::<store::AttemptTerminal>::new(request, artifacts)?.into(),
        );
    }
    if has_retention_manifest_projection {
        return Ok(store::PreparedCommit::<store::Retention>::new(request, artifacts)?.into());
    }
    if payloads
        .iter()
        .any(events::KernelEventPayload::is_side_effect_terminal)
    {
        return Ok(
            store::PreparedCommit::<store::SideEffectTerminal>::new(request, artifacts)?.into(),
        );
    }
    if payloads
        .iter()
        .any(events::KernelEventPayload::is_attempt_terminal)
    {
        return Ok(
            store::PreparedCommit::<store::AttemptTerminal>::new(request, artifacts)?.into(),
        );
    }
    if payloads
        .iter()
        .any(|payload| payload.side_effect_ledger_ref().is_some())
    {
        return Ok(
            store::PreparedCommit::<store::SideEffectProgress>::new(request, artifacts)?.into(),
        );
    }
    if payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::RetentionRefsAppended(_)
        )
    }) {
        return Ok(store::PreparedCommit::<store::Retention>::new(request, artifacts)?.into());
    }
    Ok(store::PreparedCommit::<store::AttemptTerminal>::new(request, artifacts)?.into())
}
