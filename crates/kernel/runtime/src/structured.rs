//! One-action interpreter over the sole structured RunHistory fold.

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use mfm_canonical::sha256_digest_bytes;
use mfm_certify::structured::{
    AccessTargetSelection, AdmissionVerificationRegistry, EffectPhysicalBindingKind,
    PhysicalBindingKind, QualifiedAccessCompletion, QualifiedComponentIdentity,
    QualifiedPhysicalBinding, QualifiedProcessFault, QualifiedProcessFaultCode,
    QualifiedProgramRegistry, QualifiedStateProposal, QualifiedStateProposalValue,
    QualifiedStateSettlement, ReadPhysicalBindingKind, RuntimeProcessRegistry,
};
use mfm_ids::{AccessAttemptId, AppendRequestId, ContentRef, OccurrenceId, RunId, StableId};
use mfm_journal::structured::{JournalHead, ObservationOutcome, RecordRef, RunRecord};
use mfm_spec::structured::{StructuredComponentKind, StructuredExecutionKind};
use mfm_spec::CanonicalJsonValue;
use mfm_store::structured::{
    AccessAuthorizationProposal, AccessObservationProposal, BackendAppendOutcome,
    NewlyAppendedAuthorization, ObservationCommit, ObservationQualification,
    ProposedCanonicalValue, ProposedObservationOutcome, StateLeaf, StateTransitionProposal,
    StructuredAdmissionRequest, StructuredAppendAttempt, StructuredFrontier,
    StructuredHistoryBackend, StructuredProgramVerifier, StructuredRunHistoryWriter,
    StructuredStoreError, StructuredStoreIdentity, VerifiedProgramData, VerifiedStructuredRun,
};

/// Result returned by structured Runtime preparation and drive operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Closed redaction-safe structured Runtime fault code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFaultCode {
    /// A registry-qualified callback failed to return normally.
    CallbackFault,
    /// Exact canonical typed encoding or decoding failed.
    CodecFault,
    /// A qualified semantic or physical component contract did not match.
    ContractFault,
    /// A state callback rejected already committed normal evidence.
    InvalidEvidence,
    /// A proposed append or fold-derived candidate was rejected before mutation.
    CandidateRejected,
    /// No current qualified public physical binding was available.
    PhysicalBindingUnavailable,
    /// Loaded or resolved durable history failed verification.
    StoreInvalid,
    /// Durable history could not be read, appended, or resolved.
    StoreUnavailable,
}

/// Exact Runtime boundary at which a fault was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFaultPhase {
    /// Invoke a qualified Pure state callback.
    InvokePure,
    /// Author a typed Read or Effect request.
    AuthorRequest,
    /// Select, preflight, or enter a qualified physical binding.
    QualifyAccess,
    /// Settle one already committed normal observation.
    SettleObservation,
    /// Build or callback-free qualify one proposed history candidate.
    QualifyCandidate,
    /// Atomically append one qualified candidate.
    AppendCandidate,
    /// Load and callback-free verify existing history.
    LoadHistory,
    /// Resolve the unchanged identity of an ambiguous append.
    ResolveAppend,
}

/// Closed store disposition retained without backend diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStoreFaultKind {
    /// The requested run does not exist.
    RunNotFound,
    /// Durable history failed structural verification.
    InvalidHistory,
    /// Persisted certification failed qualified verification.
    Certification,
    /// The exact expected append head changed.
    StaleHead,
    /// One append identity was reused for different content.
    AppendConflict,
    /// The durable backend was unavailable.
    BackendUnavailable,
    /// An ambiguous acknowledgement remained unresolved.
    AcknowledgementUnknown,
}

/// Exact qualified authority attributed with one Runtime fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeFaultSubject {
    /// Registry-issued live process component identity.
    Process(Box<QualifiedComponentIdentity>),
    /// Immutable store lineage and authoritative writer epoch.
    Store(StructuredStoreIdentity),
}

/// One contextual, repeatable, redaction-safe structured Runtime fault.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("structured Runtime fault")]
pub struct RuntimeError {
    code: RuntimeFaultCode,
    phase: RuntimeFaultPhase,
    run_id: RunId,
    pre_fault_head: Option<JournalHead>,
    occurrence_id: Option<OccurrenceId>,
    subject: RuntimeFaultSubject,
    store_fault_kind: Option<RuntimeStoreFaultKind>,
}

impl RuntimeError {
    /// Returns the closed Runtime fault code.
    pub const fn code(&self) -> RuntimeFaultCode {
        self.code
    }

    /// Returns the exact Runtime boundary that detected the fault.
    pub const fn phase(&self) -> RuntimeFaultPhase {
        self.phase
    }

    /// Returns the affected run.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the last verified journal head, or `None` before any head was verified.
    pub const fn pre_fault_head(&self) -> Option<&JournalHead> {
        self.pre_fault_head.as_ref()
    }

    /// Returns the affected executable occurrence when one was current.
    pub const fn occurrence_id(&self) -> Option<&OccurrenceId> {
        self.occurrence_id.as_ref()
    }

    /// Returns the exact qualified process or store authority attribution.
    pub const fn subject(&self) -> &RuntimeFaultSubject {
        &self.subject
    }

    /// Returns the closed underlying store disposition, when applicable.
    pub const fn store_fault_kind(&self) -> Option<RuntimeStoreFaultKind> {
        self.store_fault_kind
    }
}

impl From<QualifiedProcessFaultCode> for RuntimeFaultCode {
    fn from(code: QualifiedProcessFaultCode) -> Self {
        match code {
            QualifiedProcessFaultCode::Callback => Self::CallbackFault,
            QualifiedProcessFaultCode::Codec => Self::CodecFault,
            QualifiedProcessFaultCode::Contract => Self::ContractFault,
            QualifiedProcessFaultCode::Candidate => Self::CandidateRejected,
        }
    }
}

/// One bounded `drive_once` disposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveOutcome {
    /// One exact state transition committed, possibly with adjacent root closure.
    TransitionCommitted {
        /// Whether the same atomic append also committed `RunClosed`.
        closed: bool,
    },
    /// One authorization was invoked exactly once and its completion committed.
    AccessObserved,
    /// Another worker changed the cursor before this semantic action committed.
    ConcurrentProgress,
    /// Every currently eligible fan-out lane is waiting on an unmatched Read.
    WaitingReads,
    /// Possible Effect entry blocks progress.
    PossibleEntry,
    /// Committed integrity evidence blocks progress.
    BlockedIntegrity,
    /// The exact root outcome is already durably closed.
    Closed,
}

/// Callback-free adapter from the certifier's qualified admission snapshot to
/// the store's persisted-program verification port.
#[derive(Debug, Clone)]
pub struct StoreProgramVerifier {
    registry: AdmissionVerificationRegistry,
    verified_programs: Arc<Mutex<BTreeMap<ContentRef, Arc<VerifiedProgramData>>>>,
}

impl StoreProgramVerifier {
    /// Wraps the exact callback-free registry half returned by qualified assembly.
    pub fn new(registry: AdmissionVerificationRegistry) -> Self {
        Self {
            registry,
            verified_programs: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

impl StructuredProgramVerifier for StoreProgramVerifier {
    fn verify(
        &self,
        entry_point_id: &StableId,
        root: &mfm_spec::structured::CertifiedProgramRoot,
        authored: &mfm_spec::CanonicalJsonValue,
    ) -> std::result::Result<Arc<VerifiedProgramData>, StructuredStoreError> {
        let program_ref = root
            .content_ref()
            .map_err(|_| StructuredStoreError::Certification)?;
        if let Some(cached) = self
            .verified_programs
            .lock()
            .map_err(|_| StructuredStoreError::Certification)?
            .get(&program_ref)
            .cloned()
        {
            let authored_matches = cached
                .document()
                .component_closure
                .iter()
                .find(|object| {
                    object.content_ref == cached.document().root.components.authored_program_ref
                })
                .is_some_and(|object| &object.value == authored);
            return if &cached.document().root == root
                && cached.expanded().operation_id == *entry_point_id
                && authored_matches
            {
                Ok(cached)
            } else {
                Err(StructuredStoreError::Certification)
            };
        }
        let certified = self
            .registry
            .verify_root(entry_point_id, root, authored)
            .map_err(|_| StructuredStoreError::Certification)?;
        let (document, expanded, value_schemas) = certified.into_verification_parts();
        let verified = Arc::new(VerifiedProgramData::new(document, expanded, value_schemas));
        let mut cache = self
            .verified_programs
            .lock()
            .map_err(|_| StructuredStoreError::Certification)?;
        match cache.get(&program_ref) {
            Some(existing) if existing.document().root == verified.document().root => {
                Ok(Arc::clone(existing))
            }
            Some(_) => Err(StructuredStoreError::Certification),
            None => {
                cache.insert(program_ref, Arc::clone(&verified));
                Ok(verified)
            }
        }
    }
}

/// Consumes one qualified program registry into matching store-verification and
/// Runtime process authorities.
pub fn split_qualified_registry(
    registry: QualifiedProgramRegistry,
) -> (Arc<StoreProgramVerifier>, RuntimeProcessRegistry) {
    let (admission, processes) = registry.into_runtime_parts();
    (Arc::new(StoreProgramVerifier::new(admission)), processes)
}

/// Sole structured Runtime holder of a non-cloneable history writer.
pub struct Runtime<B: StructuredHistoryBackend> {
    writer: StructuredRunHistoryWriter<B>,
    processes: RuntimeProcessRegistry,
}

impl<B: StructuredHistoryBackend> Runtime<B> {
    /// Binds the sole writer to its matching process registry.
    pub fn new(writer: StructuredRunHistoryWriter<B>, processes: RuntimeProcessRegistry) -> Self {
        Self { writer, processes }
    }

    /// Verifies and atomically admits one exact structured run.
    pub async fn admit_run(
        &self,
        request: StructuredAdmissionRequest,
    ) -> Result<StructuredAppendAttempt> {
        let run_id = request.run_id().clone();
        self.writer.admit_run(request).await.map_err(|error| {
            self.store_fault(
                error,
                RuntimeFaultPhase::AppendCandidate,
                run_id,
                None,
                None,
            )
        })
    }

    fn process_fault(
        &self,
        fault: QualifiedProcessFault,
        phase: RuntimeFaultPhase,
        run_id: RunId,
        pre_fault_head: Option<JournalHead>,
        occurrence_id: Option<OccurrenceId>,
    ) -> RuntimeError {
        RuntimeError {
            code: fault.code().into(),
            phase,
            run_id,
            pre_fault_head,
            occurrence_id,
            subject: RuntimeFaultSubject::Process(Box::new(fault.component().clone())),
            store_fault_kind: None,
        }
    }

    fn component_fault(
        &self,
        code: RuntimeFaultCode,
        phase: RuntimeFaultPhase,
        run_id: RunId,
        pre_fault_head: Option<JournalHead>,
        occurrence_id: Option<OccurrenceId>,
        component: QualifiedComponentIdentity,
    ) -> RuntimeError {
        RuntimeError {
            code,
            phase,
            run_id,
            pre_fault_head,
            occurrence_id,
            subject: RuntimeFaultSubject::Process(Box::new(component)),
            store_fault_kind: None,
        }
    }

    fn candidate_fault(
        &self,
        phase: RuntimeFaultPhase,
        run_id: RunId,
        pre_fault_head: Option<JournalHead>,
        occurrence_id: Option<OccurrenceId>,
    ) -> RuntimeError {
        RuntimeError {
            code: RuntimeFaultCode::CandidateRejected,
            phase,
            run_id,
            pre_fault_head,
            occurrence_id,
            subject: RuntimeFaultSubject::Store(self.writer.store_identity().clone()),
            store_fault_kind: None,
        }
    }

    fn proposal_store_fault(
        &self,
        error: StructuredStoreError,
        phase: RuntimeFaultPhase,
        run_id: RunId,
        pre_fault_head: Option<JournalHead>,
        occurrence_id: Option<OccurrenceId>,
        component: QualifiedComponentIdentity,
    ) -> RuntimeError {
        if error == StructuredStoreError::CandidateRejected {
            self.component_fault(
                RuntimeFaultCode::CandidateRejected,
                phase,
                run_id,
                pre_fault_head,
                occurrence_id,
                component,
            )
        } else {
            self.store_fault(error, phase, run_id, pre_fault_head, occurrence_id)
        }
    }

    fn store_fault(
        &self,
        error: StructuredStoreError,
        phase: RuntimeFaultPhase,
        run_id: RunId,
        pre_fault_head: Option<JournalHead>,
        occurrence_id: Option<OccurrenceId>,
    ) -> RuntimeError {
        let store_fault_kind = match error {
            StructuredStoreError::RunNotFound => RuntimeStoreFaultKind::RunNotFound,
            StructuredStoreError::InvalidHistory => RuntimeStoreFaultKind::InvalidHistory,
            StructuredStoreError::CandidateRejected => {
                return self.candidate_fault(phase, run_id, pre_fault_head, occurrence_id);
            }
            StructuredStoreError::Certification => RuntimeStoreFaultKind::Certification,
            StructuredStoreError::StaleHead => RuntimeStoreFaultKind::StaleHead,
            StructuredStoreError::AppendConflict => RuntimeStoreFaultKind::AppendConflict,
            StructuredStoreError::BackendUnavailable => RuntimeStoreFaultKind::BackendUnavailable,
            StructuredStoreError::AcknowledgementUnknown => {
                RuntimeStoreFaultKind::AcknowledgementUnknown
            }
        };
        let code = match store_fault_kind {
            RuntimeStoreFaultKind::BackendUnavailable
            | RuntimeStoreFaultKind::AcknowledgementUnknown => RuntimeFaultCode::StoreUnavailable,
            RuntimeStoreFaultKind::StaleHead | RuntimeStoreFaultKind::AppendConflict => {
                RuntimeFaultCode::CandidateRejected
            }
            _ => RuntimeFaultCode::StoreInvalid,
        };
        RuntimeError {
            code,
            phase,
            run_id,
            pre_fault_head,
            occurrence_id,
            subject: RuntimeFaultSubject::Store(self.writer.store_identity().clone()),
            store_fault_kind: Some(store_fault_kind),
        }
    }

    /// Interprets and performs at most one fold-derived semantic or audited action.
    pub async fn drive_once(&self, run_id: &RunId) -> Result<DriveOutcome> {
        let verified = self.writer.load_verified(run_id).await.map_err(|error| {
            self.store_fault(
                error,
                RuntimeFaultPhase::LoadHistory,
                run_id.clone(),
                None,
                None,
            )
        })?;
        let state = match verified.frontier() {
            StructuredFrontier::Complete => return Ok(DriveOutcome::Closed),
            StructuredFrontier::WaitingReads => return Ok(DriveOutcome::WaitingReads),
            StructuredFrontier::PossibleEntry => return Ok(DriveOutcome::PossibleEntry),
            StructuredFrontier::BlockedIntegrity => return Ok(DriveOutcome::BlockedIntegrity),
            StructuredFrontier::Actions(actions) => actions
                .iter()
                .min_by(|left, right| left.occurrence_path.cmp(&right.occurrence_path))
                .cloned()
                .ok_or_else(|| {
                    self.candidate_fault(
                        RuntimeFaultPhase::QualifyCandidate,
                        run_id.clone(),
                        Some(verified.journal_head().clone()),
                        None,
                    )
                })?,
        };
        let occurrence_id = state.occurrence_id.clone();
        let pre_fault_head = verified.journal_head().clone();
        let input = canonical_object(&verified, &state.input.value.value_ref).map_err(|()| {
            self.candidate_fault(
                RuntimeFaultPhase::QualifyCandidate,
                run_id.clone(),
                Some(pre_fault_head.clone()),
                Some(occurrence_id.clone()),
            )
        })?;
        let state_identity = self
            .processes
            .component_identity(StructuredComponentKind::State, &state.state_contract_ref)
            .ok_or_else(|| {
                self.candidate_fault(
                    RuntimeFaultPhase::QualifyCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                )
            })?;
        match (&state.execution_kind, &state.leaf) {
            (StructuredExecutionKind::Pure, StateLeaf::Ready) => {
                let proposal = self
                    .processes
                    .invoke_pure(&state_identity, &input)
                    .map_err(|fault| {
                        self.process_fault(
                            fault,
                            RuntimeFaultPhase::InvokePure,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        )
                    })?;
                self.commit_state_proposal(verified, state, proposal).await
            }
            (
                StructuredExecutionKind::Read | StructuredExecutionKind::Effect,
                StateLeaf::ObservedForSettlement {
                    access_attempt_id, ..
                },
            ) => {
                let observation =
                    committed_observation(&verified, access_attempt_id).map_err(|()| {
                        self.candidate_fault(
                            RuntimeFaultPhase::QualifyCandidate,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        )
                    })?;
                match self
                    .processes
                    .settle_observation(&state_identity, &input, &observation)
                    .map_err(|fault| {
                        self.process_fault(
                            fault,
                            RuntimeFaultPhase::SettleObservation,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        )
                    })? {
                    QualifiedStateSettlement::Proposed(proposal) => {
                        self.commit_state_proposal(verified, state, proposal).await
                    }
                    QualifiedStateSettlement::InvalidEvidence(component) => Err(self
                        .component_fault(
                            RuntimeFaultCode::InvalidEvidence,
                            RuntimeFaultPhase::SettleObservation,
                            run_id.clone(),
                            Some(pre_fault_head),
                            Some(occurrence_id),
                            component,
                        )),
                }
            }
            (StructuredExecutionKind::Read, StateLeaf::Ready) => {
                self.drive_access::<ReadPhysicalBindingKind>(verified, state, state_identity, input)
                    .await
            }
            (StructuredExecutionKind::Effect, StateLeaf::Ready | StateLeaf::Refreshable { .. }) => {
                self.drive_access::<EffectPhysicalBindingKind>(
                    verified,
                    state,
                    state_identity,
                    input,
                )
                .await
            }
            _ => Err(self.candidate_fault(
                RuntimeFaultPhase::QualifyCandidate,
                run_id.clone(),
                Some(pre_fault_head),
                Some(occurrence_id),
            )),
        }
    }

    async fn commit_state_proposal(
        &self,
        mut verified: VerifiedStructuredRun,
        state: mfm_store::structured::ActionableState,
        proposal: QualifiedStateProposal,
    ) -> Result<DriveOutcome> {
        let run_id = verified.run_id().clone();
        let semantic_head = verified.semantic_head().clone();
        let occurrence_id = state.occurrence_id.clone();
        let proposal_origin = proposal.origin().clone();
        loop {
            let pre_fault_head = verified.journal_head().clone();
            let append_id = append_id(
                "transition",
                &run_id,
                verified.journal_head(),
                state.occurrence_id.as_str(),
            )
            .map_err(|()| {
                self.candidate_fault(
                    RuntimeFaultPhase::QualifyCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                )
            })?;
            let transition =
                state_transition_proposal(append_id, proposal.clone()).map_err(|()| {
                    self.component_fault(
                        RuntimeFaultCode::CodecFault,
                        RuntimeFaultPhase::QualifyCandidate,
                        run_id.clone(),
                        Some(pre_fault_head.clone()),
                        Some(occurrence_id.clone()),
                        proposal_origin.clone(),
                    )
                })?;
            let mut attempt = self
                .writer
                .commit_state_transition(verified, &transition)
                .await
                .map_err(|error| {
                    self.proposal_store_fault(
                        error,
                        RuntimeFaultPhase::AppendCandidate,
                        run_id.clone(),
                        Some(pre_fault_head.clone()),
                        Some(occurrence_id.clone()),
                        proposal_origin.clone(),
                    )
                })?;
            match attempt.outcome() {
                BackendAppendOutcome::NewlyCommitted(batch)
                | BackendAppendOutcome::ExistingSame(batch) => {
                    let closed = batch
                        .records
                        .iter()
                        .any(|record| matches!(&record.record, RunRecord::RunClosed(_)));
                    let _successor = attempt.into_committed_successor().ok_or_else(|| {
                        self.candidate_fault(
                            RuntimeFaultPhase::QualifyCandidate,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        )
                    })?;
                    return Ok(DriveOutcome::TransitionCommitted { closed });
                }
                BackendAppendOutcome::AcknowledgementUnknown => {
                    if self
                        .writer
                        .resolve_attempt(&mut attempt)
                        .await
                        .map_err(|error| {
                            self.store_fault(
                                error,
                                RuntimeFaultPhase::ResolveAppend,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            )
                        })?
                    {
                        let batch = attempt.committed().ok_or_else(|| {
                            self.candidate_fault(
                                RuntimeFaultPhase::QualifyCandidate,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            )
                        })?;
                        let closed = batch
                            .records
                            .iter()
                            .any(|record| matches!(&record.record, RunRecord::RunClosed(_)));
                        let _successor = attempt.into_committed_successor().ok_or_else(|| {
                            self.candidate_fault(
                                RuntimeFaultPhase::QualifyCandidate,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            )
                        })?;
                        return Ok(DriveOutcome::TransitionCommitted { closed });
                    }
                }
                BackendAppendOutcome::StaleHead => {}
            }
            let current = self.writer.load_verified(&run_id).await.map_err(|error| {
                self.store_fault(
                    error,
                    RuntimeFaultPhase::LoadHistory,
                    run_id.clone(),
                    Some(pre_fault_head),
                    Some(occurrence_id.clone()),
                )
            })?;
            if current.semantic_head() != &semantic_head
                || minimum_action(current.frontier()) != Some(&state)
            {
                return Ok(DriveOutcome::ConcurrentProgress);
            }
            verified = current;
        }
    }

    async fn drive_access<K: AccessMarker>(
        &self,
        verified: VerifiedStructuredRun,
        state: mfm_store::structured::ActionableState,
        state_identity: QualifiedComponentIdentity,
        input: CanonicalJsonValue,
    ) -> Result<DriveOutcome> {
        let run_id = verified.run_id().clone();
        let pre_fault_head = verified.journal_head().clone();
        let occurrence_id = state.occurrence_id.clone();
        let capability_contract_ref = state.capability_contract_ref.as_ref().ok_or_else(|| {
            self.candidate_fault(
                RuntimeFaultPhase::QualifyCandidate,
                run_id.clone(),
                Some(pre_fault_head.clone()),
                Some(occurrence_id.clone()),
            )
        })?;
        let request = self
            .processes
            .author_request(&state_identity, &input)
            .map_err(|fault| {
                self.process_fault(
                    fault,
                    RuntimeFaultPhase::AuthorRequest,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                )
            })?;
        let capability_identity = self
            .processes
            .component_identity(StructuredComponentKind::Capability, capability_contract_ref)
            .ok_or_else(|| {
                self.candidate_fault(
                    RuntimeFaultPhase::QualifyCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                )
            })?;
        let adapter_identity = self
            .processes
            .access_adapter_identity(&capability_identity)
            .map_err(|fault| {
                self.process_fault(
                    fault,
                    RuntimeFaultPhase::QualifyAccess,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                )
            })?;
        let minimum_lineage_head_ref = match &state.leaf {
            StateLeaf::Refreshable {
                public_lineage_head_ref,
                ..
            } => Some(public_lineage_head_ref),
            _ => None,
        };
        let target = AccessTargetSelection {
            run_id: verified.run_id(),
            occurrence_id: &state.occurrence_id,
            state_input_ref: &state.input,
            store_scope_id: &verified.admission().store_scope_id,
            store_epoch: verified.admission().store_epoch,
            tenant_scope_id: &verified.admission().tenant_scope_id,
            admitted_prior_run_source_manifest_ref: &verified
                .admission()
                .admission_material_refs
                .prior_run_source_manifest_ref,
            admitted_routing_policy_ref: &verified
                .admission()
                .admission_material_refs
                .routing_policy_ref,
            stable_resource_lineage_contract_ref: state
                .stable_resource_lineage_contract_ref
                .as_ref(),
            minimum_lineage_head_ref,
        };
        let binding = match AssertUnwindSafe(self.processes.prepare_access::<K>(
            &capability_identity,
            target,
            request,
        ))
        .catch_unwind()
        .await
        {
            Ok(Ok(Some(binding))) => binding,
            Ok(Ok(None)) => {
                return Err(self.component_fault(
                    RuntimeFaultCode::PhysicalBindingUnavailable,
                    RuntimeFaultPhase::QualifyAccess,
                    run_id,
                    Some(pre_fault_head),
                    Some(occurrence_id),
                    adapter_identity,
                ));
            }
            Ok(Err(fault)) => {
                return Err(self.process_fault(
                    fault,
                    RuntimeFaultPhase::QualifyAccess,
                    run_id,
                    Some(pre_fault_head),
                    Some(occurrence_id),
                ));
            }
            Err(_) => {
                return Err(self.component_fault(
                    RuntimeFaultCode::CallbackFault,
                    RuntimeFaultPhase::QualifyAccess,
                    run_id,
                    Some(pre_fault_head),
                    Some(occurrence_id),
                    adapter_identity,
                ));
            }
        };
        let prepared = Prepared::<K> {
            state,
            binding,
            verified,
            request_origin: state_identity,
            adapter_origin: adapter_identity,
        };
        match self.authorize(prepared).await? {
            None => Ok(DriveOutcome::ConcurrentProgress),
            Some(authorized) => {
                let invoked = self.invoke_authorized(authorized).await?;
                match self.qualify_invoked_observation(invoked).await? {
                    Some(pending) => self.commit_pending_observation(pending).await,
                    None => Ok(DriveOutcome::AccessObserved),
                }
            }
        }
    }

    async fn authorize<K: AccessMarker>(
        &self,
        prepared: Prepared<K>,
    ) -> Result<Option<Authorized<K>>> {
        let Prepared {
            state,
            binding,
            mut verified,
            request_origin,
            adapter_origin,
        } = prepared;
        let run_id = verified.run_id().clone();
        let semantic_head = verified.semantic_head().clone();
        let occurrence_id = state.occurrence_id.clone();
        loop {
            let pre_fault_head = verified.journal_head().clone();
            let append_id = append_id(
                "authorize",
                &run_id,
                verified.journal_head(),
                state.occurrence_id.as_str(),
            )
            .map_err(|()| {
                self.candidate_fault(
                    RuntimeFaultPhase::QualifyCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                )
            })?;
            let proposal = AccessAuthorizationProposal::new(
                append_id,
                state.input.clone(),
                proposed(binding.request()).map_err(|()| {
                    self.component_fault(
                        RuntimeFaultCode::CodecFault,
                        RuntimeFaultPhase::QualifyCandidate,
                        run_id.clone(),
                        Some(pre_fault_head.clone()),
                        Some(occurrence_id.clone()),
                        request_origin.clone(),
                    )
                })?,
                binding.public_certificate().clone(),
            );
            let mut attempt = self
                .writer
                .authorize_access(verified, &proposal)
                .await
                .map_err(|error| {
                    self.proposal_store_fault(
                        error,
                        RuntimeFaultPhase::AppendCandidate,
                        run_id.clone(),
                        Some(pre_fault_head.clone()),
                        Some(occurrence_id.clone()),
                        adapter_origin.clone(),
                    )
                })?;
            match attempt.outcome() {
                BackendAppendOutcome::NewlyCommitted(_) => {
                    let (authorization, verified) =
                        attempt.into_newly_appended_authorization().ok_or_else(|| {
                            self.candidate_fault(
                                RuntimeFaultPhase::QualifyCandidate,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            )
                        })?;
                    return Ok(Some(Authorized {
                        binding,
                        authorization,
                        verified,
                        adapter_origin,
                    }));
                }
                BackendAppendOutcome::ExistingSame(_) => {
                    let _successor = attempt.into_committed_successor().ok_or_else(|| {
                        self.candidate_fault(
                            RuntimeFaultPhase::QualifyCandidate,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        )
                    })?;
                    return Ok(None);
                }
                BackendAppendOutcome::AcknowledgementUnknown => {
                    if self
                        .writer
                        .resolve_attempt(&mut attempt)
                        .await
                        .map_err(|error| {
                            self.store_fault(
                                error,
                                RuntimeFaultPhase::ResolveAppend,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            )
                        })?
                    {
                        let _successor = attempt.into_committed_successor().ok_or_else(|| {
                            self.candidate_fault(
                                RuntimeFaultPhase::QualifyCandidate,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            )
                        })?;
                        return Ok(None);
                    }
                }
                BackendAppendOutcome::StaleHead => {}
            }
            let current = self.writer.load_verified(&run_id).await.map_err(|error| {
                self.store_fault(
                    error,
                    RuntimeFaultPhase::LoadHistory,
                    run_id.clone(),
                    Some(pre_fault_head),
                    Some(occurrence_id.clone()),
                )
            })?;
            if current.semantic_head() != &semantic_head
                || minimum_action(current.frontier()) != Some(&state)
            {
                return Ok(None);
            }
            verified = current;
        }
    }

    async fn invoke_authorized<K: AccessMarker>(
        &self,
        authorized: Authorized<K>,
    ) -> Result<InvokedObservation<K>> {
        let Authorized {
            binding,
            authorization,
            verified,
            adapter_origin,
        } = authorized;
        let authorization_ref = authorization.authorization_ref().clone();
        let access_attempt_id = authorization.access_attempt_id().clone();
        let occurrence_id = authorization.authorization().occurrence_id.clone();
        let run_id = verified.run_id().clone();
        let pre_fault_head = verified.journal_head().clone();
        let completion = AssertUnwindSafe(
            self.processes
                .invoke_qualified_physical_binding(binding, authorization),
        )
        .catch_unwind()
        .await
        .map_err(|_| {
            self.component_fault(
                RuntimeFaultCode::CallbackFault,
                RuntimeFaultPhase::QualifyAccess,
                run_id.clone(),
                Some(pre_fault_head.clone()),
                Some(occurrence_id.clone()),
                adapter_origin.clone(),
            )
        })?;
        let outcome = self.observation_outcome::<K>(completion).map_err(|code| {
            self.component_fault(
                code,
                RuntimeFaultPhase::QualifyCandidate,
                run_id.clone(),
                Some(pre_fault_head),
                Some(occurrence_id),
                adapter_origin.clone(),
            )
        })?;
        Ok(InvokedObservation {
            run_id,
            authorization_ref,
            access_attempt_id,
            outcome,
            verified,
            adapter_origin,
            _kind: PhantomData,
        })
    }

    async fn qualify_invoked_observation<K: AccessMarker>(
        &self,
        invoked: InvokedObservation<K>,
    ) -> Result<Option<PendingObservation<K>>> {
        let InvokedObservation {
            run_id,
            authorization_ref,
            access_attempt_id,
            outcome,
            verified,
            adapter_origin,
            ..
        } = invoked;
        let pre_fault_head = verified.journal_head().clone();
        let occurrence_id = verified
            .authorization(&access_attempt_id)
            .map(|(_, authorization)| authorization.occurrence_id.clone())
            .ok_or_else(|| {
                self.candidate_fault(
                    RuntimeFaultPhase::QualifyCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    None,
                )
            })?;
        match self
            .writer
            .qualify_observation(&verified, &authorization_ref, &outcome)
            .await
            .map_err(|error| {
                self.proposal_store_fault(
                    error,
                    RuntimeFaultPhase::QualifyCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                    adapter_origin.clone(),
                )
            })? {
            ObservationQualification::Ready => Ok(Some(PendingObservation {
                run_id,
                authorization_ref,
                access_attempt_id,
                outcome,
                verified,
                adapter_origin,
                _kind: PhantomData,
            })),
            ObservationQualification::ExistingSame => {
                let _committed = CommittedObservation::<K> {
                    _access_attempt_id: access_attempt_id,
                    _kind: PhantomData,
                };
                Ok(None)
            }
            ObservationQualification::InvalidSupersessionEvidence => Err(self.component_fault(
                RuntimeFaultCode::CandidateRejected,
                RuntimeFaultPhase::QualifyCandidate,
                run_id,
                Some(pre_fault_head),
                Some(occurrence_id),
                adapter_origin,
            )),
        }
    }

    fn observation_outcome<K: AccessMarker>(
        &self,
        completion: QualifiedAccessCompletion,
    ) -> std::result::Result<ProposedObservationOutcome, RuntimeFaultCode> {
        match completion {
            QualifiedAccessCompletion::Returned(value) => proposed(&value)
                .map(ProposedObservationOutcome::Returned)
                .map_err(|()| RuntimeFaultCode::CodecFault),
            QualifiedAccessCompletion::SafeFailure(value) => proposed(&value)
                .map(ProposedObservationOutcome::SafeFailure)
                .map_err(|()| RuntimeFaultCode::CodecFault),
            QualifiedAccessCompletion::SupersededBeforeEntry {
                evidence,
                public_lineage_head,
            } if K::EFFECT => proposed(&evidence)
                .map(
                    |evidence| ProposedObservationOutcome::SupersededBeforeEntry {
                        public_lineage_head,
                        evidence,
                    },
                )
                .map_err(|()| RuntimeFaultCode::CodecFault),
            QualifiedAccessCompletion::EntryUnknown(code) if K::EFFECT => {
                Ok(ProposedObservationOutcome::EntryUnknown { fault_code: code })
            }
            QualifiedAccessCompletion::IntegrityFault(code) => {
                Ok(ProposedObservationOutcome::IntegrityFault { fault_code: code })
            }
            QualifiedAccessCompletion::SupersededBeforeEntry { .. }
            | QualifiedAccessCompletion::EntryUnknown(_) => Err(RuntimeFaultCode::ContractFault),
        }
    }

    async fn commit_pending_observation<K: AccessMarker>(
        &self,
        pending: PendingObservation<K>,
    ) -> Result<DriveOutcome> {
        let PendingObservation {
            run_id,
            authorization_ref,
            access_attempt_id,
            outcome,
            mut verified,
            adapter_origin,
            ..
        } = pending;
        let mut backoff = ObservationRetryBackoff::new();
        let mut last_verified_head = Some(verified.journal_head().clone());
        loop {
            let pre_fault_head = verified.journal_head().clone();
            let occurrence_id = verified
                .authorization(&access_attempt_id)
                .map(|(_, authorization)| authorization.occurrence_id.clone())
                .ok_or_else(|| {
                    self.candidate_fault(
                        RuntimeFaultPhase::QualifyCandidate,
                        run_id.clone(),
                        Some(pre_fault_head.clone()),
                        None,
                    )
                })?;
            let append_id = append_id(
                "observe",
                &run_id,
                verified.journal_head(),
                access_attempt_id.as_str(),
            )
            .map_err(|()| {
                self.candidate_fault(
                    RuntimeFaultPhase::QualifyCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                )
            })?;
            let proposal = AccessObservationProposal::new(
                append_id,
                authorization_ref.clone(),
                outcome.clone(),
            );
            let commit = match self.writer.commit_observation(verified, &proposal).await {
                Ok(commit) => commit,
                Err(StructuredStoreError::BackendUnavailable) => {
                    backoff.wait().await;
                    verified = loop {
                        match self.writer.load_verified(&run_id).await {
                            Ok(current) => break current,
                            Err(StructuredStoreError::BackendUnavailable) => {
                                backoff.wait().await;
                            }
                            Err(error) => {
                                return Err(self.store_fault(
                                    error,
                                    RuntimeFaultPhase::LoadHistory,
                                    run_id.clone(),
                                    Some(pre_fault_head.clone()),
                                    Some(occurrence_id.clone()),
                                ));
                            }
                        }
                    };
                    continue;
                }
                Err(error) => {
                    return Err(self.proposal_store_fault(
                        error,
                        RuntimeFaultPhase::AppendCandidate,
                        run_id.clone(),
                        Some(pre_fault_head.clone()),
                        Some(occurrence_id.clone()),
                        adapter_origin.clone(),
                    ));
                }
            };
            let mut attempt = match commit {
                ObservationCommit::ExistingSame(_verified) => {
                    let _committed = CommittedObservation::<K> {
                        _access_attempt_id: access_attempt_id,
                        _kind: PhantomData,
                    };
                    return Ok(DriveOutcome::AccessObserved);
                }
                ObservationCommit::Attempt(attempt) => attempt,
            };
            match attempt.outcome() {
                BackendAppendOutcome::NewlyCommitted(_) | BackendAppendOutcome::ExistingSame(_) => {
                    let _successor = attempt.into_committed_successor().ok_or_else(|| {
                        self.candidate_fault(
                            RuntimeFaultPhase::QualifyCandidate,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        )
                    })?;
                    let _committed = CommittedObservation::<K> {
                        _access_attempt_id: access_attempt_id,
                        _kind: PhantomData,
                    };
                    return Ok(DriveOutcome::AccessObserved);
                }
                BackendAppendOutcome::AcknowledgementUnknown => loop {
                    match self.writer.resolve_attempt(&mut attempt).await {
                        Ok(true) => {
                            let _successor =
                                attempt.into_committed_successor().ok_or_else(|| {
                                    self.candidate_fault(
                                        RuntimeFaultPhase::QualifyCandidate,
                                        run_id.clone(),
                                        Some(pre_fault_head.clone()),
                                        Some(occurrence_id.clone()),
                                    )
                                })?;
                            let _committed = CommittedObservation::<K> {
                                _access_attempt_id: access_attempt_id,
                                _kind: PhantomData,
                            };
                            return Ok(DriveOutcome::AccessObserved);
                        }
                        Ok(false) => break,
                        Err(StructuredStoreError::BackendUnavailable) => {
                            backoff.wait().await;
                        }
                        Err(error) => {
                            return Err(self.store_fault(
                                error,
                                RuntimeFaultPhase::ResolveAppend,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            ));
                        }
                    }
                },
                BackendAppendOutcome::StaleHead => {}
            }
            verified = loop {
                match self.writer.load_verified(&run_id).await {
                    Ok(current) => break current,
                    Err(StructuredStoreError::BackendUnavailable) => {
                        backoff.wait().await;
                    }
                    Err(error) => {
                        return Err(self.store_fault(
                            error,
                            RuntimeFaultPhase::LoadHistory,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        ));
                    }
                }
            };
            if last_verified_head.as_ref() != Some(verified.journal_head()) {
                backoff.reset();
            }
            last_verified_head = Some(verified.journal_head().clone());
        }
    }
}

trait AccessMarker: PhysicalBindingKind {}

impl AccessMarker for ReadPhysicalBindingKind {}
impl AccessMarker for EffectPhysicalBindingKind {}

struct Prepared<K: AccessMarker> {
    state: mfm_store::structured::ActionableState,
    binding: QualifiedPhysicalBinding<K>,
    verified: VerifiedStructuredRun,
    request_origin: QualifiedComponentIdentity,
    adapter_origin: QualifiedComponentIdentity,
}

struct Authorized<K: AccessMarker> {
    binding: QualifiedPhysicalBinding<K>,
    authorization: NewlyAppendedAuthorization,
    verified: VerifiedStructuredRun,
    adapter_origin: QualifiedComponentIdentity,
}

struct InvokedObservation<K: AccessMarker> {
    run_id: RunId,
    authorization_ref: RecordRef,
    access_attempt_id: AccessAttemptId,
    outcome: ProposedObservationOutcome,
    verified: VerifiedStructuredRun,
    adapter_origin: QualifiedComponentIdentity,
    _kind: PhantomData<fn() -> K>,
}

struct PendingObservation<K: AccessMarker> {
    run_id: RunId,
    authorization_ref: RecordRef,
    access_attempt_id: AccessAttemptId,
    outcome: ProposedObservationOutcome,
    verified: VerifiedStructuredRun,
    adapter_origin: QualifiedComponentIdentity,
    _kind: PhantomData<fn() -> K>,
}

struct CommittedObservation<K: AccessMarker> {
    _access_attempt_id: AccessAttemptId,
    _kind: PhantomData<fn() -> K>,
}

struct ObservationRetryBackoff {
    next_delay_ms: u64,
}

impl ObservationRetryBackoff {
    const fn new() -> Self {
        Self { next_delay_ms: 10 }
    }

    const fn reset(&mut self) {
        self.next_delay_ms = 10;
    }

    async fn wait(&mut self) {
        tokio::time::sleep(std::time::Duration::from_millis(self.next_delay_ms)).await;
        self.next_delay_ms = self.next_delay_ms.saturating_mul(2).min(1_000);
    }
}

fn state_transition_proposal(
    append_request_id: AppendRequestId,
    proposal: QualifiedStateProposal,
) -> std::result::Result<StateTransitionProposal, ()> {
    let (_, value) = proposal.into_parts();
    match value {
        QualifiedStateProposalValue::Success { value, facts } => Ok(
            StateTransitionProposal::success(append_request_id, proposed(&value)?, facts),
        ),
        QualifiedStateProposalValue::Failure(value) => Ok(StateTransitionProposal::failure(
            append_request_id,
            proposed(&value)?,
        )),
    }
}

fn proposed(value: &CanonicalJsonValue) -> std::result::Result<ProposedCanonicalValue, ()> {
    let canonical = value.canonical_json().map_err(|_| ())?;
    ProposedCanonicalValue::from_json(canonical.as_str()).map_err(|_| ())
}

fn canonical_object(
    verified: &VerifiedStructuredRun,
    content_ref: &ContentRef,
) -> std::result::Result<CanonicalJsonValue, ()> {
    let object = verified.object(content_ref).ok_or(())?;
    CanonicalJsonValue::from_canonical_json(object.canonical_json.as_bytes()).map_err(|_| ())
}

fn committed_observation(
    verified: &VerifiedStructuredRun,
    access_attempt_id: &AccessAttemptId,
) -> std::result::Result<CanonicalJsonValue, ()> {
    let (_, observation) = verified.observation(access_attempt_id).ok_or(())?;
    let (kind, value_ref) = match &observation.outcome {
        ObservationOutcome::Returned { value } => ("returned", &value.value_ref),
        ObservationOutcome::SafeFailure { value } => ("safe_failure", &value.value_ref),
        ObservationOutcome::SupersededBeforeEntry { .. }
        | ObservationOutcome::EntryUnknown { .. }
        | ObservationOutcome::IntegrityFault { .. } => {
            return Err(());
        }
    };
    let value = canonical_object(verified, value_ref)?;
    CanonicalJsonValue::new(serde_json::json!({
        "kind": kind,
        "value": value.as_json(),
    }))
    .map_err(|_| ())
}

fn minimum_action(
    frontier: &StructuredFrontier,
) -> Option<&mfm_store::structured::ActionableState> {
    let StructuredFrontier::Actions(actions) = frontier else {
        return None;
    };
    actions
        .iter()
        .min_by(|left, right| left.occurrence_path.cmp(&right.occurrence_path))
}

fn append_id(
    phase: &str,
    run_id: &RunId,
    head: &JournalHead,
    action_identity: &str,
) -> std::result::Result<AppendRequestId, ()> {
    let preimage = serde_json::json!({
        "action_identity": action_identity,
        "head": head,
        "phase": phase,
        "run_id": run_id,
    });
    let canonical = mfm_journal::structured::canonical_json(&preimage).map_err(|_| ())?;
    let digest = sha256_digest_bytes(canonical.as_bytes());
    AppendRequestId::new(format!("structured-runtime.{phase}.{digest}")).map_err(|_| ())
}
