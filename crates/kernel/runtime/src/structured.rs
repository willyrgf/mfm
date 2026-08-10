//! One-action interpreter over the sole structured RunHistory reducer.

use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;
use mfm_canonical::sha256_digest_bytes;
#[cfg(feature = "store-authority")]
use mfm_certify::structured::RuntimeAssemblyToken;
use mfm_certify::structured::{
    AccessTargetSelection, CertifiedAccessAuthorization, CertifiedProcessRegistry,
    EffectPhysicalBindingKind, PhysicalBindingKind, QualifiedAccessCompletion,
    QualifiedComponentIdentity, QualifiedPhysicalBinding, QualifiedProcessFault,
    QualifiedProcessFaultCode, QualifiedStateProposal, QualifiedStateProposalValue,
    QualifiedStateSettlement, ReadPhysicalBindingKind,
};
use mfm_ids::{AccessAttemptId, AppendRequestId, ContentRef, OccurrenceId, RunId};
use mfm_journal::structured::{JournalHead, ObservationOutcome, RecordRef};
use mfm_spec::structured::{StructuredComponentKind, StructuredExecutionKind};
use mfm_spec::CanonicalJsonValue;
use mfm_values::CanonicalJsonPersistedSchema;

/// Runtime-owned holder for the complete qualified callback registry.
/// Certification only supplies the deterministic qualified component set;
/// Runtime is the sole owner that can prepare and consume live bindings.
pub struct RuntimeProcessRegistry {
    inner: CertifiedProcessRegistry,
}

/// Assembly failed because process and seal identities did not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("runtime assembly seal does not match the qualified process registry")]
pub struct RuntimeAssemblyError;

impl std::fmt::Debug for RuntimeProcessRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeProcessRegistry")
            .finish_non_exhaustive()
    }
}

impl RuntimeProcessRegistry {
    /// Moves the complete certified registry into Runtime ownership.
    #[doc(hidden)]
    #[cfg(feature = "store-authority")]
    pub fn from_certified<C: mfm_authority_seal::RuntimeAssemblyConsumerSeal>(
        inner: CertifiedProcessRegistry,
        token: &RuntimeAssemblyToken,
        _consumer: C,
    ) -> std::result::Result<Self, RuntimeAssemblyError> {
        if !inner.matches_assembly_token(token) {
            return Err(RuntimeAssemblyError);
        }
        Ok(Self { inner })
    }

    #[cfg(feature = "store-authority")]
    fn matches_assembly_token(&self, token: &RuntimeAssemblyToken) -> bool {
        self.inner.matches_assembly_token(token)
    }

    fn component_identity(
        &self,
        kind: StructuredComponentKind,
        semantic_contract_ref: &ContentRef,
    ) -> Option<QualifiedComponentIdentity> {
        self.inner.component_identity(kind, semantic_contract_ref)
    }

    fn invoke_pure(
        &self,
        state: &QualifiedComponentIdentity,
        input: &CanonicalJsonValue,
    ) -> std::result::Result<QualifiedStateProposal, QualifiedProcessFault> {
        self.inner.invoke_pure(state, input)
    }

    fn author_request(
        &self,
        state: &QualifiedComponentIdentity,
        input: &CanonicalJsonValue,
    ) -> std::result::Result<CanonicalJsonValue, QualifiedProcessFault> {
        self.inner.author_request(state, input)
    }

    fn settle_returned(
        &self,
        state: &QualifiedComponentIdentity,
        input: &CanonicalJsonValue,
        returned: &CanonicalJsonValue,
    ) -> std::result::Result<QualifiedStateSettlement, QualifiedProcessFault> {
        self.inner.settle_returned(state, input, returned)
    }

    fn settle_safe_failure(
        &self,
        state: &QualifiedComponentIdentity,
        input: &CanonicalJsonValue,
        safe_failure: &CanonicalJsonValue,
    ) -> std::result::Result<QualifiedStateSettlement, QualifiedProcessFault> {
        self.inner.settle_safe_failure(state, input, safe_failure)
    }

    fn access_adapter_identity(
        &self,
        capability: &QualifiedComponentIdentity,
    ) -> std::result::Result<QualifiedComponentIdentity, QualifiedProcessFault> {
        self.inner.access_adapter_identity(capability)
    }

    async fn qualify_access<K: PhysicalBindingKind>(
        &self,
        capability_identity: &QualifiedComponentIdentity,
        target: AccessTargetSelection<'_>,
        request: CanonicalJsonValue,
    ) -> std::result::Result<Option<QualifiedPhysicalBinding<K>>, QualifiedProcessFault> {
        self.inner
            .qualify_access::<K>(capability_identity, target, request)
            .await
    }

    async fn invoke_certified_binding<K>(
        &self,
        binding: QualifiedPhysicalBinding<K>,
        authorization: CertifiedAccessAuthorization,
    ) -> QualifiedAccessCompletion {
        self.inner
            .invoke_certified_binding(binding, authorization)
            .await
    }
}

use crate::history::{
    AccessAuthorizationProposal, AccessObservationProposal, ActionableState,
    CommittedAccessAuthorization, EffectEntrySubject, HistoryAppendOutcome, HistoryError,
    ProposedCanonicalValue, ProposedObservationOutcome, QualifiedRuntimeIntent, RuntimeHistoryPort,
    StateLeaf, StateTransitionProposal, StructuredAdmissionCommand, StructuredAppendAttempt,
    StructuredFrontier, StructuredStoreIdentity, VerifiedRunView,
};

/// Kernel-owned fault code for the closing observation of a crashed attempt.
///
/// This is the only thing distinguishing a Runtime-synthesized closure from an
/// adapter-reported ambiguity anywhere downstream: the two records are
/// shape-identical and carry no origin. It is reserved, unreachable from any
/// adapter, and must never be reused by a domain.
pub const INVOKER_AUTHORITY_LOST: &str = "mfm.kernel/invoker-authority-lost";

fn invoker_authority_lost_fault_code() -> mfm_ids::StableId {
    mfm_ids::StableId::new(INVOKER_AUTHORITY_LOST)
        .expect("the kernel invoker-authority-lost code is a valid stable id")
}

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
    /// A proposed append or reducer-derived candidate was rejected before mutation.
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
}

/// Closed store disposition retained without backend diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStoreFaultKind {
    /// The requested run does not exist.
    RunNotFound,
    /// Durable history failed structural verification.
    InvalidHistory,
    /// Valid retained history exceeded the fixed run-capacity contract.
    CapacityExceeded,
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
    /// One committed completion. Either an authorization was invoked exactly
    /// once and its completion committed, or a crashed absorbing Effect attempt
    /// was closed without reaching any adapter.
    AccessObserved,
    /// Another worker changed the cursor before this semantic action committed.
    ConcurrentProgress,
    /// Possible Effect entry of one exact occurrence blocks progress.
    PossibleEntry(Box<EffectEntrySubject>),
    /// Committed integrity evidence blocks progress.
    BlockedIntegrity,
    /// The exact root outcome is already durably closed.
    Closed,
}

/// Sole structured Runtime holder of history-port mutation authority.
pub struct Runtime<P: RuntimeHistoryPort> {
    history: P,
    processes: RuntimeProcessRegistry,
}

impl<P: RuntimeHistoryPort> Runtime<P> {
    /// Binds one history port to its matching process registry.
    ///
    /// Production assembly constructs both halves from one complete qualified
    /// registry and a private store adapter. Callers cannot extract the port.
    /// Constructs a Runtime from the one paired production assembly bundle.
    ///
    /// Callers should obtain the history and registry only from store assembly;
    /// the constructor intentionally does not accept independently split ports.
    #[doc(hidden)]
    #[cfg(feature = "store-authority")]
    pub fn from_assembled<C: mfm_authority_seal::RuntimeAssemblyConsumerSeal>(
        history: P,
        processes: RuntimeProcessRegistry,
        token: RuntimeAssemblyToken,
        _consumer: C,
    ) -> std::result::Result<Self, RuntimeAssemblyError> {
        if !processes.matches_assembly_token(&token) {
            return Err(RuntimeAssemblyError);
        }
        Ok(Self { history, processes })
    }

    /// Verifies and atomically admits one exact structured run.
    ///
    /// Run identity is derived by the store adapter; the derived id is returned
    /// with the append attempt.
    pub async fn admit_run(
        &self,
        command: StructuredAdmissionCommand,
    ) -> Result<(RunId, StructuredAppendAttempt)> {
        self.history
            .commit_event(None, QualifiedRuntimeIntent::Admission(Box::new(command)))
            .await
            .map_err(|error| {
                self.store_fault(
                    error,
                    RuntimeFaultPhase::AppendCandidate,
                    // Run id is not known before derivation failures; use a placeholder digest-free fault subject only via store identity.
                    RunId::from_digest(
                        mfm_ids::DigestAlgorithm::Sha256JcsV1,
                        mfm_canonical::sha256_digest_bytes(b"mfm.runtime.admission-fault.v1"),
                    ),
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
            subject: RuntimeFaultSubject::Store(self.history.store_identity().clone()),
            store_fault_kind: None,
        }
    }

    fn proposal_store_fault(
        &self,
        error: HistoryError,
        phase: RuntimeFaultPhase,
        run_id: RunId,
        pre_fault_head: Option<JournalHead>,
        occurrence_id: Option<OccurrenceId>,
        component: QualifiedComponentIdentity,
    ) -> RuntimeError {
        if error == HistoryError::CandidateRejected {
            self.component_fault(
                RuntimeFaultCode::CandidateRejected,
                RuntimeFaultPhase::QualifyCandidate,
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
        error: HistoryError,
        phase: RuntimeFaultPhase,
        run_id: RunId,
        pre_fault_head: Option<JournalHead>,
        occurrence_id: Option<OccurrenceId>,
    ) -> RuntimeError {
        let store_fault_kind = match error {
            HistoryError::RunNotFound => RuntimeStoreFaultKind::RunNotFound,
            HistoryError::InvalidHistory => RuntimeStoreFaultKind::InvalidHistory,
            HistoryError::CapacityExceeded => RuntimeStoreFaultKind::CapacityExceeded,
            HistoryError::CandidateRejected => {
                return self.candidate_fault(phase, run_id, pre_fault_head, occurrence_id);
            }
            HistoryError::Certification => RuntimeStoreFaultKind::Certification,
            HistoryError::StaleHead => RuntimeStoreFaultKind::StaleHead,
            HistoryError::AppendConflict => RuntimeStoreFaultKind::AppendConflict,
            HistoryError::BackendUnavailable => RuntimeStoreFaultKind::BackendUnavailable,
            HistoryError::AcknowledgementUnknown => RuntimeStoreFaultKind::AcknowledgementUnknown,
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
            subject: RuntimeFaultSubject::Store(self.history.store_identity().clone()),
            store_fault_kind: Some(store_fault_kind),
        }
    }

    /// Interprets and performs at most one reducer-derived semantic or audited action.
    pub async fn drive_once(&self, run_id: &RunId) -> Result<DriveOutcome> {
        let verified = self.history.load_verified(run_id).await.map_err(|error| {
            self.store_fault(
                error,
                RuntimeFaultPhase::LoadHistory,
                run_id.clone(),
                None,
                None,
            )
        })?;
        let state = match verified.frontier() {
            StructuredFrontier::Complete => {
                return Ok(DriveOutcome::Closed);
            }
            StructuredFrontier::PossibleEntry(subject) => {
                let subject = subject.clone();
                return Ok(DriveOutcome::PossibleEntry(subject));
            }
            StructuredFrontier::BlockedIntegrity => {
                return Ok(DriveOutcome::BlockedIntegrity);
            }
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
                let (_, observation) =
                    verified.observation(access_attempt_id).ok_or_else(|| {
                        self.candidate_fault(
                            RuntimeFaultPhase::QualifyCandidate,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        )
                    })?;
                let settlement = match &observation.outcome {
                    ObservationOutcome::Returned { value } => self.processes.settle_returned(
                        &state_identity,
                        &input,
                        &canonical_object(&verified, &value.value_ref).map_err(|()| {
                            self.candidate_fault(
                                RuntimeFaultPhase::QualifyCandidate,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            )
                        })?,
                    ),
                    ObservationOutcome::SafeFailure { value } => {
                        self.processes.settle_safe_failure(
                            &state_identity,
                            &input,
                            &canonical_object(&verified, &value.value_ref).map_err(|()| {
                                self.candidate_fault(
                                    RuntimeFaultPhase::QualifyCandidate,
                                    run_id.clone(),
                                    Some(pre_fault_head.clone()),
                                    Some(occurrence_id.clone()),
                                )
                            })?,
                        )
                    }
                    ObservationOutcome::SupersededBeforeEntry { .. }
                    | ObservationOutcome::EntryUnknown { .. }
                    | ObservationOutcome::IntegrityFault { .. } => {
                        return Err(self.candidate_fault(
                            RuntimeFaultPhase::QualifyCandidate,
                            run_id.clone(),
                            Some(pre_fault_head.clone()),
                            Some(occurrence_id.clone()),
                        ));
                    }
                };
                match settlement.map_err(|fault| {
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
            (StructuredExecutionKind::Read, StateLeaf::Ready | StateLeaf::Reassertable { .. }) => {
                self.drive_access::<ReadPhysicalBindingKind>(verified, state, state_identity, input)
                    .await
            }
            (
                StructuredExecutionKind::Effect,
                StateLeaf::Ready | StateLeaf::Refreshable { .. } | StateLeaf::Reassertable { .. },
            ) => {
                self.drive_access::<EffectPhysicalBindingKind>(
                    verified,
                    state,
                    state_identity,
                    input,
                )
                .await
            }
            (StructuredExecutionKind::Effect, StateLeaf::EntryClosable { access_attempt_id }) => {
                let access_attempt_id = access_attempt_id.clone();
                self.close_parked_attempt(verified, &state, access_attempt_id)
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

    /// Commits the closing observation of one crashed absorbing Effect attempt.
    ///
    /// This path authors nothing, invokes nothing, and reaches no adapter. It
    /// asserts nothing about the external system: it asserts that the invoker
    /// authority for this attempt is lost and entry is unknown, which is the
    /// literal truth of a crash. Observation admission already requires exactly
    /// an existing authorization with no prior observation, so the record is
    /// legal history with no contract change — and this is the one observation
    /// Runtime can synthesize without invoking anything and without lying. If a
    /// later edit makes Runtime synthesize any other outcome, that property is
    /// gone.
    ///
    /// Two workers may race this closure, or close a slow attempt that is still
    /// live. The first commit wins and the second is rejected by the
    /// one-observation-per-attempt rule; a live invoker whose attempt was closed
    /// under it has its real completion rejected the same way, and whatever it
    /// did to the external system is absorbed by the re-assertion at the next
    /// ordinal. That is the collapse absorption already declares harmless, which
    /// is exactly why closing is scoped to a declared `EntryAbsorbing` and an
    /// `EntryOnce` park is never touched.
    async fn close_parked_attempt(
        &self,
        verified: P::VerifiedRun,
        state: &ActionableState,
        access_attempt_id: AccessAttemptId,
    ) -> Result<DriveOutcome> {
        let run_id = verified.run_id().clone();
        let pre_fault_head = verified.journal_head().clone();
        let occurrence_id = state.occurrence_id.clone();
        let candidate_fault = || {
            self.candidate_fault(
                RuntimeFaultPhase::QualifyCandidate,
                run_id.clone(),
                Some(pre_fault_head.clone()),
                Some(occurrence_id.clone()),
            )
        };
        // The observation pipeline resolves the authorization out of reduced
        // history, so the in-process record the crash destroyed is not required.
        let authorization_ref = verified
            .authorization(&access_attempt_id)
            .map(|(authorization_ref, _)| authorization_ref.clone())
            .ok_or_else(candidate_fault)?;
        let capability_contract_ref = state
            .capability_contract_ref
            .as_ref()
            .ok_or_else(candidate_fault)?;
        let capability_identity = self
            .processes
            .component_identity(StructuredComponentKind::Capability, capability_contract_ref)
            .ok_or_else(candidate_fault)?;
        let invoked = InvokedObservation::<EffectPhysicalBindingKind, P::VerifiedRun> {
            run_id,
            authorization_ref,
            access_attempt_id,
            outcome: ProposedObservationOutcome::EntryUnknown {
                fault_code: invoker_authority_lost_fault_code(),
            },
            verified,
            adapter_origin: capability_identity,
            _kind: PhantomData,
        };
        self.commit_invoked_observation(invoked).await
    }

    async fn commit_state_proposal(
        &self,
        verified: P::VerifiedRun,
        state: ActionableState,
        proposal: QualifiedStateProposal,
    ) -> Result<DriveOutcome> {
        let run_id = verified.run_id().clone();
        let occurrence_id = state.occurrence_id.clone();
        let proposal_origin = proposal.origin().clone();
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
        let transition = state_transition_proposal(append_id, proposal).map_err(|()| {
            self.component_fault(
                RuntimeFaultCode::CodecFault,
                RuntimeFaultPhase::QualifyCandidate,
                run_id.clone(),
                Some(pre_fault_head.clone()),
                Some(occurrence_id.clone()),
                proposal_origin.clone(),
            )
        })?;
        let (_, attempt) = self
            .history
            .commit_event(
                Some(verified),
                QualifiedRuntimeIntent::Transition(transition),
            )
            .await
            .map_err(|error| {
                self.proposal_store_fault(
                    error,
                    RuntimeFaultPhase::AppendCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                    proposal_origin,
                )
            })?;
        match attempt.outcome() {
            HistoryAppendOutcome::NewlyCommitted(_) | HistoryAppendOutcome::ExistingSame(_) => {
                Ok(DriveOutcome::TransitionCommitted {
                    closed: attempt.closed(),
                })
            }
            HistoryAppendOutcome::StaleHead => Ok(DriveOutcome::ConcurrentProgress),
            HistoryAppendOutcome::AcknowledgementUnknown => Err(self.store_fault(
                HistoryError::AcknowledgementUnknown,
                RuntimeFaultPhase::AppendCandidate,
                run_id,
                Some(pre_fault_head),
                Some(occurrence_id),
            )),
        }
    }

    async fn drive_access<K: AccessMarker>(
        &self,
        verified: P::VerifiedRun,
        state: ActionableState,
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
        let occurrence_path_ref = state.occurrence_path.content_ref().map_err(|_| {
            self.candidate_fault(
                RuntimeFaultPhase::QualifyCandidate,
                run_id.clone(),
                Some(pre_fault_head.clone()),
                Some(occurrence_id.clone()),
            )
        })?;
        let attempt_ordinal = match &state.leaf {
            StateLeaf::Ready => 0,
            StateLeaf::Refreshable {
                next_attempt_ordinal,
                ..
            }
            | StateLeaf::Reassertable {
                next_attempt_ordinal,
                ..
            } => *next_attempt_ordinal,
            _ => {
                return Err(self.candidate_fault(
                    RuntimeFaultPhase::QualifyCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                ));
            }
        };
        let target = AccessTargetSelection {
            run_id: verified.run_id(),
            occurrence_id: &state.occurrence_id,
            occurrence_path_ref: &occurrence_path_ref,
            semantic_call_id: &state.semantic_call_id,
            semantic_head: verified.semantic_head(),
            attempt_ordinal,
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
        let binding = match AssertUnwindSafe(self.processes.qualify_access::<K>(
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
        let prepared = Prepared::<K, P::VerifiedRun> {
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
                self.commit_invoked_observation(invoked).await
            }
        }
    }

    async fn authorize<K: AccessMarker>(
        &self,
        prepared: Prepared<K, P::VerifiedRun>,
    ) -> Result<Option<Authorized<K, P::VerifiedRun>>> {
        let Prepared {
            state,
            binding,
            verified,
            request_origin,
            adapter_origin,
        } = prepared;
        let run_id = verified.run_id().clone();
        let occurrence_id = state.occurrence_id.clone();
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
                    request_origin,
                )
            })?,
            binding.public_certificate().clone(),
        );
        let (_, attempt) = self
            .history
            .commit_event(
                Some(verified),
                QualifiedRuntimeIntent::Authorization(Box::new(proposal)),
            )
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
            HistoryAppendOutcome::NewlyCommitted(_) => {
                let authorization =
                    attempt
                        .into_committed_access_authorization()
                        .ok_or_else(|| {
                            self.candidate_fault(
                                RuntimeFaultPhase::QualifyCandidate,
                                run_id.clone(),
                                Some(pre_fault_head.clone()),
                                Some(occurrence_id.clone()),
                            )
                        })?;
                let verified = self.history.load_verified(&run_id).await.map_err(|error| {
                    self.store_fault(
                        error,
                        RuntimeFaultPhase::LoadHistory,
                        run_id.clone(),
                        Some(pre_fault_head.clone()),
                        Some(occurrence_id.clone()),
                    )
                })?;
                Ok(Some(Authorized {
                    binding,
                    authorization,
                    predecessor_head: pre_fault_head,
                    verified,
                    adapter_origin,
                }))
            }
            HistoryAppendOutcome::ExistingSame(_)
            | HistoryAppendOutcome::StaleHead
            | HistoryAppendOutcome::AcknowledgementUnknown => Ok(None),
        }
    }

    async fn invoke_authorized<K: AccessMarker>(
        &self,
        authorized: Authorized<K, P::VerifiedRun>,
    ) -> Result<InvokedObservation<K, P::VerifiedRun>> {
        let Authorized {
            binding,
            authorization,
            predecessor_head,
            verified,
            adapter_origin,
        } = authorized;
        let authorization_ref = authorization.authorization_ref().clone();
        let access_attempt_id = authorization.access_attempt_id().clone();
        let occurrence_id = authorization.authorization().occurrence_id.clone();
        let run_id = verified.run_id().clone();
        let pre_fault_head = predecessor_head;
        let Some((committed_ref, committed_authorization)) =
            VerifiedRunView::authorization(&verified, &access_attempt_id)
        else {
            return Err(self.store_fault(
                HistoryError::InvalidHistory,
                RuntimeFaultPhase::QualifyAccess,
                run_id.clone(),
                Some(pre_fault_head.clone()),
                Some(occurrence_id.clone()),
            ));
        };
        let expected_successor_sequence = pre_fault_head.run_sequence.checked_add(1);
        if committed_ref != authorization.authorization_ref()
            || committed_authorization != authorization.authorization()
            || committed_ref.run_id != run_id
            || authorization.predecessor_head() != Some(&pre_fault_head)
            || expected_successor_sequence != Some(committed_ref.run_sequence)
            || authorization.successor_head().run_sequence != committed_ref.run_sequence
            || verified.journal_head().run_sequence < committed_ref.run_sequence
        {
            return Err(self.store_fault(
                HistoryError::InvalidHistory,
                RuntimeFaultPhase::QualifyAccess,
                run_id.clone(),
                Some(pre_fault_head.clone()),
                Some(occurrence_id.clone()),
            ));
        }
        let completion = AssertUnwindSafe(
            self.processes
                .invoke_certified_binding(binding, authorization.into_certified()),
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

    async fn commit_invoked_observation<K: AccessMarker>(
        &self,
        invoked: InvokedObservation<K, P::VerifiedRun>,
    ) -> Result<DriveOutcome> {
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
        let proposal = AccessObservationProposal::new(append_id, authorization_ref, outcome);
        let (_, attempt) = self
            .history
            .commit_event(
                Some(verified),
                QualifiedRuntimeIntent::Observation(proposal),
            )
            .await
            .map_err(|error| {
                self.proposal_store_fault(
                    error,
                    RuntimeFaultPhase::AppendCandidate,
                    run_id.clone(),
                    Some(pre_fault_head.clone()),
                    Some(occurrence_id.clone()),
                    adapter_origin,
                )
            })?;
        match attempt.outcome() {
            HistoryAppendOutcome::NewlyCommitted(_) | HistoryAppendOutcome::ExistingSame(_) => {
                Ok(DriveOutcome::AccessObserved)
            }
            HistoryAppendOutcome::StaleHead => Ok(DriveOutcome::ConcurrentProgress),
            HistoryAppendOutcome::AcknowledgementUnknown => Err(self.store_fault(
                HistoryError::AcknowledgementUnknown,
                RuntimeFaultPhase::AppendCandidate,
                run_id,
                Some(pre_fault_head),
                Some(occurrence_id),
            )),
        }
    }
}

trait AccessMarker: PhysicalBindingKind {}

impl AccessMarker for ReadPhysicalBindingKind {}
impl AccessMarker for EffectPhysicalBindingKind {}

struct Prepared<K: AccessMarker, V> {
    state: ActionableState,
    binding: QualifiedPhysicalBinding<K>,
    verified: V,
    request_origin: QualifiedComponentIdentity,
    adapter_origin: QualifiedComponentIdentity,
}

struct Authorized<K: AccessMarker, V> {
    binding: QualifiedPhysicalBinding<K>,
    authorization: CommittedAccessAuthorization,
    predecessor_head: JournalHead,
    verified: V,
    adapter_origin: QualifiedComponentIdentity,
}

struct InvokedObservation<K: AccessMarker, V> {
    run_id: RunId,
    authorization_ref: RecordRef,
    access_attempt_id: AccessAttemptId,
    outcome: ProposedObservationOutcome,
    verified: V,
    adapter_origin: QualifiedComponentIdentity,
    _kind: PhantomData<fn() -> K>,
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

fn canonical_object<V: VerifiedRunView>(
    verified: &V,
    content_ref: &ContentRef,
) -> std::result::Result<CanonicalJsonValue, ()> {
    let object = verified.object(content_ref).ok_or(())?;
    CanonicalJsonValue::from_canonical_json(object.canonical_json.as_bytes()).map_err(|_| ())
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
