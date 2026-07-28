use std::collections::{BTreeMap, BTreeSet};

use mfm_events::v1 as events;
use mfm_ids::RunId;
use mfm_store::v1 as store;

use crate::admission::RunAdmissionLifecycle;
use crate::attempt::{
    async_error_is_stale_expected_next_seq, AttemptLifecycle, AttemptRunResult, AttemptRunStatus,
    ResourceLaneBlockWitness,
};
use crate::binding::{BoundRuntimeContext, BoundRuntimeContextLoader};
use crate::commit::{prepared_commit_bundle, PreparedRunLaunch, RunLaunchEvidence};
use crate::error::async_store_error;
use crate::framework_lifecycle::FrameworkAttemptLifecycle;
use crate::history::{
    refresh_current_after_commit, refresh_current_after_stale_append, verify_current_run,
    VerifiedCurrentRun,
};
use crate::manual_resolution::{
    prepare_manual_resolution_commit, verify_manual_resolution_for_prefix,
    ManualResolutionCommitInput, ManualResolutionEvidenceArtifact,
};
use crate::recovery::{AttemptRecoveryLifecycle, RecoveryDispatch};
use crate::runners::ErasedRunnerRegistry;
use crate::transition::{TransitionAttempt, TransitionDecision, TransitionLifecycle};
use crate::{CertifiedRuntimeSpec, Result, RuntimeError};

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

/// One scheduler result paired with the sole verified current-run authority.
#[derive(Debug)]
pub struct SchedulerDriveResult {
    current_run: VerifiedCurrentRun,
    status: SchedulerStatus,
}

impl SchedulerDriveResult {
    /// Returns the verified current run after this drive call.
    pub fn current_run(&self) -> &VerifiedCurrentRun {
        &self.current_run
    }

    /// Returns the scheduler status.
    pub fn status(&self) -> SchedulerStatus {
        self.status
    }

    /// Consumes this result and returns its current-run authority.
    pub fn into_current_run(self) -> VerifiedCurrentRun {
        self.current_run
    }

    /// Consumes this result into its authority and status.
    pub fn into_parts(self) -> (VerifiedCurrentRun, SchedulerStatus) {
        (self.current_run, self.status)
    }
}

/// Manual resolution input verified against the certified saga policy and current journal prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionRequest {
    /// Operator-selected terminal resolution outcome.
    pub outcome: events::ManualResolutionOutcome,
    /// Artifact containing the retained manual evidence bytes.
    pub evidence_artifact: ManualResolutionEvidenceArtifact,
    /// Canonical proof bytes for the manual resolution claim.
    pub proof_bytes: Vec<u8>,
    /// Optional retained note attached to the resolution.
    pub note: Option<events::ManualResolutionNote>,
}

struct ManualResolutionCommitExpectation {
    run_id: RunId,
    sequence: store::StreamSeq,
    commit_key: store::CommitKey,
    payloads: Vec<events::KernelEventPayload>,
    artifacts: BTreeMap<store::ArtifactAuthorityKey, ExpectedManualResolutionArtifact>,
}

struct ExpectedManualResolutionArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

impl ManualResolutionCommitExpectation {
    fn from_bundle(bundle: &store::PreparedCommitBundle) -> Result<Self> {
        let request = bundle.request();
        if request.payloads().is_empty()
            || !matches!(
                request.payloads().last(),
                Some(events::KernelEventPayload::ManualResolutionRecorded(_))
            )
            || request.payloads()[..request.payloads().len() - 1]
                .iter()
                .any(|payload| {
                    !matches!(
                        payload,
                        events::KernelEventPayload::ResourceLaneReleaseIntent(_)
                    )
                })
        {
            return Err(RuntimeError::InvalidRunStream(
                "sealed manual resolution commit has an invalid payload shape".to_owned(),
            ));
        }
        if !bundle.existing_artifacts().is_empty() {
            return Err(RuntimeError::InvalidRunStream(
                "sealed manual resolution commit unexpectedly reused artifact authority".to_owned(),
            ));
        }

        let mut artifacts = BTreeMap::new();
        for artifact in bundle.artifact_bytes() {
            let key = (
                artifact.evidence().artifact_id.clone(),
                artifact.evidence_hash().clone(),
            );
            if artifacts
                .insert(
                    key,
                    ExpectedManualResolutionArtifact {
                        bytes: artifact.bytes().to_vec(),
                        evidence: artifact.evidence().clone(),
                    },
                )
                .is_some()
            {
                return Err(RuntimeError::InvalidRunStream(
                    "sealed manual resolution commit repeats artifact authority".to_owned(),
                ));
            }
        }

        let mut admitted = BTreeMap::new();
        for evidence in bundle.admitted_artifacts() {
            let key = (evidence.artifact_id.clone(), evidence.evidence_hash()?);
            if admitted.insert(key, evidence).is_some() {
                return Err(RuntimeError::InvalidRunStream(
                    "sealed manual resolution commit repeats admitted artifact authority"
                        .to_owned(),
                ));
            }
        }
        if admitted.len() != artifacts.len()
            || admitted.iter().any(|(key, evidence)| {
                artifacts
                    .get(key)
                    .is_none_or(|artifact| &artifact.evidence != *evidence)
            })
        {
            return Err(RuntimeError::InvalidRunStream(
                "sealed manual resolution artifact bytes do not match admitted authority"
                    .to_owned(),
            ));
        }

        Ok(Self {
            run_id: request.run_id().clone(),
            sequence: request.expected_next_seq(),
            commit_key: request.commit_key().clone(),
            payloads: request.payloads().to_vec(),
            artifacts,
        })
    }

    fn matches(&self, current: &VerifiedCurrentRun) -> bool {
        let lifecycle = current.lifecycle();
        let mut valid = true;
        let mut record_count = 0usize;
        let mut requirements =
            BTreeMap::<store::ArtifactAuthorityKey, store::EventArtifactRequirement>::new();
        let _ = lifecycle.visit_records::<()>(|record| {
            if record.sequence() != self.sequence {
                return std::ops::ControlFlow::Continue(());
            }
            let Some(expected) = self.payloads.get(record_count) else {
                valid = false;
                return std::ops::ControlFlow::Break(());
            };
            let Ok(expected_ordinal) = u32::try_from(record_count) else {
                valid = false;
                return std::ops::ControlFlow::Break(());
            };
            if record.commit_key() != &self.commit_key
                || record.run_id() != &self.run_id
                || record.ordinal().as_u32() != expected_ordinal
                || !manual_resolution_record_matches(expected, record.kind())
            {
                valid = false;
                return std::ops::ControlFlow::Break(());
            }
            let _ = record.visit_artifact_requirements::<()>(|requirement| {
                let key = (
                    requirement.artifact_id.clone(),
                    requirement.evidence_hash.clone(),
                );
                if requirements.insert(key, requirement.clone()).is_some() {
                    valid = false;
                    return std::ops::ControlFlow::Break(());
                }
                std::ops::ControlFlow::Continue(())
            });
            if !valid {
                return std::ops::ControlFlow::Break(());
            }
            record_count += 1;
            std::ops::ControlFlow::Continue(())
        });
        if !valid
            || record_count != self.payloads.len()
            || requirements.len() != self.artifacts.len()
            || requirements.keys().ne(self.artifacts.keys())
        {
            return false;
        }
        requirements.iter().all(|(key, requirement)| {
            let Some(expected) = self.artifacts.get(key) else {
                return false;
            };
            lifecycle
                .object_for_requirement(requirement)
                .is_some_and(|object| {
                    object.evidence() == &expected.evidence
                        && object.bytes() == expected.bytes.as_slice()
                })
        })
    }
}

fn manual_resolution_record_matches(
    expected: &events::KernelEventPayload,
    actual: store::current_lifecycle::CurrentRecordKindRef<'_>,
) -> bool {
    match (expected, actual) {
        (
            events::KernelEventPayload::ResourceLaneReleaseIntent(expected),
            store::current_lifecycle::CurrentRecordKindRef::ResourceLaneReleased(actual),
        ) => {
            actual.spec_hash == expected.spec_hash
                && actual.ledger_key == expected.ledger_key
                && actual.ledger_purpose == expected.ledger_purpose
                && actual.pair_id == expected.pair_id
                && actual.pair_role == expected.pair_role
                && actual.invocation_epoch == expected.invocation_epoch
                && actual.claim_id == expected.claim_id
                && actual.release_authority == expected.release_authority
                && actual.release_reason == expected.release_reason
        }
        (
            events::KernelEventPayload::ManualResolutionRecorded(expected),
            store::current_lifecycle::CurrentRecordKindRef::ManualResolutionRecorded(actual),
        ) => actual == expected,
        _ => false,
    }
}

#[cfg(test)]
pub(crate) struct ManualResolutionCommitExpectationForTest(ManualResolutionCommitExpectation);

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum ManualResolutionExpectationMutationForTest {
    RunId,
    Sequence,
    CommitKey,
    RecordCount,
    RecordPosition,
    ManualOutcome,
    ManualNote,
    ManualEvidence,
    ManualAuthorization,
    ReleaseFields,
    ArtifactKey,
    ArtifactObject,
}

#[cfg(test)]
impl ManualResolutionCommitExpectationForTest {
    pub(crate) fn from_bundle(bundle: &store::PreparedCommitBundle) -> Result<Self> {
        ManualResolutionCommitExpectation::from_bundle(bundle).map(Self)
    }

    pub(crate) fn matches(&self, current: &VerifiedCurrentRun) -> bool {
        self.0.matches(current)
    }

    pub(crate) fn mutated(
        mut self,
        mutation: ManualResolutionExpectationMutationForTest,
    ) -> Result<Self> {
        match mutation {
            ManualResolutionExpectationMutationForTest::RunId => {
                let first = mfm_ids::RunId::from_digest(
                    mfm_ids::DigestAlgorithm::Sha256JcsV1,
                    mfm_ids::DigestBytes::from_array([0x5b; 32]),
                );
                self.0.run_id = if self.0.run_id == first {
                    mfm_ids::RunId::from_digest(
                        mfm_ids::DigestAlgorithm::Sha256JcsV1,
                        mfm_ids::DigestBytes::from_array([0x5c; 32]),
                    )
                } else {
                    first
                };
            }
            ManualResolutionExpectationMutationForTest::Sequence => {
                self.0.sequence = store::StreamSeq::new(
                    self.0.sequence.as_u64().checked_add(1).ok_or_else(|| {
                        RuntimeError::InvalidRunStream("test manual sequence overflowed".to_owned())
                    })?,
                )?;
            }
            ManualResolutionExpectationMutationForTest::CommitKey => {
                self.0.commit_key = store::CommitKey::new("manual-resolution-mismatched-for-test")?;
            }
            ManualResolutionExpectationMutationForTest::RecordCount => {
                let release = self
                    .0
                    .payloads
                    .iter()
                    .find(|payload| {
                        matches!(
                            payload,
                            events::KernelEventPayload::ResourceLaneReleaseIntent(_)
                        )
                    })
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunStream(
                            "test manual expectation has no release record to duplicate".to_owned(),
                        )
                    })?;
                let terminal = self.0.payloads.len().checked_sub(1).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(
                        "test manual expectation has no terminal record".to_owned(),
                    )
                })?;
                self.0.payloads.insert(terminal, release);
            }
            ManualResolutionExpectationMutationForTest::RecordPosition => {
                if self.0.payloads.len() < 2 {
                    return Err(RuntimeError::InvalidRunStream(
                        "test manual expectation has no release record to reorder".to_owned(),
                    ));
                }
                let last = self.0.payloads.len() - 1;
                self.0.payloads.swap(0, last);
            }
            ManualResolutionExpectationMutationForTest::ManualOutcome => {
                let Some(events::KernelEventPayload::ManualResolutionRecorded(payload)) =
                    self.0.payloads.last_mut()
                else {
                    return Err(RuntimeError::InvalidRunStream(
                        "test manual expectation has no resolution record".to_owned(),
                    ));
                };
                payload.outcome = match payload.outcome {
                    events::ManualResolutionOutcome::ConfirmRemediated => {
                        events::ManualResolutionOutcome::FailWithoutAcdcClaim
                    }
                    events::ManualResolutionOutcome::FailWithoutAcdcClaim => {
                        events::ManualResolutionOutcome::ConfirmRemediated
                    }
                };
            }
            ManualResolutionExpectationMutationForTest::ManualNote => {
                let Some(events::KernelEventPayload::ManualResolutionRecorded(payload)) =
                    self.0.payloads.last_mut()
                else {
                    return Err(RuntimeError::InvalidRunStream(
                        "test manual expectation has no resolution record".to_owned(),
                    ));
                };
                payload.note = if payload.note.is_some() {
                    None
                } else {
                    Some(events::ManualResolutionNote::new(
                        "mismatched manual note for test",
                    )?)
                };
            }
            ManualResolutionExpectationMutationForTest::ManualEvidence => {
                let Some(events::KernelEventPayload::ManualResolutionRecorded(payload)) =
                    self.0.payloads.last_mut()
                else {
                    return Err(RuntimeError::InvalidRunStream(
                        "test manual expectation has no resolution record".to_owned(),
                    ));
                };
                payload.evidence_hash = mfm_ids::ContentDigest::from_digest(
                    payload.evidence_hash.algorithm(),
                    mfm_ids::DigestBytes::from_array([0x5d; 32]),
                );
            }
            ManualResolutionExpectationMutationForTest::ManualAuthorization => {
                let Some(events::KernelEventPayload::ManualResolutionRecorded(payload)) =
                    self.0.payloads.last_mut()
                else {
                    return Err(RuntimeError::InvalidRunStream(
                        "test manual expectation has no resolution record".to_owned(),
                    ));
                };
                payload.authorization_hash = mfm_ids::ContentDigest::from_digest(
                    payload.authorization_hash.algorithm(),
                    mfm_ids::DigestBytes::from_array([0x5e; 32]),
                );
            }
            ManualResolutionExpectationMutationForTest::ReleaseFields => {
                let Some(events::KernelEventPayload::ResourceLaneReleaseIntent(payload)) =
                    self.0.payloads.iter_mut().find(|payload| {
                        matches!(
                            payload,
                            events::KernelEventPayload::ResourceLaneReleaseIntent(_)
                        )
                    })
                else {
                    return Err(RuntimeError::InvalidRunStream(
                        "test manual expectation has no release record".to_owned(),
                    ));
                };
                payload.release_reason =
                    events::ResourceLaneReleaseReason::new("mismatched_for_test")?;
            }
            ManualResolutionExpectationMutationForTest::ArtifactKey => {
                let Some((key, artifact)) = self.0.artifacts.pop_first() else {
                    return Err(RuntimeError::InvalidRunStream(
                        "test manual expectation has no artifact".to_owned(),
                    ));
                };
                let mismatched_hash = mfm_ids::ContentDigest::from_digest(
                    mfm_ids::DigestAlgorithm::Sha256JcsV1,
                    mfm_ids::DigestBytes::from_array([0x5a; 32]),
                );
                self.0.artifacts.insert((key.0, mismatched_hash), artifact);
            }
            ManualResolutionExpectationMutationForTest::ArtifactObject => {
                let Some(mut artifact) = self.0.artifacts.first_entry() else {
                    return Err(RuntimeError::InvalidRunStream(
                        "test manual expectation has no artifact".to_owned(),
                    ));
                };
                artifact.get_mut().bytes.push(0);
            }
        }
        Ok(self)
    }
}

struct DriveStep {
    current_run: VerifiedCurrentRun,
    status: DriveStepStatus,
}

enum DriveStepStatus {
    Advanced,
    StaleView,
    Blocked,
    PublicOutputProjected,
    BlockedOnResourceLane {
        witness: Box<ResourceLaneBlockWitness>,
        advanced: bool,
    },
}

/// Serial typed scheduler.
#[derive(Clone)]
pub struct SerialTypedScheduler {
    runtime_contexts: BoundRuntimeContextLoader,
}

impl SerialTypedScheduler {
    /// Creates a scheduler using a certified runner registry.
    pub fn new(runners: ErasedRunnerRegistry) -> Self {
        Self {
            runtime_contexts: BoundRuntimeContextLoader::new(runners),
        }
    }

    /// Prepares sealed admission launch authority for a certified run.
    pub async fn prepare_run_launch(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        identity_material: events::RunIdentityMaterialV1,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
    ) -> Result<PreparedRunLaunch> {
        let bound_context = self.runtime_contexts.load(runtime_spec)?;
        bound_context
            .validate_launch_ingress(runtime_spec, &evidence)
            .await?;
        RunAdmissionLifecycle::prepare_run_launch(
            runtime_spec,
            identity_material,
            evidence,
            expected_next_seq,
            &bound_context,
        )
    }

    /// Validates this scheduler's executable bindings against stored admission evidence.
    pub fn validate_admitted_run_binding(&self, current: &VerifiedCurrentRun) -> Result<()> {
        let runtime_spec = current.runtime_spec();
        let bound_context = self.runtime_contexts.load_current(&runtime_spec)?;
        bound_context.validate_current_admission_binding(&current.lifecycle().admission()?)
    }

    /// Validates deployment ingress for an already admitted run from retained launch evidence.
    pub async fn validate_admitted_run_ingress(
        &self,
        current: &VerifiedCurrentRun,
        evidence: &RunLaunchEvidence,
    ) -> Result<()> {
        let runtime_spec = current.runtime_spec();
        let bound_context = self.runtime_contexts.load_current(&runtime_spec)?;
        bound_context
            .validate_launch_ingress(&runtime_spec, evidence)
            .await
    }

    /// Validates resume ingress only for domain nodes whose output remains non-terminal.
    pub async fn validate_admitted_run_ingress_for_pending_nodes(
        &self,
        current: &VerifiedCurrentRun,
        evidence: &RunLaunchEvidence,
    ) -> Result<()> {
        let runtime_spec = current.runtime_spec();
        let lifecycle = current.lifecycle();
        let bound_context = self.runtime_contexts.load_current(&runtime_spec)?;
        bound_context
            .validate_pending_launch_ingress(&runtime_spec, &lifecycle, evidence)
            .await
    }

    /// Appends the prepared typed admission commit through an async typed store.
    pub async fn start_run<S: store::RunJournalStore + ?Sized>(
        &self,
        store: &S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        let bundle = launch.into_prepared_commit_bundle()?;
        store
            .append_prepared_commit_bundle(bundle)
            .await
            .map_err(async_store_error)
    }

    /// Appends run admission while atomically acquiring the execution claim.
    pub async fn start_run_with_execution_claim<S: store::RunJournalStore + ?Sized>(
        &self,
        store: &S,
        launch: PreparedRunLaunch,
        claim: store::PreparedExecutionClaim,
    ) -> Result<store::CommitOutcome> {
        let bundle = launch
            .into_prepared_commit_bundle()?
            .with_execution_claim(claim)?;
        store
            .append_prepared_commit_bundle(bundle)
            .await
            .map_err(async_store_error)
    }

    /// Consumes certified authority after admission and loads the sole verified current run.
    pub async fn load_admitted_run<S: store::RunJournalStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<VerifiedCurrentRun> {
        let bound_context = self.runtime_contexts.load(&runtime_spec)?;
        let journal = store
            .load_committed_journal(run_id)
            .await
            .map_err(async_store_error)?;
        let current = verify_current_run(journal, runtime_spec)?;
        bound_context.validate_current_admission_binding(&current.lifecycle().admission()?)?;
        Ok(current)
    }

    /// Appends a verified manual resolution and returns its verified successor.
    pub async fn record_manual_resolution<S: store::RunJournalStore + ?Sized>(
        &self,
        store: &S,
        current: VerifiedCurrentRun,
        request: ManualResolutionRequest,
    ) -> Result<VerifiedCurrentRun> {
        let ManualResolutionRequest {
            outcome,
            evidence_artifact,
            proof_bytes,
            note,
        } = request;
        let (commit, artifacts_to_stage) = {
            let runtime_spec = current.runtime_spec();
            let lifecycle = current.lifecycle();
            let prefix = lifecycle
                .manual_resolution_prefix_authority()
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
            let verified = verify_manual_resolution_for_prefix(
                prefix,
                outcome,
                &evidence_artifact,
                proof_bytes,
            )?;
            prepare_manual_resolution_commit(ManualResolutionCommitInput {
                runtime_spec,
                lifecycle,
                verified,
                evidence_artifact,
                note,
            })?
        };
        let bundle = prepared_commit_bundle(commit.into(), artifacts_to_stage)?;
        let expectation = ManualResolutionCommitExpectation::from_bundle(&bundle)?;
        let outcome = match store.append_prepared_commit_bundle(bundle).await {
            Ok(outcome) => outcome,
            Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                let successor = refresh_current_after_stale_append(store, current).await?;
                if expectation.matches(&successor) {
                    return Ok(successor);
                }
                return Err(RuntimeError::ManualResolutionRequestStale {
                    expected_sequence: expectation.sequence,
                });
            }
            Err(error) => return Err(async_store_error(error)),
        };
        match outcome {
            store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_) => {
                refresh_current_after_commit(store, current, &outcome).await
            }
            store::CommitOutcome::AdmissionBlocked(block) => {
                Err(RuntimeError::InvalidRunStream(format!(
                    "manual resolution was blocked by resource lane {}:{}",
                    block.resource_lane_key.namespace, block.resource_lane_key.key
                )))
            }
            store::CommitOutcome::ExecutionClaimBusy(_) => Err(RuntimeError::InvalidRunStream(
                "manual resolution requested an execution claim outside admission".to_owned(),
            )),
        }
    }

    /// Runs one deterministic runnable node, returning the sole verified successor authority.
    pub async fn drive_once<S>(
        &self,
        store: &S,
        mut current: VerifiedCurrentRun,
        execution_scope: &store::ExecutionClaimScope,
        execution_claim_token: store::AdmissionToken,
    ) -> Result<SchedulerDriveResult>
    where
        S: store::RunJournalStore + store::ExecutionClaimStore + ?Sized,
    {
        let mut blocked_lanes = BTreeSet::new();
        loop {
            let run_id = current.view().run_id().clone();
            require_live_execution_claim(store, execution_scope, &run_id, &execution_claim_token)
                .await?;
            let step = self
                .drive_once_with_blocked_lanes(store, current, &blocked_lanes)
                .await?;
            current = step.current_run;
            match step.status {
                DriveStepStatus::Advanced => {
                    return Ok(SchedulerDriveResult {
                        current_run: current,
                        status: SchedulerStatus::Advanced,
                    });
                }
                DriveStepStatus::StaleView => continue,
                DriveStepStatus::Blocked => {
                    return Ok(SchedulerDriveResult {
                        current_run: current,
                        status: SchedulerStatus::Blocked,
                    });
                }
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerDriveResult {
                        current_run: current,
                        status: SchedulerStatus::PublicOutputProjected,
                    });
                }
                DriveStepStatus::BlockedOnResourceLane { witness, advanced } => {
                    if advanced {
                        return Ok(SchedulerDriveResult {
                            current_run: current,
                            status: SchedulerStatus::Advanced,
                        });
                    }
                    blocked_lanes.insert(*witness);
                }
            }
        }
    }

    /// Runs deterministic runnable nodes until blocked, returning the verified successor.
    pub async fn drive_until_blocked<S>(
        &self,
        store: &S,
        mut current: VerifiedCurrentRun,
        execution_scope: &store::ExecutionClaimScope,
        execution_claim_token: store::AdmissionToken,
    ) -> Result<SchedulerDriveResult>
    where
        S: store::RunJournalStore + store::ExecutionClaimStore + ?Sized,
    {
        let mut advanced = false;
        let mut blocked_lanes = BTreeSet::new();
        loop {
            let run_id = current.view().run_id().clone();
            require_live_execution_claim(store, execution_scope, &run_id, &execution_claim_token)
                .await?;
            let step = self
                .drive_once_with_blocked_lanes(store, current, &blocked_lanes)
                .await?;
            current = step.current_run;
            match step.status {
                DriveStepStatus::Advanced => advanced = true,
                DriveStepStatus::StaleView => continue,
                DriveStepStatus::BlockedOnResourceLane {
                    witness,
                    advanced: step_advanced,
                } => {
                    advanced |= step_advanced;
                    blocked_lanes.insert(*witness);
                }
                DriveStepStatus::Blocked => {
                    return Ok(SchedulerDriveResult {
                        current_run: current,
                        status: if advanced {
                            SchedulerStatus::Advanced
                        } else {
                            SchedulerStatus::Blocked
                        },
                    });
                }
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerDriveResult {
                        current_run: current,
                        status: SchedulerStatus::PublicOutputProjected,
                    });
                }
            }
        }
    }

    async fn drive_once_with_blocked_lanes<S>(
        &self,
        store: &S,
        current: VerifiedCurrentRun,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<DriveStep>
    where
        S: store::RunJournalStore + ?Sized,
    {
        let (bound_context, decision) = {
            let runtime_spec = current.runtime_spec();
            let lifecycle = current.lifecycle();
            let bound_context = self.runtime_contexts.load_current(&runtime_spec)?;
            bound_context.validate_current_admission_binding(&lifecycle.admission()?)?;
            let decision = TransitionLifecycle::decide(&runtime_spec, &lifecycle, blocked_lanes)?;
            (bound_context, decision)
        };
        match decision {
            TransitionDecision::StartNode(attempt)
            | TransitionDecision::StartRemediation(attempt)
            | TransitionDecision::ResolveSagaTerminal(attempt)
            | TransitionDecision::ContinueAttempt(attempt) => {
                let result = self
                    .run_node_attempt(store, current, &bound_context, attempt)
                    .await?;
                let (current_run, status) = result.into_parts();
                let status = match status {
                    AttemptRunStatus::Advanced => DriveStepStatus::Advanced,
                    AttemptRunStatus::StaleView => DriveStepStatus::StaleView,
                    AttemptRunStatus::BlockedOnResourceLane { witness, advanced } => {
                        DriveStepStatus::BlockedOnResourceLane { witness, advanced }
                    }
                    AttemptRunStatus::OperationalBlock => DriveStepStatus::Blocked,
                };
                Ok(DriveStep {
                    current_run,
                    status,
                })
            }
            TransitionDecision::AwaitManualResolution | TransitionDecision::Blocked => {
                let status = {
                    let runtime_spec = current.runtime_spec();
                    let lifecycle = current.lifecycle();
                    if TransitionLifecycle::public_output_projected(
                        &runtime_spec,
                        &lifecycle,
                        blocked_lanes,
                    )? {
                        DriveStepStatus::PublicOutputProjected
                    } else {
                        DriveStepStatus::Blocked
                    }
                };
                Ok(DriveStep {
                    current_run: current,
                    status,
                })
            }
        }
    }

    async fn run_node_attempt<S>(
        &self,
        store: &S,
        current: VerifiedCurrentRun,
        bound_context: &BoundRuntimeContext,
        attempt: TransitionAttempt,
    ) -> Result<AttemptRunResult>
    where
        S: store::RunJournalStore + ?Sized,
    {
        let run_id = current.view().run_id().clone();
        let recovery = {
            let runtime_spec = current.runtime_spec();
            let lifecycle = current.lifecycle();
            AttemptRecoveryLifecycle::dispatch_open_attempt_for_attempt(
                store,
                &runtime_spec,
                &run_id,
                &lifecycle,
                &attempt,
            )
            .await?
        };
        let current = match recovery {
            RecoveryDispatch::Continue => current,
            RecoveryDispatch::OperationalBlock => {
                return Ok(AttemptRunResult::new(
                    current,
                    AttemptRunStatus::OperationalBlock,
                ));
            }
            RecoveryDispatch::Commit(outcome) => {
                let current = refresh_current_after_commit(store, current, &outcome).await?;
                return Ok(AttemptRunResult::new(current, AttemptRunStatus::Advanced));
            }
            RecoveryDispatch::StaleView => {
                let current = refresh_current_after_stale_append(store, current).await?;
                return Ok(AttemptRunResult::new(current, AttemptRunStatus::StaleView));
            }
        };
        let is_framework = {
            let runtime_spec = current.runtime_spec();
            let node = runtime_spec.node(&attempt.node_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "scheduler selected missing certified node {}",
                    attempt.node_id
                ))
            })?;
            FrameworkAttemptLifecycle::owns_node(node)
        };
        if is_framework {
            return FrameworkAttemptLifecycle::new()
                .run(store, current, bound_context, attempt)
                .await;
        }
        AttemptLifecycle::new()
            .run(store, current, bound_context, attempt)
            .await
    }
}

async fn require_live_execution_claim<S>(
    store: &S,
    execution_scope: &store::ExecutionClaimScope,
    run_id: &RunId,
    token: &store::AdmissionToken,
) -> Result<()>
where
    S: store::ExecutionClaimStore + ?Sized,
{
    match store
        .execution_claim_status(execution_scope)
        .await
        .map_err(async_store_error)?
    {
        store::ExecutionClaimStatus::Live(lease)
            if &lease.token == token && &lease.holder_run_id == run_id =>
        {
            Ok(())
        }
        store::ExecutionClaimStatus::Live(lease) if &lease.token == token => Err(
            RuntimeError::ExecutionClaim("execution claim is held for a different run".to_owned()),
        ),
        store::ExecutionClaimStatus::Live(_) => Err(RuntimeError::ExecutionClaim(
            "execution claim is held by a different token".to_owned(),
        )),
        store::ExecutionClaimStatus::Expired(lease)
            if &lease.token == token && &lease.holder_run_id == run_id =>
        {
            Err(RuntimeError::ExecutionClaim(
                "execution claim is expired".to_owned(),
            ))
        }
        store::ExecutionClaimStatus::Expired(lease) if &lease.token == token => {
            Err(RuntimeError::ExecutionClaim(
                "execution claim is expired for a different run".to_owned(),
            ))
        }
        store::ExecutionClaimStatus::Expired(_) => Err(RuntimeError::ExecutionClaim(
            "execution claim is expired under a different token".to_owned(),
        )),
        store::ExecutionClaimStatus::Unclaimed => Err(RuntimeError::ExecutionClaim(
            "execution claim is unclaimed".to_owned(),
        )),
    }
}
