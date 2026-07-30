use mfm_ids::{
    AppendRequestId, InvocationIdentity, JournalCandidateDigest, RunId, StableId, TenantScopeId,
};
use mfm_journal::{
    AuthorizationRef, BatchPurpose, CandidateRecordEnvelope, CommitCandidatePreimage,
    CommitDigestPreimage, CommitEnvelope, ExternalAccessAuthorized, ExternalAccessObserved,
    GenesisPreimage, JournalHead, JournalPredecessor, LegalCommitBatch, ObservationRef,
    RecordHashPreimage, RecordIdPreimage, RecordLogicalKey, RecordRef, RunAdmitted, RunClosed,
    RunJournalRecord, RunPhase, StateTransitionCommitted, TenantFactCoordinate,
    TenantFactCoordinateFields, TenantFactFrontier, TransitionBodyFields, TransitionSlot,
};

use super::{
    CommittedJournalCommit, CommittedJournalRecord, PendingFactScanAttestation,
    PersistedFactScanAttestation, PreparedObjectGraph, Result, StoreError, StoreIdentity,
    VerifiedRunView,
};

/// Reserved capability operation whose authorization freezes a same-store fact frontier.
pub const FACT_SELECTION_OPERATION_ID: &str = "mfm.journal.fact-selection.v1";

/// The exhaustive purpose of one prepared journal append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparedAppendKind {
    /// The sole immutable run root.
    AdmitRun,
    /// One semantic transition and its inseparable closure when terminal.
    CommitTransition,
    /// One exact ambient-operation authorization.
    AuthorizeExternalAccess,
    /// One exact observed ambient-operation outcome.
    ObserveExternalAccess,
}

/// Result of re-verifying one prepared successor against the backend's locked current prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuccessorDisposition {
    /// The complete prefix and candidate form one valid next append.
    Ready,
    /// The prepared physical predecessor no longer names the locked current head.
    StaleHead {
        /// Exact predecessor frozen into the prepared candidate.
        expected: JournalPredecessor,
        /// Current fully verified physical head.
        actual: Box<JournalHead>,
    },
    /// The semantic run is permanently closed for this append kind.
    RunClosed,
}

/// Exact store-owned resolution of one immutable logical or physical identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactResolution<T> {
    /// No committed value occupies the requested identity.
    Absent,
    /// The requested identity is committed with byte-identical content.
    Identical(T),
    /// The requested identity is committed with different immutable content.
    Conflict,
}

/// Exact logical identity of one external-access observation.
///
/// Construction is sealed to store-prepared observation material. The raw bytes are retained
/// alongside their annex-derived content identity so an ambiguous physical append never requires
/// reinvoking the external operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalObservationIdentity {
    authorization_ref: AuthorizationRef,
    content_ref: mfm_ids::ContentRef,
    canonical_bytes: Vec<u8>,
}

impl LogicalObservationIdentity {
    /// Returns the authorization's unique logical observation key.
    pub const fn authorization_ref(&self) -> &AuthorizationRef {
        &self.authorization_ref
    }

    /// Returns the annex-derived digest identity of the exact logical material.
    pub const fn content_ref(&self) -> &mfm_ids::ContentRef {
        &self.content_ref
    }

    /// Returns the exact canonical logical observation bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
}

/// Exact physical identity retained across an ambiguous append acknowledgement.
///
/// Construction is sealed to store-prepared append material. The candidate preimage bytes retain
/// the complete predecessor, purpose, records, and object-authority declarations in addition to
/// the idempotency key and digest used for resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalAppendIdentity {
    append_request_id: AppendRequestId,
    expected_predecessor: JournalPredecessor,
    candidate_digest: JournalCandidateDigest,
    candidate_bytes: Vec<u8>,
}

impl PhysicalAppendIdentity {
    /// Returns the exact idempotency key.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the exact compare-and-swap predecessor.
    pub const fn expected_predecessor(&self) -> &JournalPredecessor {
        &self.expected_predecessor
    }

    /// Returns the digest of the exact unassigned candidate.
    pub const fn candidate_digest(&self) -> &JournalCandidateDigest {
        &self.candidate_digest
    }

    /// Returns the exact canonical candidate-preimage bytes.
    pub fn candidate_bytes(&self) -> &[u8] {
        &self.candidate_bytes
    }
}

/// Store-verified logical observation placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLogicalObservation {
    observation_ref: ObservationRef,
    journal_head: JournalHead,
}

impl ResolvedLogicalObservation {
    /// Returns the exact committed observation record.
    pub const fn observation_ref(&self) -> &ObservationRef {
        &self.observation_ref
    }

    /// Returns the physical head containing the observation.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }
}

/// Store-verified result of resolving one retained physical append identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalAppendResolution {
    exact: ExactResolution<CommittedAppend>,
    current_head: JournalHead,
}

impl PhysicalAppendResolution {
    /// Returns whether the append id is absent, identical, or conflicting.
    pub const fn exact(&self) -> &ExactResolution<CommittedAppend> {
        &self.exact
    }

    /// Returns the verified current physical head used for absence/staleness decisions.
    pub const fn current_head(&self) -> &JournalHead {
        &self.current_head
    }
}

pub(super) struct PreparedAppendCore {
    pub(super) store_identity: StoreIdentity,
    pub(super) tenant_scope_id: TenantScopeId,
    pub(super) run_id: RunId,
    pub(super) append_request_id: AppendRequestId,
    pub(super) expected_predecessor: JournalPredecessor,
    pub(super) candidate_records: Vec<CandidateRecordEnvelope>,
    pub(super) candidate_preimage: CommitCandidatePreimage,
    pub(super) candidate_digest: JournalCandidateDigest,
    pub(super) objects: PreparedObjectGraph,
}

struct PreparedAppendIdentity {
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    append_request_id: AppendRequestId,
    expected_predecessor: JournalPredecessor,
}

impl PreparedAppendCore {
    fn new(
        identity: PreparedAppendIdentity,
        batch_purpose: BatchPurpose,
        candidate_records: Vec<CandidateRecordEnvelope>,
        objects: PreparedObjectGraph,
    ) -> Result<Self> {
        let PreparedAppendIdentity {
            store_identity,
            tenant_scope_id,
            run_id,
            append_request_id,
            expected_predecessor,
        } = identity;
        objects.validate_record_count(candidate_records.len())?;
        let candidate_preimage = CommitCandidatePreimage::new(
            &expected_predecessor,
            batch_purpose,
            &candidate_records,
            objects.bindings(),
            objects.admission_intents(),
        )?;
        let candidate_digest = candidate_preimage.candidate_digest()?;
        Ok(Self {
            store_identity,
            tenant_scope_id,
            run_id,
            append_request_id,
            expected_predecessor,
            candidate_records,
            candidate_preimage,
            candidate_digest,
            objects,
        })
    }
}

/// Sealed prepared admission of one immutable root.
pub struct AdmitRun {
    pub(super) core: PreparedAppendCore,
    admission: RunAdmitted,
}

impl AdmitRun {
    /// Prepares the sole genesis-root append.
    pub(super) fn new(
        store_identity: &StoreIdentity,
        append_request_id: AppendRequestId,
        admission: RunAdmitted,
        objects: PreparedObjectGraph,
    ) -> Result<Self> {
        let fields = admission.fields()?;
        let derived_genesis = GenesisPreimage::new(
            store_identity.store_scope_id(),
            store_identity.store_epoch(),
            &fields.run_id,
        )?
        .genesis_digest()?;
        if fields.genesis_digest != derived_genesis {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "admission genesis digest does not match the exact store lineage",
            });
        }
        let expected_predecessor = JournalPredecessor::genesis(
            store_identity.store_scope_id(),
            store_identity.store_epoch(),
            &fields.run_id,
            &derived_genesis,
        )?;
        let payload = RunJournalRecord::run_admitted(&admission)?;
        let candidate =
            candidate_record(0, RecordLogicalKey::run_admission(&fields.run_id)?, payload)?;
        LegalCommitBatch::run_admission(&admission)?;
        let core = PreparedAppendCore::new(
            PreparedAppendIdentity {
                store_identity: store_identity.clone(),
                tenant_scope_id: fields.tenant_scope_id,
                run_id: fields.run_id,
                append_request_id,
                expected_predecessor,
            },
            BatchPurpose::RunAdmission,
            vec![candidate],
            objects,
        )?;
        Ok(Self { core, admission })
    }

    /// Returns the exact immutable root.
    pub const fn admission(&self) -> &RunAdmitted {
        &self.admission
    }
}

/// Sealed prepared semantic transition append.
pub struct CommitTransition {
    core: PreparedAppendCore,
    transition: StateTransitionCommitted,
    closure: Option<RunClosed>,
}

impl CommitTransition {
    /// Prepares a transition against one verified physical and semantic head.
    ///
    /// A transition whose after-state is closed receives its mandatory ordinal-one
    /// [`RunClosed`] automatically. Callers cannot prepare a standalone closure.
    pub(super) fn new(
        view: &VerifiedRunView,
        append_request_id: AppendRequestId,
        transition: StateTransitionCommitted,
        objects: PreparedObjectGraph,
    ) -> Result<Self> {
        let transition_fields = transition.fields()?;
        let purpose = purpose_for_transition(&transition_fields.body.fields()?)?;
        validate_slot_for_purpose(transition_fields.slot, purpose)?;
        let transition_payload = RunJournalRecord::state_transition_committed(&transition)?;
        let transition_candidate = candidate_record(
            0,
            RecordLogicalKey::transition(
                view.run_id(),
                &transition_fields.node_id,
                transition_fields.slot,
            )?,
            transition_payload,
        )?;
        let transition_hash =
            RecordHashPreimage::from_candidate(&transition_candidate)?.record_hash()?;
        let after = transition_fields.after.fields()?;
        let closure = if after.run_phase == RunPhase::Closed {
            Some(RunClosed::new(&transition_hash)?)
        } else {
            None
        };
        let mut candidates = vec![transition_candidate];
        if let Some(closure) = &closure {
            candidates.push(candidate_record(
                1,
                RecordLogicalKey::closure(view.run_id())?,
                RunJournalRecord::run_closed(closure)?,
            )?);
        }
        LegalCommitBatch::transition(&transition, closure.as_ref())?;
        let core = PreparedAppendCore::new(
            PreparedAppendIdentity {
                store_identity: view.store_identity().clone(),
                tenant_scope_id: view.tenant_scope_id().clone(),
                run_id: view.run_id().clone(),
                append_request_id,
                expected_predecessor: JournalPredecessor::journal_head(view.journal_head())?,
            },
            purpose,
            candidates,
            objects,
        )?;
        view.validate_prepared_transition(&core.candidate_preimage, &core.objects)?;
        Ok(Self {
            core,
            transition,
            closure,
        })
    }

    /// Returns the exact transition.
    pub const fn transition(&self) -> &StateTransitionCommitted {
        &self.transition
    }

    /// Returns the store-derived inseparable closure, when terminal.
    pub const fn closure(&self) -> Option<&RunClosed> {
        self.closure.as_ref()
    }
}

/// Sealed prepared ambient-operation authorization append.
pub struct AuthorizeExternalAccess {
    core: PreparedAppendCore,
    authorization: ExternalAccessAuthorized,
}

impl AuthorizeExternalAccess {
    /// Prepares one exact external-access authorization against a verified run.
    pub(super) fn new(
        view: &VerifiedRunView,
        append_request_id: AppendRequestId,
        authorization: ExternalAccessAuthorized,
        objects: PreparedObjectGraph,
    ) -> Result<Self> {
        let payload = RunJournalRecord::external_access_authorized(&authorization)?;
        let candidate = candidate_record(
            0,
            RecordLogicalKey::authorization(&append_request_id)?,
            payload,
        )?;
        LegalCommitBatch::authorization(&authorization)?;
        let core = PreparedAppendCore::new(
            PreparedAppendIdentity {
                store_identity: view.store_identity().clone(),
                tenant_scope_id: view.tenant_scope_id().clone(),
                run_id: view.run_id().clone(),
                append_request_id,
                expected_predecessor: JournalPredecessor::journal_head(view.journal_head())?,
            },
            BatchPurpose::ExternalAccessAuthorization,
            vec![candidate],
            objects,
        )?;
        view.validate_prepared_authorization(&core.candidate_preimage)?;
        Ok(Self {
            core,
            authorization,
        })
    }

    /// Returns the exact authorized operation.
    pub const fn authorization(&self) -> &ExternalAccessAuthorized {
        &self.authorization
    }
}

/// Sealed prepared ambient-operation observation append.
pub struct ObserveExternalAccess {
    core: PreparedAppendCore,
    observation: ExternalAccessObserved,
    pending_fact_attestation: Option<PendingFactScanAttestation>,
}

impl ObserveExternalAccess {
    /// Prepares one exact observation linked to its committed authorization.
    pub(super) fn new(
        view: &VerifiedRunView,
        append_request_id: AppendRequestId,
        observation: ExternalAccessObserved,
        objects: PreparedObjectGraph,
        pending_fact_attestation: Option<PendingFactScanAttestation>,
        allow_already_observed: bool,
    ) -> Result<Self> {
        let authorization_ref = observation.fields()?.authorization_ref;
        let authorization = view
            .authorizations()
            .find_map(|(reference, authorization)| {
                (reference == &authorization_ref).then_some(authorization)
            })
            .ok_or(StoreError::UnknownAuthorization)?;
        let authorization_fields = authorization.fields()?;
        let observation_fields = observation.fields()?;
        let reserved =
            authorization_fields.capability_operation_id.as_str() == FACT_SELECTION_OPERATION_ID;
        let returned = matches!(
            observation_fields.outcome.fields()?,
            mfm_journal::ObservationOutcomeFields::Returned { .. }
        );
        let has_attestation = observation_fields
            .fact_selection_scan_attestation_ref
            .is_some();
        if if pending_fact_attestation.is_some() {
            !reserved || !returned || !has_attestation
        } else {
            has_attestation || (reserved && returned)
        } {
            return Err(StoreError::FactScanBindingMismatch);
        }
        let payload = RunJournalRecord::external_access_observed(&observation)?;
        let candidate = candidate_record(
            0,
            RecordLogicalKey::observation(&authorization_ref)?,
            payload,
        )?;
        LegalCommitBatch::observation(&observation)?;
        let core = PreparedAppendCore::new(
            PreparedAppendIdentity {
                store_identity: view.store_identity().clone(),
                tenant_scope_id: view.tenant_scope_id().clone(),
                run_id: view.run_id().clone(),
                append_request_id,
                expected_predecessor: JournalPredecessor::journal_head(view.journal_head())?,
            },
            BatchPurpose::ExternalAccessObservation,
            vec![candidate],
            objects,
        )?;
        match view.validate_prepared_observation(&core.candidate_preimage, &core.objects) {
            Ok(()) => {}
            Err(StoreError::ObservationAlreadyCommitted) if allow_already_observed => {}
            Err(error) => return Err(error),
        }
        Ok(Self {
            core,
            observation,
            pending_fact_attestation,
        })
    }

    /// Returns the exact observed outcome.
    pub const fn observation(&self) -> &ExternalAccessObserved {
        &self.observation
    }

    pub(super) const fn pending_fact_attestation(&self) -> Option<&PendingFactScanAttestation> {
        self.pending_fact_attestation.as_ref()
    }
}

/// The only four append variants accepted by a recoverability-v1 run journal.
pub enum PreparedJournalAppend {
    /// The sole immutable root.
    AdmitRun(AdmitRun),
    /// One state transition and its optional inseparable closure.
    CommitTransition(CommitTransition),
    /// One exact external operation authorization.
    AuthorizeExternalAccess(AuthorizeExternalAccess),
    /// One exact linked external observation.
    ObserveExternalAccess(Box<ObserveExternalAccess>),
}

impl From<AdmitRun> for PreparedJournalAppend {
    fn from(value: AdmitRun) -> Self {
        Self::AdmitRun(value)
    }
}

impl From<CommitTransition> for PreparedJournalAppend {
    fn from(value: CommitTransition) -> Self {
        Self::CommitTransition(value)
    }
}

impl From<AuthorizeExternalAccess> for PreparedJournalAppend {
    fn from(value: AuthorizeExternalAccess) -> Self {
        Self::AuthorizeExternalAccess(value)
    }
}

impl From<ObserveExternalAccess> for PreparedJournalAppend {
    fn from(value: ObserveExternalAccess) -> Self {
        Self::ObserveExternalAccess(Box::new(value))
    }
}

impl PreparedJournalAppend {
    /// Returns the exhaustive append variant.
    pub const fn kind(&self) -> PreparedAppendKind {
        match self {
            Self::AdmitRun(_) => PreparedAppendKind::AdmitRun,
            Self::CommitTransition(_) => PreparedAppendKind::CommitTransition,
            Self::AuthorizeExternalAccess(_) => PreparedAppendKind::AuthorizeExternalAccess,
            Self::ObserveExternalAccess(_) => PreparedAppendKind::ObserveExternalAccess,
        }
    }

    /// Freezes the exact physical identity needed to resolve or retry this append.
    pub fn physical_identity(&self) -> PhysicalAppendIdentity {
        let core = self.core();
        PhysicalAppendIdentity {
            append_request_id: core.append_request_id.clone(),
            expected_predecessor: core.expected_predecessor.clone(),
            candidate_digest: core.candidate_digest.clone(),
            candidate_bytes: core.candidate_preimage.as_bytes().to_vec(),
        }
    }

    /// Freezes the exact logical observation identity, when this is an observation append.
    pub fn logical_observation_identity(&self) -> Result<Option<LogicalObservationIdentity>> {
        let Self::ObserveExternalAccess(observation) = self else {
            return Ok(None);
        };
        let fields = observation.observation.fields()?;
        Ok(Some(LogicalObservationIdentity {
            authorization_ref: fields.authorization_ref,
            content_ref: observation.observation.content_ref()?,
            canonical_bytes: observation.observation.as_bytes().to_vec(),
        }))
    }

    pub(super) const fn core(&self) -> &PreparedAppendCore {
        match self {
            Self::AdmitRun(value) => &value.core,
            Self::CommitTransition(value) => &value.core,
            Self::AuthorizeExternalAccess(value) => &value.core,
            Self::ObserveExternalAccess(value) => &value.core,
        }
    }

    fn authorization(&self) -> Option<&ExternalAccessAuthorized> {
        match self {
            Self::AuthorizeExternalAccess(value) => Some(&value.authorization),
            Self::AdmitRun(_) | Self::CommitTransition(_) | Self::ObserveExternalAccess(_) => None,
        }
    }
}

impl VerifiedRunView {
    /// Resolves one logical observation key independently of any physical append attempt.
    ///
    /// A matching committed observation is identical even when it became visible before the
    /// caller prepared a local physical candidate.
    pub fn resolve_logical_observation(
        &self,
        identity: &LogicalObservationIdentity,
    ) -> Result<ExactResolution<ResolvedLogicalObservation>> {
        let Some(entry) = self
            .access_audit_entries()
            .find(|entry| entry.authorization_ref() == identity.authorization_ref())
        else {
            return Ok(ExactResolution::Absent);
        };
        let Some((observation_ref, observation, journal_head)) = entry.observation() else {
            return Ok(ExactResolution::Absent);
        };
        if observation.as_bytes() == identity.canonical_bytes()
            && observation.content_ref()? == *identity.content_ref()
        {
            Ok(ExactResolution::Identical(ResolvedLogicalObservation {
                observation_ref: observation_ref.clone(),
                journal_head: journal_head.clone(),
            }))
        } else {
            Ok(ExactResolution::Conflict)
        }
    }

    /// Resolves one physical append id independently of its logical record key.
    ///
    /// The returned current head lets a caller distinguish an unchanged retryable predecessor
    /// from a definitely stale attempt after an absent result.
    pub fn resolve_physical_append(
        &self,
        identity: &PhysicalAppendIdentity,
    ) -> Result<PhysicalAppendResolution> {
        let committed = self
            .journal()
            .commits()
            .iter()
            .find(|commit| {
                commit.envelope().fields().is_ok_and(|fields| {
                    fields.core.append_request_id == *identity.append_request_id()
                })
            })
            .map(|commit| {
                let fields = commit.envelope().fields()?;
                let candidate = super::journal::committed_candidate_preimage(commit)?;
                if fields.core.predecessor != *identity.expected_predecessor()
                    || fields.core.candidate_digest != *identity.candidate_digest()
                    || candidate.as_bytes() != identity.candidate_bytes()
                {
                    return Ok(ExactResolution::Conflict);
                }
                let frontier = frontier_for(
                    self.store_identity(),
                    self.tenant_scope_id(),
                    &fields.core.tenant_fact_coordinate,
                )?;
                CommittedAppend::from_commit(commit, frontier).map(ExactResolution::Identical)
            })
            .transpose()?
            .unwrap_or(ExactResolution::Absent);
        Ok(PhysicalAppendResolution {
            exact: committed,
            current_head: self.journal_head().clone(),
        })
    }
}

fn candidate_record(
    ordinal: u32,
    logical_key: RecordLogicalKey,
    payload: RunJournalRecord,
) -> Result<CandidateRecordEnvelope> {
    let emits_facts = payload.emits_facts()?;
    CandidateRecordEnvelope::new(
        ordinal,
        payload.schema_id(),
        &logical_key,
        &payload,
        emits_facts,
    )
    .map_err(Into::into)
}

fn purpose_for_transition(body: &TransitionBodyFields) -> Result<BatchPurpose> {
    Ok(match body {
        TransitionBodyFields::PureSettled { .. } => BatchPurpose::PureSettlement,
        TransitionBodyFields::ReadSettled { .. } => BatchPurpose::ReadSettlement,
        TransitionBodyFields::EffectRequested { .. } => BatchPurpose::EffectRequest,
        TransitionBodyFields::EffectSettled { .. } => BatchPurpose::EffectSettlement,
        TransitionBodyFields::DependencySkipped { .. } => BatchPurpose::DependencySkip,
    })
}

fn validate_slot_for_purpose(slot: TransitionSlot, purpose: BatchPurpose) -> Result<()> {
    let expected = if purpose == BatchPurpose::EffectRequest {
        TransitionSlot::Request
    } else {
        TransitionSlot::Settlement
    };
    if slot != expected {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "commit_transition",
            message: "transition body and logical slot disagree",
        });
    }
    Ok(())
}

/// Store-owned verifier handed to a durable backend for one prepared append.
///
/// Backends can inspect and assign this candidate, but cannot construct one or change its
/// predecessor, purpose, records, objects, or digest.
pub struct JournalAppendVerifier {
    prepared: PreparedJournalAppend,
}

impl JournalAppendVerifier {
    pub(super) const fn new(prepared: PreparedJournalAppend) -> Self {
        Self { prepared }
    }

    /// Returns the exhaustive append kind.
    pub const fn kind(&self) -> PreparedAppendKind {
        self.prepared.kind()
    }

    /// Returns the exact target run.
    pub fn run_id(&self) -> &RunId {
        &self.prepared.core().run_id
    }

    /// Returns the exact tenant.
    pub fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.prepared.core().tenant_scope_id
    }

    /// Returns the caller-selected idempotency key.
    pub fn append_request_id(&self) -> &AppendRequestId {
        &self.prepared.core().append_request_id
    }

    /// Returns the exact compare-and-swap predecessor.
    pub fn expected_predecessor(&self) -> &JournalPredecessor {
        &self.prepared.core().expected_predecessor
    }

    /// Returns the exact unassigned candidate digest.
    pub fn candidate_digest(&self) -> &JournalCandidateDigest {
        &self.prepared.core().candidate_digest
    }

    /// Returns the exact semantic batch purpose frozen into this candidate.
    pub fn batch_purpose(&self) -> Result<BatchPurpose> {
        Ok(self
            .prepared
            .core()
            .candidate_preimage
            .fields()?
            .batch_purpose)
    }

    /// Returns exact immutable object bindings, intents, and staged bytes before assignment.
    pub fn objects(&self) -> &PreparedObjectGraph {
        &self.prepared.core().objects
    }

    /// Returns the inseparable private fact-attestation intent, when present.
    pub fn pending_fact_scan_attestation(&self) -> Option<&PendingFactScanAttestation> {
        match &self.prepared {
            PreparedJournalAppend::ObserveExternalAccess(observation) => {
                observation.pending_fact_attestation()
            }
            PreparedJournalAppend::AdmitRun(_)
            | PreparedJournalAppend::CommitTransition(_)
            | PreparedJournalAppend::AuthorizeExternalAccess(_) => None,
        }
    }

    /// Returns the exact admission entry-point operation, only for an admission candidate.
    pub fn admission_entry_point_operation_id(&self) -> Result<Option<StableId>> {
        match &self.prepared {
            PreparedJournalAppend::AdmitRun(value) => {
                Ok(Some(value.admission.fields()?.entry_point_operation_id))
            }
            PreparedJournalAppend::CommitTransition(_)
            | PreparedJournalAppend::AuthorizeExternalAccess(_)
            | PreparedJournalAppend::ObserveExternalAccess(_) => Ok(None),
        }
    }

    /// Returns the exact admission invocation identity, only for an admission candidate.
    pub fn admission_invocation_identity(&self) -> Result<Option<InvocationIdentity>> {
        match &self.prepared {
            PreparedJournalAppend::AdmitRun(value) => {
                Ok(Some(value.admission.fields()?.invocation_identity))
            }
            PreparedJournalAppend::CommitTransition(_)
            | PreparedJournalAppend::AuthorizeExternalAccess(_)
            | PreparedJournalAppend::ObserveExternalAccess(_) => Ok(None),
        }
    }

    /// Returns whether this candidate publishes one or more facts.
    pub fn emits_facts(&self) -> bool {
        self.prepared
            .core()
            .candidate_records
            .iter()
            .any(|record| record.fields().is_ok_and(|fields| fields.emits_facts))
    }

    /// Returns whether this authorization reserves a fact-selection barrier.
    pub fn reserves_fact_selection_barrier(&self) -> bool {
        self.prepared
            .authorization()
            .and_then(|value| value.fields().ok())
            .is_some_and(|fields| {
                fields.capability_operation_id.as_str() == FACT_SELECTION_OPERATION_ID
            })
    }

    /// Revalidates this entire successor against the current locked verified prefix.
    ///
    /// Durable backends call this after loading the exact current journal inside their append
    /// serialization boundary and before coordinate assignment or publication.
    pub fn verify_successor(&self, view: &VerifiedRunView) -> Result<()> {
        let core = self.prepared.core();
        if &core.store_identity != view.store_identity()
            || &core.tenant_scope_id != view.tenant_scope_id()
            || &core.run_id != view.run_id()
        {
            return Err(StoreError::AccessDenied {
                purpose: "append_successor",
            });
        }
        match &self.prepared {
            PreparedJournalAppend::AdmitRun(_) => Err(StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "admission is valid only against an absent run",
            }),
            PreparedJournalAppend::CommitTransition(_) => {
                view.validate_prepared_transition(&core.candidate_preimage, &core.objects)
            }
            PreparedJournalAppend::AuthorizeExternalAccess(_) => {
                view.validate_prepared_authorization(&core.candidate_preimage)
            }
            PreparedJournalAppend::ObserveExternalAccess(_) => {
                view.validate_prepared_observation(&core.candidate_preimage, &core.objects)
            }
        }
    }

    /// Verifies decoded current rows and this successor with the one store-owned reducer.
    ///
    /// External durable backends use this inside their locked transaction. The candidate itself
    /// fixes the exact store, tenant, and run, so this helper cannot be used as a general raw-read
    /// authority.
    pub fn classify_successor_rows(
        &self,
        commits: Vec<CommittedJournalCommit>,
        objects: Vec<super::CommittedObject>,
    ) -> Result<SuccessorDisposition> {
        let core = self.prepared.core();
        let view = super::verify_offline_recorded_history(
            core.store_identity.clone(),
            core.tenant_scope_id.clone(),
            core.run_id.clone(),
            commits,
            objects,
        )?;
        match self.verify_successor(&view) {
            Ok(()) => Ok(SuccessorDisposition::Ready),
            Err(StoreError::HeadMismatch { .. }) => Ok(SuccessorDisposition::StaleHead {
                expected: core.expected_predecessor.clone(),
                actual: Box::new(view.journal_head().clone()),
            }),
            Err(StoreError::RunClosed) => Ok(SuccessorDisposition::RunClosed),
            Err(error) => Err(error),
        }
    }

    /// Assigns the fields owned by the authoritative store transaction.
    pub fn assign(
        self,
        run_sequence: u64,
        tenant_fact_coordinate: TenantFactCoordinate,
        committed_at: u64,
    ) -> Result<AssignedJournalAppend> {
        validate_assigned_coordinate(&self, &tenant_fact_coordinate)?;
        AssignedJournalAppend::new(
            self.prepared,
            run_sequence,
            tenant_fact_coordinate,
            committed_at,
        )
    }

    /// Verifies a complete loaded prefix and resolves its selected idempotent commit.
    ///
    /// Durable backends use this instead of trusting an isolated matching row. Physical hashes,
    /// predecessors, object closure, and the complete semantic fold are verified before the
    /// selected commit is compared with this candidate. Admission keeps its logical-key
    /// attachment rule; every other append also requires the exact append-request id.
    pub fn already_committed_rows(
        self,
        commits: Vec<CommittedJournalCommit>,
        objects: Vec<super::CommittedObject>,
        selected_run_sequence: u64,
        fact_frontier: Option<TenantFactFrontier>,
    ) -> Result<AppendOutcome> {
        let core = self.prepared.core();
        let view = super::verify_offline_recorded_history(
            core.store_identity.clone(),
            core.tenant_scope_id.clone(),
            core.run_id.clone(),
            commits,
            objects,
        )?;
        let selected_index = usize::try_from(
            selected_run_sequence
                .checked_sub(1)
                .ok_or(StoreError::AppendRequestConflict)?,
        )
        .map_err(|_| StoreError::SequenceOverflow)?;
        let selected = view
            .journal()
            .commits()
            .get(selected_index)
            .ok_or(StoreError::AppendRequestConflict)?;
        let require_append_request_id = self.kind() != PreparedAppendKind::AdmitRun;
        self.resolve_existing(selected, fact_frontier, require_append_request_id)
    }

    fn resolve_existing(
        self,
        commit: &CommittedJournalCommit,
        fact_frontier: Option<TenantFactFrontier>,
        require_append_request_id: bool,
    ) -> Result<AppendOutcome> {
        let expected = self.prepared.core();
        let actual = commit.envelope().fields()?;
        let actual_candidates = commit
            .records()
            .iter()
            .map(|record| record.candidate().clone())
            .collect::<Vec<_>>();
        if actual.core.store_scope_id != *expected.store_identity.store_scope_id()
            || actual.core.run_id != expected.run_id
            || actual.core.predecessor != expected.expected_predecessor
            || (require_append_request_id
                && actual.core.append_request_id != expected.append_request_id)
            || actual.core.candidate_digest != expected.candidate_digest
            || actual_candidates != expected.candidate_records
        {
            return Err(StoreError::AppendRequestConflict);
        }
        CommittedAppend::from_commit(commit, fact_frontier).map(AppendOutcome::AlreadyCommitted)
    }

    /// Resolves a deterministic rejection without creating live access authority.
    pub fn rejected(self, rejection: AppendRejection) -> AppendOutcome {
        drop(self);
        AppendOutcome::Rejected(rejection)
    }

    /// Resolves an ambiguous acknowledgement without creating live access authority.
    pub fn outcome_unknown(self) -> AppendOutcome {
        drop(self);
        AppendOutcome::OutcomeUnknown
    }
}

fn validate_assigned_coordinate(
    verifier: &JournalAppendVerifier,
    coordinate: &TenantFactCoordinate,
) -> Result<()> {
    match coordinate.fields()? {
        TenantFactCoordinateFields::None
            if !verifier.emits_facts() && !verifier.reserves_fact_selection_barrier() =>
        {
            Ok(())
        }
        TenantFactCoordinateFields::FactPublication {
            tenant_scope_id,
            fact_order,
        } if verifier.emits_facts()
            && !verifier.reserves_fact_selection_barrier()
            && &tenant_scope_id == verifier.tenant_scope_id()
            && fact_order > 0 =>
        {
            Ok(())
        }
        TenantFactCoordinateFields::FactSelectionBarrier {
            tenant_scope_id, ..
        } if !verifier.emits_facts()
            && verifier.reserves_fact_selection_barrier()
            && &tenant_scope_id == verifier.tenant_scope_id() =>
        {
            Ok(())
        }
        _ => Err(StoreError::InvalidFactCoordinate),
    }
}

/// A fully assigned append whose publication still belongs to the backend transaction.
pub struct AssignedJournalAppend {
    prepared: PreparedJournalAppend,
    commit: CommittedJournalCommit,
    fact_frontier: Option<TenantFactFrontier>,
}

impl AssignedJournalAppend {
    fn new(
        prepared: PreparedJournalAppend,
        run_sequence: u64,
        tenant_fact_coordinate: TenantFactCoordinate,
        committed_at: u64,
    ) -> Result<Self> {
        if prepared.kind() == PreparedAppendKind::AuthorizeExternalAccess {
            run_sequence
                .checked_add(1)
                .ok_or(StoreError::SequenceOverflow)?;
        }
        let core = prepared.core();
        let expected_sequence = match core.expected_predecessor.fields()? {
            mfm_journal::JournalPredecessorFields::Genesis { .. } => 1,
            mfm_journal::JournalPredecessorFields::JournalHead(head) => head
                .run_sequence
                .checked_add(1)
                .ok_or(StoreError::SequenceOverflow)?,
        };
        if run_sequence != expected_sequence {
            return Err(StoreError::PersistedMismatch {
                field: "assigned_run_sequence",
            });
        }

        let mut record_hashes = Vec::with_capacity(core.candidate_records.len());
        let mut records = Vec::with_capacity(core.candidate_records.len());
        for candidate in &core.candidate_records {
            let fields = candidate.fields()?;
            let record_hash = RecordHashPreimage::from_candidate(candidate)?.record_hash()?;
            let record_id_preimage = RecordIdPreimage::new(
                core.store_identity.store_scope_id(),
                &core.run_id,
                run_sequence,
                fields.ordinal,
                &record_hash,
            )?;
            let record_id = record_id_preimage.record_id()?;
            record_hashes.push(record_hash.clone());
            records.push(CommittedJournalRecord::assigned(
                record_id,
                record_hash,
                candidate.clone(),
            ));
        }

        let digest_preimage = CommitDigestPreimage::new(
            core.store_identity.store_scope_id(),
            &core.run_id,
            run_sequence,
            &core.expected_predecessor,
            &core.append_request_id,
            &core.candidate_digest,
            &tenant_fact_coordinate,
            &record_hashes,
            core.objects.bindings(),
            core.objects.admission_intents(),
        )?;
        let commit_digest = digest_preimage.commit_digest()?;
        let envelope = CommitEnvelope::new(
            core.store_identity.store_scope_id(),
            &core.run_id,
            run_sequence,
            &core.expected_predecessor,
            &core.append_request_id,
            &core.candidate_digest,
            &commit_digest,
            &tenant_fact_coordinate,
            &record_hashes,
            core.objects.bindings(),
            core.objects.admission_intents(),
            committed_at,
        )?;
        let fact_frontier = frontier_for(
            &core.store_identity,
            &core.tenant_scope_id,
            &tenant_fact_coordinate,
        )?;
        Ok(Self {
            prepared,
            commit: CommittedJournalCommit::assigned(envelope, records),
            fact_frontier,
        })
    }

    /// Returns the complete assigned commit envelope and record wrappers.
    pub const fn commit(&self) -> &CommittedJournalCommit {
        &self.commit
    }

    /// Returns exact staged object bytes and immutable authority intents.
    pub fn objects(&self) -> &PreparedObjectGraph {
        &self.prepared.core().objects
    }

    /// Derives the inseparable private fact-attestation row, when this is a reserved observation.
    pub fn persisted_fact_scan_attestation(&self) -> Result<Option<PersistedFactScanAttestation>> {
        let PreparedJournalAppend::ObserveExternalAccess(observation) = &self.prepared else {
            return Ok(None);
        };
        observation
            .pending_fact_attestation()
            .map(|pending| pending.persisted_for_commit(&self.commit))
            .transpose()
    }

    /// Marks an append whose commit result was observed directly by this writer.
    ///
    /// Call this only after the same transaction has durably published the commit and all exact
    /// objects. Ambiguous acknowledgements must use [`Self::outcome_unknown`].
    pub fn directly_committed(self) -> Result<AppendOutcome> {
        let summary = CommittedAppend::from_commit(&self.commit, self.fact_frontier.clone())?;
        let newly = match self.prepared {
            PreparedJournalAppend::AdmitRun(value) => {
                NewlyAppended::RunAdmitted(NewlyAdmittedRun {
                    run_id: value.core.run_id,
                    tenant_scope_id: value.core.tenant_scope_id,
                    committed: summary,
                })
            }
            PreparedJournalAppend::CommitTransition(_) => NewlyAppended::Transition(summary),
            PreparedJournalAppend::AuthorizeExternalAccess(value) => {
                let record = self
                    .commit
                    .records()
                    .first()
                    .ok_or(StoreError::EmptyJournal)?;
                let authorization_ref = AuthorizationRef::new(&record.record_ref(
                    &self.commit.envelope().fields()?.core.run_id,
                    self.commit.envelope().fields()?.core.run_sequence,
                )?)?;
                NewlyAppended::Authorization(Box::new(NewlyAppendedAuthorization {
                    tenant_scope_id: value.core.tenant_scope_id,
                    run_id: value.core.run_id,
                    authorization_ref,
                    authorization: value.authorization,
                    committed: summary,
                }))
            }
            PreparedJournalAppend::ObserveExternalAccess(_) => NewlyAppended::Observation(summary),
        };
        Ok(AppendOutcome::NewlyAppended(newly))
    }

    /// Returns an authority-free ambiguous outcome after commit acknowledgement was lost.
    pub fn outcome_unknown(self) -> AppendOutcome {
        drop(self);
        AppendOutcome::OutcomeUnknown
    }
}

fn frontier_for(
    store_identity: &StoreIdentity,
    tenant_scope_id: &TenantScopeId,
    coordinate: &TenantFactCoordinate,
) -> Result<Option<TenantFactFrontier>> {
    let order = match coordinate.fields()? {
        TenantFactCoordinateFields::None => return Ok(None),
        TenantFactCoordinateFields::FactPublication {
            tenant_scope_id: coordinate_tenant,
            fact_order,
        } => {
            if &coordinate_tenant != tenant_scope_id {
                return Err(StoreError::InvalidFactCoordinate);
            }
            fact_order
        }
        TenantFactCoordinateFields::FactSelectionBarrier {
            tenant_scope_id: coordinate_tenant,
            frontier_fact_order,
        } => {
            if &coordinate_tenant != tenant_scope_id {
                return Err(StoreError::InvalidFactCoordinate);
            }
            frontier_fact_order
        }
    };
    TenantFactFrontier::new(
        store_identity.store_scope_id(),
        store_identity.store_epoch(),
        tenant_scope_id,
        order,
    )
    .map(Some)
    .map_err(Into::into)
}

/// Store-owned summary of one known committed append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedAppend {
    journal_head: JournalHead,
    record_refs: Vec<RecordRef>,
    fact_frontier: Option<TenantFactFrontier>,
}

impl CommittedAppend {
    pub(super) fn from_commit(
        commit: &CommittedJournalCommit,
        fact_frontier: Option<TenantFactFrontier>,
    ) -> Result<Self> {
        let fields = commit.envelope().fields()?;
        let record_refs = commit
            .records()
            .iter()
            .map(|record| record.record_ref(&fields.core.run_id, fields.core.run_sequence))
            .collect::<Result<_>>()?;
        Ok(Self {
            journal_head: commit.envelope().journal_head()?,
            record_refs,
            fact_frontier,
        })
    }

    /// Returns the exact committed physical head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns exact references to every record in ordinal order.
    pub fn record_refs(&self) -> &[RecordRef] {
        &self.record_refs
    }

    /// Returns the exact tenant fact frontier assigned by this commit, when any.
    pub const fn fact_frontier(&self) -> Option<&TenantFactFrontier> {
        self.fact_frontier.as_ref()
    }
}

/// Directly observed admission result.
pub struct NewlyAdmittedRun {
    run_id: RunId,
    tenant_scope_id: TenantScopeId,
    committed: CommittedAppend,
}

impl NewlyAdmittedRun {
    /// Returns the admitted run.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the admitted tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the exact assigned commit summary.
    pub const fn committed(&self) -> &CommittedAppend {
        &self.committed
    }
}

/// Directly observed authorization result carrying the one live-execution permit.
///
/// This value intentionally implements neither `Debug`, `Clone`, `Copy`, nor Serde. Retrying an
/// already committed or outcome-unknown append never reconstructs it.
#[must_use = "the external operation may run only while this fresh authorization is owned"]
pub struct NewlyAppendedAuthorization {
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    authorization_ref: AuthorizationRef,
    authorization: ExternalAccessAuthorized,
    committed: CommittedAppend,
}

impl NewlyAppendedAuthorization {
    /// Returns the tenant whose policy and fact frontier bind the operation.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the exact run.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact committed authorization reference.
    pub const fn authorization_ref(&self) -> &AuthorizationRef {
        &self.authorization_ref
    }

    /// Returns the exact authorized operation.
    pub const fn authorization(&self) -> &ExternalAccessAuthorized {
        &self.authorization
    }

    /// Returns the exact assigned commit summary.
    pub const fn committed(&self) -> &CommittedAppend {
        &self.committed
    }
}

/// Directly observed append variants.
pub enum NewlyAppended {
    /// A new immutable root was directly observed.
    RunAdmitted(NewlyAdmittedRun),
    /// A new transition was directly observed.
    Transition(CommittedAppend),
    /// A new external authorization and its one live-execution permit were directly observed.
    Authorization(Box<NewlyAppendedAuthorization>),
    /// A new linked observation was directly observed.
    Observation(CommittedAppend),
}

/// A deterministic append rejection that does not grant live execution authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendRejection {
    /// The candidate named a stale physical head.
    StaleHead {
        /// Candidate head.
        expected: JournalHead,
        /// Current head.
        actual: Box<JournalHead>,
    },
    /// One append id was reused with different immutable content.
    AppendRequestConflict,
    /// One admission logical key already names different root content.
    AdmissionConflict,
    /// The run cannot accept this post-closure append.
    RunClosed,
}

/// Exhaustive authority-safe result of one append request.
pub enum AppendOutcome {
    /// The writer directly observed this exact newly committed append.
    NewlyAppended(NewlyAppended),
    /// The exact append was already committed; no live-operation authority is recreated.
    AlreadyCommitted(CommittedAppend),
    /// The request was deterministically rejected.
    Rejected(AppendRejection),
    /// Commit visibility is ambiguous; no live-operation authority is returned.
    OutcomeUnknown,
}
