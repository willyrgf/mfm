use super::*;

/// Commit preconditions checked atomically with appending the payload batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitPreconditions {
    /// Required run state.
    pub required_run_state: RequiredRunState,
    /// Logical keys that must not exist before the commit.
    pub required_absent_logical_keys: Vec<LogicalEventKey>,
    /// Logical keys that must exist before the commit.
    pub required_present_logical_keys: Vec<LogicalEventKey>,
    /// Required cell states.
    pub required_cell_states: Vec<CellStatePrecondition>,
    /// Required side-effect states.
    pub required_side_effect_states: Vec<SideEffectStatePrecondition>,
    /// Whether no public-output projection may exist.
    pub required_public_output_absent: bool,
    /// Certified run store authority used by policy-bound and side-effect commits.
    pub certified_run_authority: Option<CertifiedRunStoreAuthority>,
}

/// Payload-level typed commit request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    /// Run id to append to.
    pub(super) run_id: RunId,
    /// Caller's expected next store-owned stream sequence.
    pub(super) expected_next_seq: StreamSeq,
    /// Commit key for idempotency.
    pub(super) commit_key: CommitKey,
    /// Ordered typed event payloads.
    pub(super) payloads: Vec<KernelEventPayload>,
    /// Artifact evidence refs required for the commit.
    pub(super) required_artifacts: Vec<ArtifactEvidenceRef>,
    /// Atomic commit preconditions.
    pub(super) preconditions: CommitPreconditions,
}

impl CommitRequest {
    /// Creates a typed commit request after validating the raw payload collection.
    pub fn from_payloads(
        run_id: RunId,
        expected_next_seq: StreamSeq,
        commit_key: CommitKey,
        payloads: Vec<KernelEventPayload>,
        required_artifacts: Vec<ArtifactEvidenceRef>,
        preconditions: CommitPreconditions,
    ) -> Result<Self> {
        if payloads.is_empty() {
            return Err(StoreError::EmptyCommit);
        }
        validate_payload_run_and_spec(&run_id, &payloads)?;
        Ok(Self {
            run_id,
            expected_next_seq,
            commit_key,
            payloads,
            required_artifacts,
            preconditions,
        })
    }

    /// Returns the run id to append to.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the caller's expected next store-owned stream sequence.
    pub fn expected_next_seq(&self) -> StreamSeq {
        self.expected_next_seq
    }

    /// Returns the commit key for idempotency.
    pub fn commit_key(&self) -> &CommitKey {
        &self.commit_key
    }

    /// Returns the ordered typed event payloads.
    pub fn payloads(&self) -> &[KernelEventPayload] {
        &self.payloads
    }

    /// Returns artifact evidence refs required for the commit.
    pub fn required_artifacts(&self) -> &[ArtifactEvidenceRef] {
        &self.required_artifacts
    }

    /// Returns atomic commit preconditions.
    pub fn preconditions(&self) -> &CommitPreconditions {
        &self.preconditions
    }

    /// Returns this request with a different expected next sequence.
    pub fn with_expected_next_seq(mut self, expected_next_seq: StreamSeq) -> Self {
        self.expected_next_seq = expected_next_seq;
        self
    }

    /// Returns this request with different required artifact evidence refs.
    pub fn with_required_artifacts(mut self, required_artifacts: Vec<ArtifactEvidenceRef>) -> Self {
        self.required_artifacts = required_artifacts;
        self
    }

    /// Returns this request with different atomic preconditions.
    pub fn with_preconditions(mut self, preconditions: CommitPreconditions) -> Self {
        self.preconditions = preconditions;
        self
    }
}

/// Artifact evidence bound to a prepared commit constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitArtifactEvidenceSet {
    pub(super) required_artifacts: Vec<ArtifactEvidenceRef>,
    pub(super) admitted_artifacts: Vec<ArtifactEvidenceRef>,
}

impl CommitArtifactEvidenceSet {
    /// Creates the required and admitted artifact evidence set for a commit.
    pub fn new(
        required_artifacts: Vec<ArtifactEvidenceRef>,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    ) -> Result<Self> {
        validate_unique_artifact_evidence("required", &required_artifacts)?;
        validate_unique_artifact_evidence("admitted", &admitted_artifacts)?;
        Ok(Self {
            required_artifacts,
            admitted_artifacts,
        })
    }

    /// Creates an empty artifact evidence set.
    pub fn empty() -> Self {
        Self {
            required_artifacts: Vec::new(),
            admitted_artifacts: Vec::new(),
        }
    }

    /// Returns the required artifact evidence refs.
    pub fn required_artifacts(&self) -> &[ArtifactEvidenceRef] {
        &self.required_artifacts
    }

    /// Returns the artifact evidence refs to admit atomically.
    pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        &self.admitted_artifacts
    }
}

/// Sealed purpose marker for a run-admission commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunAdmission;

/// Sealed purpose marker for a standalone state-attempt-start commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateAttemptStarted;

/// Sealed purpose marker for an attempt-terminal commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptTerminal;

/// Sealed purpose marker for a side-effect terminal commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectTerminal;

/// Sealed purpose marker for a side-effect progress commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectProgress;

/// Sealed purpose marker for a retention projection commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention;

/// Sealed purpose marker for a manual-resolution commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManualResolution;

/// Sealed purpose marker for a saga terminal-resolution commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SagaTerminal;

/// Commit purpose implemented only by store-owned marker types.
pub trait CommitPurpose: private::Sealed {
    /// Stable purpose name for diagnostics.
    const NAME: &'static str;
}

macro_rules! impl_commit_purpose {
    ($purpose:ty, $name:literal) => {
        impl private::Sealed for $purpose {}
        impl CommitPurpose for $purpose {
            const NAME: &'static str = $name;
        }
    };
}

impl_commit_purpose!(RunAdmission, "run_admission");
impl_commit_purpose!(StateAttemptStarted, "state_attempt_started");
impl_commit_purpose!(AttemptTerminal, "attempt_terminal");
impl_commit_purpose!(SideEffectTerminal, "side_effect_terminal");
impl_commit_purpose!(SideEffectProgress, "side_effect_progress");
impl_commit_purpose!(Retention, "retention");
impl_commit_purpose!(ManualResolution, "manual_resolution");
impl_commit_purpose!(SagaTerminal, "saga_terminal");

/// Purpose-specific prepared commit authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommit<Purpose: CommitPurpose> {
    inner: PreparedCommitInner,
    _purpose: PhantomData<Purpose>,
}

impl<Purpose: CommitPurpose> PreparedCommit<Purpose> {
    fn prepare_with_authority(
        request: CommitRequest,
        artifacts: CommitArtifactEvidenceSet,
        validate: impl FnOnce(&CommitRequest) -> Result<()>,
        allow_saga_terminal: bool,
        allow_manual_resolution: bool,
    ) -> Result<Self> {
        if artifacts.required_artifacts != request.required_artifacts {
            return Err(invalid_prepared_commit_purpose(
                Purpose::NAME,
                "required artifact evidence set does not match request",
            ));
        }
        validate_required_artifacts_cover_payload_references(Purpose::NAME, &request)?;
        reject_store_materialized_resource_lane_payloads(Purpose::NAME, &request)?;
        validate(&request)?;
        validate_fact_response_artifact_admissions(&request, &artifacts.admitted_artifacts)?;
        let inner = PreparedCommitInner::new_with_authority(
            request,
            artifacts.admitted_artifacts,
            allow_saga_terminal,
            allow_manual_resolution,
        )?;
        Ok(Self {
            inner,
            _purpose: PhantomData,
        })
    }

    fn prepare_with(
        request: CommitRequest,
        artifacts: CommitArtifactEvidenceSet,
        validate: impl FnOnce(&CommitRequest) -> Result<()>,
    ) -> Result<Self> {
        Self::prepare_with_authority(request, artifacts, validate, false, false)
    }

    /// Returns the typed request sealed into this prepared commit.
    pub fn request(&self) -> &CommitRequest {
        self.inner.request()
    }

    /// Returns artifact evidence to admit atomically with the event batch.
    pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        self.inner.admitted_artifacts()
    }

    fn inner(&self) -> &PreparedCommitInner {
        &self.inner
    }
}

macro_rules! impl_prepared_commit_new {
    ($purpose:ty, $doc:literal, $validator:path) => {
        impl PreparedCommit<$purpose> {
            #[doc = $doc]
            pub fn new(
                request: CommitRequest,
                artifacts: CommitArtifactEvidenceSet,
            ) -> Result<Self> {
                Self::prepare_with(request, artifacts, $validator)
            }
        }
    };
}

impl_prepared_commit_new!(
    RunAdmission,
    "Prepares a run-admission commit.",
    validate_run_start_commit
);
impl_prepared_commit_new!(
    StateAttemptStarted,
    "Prepares a standalone state-attempt-start commit.",
    validate_state_attempt_started_commit
);
impl_prepared_commit_new!(
    AttemptTerminal,
    "Prepares an attempt-terminal commit.",
    validate_attempt_terminal_commit
);
impl_prepared_commit_new!(
    SideEffectTerminal,
    "Prepares a side-effect terminal commit.",
    validate_side_effect_terminal_commit
);
impl_prepared_commit_new!(
    SideEffectProgress,
    "Prepares a side-effect progress commit.",
    validate_side_effect_progress_commit
);
impl_prepared_commit_new!(
    Retention,
    "Prepares a retention projection commit.",
    validate_retention_commit
);
impl PreparedCommit<ManualResolution> {
    /// Prepares a proof-backed manual-resolution commit.
    pub fn new(
        request: CommitRequest,
        artifacts: CommitArtifactEvidenceSet,
        proof: &VerifiedManualResolutionForPrefix,
    ) -> Result<Self> {
        Self::prepare_with_authority(
            request,
            artifacts,
            |request| validate_manual_resolution_commit_with_proof(request, proof),
            false,
            true,
        )
    }
}

impl PreparedCommit<SagaTerminal> {
    /// Prepares a saga terminal-resolution commit.
    pub fn new(
        request: CommitRequest,
        artifacts: CommitArtifactEvidenceSet,
        proof: &SagaTerminalProof,
    ) -> Result<Self> {
        Self::prepare_with_authority(
            request,
            artifacts,
            |request| validate_saga_terminal_commit_with_proof(request, proof),
            true,
            false,
        )
    }
}

/// Production prepared commit plan accepted by store mutation APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedCommitPlan {
    /// Run-admission commit plan.
    RunAdmission(PreparedCommit<RunAdmission>),
    /// State-attempt-start commit plan.
    StateAttemptStarted(PreparedCommit<StateAttemptStarted>),
    /// Attempt-terminal commit plan.
    AttemptTerminal(PreparedCommit<AttemptTerminal>),
    /// Side-effect terminal commit plan.
    SideEffectTerminal(PreparedCommit<SideEffectTerminal>),
    /// Side-effect progress commit plan.
    SideEffectProgress(PreparedCommit<SideEffectProgress>),
    /// Retention projection commit plan.
    Retention(PreparedCommit<Retention>),
    /// Manual-resolution commit plan.
    ManualResolution(PreparedCommit<ManualResolution>),
    /// Saga terminal-resolution commit plan.
    SagaTerminal(PreparedCommit<SagaTerminal>),
}

impl PreparedCommitPlan {
    /// Stable prepared commit purpose name.
    pub fn purpose_name(&self) -> &'static str {
        match self {
            Self::RunAdmission(_) => RunAdmission::NAME,
            Self::StateAttemptStarted(_) => StateAttemptStarted::NAME,
            Self::AttemptTerminal(_) => AttemptTerminal::NAME,
            Self::SideEffectTerminal(_) => SideEffectTerminal::NAME,
            Self::SideEffectProgress(_) => SideEffectProgress::NAME,
            Self::Retention(_) => Retention::NAME,
            Self::ManualResolution(_) => ManualResolution::NAME,
            Self::SagaTerminal(_) => SagaTerminal::NAME,
        }
    }

    /// Returns the sealed request for read-only planning decisions.
    pub fn request(&self) -> &CommitRequest {
        match self {
            Self::RunAdmission(commit) => commit.request(),
            Self::StateAttemptStarted(commit) => commit.request(),
            Self::AttemptTerminal(commit) => commit.request(),
            Self::SideEffectTerminal(commit) => commit.request(),
            Self::SideEffectProgress(commit) => commit.request(),
            Self::Retention(commit) => commit.request(),
            Self::ManualResolution(commit) => commit.request(),
            Self::SagaTerminal(commit) => commit.request(),
        }
    }

    /// Returns artifact evidence to admit atomically with the event batch.
    pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        self.inner().admitted_artifacts()
    }

    fn inner(&self) -> &PreparedCommitInner {
        match self {
            Self::RunAdmission(commit) => commit.inner(),
            Self::StateAttemptStarted(commit) => commit.inner(),
            Self::AttemptTerminal(commit) => commit.inner(),
            Self::SideEffectTerminal(commit) => commit.inner(),
            Self::SideEffectProgress(commit) => commit.inner(),
            Self::Retention(commit) => commit.inner(),
            Self::ManualResolution(commit) => commit.inner(),
            Self::SagaTerminal(commit) => commit.inner(),
        }
    }
}

macro_rules! impl_prepared_commit_plan_from {
    ($purpose:ty, $variant:ident) => {
        impl From<PreparedCommit<$purpose>> for PreparedCommitPlan {
            fn from(commit: PreparedCommit<$purpose>) -> Self {
                Self::$variant(commit)
            }
        }
    };
}

impl_prepared_commit_plan_from!(RunAdmission, RunAdmission);
impl_prepared_commit_plan_from!(StateAttemptStarted, StateAttemptStarted);
impl_prepared_commit_plan_from!(AttemptTerminal, AttemptTerminal);
impl_prepared_commit_plan_from!(SideEffectTerminal, SideEffectTerminal);
impl_prepared_commit_plan_from!(SideEffectProgress, SideEffectProgress);
impl_prepared_commit_plan_from!(Retention, Retention);
impl_prepared_commit_plan_from!(ManualResolution, ManualResolution);
impl_prepared_commit_plan_from!(SagaTerminal, SagaTerminal);

/// Verified artifact bytes carried by a prepared commit bundle.
///
/// This proof object is the only production path for new artifact bytes to accompany run
/// authority. Construction verifies content addressing and typed evidence before storage is
/// called; durable stores must verify the same facts again inside their append transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedArtifactBytes {
    pub(super) bytes: Vec<u8>,
    pub(super) evidence: ArtifactEvidenceRef,
    pub(super) evidence_hash: ContentDigest,
}

impl PreparedArtifactBytes {
    /// Verifies artifact bytes against exact typed evidence and returns a proof object.
    pub fn new(bytes: Vec<u8>, evidence: ArtifactEvidenceRef) -> Result<Self> {
        verify_retained_artifact_bytes(&bytes, &evidence)?;
        let evidence_hash = evidence.evidence_hash()?;
        Ok(Self {
            bytes,
            evidence,
            evidence_hash,
        })
    }

    /// Returns verified artifact bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns exact typed artifact evidence.
    pub fn evidence(&self) -> &ArtifactEvidenceRef {
        &self.evidence
    }

    /// Returns canonical evidence hash for exact-evidence authority.
    pub fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
    }

    /// Consumes this proof object into its verified parts.
    pub fn into_parts(self) -> (Vec<u8>, ArtifactEvidenceRef, ContentDigest) {
        (self.bytes, self.evidence, self.evidence_hash)
    }
}

/// Exact existing artifact evidence reused by a prepared commit bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingArtifactAdmission {
    pub(super) artifact_id: ArtifactId,
    pub(super) evidence_hash: ContentDigest,
}

impl ExistingArtifactAdmission {
    /// Creates an exact-evidence existing artifact admission reference.
    pub fn new(artifact_id: ArtifactId, evidence_hash: ContentDigest) -> Self {
        Self {
            artifact_id,
            evidence_hash,
        }
    }

    /// Artifact id being reused.
    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// Canonical evidence hash required for reuse.
    pub fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
    }
}

/// Production append authority: a prepared commit plus exact artifact byte/evidence material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommitBundle {
    pub(super) plan: PreparedCommitPlan,
    pub(super) artifact_bytes: Vec<PreparedArtifactBytes>,
    pub(super) existing_artifacts: Vec<ExistingArtifactAdmission>,
    pub(super) execution_claim: Option<PreparedExecutionClaim>,
}

impl PreparedCommitBundle {
    /// Builds a prepared commit bundle and verifies exact artifact coverage.
    pub fn new(
        plan: PreparedCommitPlan,
        artifact_bytes: Vec<PreparedArtifactBytes>,
        existing_artifacts: Vec<ExistingArtifactAdmission>,
    ) -> Result<Self> {
        validate_prepared_bundle_artifacts(&plan, &artifact_bytes, &existing_artifacts)?;
        Ok(Self {
            plan,
            artifact_bytes,
            existing_artifacts,
            execution_claim: None,
        })
    }

    /// Attaches an execution claim that must be acquired atomically with run admission.
    pub fn with_execution_claim(mut self, claim: PreparedExecutionClaim) -> Result<Self> {
        let PreparedCommitPlan::RunAdmission(commit) = &self.plan else {
            return Err(invalid_prepared_commit_purpose(
                "prepared_commit_bundle",
                "execution claims may only be attached to run admission commits",
            ));
        };
        let run_admitted = commit
            .request()
            .payloads()
            .iter()
            .find_map(|payload| match payload {
                KernelEventPayload::RunAdmitted(payload) => Some(payload),
                _ => None,
            })
            .ok_or_else(|| {
                invalid_prepared_commit_purpose(
                    "prepared_commit_bundle",
                    "execution claim run admission commit lacks RunAdmitted payload",
                )
            })?;
        if claim.holder_run_id != run_admitted.run_id {
            return Err(invalid_prepared_commit_purpose(
                "prepared_commit_bundle",
                "execution claim holder run id does not match RunAdmitted",
            ));
        }
        let expected_scope =
            ExecutionClaimScope::from_run_identity_material(&run_admitted.identity_material);
        if claim.scope != expected_scope {
            return Err(invalid_prepared_commit_purpose(
                "prepared_commit_bundle",
                "execution claim scope does not match RunAdmitted identity material",
            ));
        }
        self.execution_claim = Some(claim);
        Ok(self)
    }

    /// Builds a zero-artifact bundle for commit plans that admit no artifact evidence.
    pub fn without_artifacts(plan: PreparedCommitPlan) -> Result<Self> {
        Self::new(plan, Vec::new(), Vec::new())
    }

    /// Returns the prepared commit plan sealed into the bundle.
    pub fn plan(&self) -> &PreparedCommitPlan {
        &self.plan
    }

    /// Returns the sealed request for read-only planning decisions.
    pub fn request(&self) -> &CommitRequest {
        self.plan.request()
    }

    /// Returns artifact evidence to admit atomically with the event batch.
    pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        self.plan.admitted_artifacts()
    }

    /// Returns verified artifact bytes carried by the bundle.
    pub fn artifact_bytes(&self) -> &[PreparedArtifactBytes] {
        &self.artifact_bytes
    }

    /// Returns exact existing artifact admissions carried by the bundle.
    pub fn existing_artifacts(&self) -> &[ExistingArtifactAdmission] {
        &self.existing_artifacts
    }

    /// Returns the execution claim that must be acquired with this bundle, if any.
    pub fn execution_claim(&self) -> Option<&PreparedExecutionClaim> {
        self.execution_claim.as_ref()
    }
}

/// Execution-claim material attached to one run admission bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedExecutionClaim {
    /// Base work scope for the execution lane.
    pub scope: ExecutionClaimScope,
    /// Concrete run admitted as holder.
    pub holder_run_id: RunId,
    /// Holder token.
    pub token: AdmissionToken,
}

impl PreparedExecutionClaim {
    /// Builds execution-claim material for a run admission bundle.
    pub fn new(scope: ExecutionClaimScope, holder_run_id: RunId, token: AdmissionToken) -> Self {
        Self {
            scope,
            holder_run_id,
            token,
        }
    }
}

fn validate_prepared_bundle_artifacts(
    plan: &PreparedCommitPlan,
    artifact_bytes: &[PreparedArtifactBytes],
    existing_artifacts: &[ExistingArtifactAdmission],
) -> Result<()> {
    let mut admitted = BTreeMap::<(ArtifactId, ContentDigest), &ArtifactEvidenceRef>::new();
    for evidence in plan.admitted_artifacts() {
        admitted.insert(
            (evidence.artifact_id.clone(), evidence.evidence_hash()?),
            evidence,
        );
    }

    let mut covered = BTreeSet::<(ArtifactId, ContentDigest)>::new();
    for artifact in artifact_bytes {
        let key = (
            artifact.evidence().artifact_id.clone(),
            artifact.evidence_hash().clone(),
        );
        if !admitted.contains_key(&key) {
            return Err(StoreError::ExtraPreparedArtifactBytes {
                artifact_id: artifact.evidence().artifact_id.clone(),
            });
        }
        if !covered.insert(key) {
            return Err(StoreError::DuplicatePreparedArtifactBytes {
                artifact_id: artifact.evidence().artifact_id.clone(),
            });
        }
    }

    for existing in existing_artifacts {
        let key = (
            existing.artifact_id().clone(),
            existing.evidence_hash().clone(),
        );
        if !admitted.contains_key(&key) {
            return Err(StoreError::ExtraPreparedArtifactBytes {
                artifact_id: existing.artifact_id().clone(),
            });
        }
        if !covered.insert(key) {
            return Err(StoreError::DuplicatePreparedArtifactBytes {
                artifact_id: existing.artifact_id().clone(),
            });
        }
    }

    for (artifact_id, evidence_hash) in admitted.keys() {
        if !covered.contains(&(artifact_id.clone(), evidence_hash.clone())) {
            return Err(StoreError::MissingPreparedArtifactBytes {
                artifact_id: artifact_id.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedCommitInner {
    pub(super) request: CommitRequest,
    pub(super) admitted_artifacts: Vec<ArtifactEvidenceRef>,
}

impl PreparedCommitInner {
    fn new_with_authority(
        request: CommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
        allow_saga_terminal: bool,
        allow_manual_resolution: bool,
    ) -> Result<Self> {
        if !allow_saga_terminal && request_contains_saga_terminal_outcome(&request) {
            return Err(invalid_prepared_commit_purpose(
                SagaTerminal::NAME,
                "saga terminal resolution requires SagaTerminalProof",
            ));
        }
        if !allow_manual_resolution && request_contains_manual_resolution(&request) {
            return Err(invalid_prepared_commit_purpose(
                ManualResolution::NAME,
                "manual resolution requires verified manual resolution proof",
            ));
        }
        let referenced_artifacts = referenced_artifact_ids(&request);
        for evidence in &request.required_artifacts {
            if !referenced_artifacts.contains(&evidence.artifact_id) {
                return Err(StoreError::UnreferencedArtifactEvidence {
                    artifact_id: evidence.artifact_id.clone(),
                });
            }
        }
        let mut deduped = BTreeMap::<ArtifactAuthorityKey, ArtifactEvidenceRef>::new();
        for evidence in admitted_artifacts {
            if !referenced_artifacts.contains(&evidence.artifact_id) {
                return Err(StoreError::UnreferencedArtifactEvidence {
                    artifact_id: evidence.artifact_id,
                });
            }
            let key = artifact_authority_key(&evidence)?;
            if let Some(existing) = deduped.get(&key) {
                if existing != &evidence {
                    return Err(StoreError::ArtifactEvidenceMismatch {
                        artifact_id: evidence.artifact_id,
                        field: "artifact",
                    });
                }
                continue;
            }
            deduped.insert(key, evidence);
        }

        Ok(Self {
            request,
            admitted_artifacts: deduped.into_values().collect(),
        })
    }

    fn request(&self) -> &CommitRequest {
        &self.request
    }

    fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        &self.admitted_artifacts
    }
}

/// Batch of events committed atomically by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedBatch {
    pub(super) run_id: RunId,
    pub(super) commit_key: CommitKey,
    pub(super) fingerprint: CommitFingerprint,
    pub(super) seq: StreamSeq,
    pub(super) store_commit_order: StoreCommitOrder,
    pub(super) events: Vec<KernelEventEnvelope>,
}

impl CommittedBatch {
    /// Run id appended by this batch.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Commit key appended by this batch.
    pub fn commit_key(&self) -> &CommitKey {
        &self.commit_key
    }

    /// Canonical commit fingerprint.
    pub fn fingerprint(&self) -> &CommitFingerprint {
        &self.fingerprint
    }

    /// Store-owned sequence for this atomic commit.
    pub const fn seq(&self) -> StreamSeq {
        self.seq
    }

    /// Store-wide append coordinate for this atomic commit.
    pub const fn store_commit_order(&self) -> StoreCommitOrder {
        self.store_commit_order
    }

    /// Store-owned envelopes created for this batch.
    pub fn events(&self) -> &[KernelEventEnvelope] {
        &self.events
    }

    /// Reconstructs one committed batch from strict-loaded durable event rows.
    pub fn from_persisted_events(
        run_id: RunId,
        commit_key: CommitKey,
        fingerprint: CommitFingerprint,
        seq: StreamSeq,
        store_commit_order: StoreCommitOrder,
        events: Vec<KernelEventEnvelope>,
    ) -> Result<Self> {
        if events.is_empty() {
            return Err(StoreError::PersistedEventMismatch {
                field: "event_count",
                message: "persisted commit has no events".to_owned(),
            });
        }
        for (index, event) in events.iter().enumerate() {
            if event.run_id() != &run_id {
                return Err(StoreError::PersistedEventMismatch {
                    field: "run_id",
                    message: "persisted commit event has a different run id".to_owned(),
                });
            }
            if event.seq() != seq {
                return Err(StoreError::PersistedEventMismatch {
                    field: "seq",
                    message: "persisted commit event has a different sequence".to_owned(),
                });
            }
            if event.store_commit_order() != store_commit_order {
                return Err(StoreError::PersistedEventMismatch {
                    field: "store_commit_order",
                    message: "persisted commit event has a different store append coordinate"
                        .to_owned(),
                });
            }
            if event.commit_key() != &commit_key {
                return Err(StoreError::PersistedEventMismatch {
                    field: "commit_key",
                    message: "persisted commit event has a different commit key".to_owned(),
                });
            }
            if event.ordinal().as_u32() as usize != index {
                return Err(StoreError::PersistedEventMismatch {
                    field: "ordinal",
                    message: "persisted commit event ordinals are not contiguous".to_owned(),
                });
            }
        }
        Ok(Self {
            run_id,
            commit_key,
            fingerprint,
            seq,
            store_commit_order,
            events,
        })
    }
}

/// Result of appending a typed commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    /// A new batch was appended.
    Appended(CommittedBatch),
    /// The commit key had already appended the same canonical batch.
    Idempotent(CommittedBatch),
    /// Run admission was skipped because the execution lane already has an active holder.
    ExecutionClaimBusy(Box<NowaitSkipAdmissionBusy>),
    /// A FIFO admission was blocked before domain authority was persisted.
    AdmissionBlocked(Box<WaitFifoAdmissionBlock>),
}
