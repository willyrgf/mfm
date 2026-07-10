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
            spec_artifact: run_artifact_ref_from_store(&spec_artifact),
            certificate_artifact: run_artifact_ref_from_store(&certificate_artifact),
            config_artifacts: config_artifacts
                .iter()
                .map(run_artifact_ref_from_store)
                .collect(),
            fact_descriptor_artifacts: fact_descriptor_evidence
                .iter()
                .map(run_artifact_ref_from_store)
                .collect(),
            spec_version: runtime_spec.spec().spec_version.clone(),
            lowering_version: runtime_spec.spec().lowering_version.clone(),
            public_output_schema_id: runtime_spec.spec().public_outputs.public_schema_id.clone(),
            saga_policy_digest: runtime_spec.spec().saga.saga_policy_digest()?,
            descriptor_identities: runtime_spec.spec().descriptor_identities.clone(),
            runner_executables: bound_context.runner_executables().to_vec(),
            adapter_executables,
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
        ));
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
        if payloads.iter().any(is_side_effect_terminal_payload) {
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
    if payloads.iter().any(is_side_effect_terminal_payload) {
        return Ok(
            store::PreparedCommit::<store::SideEffectTerminal>::new(request, artifacts)?.into(),
        );
    }
    if payloads.iter().any(is_attempt_terminal_payload) {
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

fn is_attempt_terminal_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(
        payload,
        events::KernelEventPayload::StateAttemptCompleted(_)
            | events::KernelEventPayload::StateAttemptInterrupted(_)
            | events::KernelEventPayload::StateAttemptFailed(_)
            | events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::FactRecorded(_)
            | events::KernelEventPayload::ArtifactReferenced(_)
            | events::KernelEventPayload::PublicOutputProduced(_)
            | events::KernelEventPayload::PublicOutputRenderFailed(_)
            | events::KernelEventPayload::RunCompleted(_)
    )
}

fn is_side_effect_terminal_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(
        payload,
        events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_)
            | events::KernelEventPayload::ResourceLaneReleased(_)
            | events::KernelEventPayload::ResourceLaneReleaseIntent(_)
    )
}

fn is_side_effect_terminal_disposition_payload(payload: &events::KernelEventPayload) -> bool {
    is_side_effect_terminal_payload(payload)
        && !matches!(
            payload,
            events::KernelEventPayload::ResourceLaneReleased(_)
                | events::KernelEventPayload::ResourceLaneReleaseIntent(_)
        )
}

fn required_artifacts_for_payloads(
    view: &RuntimeRunView,
    required_artifacts: Vec<store::ArtifactEvidenceRef>,
    payloads: &[events::KernelEventPayload],
) -> Result<Vec<store::ArtifactEvidenceRef>> {
    let mut required_artifacts = required_artifacts;
    for payload in payloads {
        for requirement in store::event_artifact_requirements(payload) {
            if required_artifacts.iter().any(|evidence| {
                evidence.artifact_id == requirement.artifact_id
                    && store::validate_artifact_requirement_against_evidence(&requirement, evidence)
                        .is_ok()
            }) {
                continue;
            }
            let Some(evidence) = committed_artifact_for_requirement(view, &requirement) else {
                if requirement.source.is_retention() {
                    if let Some(evidence) =
                        retained_fact_artifact_evidence_for_requirement(view, &requirement)?
                    {
                        required_artifacts.push(evidence);
                    }
                    continue;
                }
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "payload references artifact {} without required evidence",
                    requirement.artifact_id
                )));
            };
            required_artifacts.push(evidence);
        }
    }
    Ok(required_artifacts)
}

fn retained_fact_artifact_evidence_for_requirement(
    view: &RuntimeRunView,
    requirement: &store::EventArtifactRequirement,
) -> Result<Option<store::ArtifactEvidenceRef>> {
    match requirement.artifact_role {
        Some(events::ArtifactRole::FactDescriptor) => {
            let descriptor_hash = requirement.digest.as_ref().ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "retention ref for artifact {} lacks descriptor digest",
                    requirement.artifact_id
                ))
            })?;
            let Some(descriptor) = view
                .projections
                .fact_descriptor(descriptor_hash)
                .filter(|descriptor| descriptor.descriptor_artifact_id == requirement.artifact_id)
                .filter(|descriptor| {
                    requirement.evidence_hash.as_ref().is_some_and(|expected| {
                        descriptor
                            .descriptor_artifact_evidence
                            .evidence_hash()
                            .is_ok_and(|actual| &actual == expected)
                    })
                })
            else {
                return Ok(None);
            };
            Ok(Some(descriptor.descriptor_artifact_evidence.clone()))
        }
        Some(events::ArtifactRole::FactResponse) => Ok(view
            .projections
            .fact_index_entries()
            .find(|(_, index)| {
                index.artifact_id == requirement.artifact_id
                    && requirement
                        .digest
                        .as_ref()
                        .is_some_and(|digest| index.response_hash == *digest)
                    && requirement
                        .evidence_hash
                        .as_ref()
                        .is_some_and(|evidence_hash| index.artifact_evidence_hash == *evidence_hash)
            })
            .and_then(|(claim_id, _index)| {
                let record = view.projections.fact_record(claim_id)?;
                let evidence = record.response_artifact_evidence.as_ref()?;
                let evidence_hash = evidence.evidence_hash().ok()?;
                (requirement.evidence_hash.as_ref() == Some(&evidence_hash))
                    .then(|| evidence.clone())
            })),
        _ => Ok(None),
    }
}

fn committed_artifact_for_requirement(
    view: &RuntimeRunView,
    requirement: &store::EventArtifactRequirement,
) -> Option<store::ArtifactEvidenceRef> {
    view.artifact_refs
        .values()
        .find(|reference| {
            store::validate_artifact_requirement_against_evidence(requirement, &reference.evidence)
                .is_ok()
        })
        .map(|reference| reference.evidence.clone())
        .or_else(|| {
            view.config_artifacts
                .values()
                .find(|artifact| artifact.artifact_id == requirement.artifact_id)
                .cloned()
        })
}

fn launch_artifacts_by_evidence(
    artifacts: Vec<RunLaunchArtifact>,
    kind: &'static str,
) -> Result<BTreeMap<(ArtifactId, ContentDigest), RunLaunchArtifact>> {
    let mut by_evidence = BTreeMap::new();
    for artifact in artifacts {
        verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
        let evidence_hash = artifact.evidence.evidence_hash()?;
        let key = (artifact.evidence.artifact_id.clone(), evidence_hash);
        if by_evidence.insert(key, artifact).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate staged {kind} launch artifact"
            )));
        }
    }
    Ok(by_evidence)
}

fn validate_fact_descriptor_launch_artifacts(
    runtime_spec: &CertifiedRuntimeSpec,
    artifacts: Vec<RunLaunchArtifact>,
) -> Result<Vec<RunLaunchArtifact>> {
    let required = runtime_spec.fact_descriptor_hashes();
    let schema_id = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
    let media_type = spec::MediaType::new("application/json")?;
    let mut by_hash = BTreeMap::new();

    for artifact in artifacts {
        verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
        let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(&artifact.bytes)
            .map_err(runtime_fact_error)?;
        let canonical =
            mfm_facts::canonical_fact_descriptor_bytes(&descriptor).map_err(runtime_fact_error)?;
        let descriptor_hash =
            mfm_facts::fact_descriptor_hash(&descriptor).map_err(runtime_fact_error)?;
        let expected_artifact_id =
            ArtifactId::from_digest(descriptor_hash.algorithm(), *descriptor_hash.digest());
        if artifact.evidence.artifact_id != expected_artifact_id
            || artifact.evidence.digest != descriptor_hash
            || artifact.evidence.byte_len != canonical.as_bytes().len() as u64
            || artifact.evidence.media_type != media_type
            || artifact.evidence.schema_id.as_ref() != Some(&schema_id)
            || artifact.evidence.semantic_type_id.is_some()
            || artifact.evidence.producer_node_id.is_some()
            || artifact.evidence.producer_seed_id.is_some()
            || artifact.evidence.artifact_role != events::ArtifactRole::FactDescriptor
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "fact descriptor artifact evidence does not match descriptor {}",
                descriptor_hash
            )));
        }
        if by_hash.insert(descriptor_hash.clone(), artifact).is_some() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "duplicate fact descriptor artifact for {}",
                descriptor_hash
            )));
        }
    }

    for required_hash in &required {
        if !by_hash.contains_key(required_hash) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "missing fact descriptor artifact for certified descriptor {}",
                required_hash
            )));
        }
    }
    for admitted_hash in by_hash.keys() {
        if !required.contains(admitted_hash) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "fact descriptor artifact {} is not certified by the runtime spec",
                admitted_hash
            )));
        }
    }

    Ok(by_hash.into_values().collect())
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
    for (payload_ordinal, payload) in payloads.iter().enumerate() {
        fragments.insert(runner_output_commit_fragment(payload_ordinal, payload)?);
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

fn runner_output_commit_fragment(
    payload_ordinal: usize,
    payload: &events::KernelEventPayload,
) -> Result<String> {
    Ok(match payload {
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
        events::KernelEventPayload::FactRecorded(payload) => {
            let payload_hash = store::payload_canonical_json(
                &events::KernelEventPayload::FactRecorded(payload.clone()),
            )?
            .content_digest();
            format!("fact-payload:{}:{}", payload_ordinal, payload_hash)
        }
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
            format!("sidefx-intent:{}", payload.pair_id)
        }
        events::KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx-claim:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx-claim-takeover:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::ResourceLaneClaimed(payload) => format!(
            "resource-lane-claimed:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.claim_id
        ),
        events::KernelEventPayload::ResourceLaneClaimIntent(payload) => format!(
            "resource-lane-claim-intent:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx-prepared:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx-started:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx-not-submitted:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx-submission:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx-submission-unknown:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx-receipt:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx-confirmation:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!("sidefx-ambiguous:{}", payload.pair_id)
        }
        events::KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx-failed:{}:{}",
            payload.pair_id, payload.invocation_epoch
        ),
        events::KernelEventPayload::ResourceLaneReleased(payload) => format!(
            "resource-lane-released:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.release_id
        ),
        events::KernelEventPayload::ResourceLaneReleaseIntent(payload) => format!(
            "resource-lane-release-intent:{}:{}:{}",
            payload.pair_id, payload.invocation_epoch, payload.claim_id
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
        events::KernelEventPayload::RunAdmitted(_)
        | events::KernelEventPayload::ManualResolutionRecorded(_)
        | events::KernelEventPayload::RunCompleted(_)
        | events::KernelEventPayload::StateAttemptStarted(_)
        | events::KernelEventPayload::StateAttemptInterrupted(_) => "scheduler-owned".to_owned(),
    })
}

fn attempt_failure_commit_fragment(payload: &events::StateAttemptFailed) -> Result<String> {
    Ok(content_digest_json(serde_json::json!({
        "code": payload.error.code.as_str(),
        "retryable": payload.retryable,
    }))?
    .to_string())
}

fn validate_attempt_failure_diagnostic_artifact(
    node: &spec::NodeSpec,
    artifact: PreparedStagedArtifact,
) -> Result<PreparedStagedArtifact> {
    verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
    if artifact.evidence.artifact_role != events::ArtifactRole::RedactedDiagnostic {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} failure diagnostic artifact has role {}",
            node.node_id,
            artifact_role_name(artifact.evidence.artifact_role)
        )));
    }
    if artifact.evidence.producer_node_id.as_ref() != Some(&node.node_id)
        || artifact.evidence.producer_seed_id.is_some()
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} failure diagnostic artifact {} has invalid producer evidence",
            node.node_id, artifact.evidence.artifact_id
        )));
    }
    if artifact.evidence.schema_id.is_none() || artifact.evidence.semantic_type_id.is_some() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} failure diagnostic artifact {} has invalid typing evidence",
            node.node_id, artifact.evidence.artifact_id
        )));
    }
    if artifact.evidence.media_type != spec::MediaType::new("application/json")? {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} failure diagnostic artifact {} has unsupported media type",
            node.node_id, artifact.evidence.artifact_id
        )));
    }
    Ok(artifact)
}

fn validate_staged_artifacts(
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    staged_artifacts: &[StagedArtifact],
) -> Result<Vec<ValidatedStagedArtifact>> {
    let mut by_artifact = BTreeMap::<(ArtifactId, ContentDigest), ValidatedStagedArtifact>::new();
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
        let evidence_key = (
            handle.evidence().artifact_id.clone(),
            handle
                .evidence()
                .evidence_hash()
                .map_err(RuntimeError::from)?,
        );
        if let Some(existing) = by_artifact.get(&evidence_key) {
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
            evidence_key,
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
    artifact_bytes: &store::ArtifactByteAuthorityMap,
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
    let expected = build_retention_manifest_artifact(
        runtime_spec,
        run_id,
        pre_projection_stream,
        artifact_bytes,
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
) -> Result<Vec<events::KernelEventPayload>> {
    let manifest_ref = manifest.evidence.retention_ref()?;
    Ok(vec![
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
    ])
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
    for staged in staged_artifacts {
        if matches!(
            staged.binding,
            StagedArtifactBindingKind::FactQueryEvidence
                | StagedArtifactBindingKind::ExternalReadEvidence
        ) {
            continue;
        }
        if !requirements
            .iter()
            .any(|requirement| staged_artifact_matches_requirement(node, staged, requirement))
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} without typed payload reference",
                node.node_id, staged.evidence.artifact_id
            )));
        }
    }

    for requirement in &requirements {
        if !staged_artifacts
            .iter()
            .any(|staged| staged_artifact_matches_requirement(node, staged, requirement))
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} referenced artifact {} without staged artifact",
                node.node_id, requirement.artifact_id
            )));
        }
    }
    Ok(())
}

fn validate_context_bound_output_artifacts(
    node: &spec::NodeSpec,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
    extractor: Option<&dyn ContextOutputExtractor>,
) -> Result<()> {
    let output_context = payloads.iter().find_map(|payload| match payload {
        events::KernelEventPayload::CellProduced(payload)
            if payload.cell_id == node.output_cell =>
        {
            Some((
                &payload.context,
                &payload.artifact_id,
                &payload.content_digest,
            ))
        }
        _ => None,
    });
    let Some((context, artifact_id, digest)) = output_context else {
        return Ok(());
    };
    if matches!(context, spec::CellContextSpec::NoContext) {
        return Ok(());
    }
    let extractor = extractor.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} produced context-bound output without a registered context output extractor",
            node.node_id
        ))
    })?;
    let staged = staged_artifacts
        .iter()
        .find(|artifact| {
            artifact.binding == StagedArtifactBindingKind::StateOutput
                && artifact.evidence.artifact_id == *artifact_id
                && artifact.evidence.digest == *digest
        })
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "node {} produced context-bound artifact {} without staged artifact evidence",
                node.node_id, artifact_id
            ))
        })?;
    let bytes = staged.bytes.as_deref().ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} produced context-bound artifact {} without staged bytes",
            node.node_id, artifact_id
        ))
    })?;
    extractor.validate_context_output(context, &staged.evidence, bytes)
}

fn staged_artifact_matches_requirement(
    node: &spec::NodeSpec,
    staged: &ValidatedStagedArtifact,
    requirement: &StagedArtifactRequirement,
) -> bool {
    validate_staged_artifact_requirement(node, &staged.evidence, &staged.binding, requirement)
        .is_ok()
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

fn staged_payload_artifact_requirements(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<Vec<StagedArtifactRequirement>> {
    let mut requirements = Vec::new();
    for payload in payloads {
        for requirement in store::event_artifact_requirements(payload) {
            let Some(binding) =
                staged_payload_artifact_binding(node, attempt_id, payload, requirement.source)?
            else {
                continue;
            };
            requirements.push(staged_artifact_requirement_from_event_requirement(
                node,
                requirement,
                binding,
            )?);
        }
    }
    Ok(requirements)
}

fn staged_payload_artifact_binding(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payload: &events::KernelEventPayload,
    source: store::EventArtifactReferenceSource,
) -> Result<Option<StagedArtifactBindingKind>> {
    if let Some(phase) = staged_side_effect_artifact_phase_for_source(source) {
        return match payload.side_effect_ref() {
            Some(side_effect) => {
                staged_side_effect_artifact_binding(node, attempt_id, side_effect, phase)
            }
            None => Ok(None),
        };
    }

    match (payload, source) {
        (
            events::KernelEventPayload::FactRecorded(payload),
            store::EventArtifactReferenceSource::FactResponse,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::FactResponse))
        }
        (
            events::KernelEventPayload::CellProduced(payload),
            store::EventArtifactReferenceSource::StateOutput,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::StateOutput))
        }
        (
            events::KernelEventPayload::PublicOutputProduced(payload),
            store::EventArtifactReferenceSource::PublicOutputRendered,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::PublicOutput))
        }
        (
            events::KernelEventPayload::PublicOutputRenderFailed(payload),
            store::EventArtifactReferenceSource::PublicOutputRenderFailureDiagnostic,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::RedactedDiagnostic))
        }
        (
            events::KernelEventPayload::StateAttemptFailed(payload),
            store::EventArtifactReferenceSource::StateAttemptFailureDiagnostic,
        ) => {
            require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::RedactedDiagnostic))
        }
        (
            events::KernelEventPayload::ArtifactReferenced(payload),
            store::EventArtifactReferenceSource::ArtifactReferenced,
        ) if payload.artifact_ref.role == events::ArtifactRole::ExternalReadEvidence => {
            let node_id = payload.node_id.as_ref().ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "node {} external read evidence reference lacks a producer node",
                    node.node_id
                ))
            })?;
            let referenced_attempt_id = payload.attempt_id.as_ref().ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "node {} external read evidence reference lacks an attempt",
                    node.node_id
                ))
            })?;
            require_attempt(node, attempt_id, node_id, referenced_attempt_id)?;
            Ok(Some(StagedArtifactBindingKind::ExternalReadEvidence))
        }
        _ => Ok(None),
    }
}

fn staged_side_effect_artifact_binding(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    side_effect: events::SideEffectEventRef<'_>,
    phase: StagedSideEffectArtifactPhase,
) -> Result<Option<StagedArtifactBindingKind>> {
    require_attempt(
        node,
        attempt_id,
        side_effect.node_id,
        side_effect.attempt_id,
    )?;
    let invocation_epoch = side_effect.invocation_epoch.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} side-effect artifact payload lacks invocation epoch",
            node.node_id
        ))
    })?;
    Ok(Some(StagedArtifactBindingKind::SideEffectEvidence {
        ledger_key: side_effect.ledger_key.clone(),
        invocation_epoch,
        phase,
    }))
}

fn staged_side_effect_artifact_phase_for_source(
    source: store::EventArtifactReferenceSource,
) -> Option<StagedSideEffectArtifactPhase> {
    let role = match source {
        store::EventArtifactReferenceSource::SideEffectIntent => {
            events::ArtifactRole::SideEffectIntent
        }
        store::EventArtifactReferenceSource::PreparedInvocation => {
            events::ArtifactRole::PreparedInvocation
        }
        store::EventArtifactReferenceSource::NotSubmittedProof => {
            events::ArtifactRole::NotSubmittedProof
        }
        store::EventArtifactReferenceSource::Submission => events::ArtifactRole::Submission,
        store::EventArtifactReferenceSource::SubmissionUnknownEvidence => {
            events::ArtifactRole::SubmissionUnknownEvidence
        }
        store::EventArtifactReferenceSource::Receipt => events::ArtifactRole::Receipt,
        store::EventArtifactReferenceSource::Confirmation => events::ArtifactRole::Confirmation,
        store::EventArtifactReferenceSource::AmbiguityEvidence => {
            events::ArtifactRole::AmbiguityEvidence
        }
        _ => return None,
    };
    staged_side_effect_artifact_phase(role)
}

fn staged_artifact_requirement_from_event_requirement(
    node: &spec::NodeSpec,
    requirement: store::EventArtifactRequirement,
    binding: StagedArtifactBindingKind,
) -> Result<StagedArtifactRequirement> {
    let role = requirement.artifact_role.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} typed payload requirement for artifact {} lacks artifact role",
            node.node_id, requirement.artifact_id
        ))
    })?;
    let digest = requirement.digest.ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "node {} typed payload requirement for artifact {} lacks digest",
            node.node_id, requirement.artifact_id
        ))
    })?;
    let expected_role = staged_artifact_binding_role(&binding);
    if role != expected_role {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} typed payload requirement role {} does not match staged binding",
            node.node_id,
            artifact_role_name(role)
        )));
    }
    Ok(StagedArtifactRequirement {
        artifact_id: requirement.artifact_id,
        digest,
        byte_len: requirement.byte_len,
        media_type: requirement.media_type,
        schema_id: requirement.schema_id,
        semantic_type_id: requirement.semantic_type_id,
        role,
        binding,
    })
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

fn bind_staged_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    projections: &store::ProjectionSnapshot,
    required_artifacts: &[store::ArtifactEvidenceRef],
    staged: Vec<StagedRetentionRefs>,
) -> Result<Vec<events::KernelEventPayload>> {
    let mut payloads = Vec::with_capacity(staged.len());
    for staged_refs in staged {
        let reason = staged_refs.reason;
        validate_staged_retention_reason(runtime_spec, node, required_artifacts, &staged_refs)?;
        if staged_refs.refs.is_empty() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged empty retention refs",
                node.node_id
            )));
        }
        validate_fact_query_evidence_retention_set(projections, &staged_refs)?;
        for retention_ref in &staged_refs.refs {
            if let Some(artifact) =
                artifact_evidence_for_retention_ref(required_artifacts, retention_ref)
            {
                if artifact.digest != retention_ref.content_digest
                    || artifact.artifact_role != retention_ref.role
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} staged retention evidence for artifact {} does not match artifact evidence",
                        node.node_id, retention_ref.artifact_id
                    )));
                }
                continue;
            }

            if !staged_retention_ref_authorized_by_existing_fact_query_evidence(
                projections,
                &staged_refs,
                retention_ref,
            ) {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged retention for artifact {} without staged artifact evidence",
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

fn validate_fact_query_evidence_retention_set(
    projections: &store::ProjectionSnapshot,
    staged_refs: &StagedRetentionRefs,
) -> Result<()> {
    let StagedRetentionRefAuthority::FactQueryEvidence { returned_refs } = &staged_refs.authority
    else {
        return Ok(());
    };

    if !staged_refs
        .refs
        .iter()
        .any(|reference| reference.role == events::ArtifactRole::FactQueryEvidence)
    {
        return Err(RuntimeError::InvalidRunnerOutput(
            "fact query evidence retention missing query evidence artifact".to_owned(),
        ));
    }

    for fact_ref in returned_refs {
        let [descriptor_ref, response_ref] =
            fact_query_returned_ref_retention_refs(projections, fact_ref)?;
        if !staged_refs.refs.contains(&descriptor_ref) {
            return Err(RuntimeError::InvalidRunnerOutput(
                "fact query evidence retention missing descriptor artifact authority".to_owned(),
            ));
        }

        if !staged_refs.refs.contains(&response_ref) {
            return Err(RuntimeError::InvalidRunnerOutput(
                "fact query evidence retention missing response artifact authority".to_owned(),
            ));
        }
    }

    Ok(())
}

fn staged_retention_ref_authorized_by_existing_fact_query_evidence(
    projections: &store::ProjectionSnapshot,
    staged_refs: &StagedRetentionRefs,
    retention_ref: &events::RetentionRef,
) -> bool {
    let StagedRetentionRefAuthority::FactQueryEvidence { returned_refs } = &staged_refs.authority
    else {
        return false;
    };

    match retention_ref.role {
        events::ArtifactRole::FactDescriptor | events::ArtifactRole::FactResponse => {
            returned_refs.iter().any(|fact_ref| {
                fact_query_returned_ref_retention_refs(projections, fact_ref)
                    .map(|refs| refs.contains(retention_ref))
                    .unwrap_or(false)
            })
        }
        _ => false,
    }
}

fn validate_staged_retention_reason(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    artifact_evidence: &[store::ArtifactEvidenceRef],
    staged_refs: &StagedRetentionRefs,
) -> Result<()> {
    match staged_refs.reason {
        events::RetentionReason::RunAdmitted | events::RetentionReason::ManifestProjection => {
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
                let artifact = artifact_evidence_for_retention_ref(artifact_evidence, retention_ref)
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

fn artifact_evidence_for_retention_ref<'a>(
    artifacts: &'a [store::ArtifactEvidenceRef],
    retention_ref: &events::RetentionRef,
) -> Option<&'a store::ArtifactEvidenceRef> {
    artifacts.iter().find(|artifact| {
        artifact.artifact_id == retention_ref.artifact_id
            && artifact.digest == retention_ref.content_digest
            && artifact.artifact_role == retention_ref.role
            && artifact
                .evidence_hash()
                .ok()
                .is_some_and(|evidence_hash| evidence_hash == retention_ref.evidence_hash)
    })
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
        let completion = run_completion_evidence(runtime_spec, run_id, projections)?;
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
    if matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    ) {
        preconditions.certified_run_authority =
            Some(certified_run_authority(runtime_spec, run_id)?);
    }
    if payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::FactRecorded(_)))
    {
        preconditions.certified_run_authority =
            Some(certified_run_authority(runtime_spec, run_id)?);
    }
    if node.side_effect.is_some() || side_effect_verify_spec(node).is_some() {
        preconditions.certified_run_authority =
            Some(certified_run_authority(runtime_spec, run_id)?);
    }

    if let Some(verify) = side_effect_verify_spec(node) {
        add_side_effect_verify_preconditions(
            runtime_spec,
            run_id,
            projections,
            node,
            verify,
            payloads,
            &mut preconditions,
        )?;
        return Ok(preconditions);
    }

    if node.side_effect.is_none() {
        return Ok(preconditions);
    }

    let terminal_side_effect_required_state = payloads.iter().find_map(|payload| match payload {
        events::KernelEventPayload::CellSkipped(_) => {
            Some(store::RequiredSideEffectState::SubmissionResult)
        }
        events::KernelEventPayload::CellProduced(_) => {
            Some(store::RequiredSideEffectState::ConfirmationObserved)
        }
        _ => None,
    });
    let prepared_in_batch = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                Some(payload.pair_id.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        pair_id: payload.pair_id.clone(),
                        required: store::RequiredSideEffectState::Absent,
                    },
                );
            }
            events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
                if !prepared_in_batch.contains(&payload.pair_id) {
                    preconditions.required_side_effect_states.push(
                        store::SideEffectStatePrecondition {
                            pair_id: payload.pair_id.clone(),
                            required: store::RequiredSideEffectState::InvocationPrepared,
                        },
                    );
                }
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        pair_id: payload.pair_id.clone(),
                        required: store::RequiredSideEffectState::SubmissionResult,
                    },
                );
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        pair_id: payload.pair_id.clone(),
                        required: store::RequiredSideEffectState::ReceiptObserved,
                    },
                );
            }
            _ => {}
        }
    }

    if let Some(required) = terminal_side_effect_required_state {
        let projection = side_effect_projection_for_attempt(
            runtime_spec,
            run_id,
            projections,
            node,
            attempt_id,
        )?
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted output without ledger evidence",
                node.node_id
            ))
        })?;
        preconditions
            .required_side_effect_states
            .push(store::SideEffectStatePrecondition {
                pair_id: projection.pair_id.clone(),
                required,
            });
    }

    Ok(preconditions)
}

fn add_side_effect_verify_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    verify: &spec::SideEffectVerifyNodeSpec,
    payloads: &[events::KernelEventPayload],
    preconditions: &mut store::CommitPreconditions,
) -> Result<()> {
    let projection = projections
        .side_effect_for_pair(run_id, &verify.pair_id)
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} has no ledger projection for pair {}",
                node.node_id, verify.pair_id
            ))
        })?;
    let terminal_required = side_effect_verify_terminal_required_state(runtime_spec, verify)?;
    for payload in payloads {
        let required = match payload {
            events::KernelEventPayload::SideEffectReceiptObserved(_) => {
                Some(store::RequiredSideEffectState::SubmissionResult)
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(_) => {
                Some(store::RequiredSideEffectState::ReceiptObserved)
            }
            events::KernelEventPayload::SideEffectFailed(_) => {
                Some(store::RequiredSideEffectState::SubmissionResult)
            }
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::StateAttemptCompleted(_) => Some(terminal_required),
            _ => None,
        };
        if let Some(required) = required {
            push_side_effect_precondition(preconditions, projection.pair_id.clone(), required);
        }
    }
    Ok(())
}

fn side_effect_verify_terminal_required_state(
    runtime_spec: &CertifiedRuntimeSpec,
    verify: &spec::SideEffectVerifyNodeSpec,
) -> Result<store::RequiredSideEffectState> {
    let pair = runtime_spec
        .spec()
        .side_effect_verify_pair_for_pair_id(&verify.pair_id)
        .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
    match &pair.submit_contract.verification {
        spec::SideEffectVerificationSpec::Receipt => {
            Ok(store::RequiredSideEffectState::ReceiptObserved)
        }
        spec::SideEffectVerificationSpec::Finalized { .. } => {
            Ok(store::RequiredSideEffectState::ConfirmationObserved)
        }
    }
}

fn certified_run_authority(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<store::CertifiedRunStoreAuthority> {
    Ok(store::CertifiedRunStoreAuthority::from_spec(
        run_id.clone(),
        runtime_spec.spec(),
    )?)
}

fn push_side_effect_precondition(
    preconditions: &mut store::CommitPreconditions,
    pair_id: mfm_ids::SideEffectPairId,
    required: store::RequiredSideEffectState,
) {
    if preconditions
        .required_side_effect_states
        .iter()
        .any(|existing| existing.pair_id == pair_id && existing.required == required)
    {
        return;
    }
    preconditions
        .required_side_effect_states
        .push(store::SideEffectStatePrecondition { pair_id, required });
}

fn side_effect_verify_spec(node: &spec::NodeSpec) -> Option<&spec::SideEffectVerifyNodeSpec> {
    match &node.framework {
        Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => Some(verify),
        _ => None,
    }
}

fn attempt_commit_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    existing_attempt: Option<&AttemptId>,
) -> Result<store::CommitPreconditions> {
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
    if let Some(attempt_id) = existing_attempt {
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))?);
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
    Ok(events::MfmErrorInfo::new(
        events::ErrorCode::new("side_effect_ambiguous")?,
        events::ErrorCategory::SideEffect,
        false,
        format!(
            "side-effect outcome is ambiguous: {}",
            payload.ambiguity_code
        ),
    )?)
}

struct RunnerOutputValidation<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    attempt_id: &'a AttemptId,
    caps: &'a CertifiedRuntimeCapabilities,
    recorded_facts: &'a RecordedFacts,
    projections: &'a store::ProjectionSnapshot,
    payloads: &'a [events::KernelEventPayload],
}

fn validate_runner_output(input: RunnerOutputValidation<'_>) -> Result<()> {
    let RunnerOutputValidation {
        runtime_spec,
        run_id,
        node,
        attempt_id,
        caps,
        recorded_facts,
        projections,
        payloads,
    } = input;
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
            events::KernelEventPayload::StateAttemptInterrupted(_) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned recovery-owned StateAttemptInterrupted",
                    node.node_id
                )));
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
                    || cell.context != payload.context
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
                    || cell.context != payload.context
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
                let fact_key = payload.claim.subject().fact_key();
                if !recorded_facts.is_empty() {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} attempted to record fact {} after committed facts existed for the same attempt",
                        node.node_id, fact_key
                    )));
                }
                let producer = payload.claim.producer();
                require_capability(
                    caps,
                    producer.capability_kind(),
                    producer.capability_version(),
                    &node.node_id,
                )?;
                require_adapter(node, producer.adapter_kind(), producer.adapter_version())?;
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
            events::KernelEventPayload::RunAdmitted(_)
            | events::KernelEventPayload::ManualResolutionRecorded(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_)
            | events::KernelEventPayload::ResourceLaneClaimed(_)
            | events::KernelEventPayload::ResourceLaneReleased(_) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned scheduler-owned payload",
                    node.node_id
                )));
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::ResourceLaneClaimIntent(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_)
            | events::KernelEventPayload::ResourceLaneReleaseIntent(_) => {
                validate_runner_side_effect_payload(
                    runtime_spec,
                    run_id,
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
    if side_effect_verify_spec(node).is_some() {
        return validate_side_effect_verify_runner_output(SideEffectVerifyRunnerOutputValidation {
            runtime_spec,
            run_id,
            node,
            projections,
            completed,
            failed,
            terminal_cell,
            public_output_produced,
            public_output_failed,
            side_effect_payload,
            side_effect_terminal_failure,
            attempt_failure_retryable,
            side_effect_terminal_failure_retryable,
            payloads,
        });
    }

    if node.side_effect.is_some() {
        validate_resume_output(
            runtime_spec,
            run_id,
            projections,
            node,
            attempt_id,
            payloads,
        )?;
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
        let terminal_skipped = payloads
            .iter()
            .any(|payload| matches!(payload, events::KernelEventPayload::CellSkipped(_)));
        validate_terminal_batch_evidence(
            runtime_spec,
            run_id,
            projections,
            node,
            attempt_id,
            terminal_skipped,
        )
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

struct SideEffectVerifyRunnerOutputValidation<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    projections: &'a store::ProjectionSnapshot,
    completed: bool,
    failed: bool,
    terminal_cell: bool,
    public_output_produced: bool,
    public_output_failed: bool,
    side_effect_payload: bool,
    side_effect_terminal_failure: bool,
    attempt_failure_retryable: Option<bool>,
    side_effect_terminal_failure_retryable: Option<bool>,
    payloads: &'a [events::KernelEventPayload],
}

fn validate_side_effect_verify_runner_output(
    input: SideEffectVerifyRunnerOutputValidation<'_>,
) -> Result<()> {
    let SideEffectVerifyRunnerOutputValidation {
        runtime_spec,
        run_id,
        node,
        projections,
        completed,
        failed,
        terminal_cell,
        public_output_produced,
        public_output_failed,
        side_effect_payload,
        side_effect_terminal_failure,
        attempt_failure_retryable,
        side_effect_terminal_failure_retryable,
        payloads,
    } = input;
    if failed {
        if !side_effect_terminal_failure {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} returned StateAttemptFailed without terminal side-effect evidence",
                node.node_id
            )));
        }
        if side_effect_terminal_failure_retryable != attempt_failure_retryable {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} returned inconsistent failure retryability",
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
            "side-effect verify node {} returned terminal side-effect evidence without StateAttemptFailed",
            node.node_id
        )));
    }
    if side_effect_payload && !terminal_cell && !completed {
        let has_resource_lane_release = payloads.iter().any(|payload| {
            matches!(
                payload,
                events::KernelEventPayload::ResourceLaneReleaseIntent(_)
            )
        });
        let has_side_effect_terminal_disposition = payloads
            .iter()
            .any(is_side_effect_terminal_disposition_payload);
        if has_resource_lane_release && !has_side_effect_terminal_disposition {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} returned a resource-lane release without terminal evidence",
                node.node_id
            )));
        }
        return Ok(());
    }
    if !completed || !terminal_cell {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} successful terminal commit must pair StateAttemptCompleted with terminal cell evidence",
            node.node_id
        )));
    }
    if public_output_failed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} returned public-output failure",
            node.node_id
        )));
    }
    if side_effect_payload
        && payloads.iter().any(|payload| {
            payload.side_effect_ledger_ref().is_some()
                && !matches!(
                    payload,
                    events::KernelEventPayload::ResourceLaneReleaseIntent(_)
                )
        })
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} mixed ledger phase events with terminal output evidence",
            node.node_id
        )));
    }
    validate_side_effect_verify_terminal_evidence(runtime_spec, run_id, node, projections)
}

fn validate_side_effect_verify_terminal_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    let Some(verify) = side_effect_verify_spec(node) else {
        return Ok(());
    };
    let projection = projections
        .side_effect_for_pair(run_id, &verify.pair_id)
        .ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "side-effect verify node {} has no ledger projection for pair {}",
                node.node_id, verify.pair_id
            ))
        })?;
    let state = projection
        .ledger_state()
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let required = side_effect_verify_terminal_required_state(runtime_spec, verify)?;
    let satisfied = match required {
        store::RequiredSideEffectState::ReceiptObserved => matches!(
            state.phase(),
            store::SideEffectLedgerPhase::ReceiptObserved { .. }
                | store::SideEffectLedgerPhase::Confirmed { .. }
        ),
        store::RequiredSideEffectState::ConfirmationObserved => {
            matches!(
                state.phase(),
                store::SideEffectLedgerPhase::Confirmed { .. }
            )
        }
        _ => false,
    };
    if satisfied {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} produced output before certified terminal evidence",
            node.node_id
        )))
    }
}

fn runtime_fact_error(error: mfm_facts::FactError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}
