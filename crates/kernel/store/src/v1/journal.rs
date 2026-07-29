use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::{CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContractV1};
use mfm_capabilities::{SafeFailureClassifierDescriptor, SafeFailureOutcome};
use mfm_executor::{
    verify_ensure_result, verify_reference_safe_failure_tuple, CommittedEffectRequest,
    DeliveryAuditFrontierRef, EffectIdentity, ExecutorBinding, ExecutorBindingRef,
    ExecutorContractDescriptor, ExecutorDeployment, ExecutorEnsureResultClaim,
    ExecutorRetainedClosureClaim, ExecutorRetainedValue, ExecutorRetainedValueRelation,
    ExecutorTerminalEvidenceClaim, ProofBasis, ResourceOwnership, SchemaQualifiedCanonicalValue,
    TerminalTombstoneRef, VerifiedExecutorBinding,
};
use mfm_facts::FactSelectionRequest;
use mfm_ids::{
    ContentRef, EffectKey, EntryPointId, FieldPath, JournalRecordHash, NodeId, RecordId, RunId,
    RunSemanticStateDigest, SemanticTypeId, StableId, TenantScopeId,
};
use mfm_journal::v1::{
    AccessAuditEntry, AccessAuditStatus, ArtifactAdmissionMode, AuthorityUse, AuthorizationRef,
    AuthorizationScopeFields, BatchPurpose, BindingDelta, BindingDeltaEntryFields,
    BlockingDestination, BlockingProducerRequirement, BlockingSource, CandidateRecordEnvelope,
    CapabilityBindingRef, ClosureRef, CommitCandidatePreimage, CommitDigestPreimage,
    CommitEnvelope, ConfigManifest, ConfiguredValueBinding, ConfiguredValueKey, ContextManifest,
    CrossRunSourceManifest, CrossRunSourceRef, CrossRunSourceRefFields, ExecutorEnsureResult,
    ExecutorEnsureResultFields, ExecutorProofBasisFields, ExternalAccessAuthorized,
    ExternalAccessObserved, FactClaimEnvelope, FactContentIdentityPreimage, FactEmission, FactRef,
    FactSelectionResponse, FactSelectionScanAttestation, FactSelectionScanContract,
    FactValueComponent, FrozenReadIntent, InitialBinding, InputManifest, InputManifestRef,
    InputSource, JournalHead, JournalPredecessor, JournalPredecessorFields, LegalCommitBatch,
    NodePhase, NodeSemanticState, NodeTerminalOutcome as JournalNodeTerminalOutcome,
    ObjectPathBinding, ObservationOutcome, ObservationOutcomeFields, ObservationRef, OutputBinding,
    OutputRef, PendingEffectState as JournalPendingEffectState, ProducerBinding,
    ProducerBindingFields, ProducerBindingKind, ReadCapabilityBinding, RecordHashPreimage,
    RecordIdPreimage, RecordRef, RunAdmitted, RunJournalRecordFields, RunPhase,
    RunSemanticStatePreimage, SafeFailure, SeedManifest, SemanticBinding,
    SemanticClosureCoordinate, SettlementFields, StateTransitionCommitted,
    TenantFactCoordinateFields, TenantFactFrontier, TerminalEffectEvidence, TransitionAfter,
    TransitionBefore, TransitionBody, TransitionBodyFields, TransitionRef, TransitionSlot,
    ValueRef,
};
use mfm_spec::v1::{
    CapabilityBindingManifest, Certificate, CertifiedFrameBinding, CertifiedNodeContract,
    CertifiedSourceSelector, CertifiedStateExecution, ExpandedCertifiedSpec, RetainedValueContract,
    StateImplementationManifest,
};

use super::frame_preparation::{
    insert_json_path, select_canonical, select_manifest_value, CONFIGURED_VALUE_PATH,
    RUN_ADMISSION_INPUT_PATH,
};
use super::objects::{derive_value_ref, validate_object_envelope, validate_value_contract};
use super::{
    CommittedObject, ObjectAuthorityKey, Result, StoreError, StoreIdentity, UntrustedObjectPayload,
    FACT_SELECTION_OPERATION_ID,
};

/// One assigned journal record with its exact portable candidate wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedJournalRecord {
    record_id: RecordId,
    record_hash: JournalRecordHash,
    candidate: CandidateRecordEnvelope,
}

impl CommittedJournalRecord {
    pub(super) const fn assigned(
        record_id: RecordId,
        record_hash: JournalRecordHash,
        candidate: CandidateRecordEnvelope,
    ) -> Self {
        Self {
            record_id,
            record_hash,
            candidate,
        }
    }

    /// Reconstructs an untrusted persisted assigned record for a load verifier.
    #[doc(hidden)]
    pub const fn from_persisted(
        record_id: RecordId,
        record_hash: JournalRecordHash,
        candidate: CandidateRecordEnvelope,
    ) -> Self {
        Self::assigned(record_id, record_hash, candidate)
    }

    /// Returns the immutable store-derived record id.
    pub const fn record_id(&self) -> &RecordId {
        &self.record_id
    }

    /// Returns the exact semantic record hash.
    pub const fn record_hash(&self) -> &JournalRecordHash {
        &self.record_hash
    }

    /// Returns the exact persisted candidate envelope.
    ///
    /// Portable replay uses these bytes, rather than payload-only bytes, to rederive the record
    /// hash including ordinal, schema, logical key, and fact-emission marker.
    pub const fn candidate(&self) -> &CandidateRecordEnvelope {
        &self.candidate
    }

    /// Constructs the exact generic reference to this assigned record.
    pub fn record_ref(&self, run_id: &RunId, run_sequence: u64) -> Result<RecordRef> {
        RecordRef::new(
            run_id,
            run_sequence,
            self.candidate.fields()?.ordinal,
            &self.record_hash,
        )
        .map_err(Into::into)
    }
}

/// One complete assigned atomic journal commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedJournalCommit {
    envelope: CommitEnvelope,
    records: Vec<CommittedJournalRecord>,
}

impl CommittedJournalCommit {
    pub(super) const fn assigned(
        envelope: CommitEnvelope,
        records: Vec<CommittedJournalRecord>,
    ) -> Self {
        Self { envelope, records }
    }

    /// Reconstructs one untrusted persisted commit for a load verifier.
    #[doc(hidden)]
    pub const fn from_persisted(
        envelope: CommitEnvelope,
        records: Vec<CommittedJournalRecord>,
    ) -> Self {
        Self::assigned(envelope, records)
    }

    /// Returns the exact assigned commit envelope.
    pub const fn envelope(&self) -> &CommitEnvelope {
        &self.envelope
    }

    /// Returns assigned records in exact ordinal order.
    pub fn records(&self) -> &[CommittedJournalRecord] {
        &self.records
    }
}

/// Store-created verifier for raw rows loaded by one exact authorized read.
///
/// It is non-cloneable and has no public constructor. A durable backend can only complete the
/// load selected by the store's authority check.
pub struct JournalLoadVerifier {
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
}

impl JournalLoadVerifier {
    pub(super) const fn new(
        store_identity: StoreIdentity,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> Self {
        Self {
            store_identity,
            tenant_scope_id,
            run_id,
        }
    }

    /// Returns the exact authorized run a backend must load.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact authorized tenant a backend must load.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Verifies raw immutable rows and reachable object bytes before creating journal authority.
    pub fn verify(
        self,
        commits: Vec<CommittedJournalCommit>,
        objects: Vec<CommittedObject>,
    ) -> Result<CommittedRunJournal> {
        let normalized_batches = verify_physical_journal(
            &self.store_identity,
            &self.tenant_scope_id,
            &self.run_id,
            &commits,
            &objects,
        )?;
        Ok(CommittedRunJournal {
            store_identity: self.store_identity,
            tenant_scope_id: self.tenant_scope_id,
            run_id: self.run_id,
            commits,
            normalized_batches,
            objects: objects
                .into_iter()
                .map(|object| (object.key().clone(), object))
                .collect(),
        })
    }
}

/// Verifies a decoded immutable offline journal and its complete retained-object closure.
///
/// This is the only callback-free offline entry point used by replay and portable-export
/// verification. It creates no store context, access token, append verifier, or live mutation
/// authority. The same physical verifier and semantic reducer used by an authorized backend load
/// are applied to these immutable rows.
pub fn verify_offline_recorded_history(
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    commits: Vec<CommittedJournalCommit>,
    objects: Vec<CommittedObject>,
) -> Result<VerifiedRunView> {
    JournalLoadVerifier::new(store_identity, tenant_scope_id, run_id)
        .verify(commits, objects)?
        .verify_recorded_history()
}

/// Verifies decoded portable rows and untrusted multi-authority object payloads offline.
pub fn verify_offline_recorded_material(
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    commits: Vec<CommittedJournalCommit>,
    object_payloads: Vec<UntrustedObjectPayload>,
) -> Result<VerifiedRunView> {
    let objects = object_payloads
        .into_iter()
        .map(UntrustedObjectPayload::into_committed)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    verify_offline_recorded_history(store_identity, tenant_scope_id, run_id, commits, objects)
}

/// Derives the canonical semantic digest of an admitted but otherwise untouched run.
///
/// Admission builders use this store-owned grammar instead of duplicating the private fold's
/// node-state and typed-binding representation.
pub fn derive_initial_run_state_digest(
    certified_spec: &ExpandedCertifiedSpec,
    _initial_bindings: &[InitialBinding],
) -> Result<RunSemanticStateDigest> {
    let ordered_node_states = certified_spec
        .nodes()
        .iter()
        .map(|node| {
            NodeSemanticState::new(node.node_id(), NodePhase::Unstarted, None)
                .map_err(StoreError::from)
        })
        .collect::<Result<Vec<_>>>()?;
    RunSemanticStatePreimage::new(
        &certified_spec.spec_hash()?,
        RunPhase::Open,
        &ordered_node_states,
        &[],
        &[],
        None,
    )?
    .run_state_digest()
    .map_err(Into::into)
}

/// Complete physically verified, append-only journal for one run.
///
/// The fields and constructor are private and this value is intentionally non-cloneable. Call
/// [`Self::verify_recorded_history`] before using semantic scheduling or replay projections.
pub struct CommittedRunJournal {
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    commits: Vec<CommittedJournalCommit>,
    normalized_batches: Vec<NormalizedBatch>,
    objects: BTreeMap<ObjectAuthorityKey, CommittedObject>,
}

impl CommittedRunJournal {
    /// Returns the exact store lineage.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the admitted tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the admitted run.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns complete commits in contiguous run-sequence order.
    pub fn commits(&self) -> &[CommittedJournalCommit] {
        &self.commits
    }

    /// Returns one exact reachable retained object.
    pub fn object(&self, key: &ObjectAuthorityKey) -> Option<&CommittedObject> {
        self.objects.get(key)
    }

    /// Returns every exact reachable object in canonical authority-key order.
    pub fn objects(&self) -> impl ExactSizeIterator<Item = &CommittedObject> {
        self.objects.values()
    }

    /// Performs the private callback-free semantic fold over every committed prefix.
    pub fn verify_recorded_history(self) -> Result<VerifiedRunView> {
        let (admission, spec) = load_admission_and_spec(&self)?;
        let admission_fields = admission.fields()?;
        let certificate = Certificate::from_canonical_json(retained_content_in_journal(
            &self,
            &admission_fields.certificate_ref,
        )?)?;
        if certificate.spec_hash() != &admission_fields.spec_hash
            || certificate.certified_spec_ref() != &admission_fields.certified_spec_ref
        {
            return Err(StoreError::PersistedMismatch {
                field: "certificate",
            });
        }
        let state_implementation_manifest =
            StateImplementationManifest::from_canonical_json(retained_content_in_journal(
                &self,
                &admission_fields.state_implementation_manifest_ref,
            )?)?;
        validate_state_manifest(&spec, &state_implementation_manifest)?;
        let capability_binding_manifest = CapabilityBindingManifest::from_canonical_json(
            retained_content_in_journal(&self, &admission_fields.capability_binding_manifest_ref)?,
        )?;
        let config_manifest = ConfigManifest::strict_decode(retained_content_in_journal(
            &self,
            &admission_fields.config_manifest_ref,
        )?)?;
        let seed_manifest = SeedManifest::strict_decode(retained_content_in_journal(
            &self,
            &admission_fields.seed_manifest_ref,
        )?)?;
        let context_manifest = ContextManifest::strict_decode(retained_content_in_journal(
            &self,
            &admission_fields.context_manifest_ref,
        )?)?;
        let cross_run_source_manifest = CrossRunSourceManifest::strict_decode(
            retained_content_in_journal(&self, &admission_fields.cross_run_source_manifest_ref)?,
        )?;
        let (admission_source_requirements, recorded_configured_value) = self
            .verify_admission_source_requirements(
                &admission,
                &spec,
                &config_manifest,
                &seed_manifest,
                &context_manifest,
                &cross_run_source_manifest,
            )?;
        let fold = FoldEngine::replay(&self, &admission, &spec, &capability_binding_manifest)?;
        Ok(VerifiedRunView {
            journal: self,
            admission,
            certified_spec: spec,
            certificate,
            state_implementation_manifest,
            capability_binding_manifest,
            config_manifest,
            seed_manifest,
            context_manifest,
            cross_run_source_manifest,
            admission_source_requirements,
            recorded_configured_value,
            fold,
        })
    }

    fn verify_admission_source_requirements(
        &self,
        admission: &RunAdmitted,
        spec: &ExpandedCertifiedSpec,
        config_manifest: &ConfigManifest,
        seed_manifest: &SeedManifest,
        context_manifest: &ContextManifest,
        cross_run_manifest: &CrossRunSourceManifest,
    ) -> Result<(
        VerifiedAdmissionSourceRequirements,
        VerifiedRecordedConfiguredValue,
    )> {
        verify_admission_source_requirements(
            self,
            admission,
            spec,
            config_manifest,
            seed_manifest,
            context_manifest,
            cross_run_manifest,
        )
    }
}

/// One folded transition and its exact physical placement.
#[derive(Clone)]
pub struct FoldedTransitionEntry {
    transition_ref: TransitionRef,
    transition: StateTransitionCommitted,
    containing_journal_head: JournalHead,
    closure_ref: Option<ClosureRef>,
}

impl FoldedTransitionEntry {
    /// Returns the exact transition record reference.
    pub const fn transition_ref(&self) -> &TransitionRef {
        &self.transition_ref
    }

    /// Returns the complete committed transition.
    pub const fn transition(&self) -> &StateTransitionCommitted {
        &self.transition
    }

    /// Returns the physical head of the commit containing this transition.
    pub const fn containing_journal_head(&self) -> &JournalHead {
        &self.containing_journal_head
    }

    /// Returns the inseparable closure record when this was the terminal transition.
    pub const fn closure_ref(&self) -> Option<&ClosureRef> {
        self.closure_ref.as_ref()
    }
}

/// One folded authorization, optional observation, and safe audit projection.
#[derive(Clone)]
pub struct FoldedAccessAuditEntry {
    authorization_ref: AuthorizationRef,
    authorization: ExternalAccessAuthorized,
    authorization_journal_head: JournalHead,
    observation: Option<(ObservationRef, ExternalAccessObserved, JournalHead)>,
    capability_binding_ref: CapabilityBindingRef,
    capability_operation_id: StableId,
    request_ref: ValueRef,
    status: AccessAuditStatus,
    result_ref: Option<ValueRef>,
    failure: Option<SafeFailure>,
    effect_key: Option<EffectKey>,
    delivery_audit_ref: Option<ValueRef>,
    audit_entry: AccessAuditEntry,
    delivery_audit_terminal: Option<bool>,
}

impl FoldedAccessAuditEntry {
    /// Returns the exact authorization reference.
    pub const fn authorization_ref(&self) -> &AuthorizationRef {
        &self.authorization_ref
    }

    /// Returns the complete authorization.
    pub const fn authorization(&self) -> &ExternalAccessAuthorized {
        &self.authorization
    }

    /// Returns the physical head at which authorization became committed.
    pub const fn authorization_journal_head(&self) -> &JournalHead {
        &self.authorization_journal_head
    }

    /// Returns the optional exact observation and its containing physical head.
    pub fn observation(&self) -> Option<(&ObservationRef, &ExternalAccessObserved, &JournalHead)> {
        self.observation
            .as_ref()
            .map(|(reference, observation, head)| (reference, observation, head))
    }

    /// Returns the redaction-safe audit projection validated from exact journal evidence.
    pub const fn audit_entry(&self) -> &AccessAuditEntry {
        &self.audit_entry
    }

    /// Returns whether a returned executor audit is terminal.
    ///
    /// `None` identifies an unobserved authorization or a non-executor observation.
    pub const fn delivery_audit_terminal(&self) -> Option<bool> {
        self.delivery_audit_terminal
    }
}

#[derive(Clone)]
pub(super) struct AccessAuditProjection {
    authorization_ref: AuthorizationRef,
    observation_ref: Option<ObservationRef>,
    authorization_journal_head: JournalHead,
    observation_journal_head: Option<JournalHead>,
    capability_binding_ref: CapabilityBindingRef,
    capability_operation_id: StableId,
    request_ref: ValueRef,
    status: AccessAuditStatus,
    result_ref: Option<ValueRef>,
    failure: Option<SafeFailure>,
    effect_key: Option<EffectKey>,
    delivery_audit_ref: Option<ValueRef>,
    delivery_audit_terminal: Option<bool>,
}

/// One redaction-safe audit projection verified exactly as of a physical journal head.
///
/// The entry can only be borrowed from a store-created [`super::VerifiedAccessAuditPage`].
pub struct VerifiedAccessAuditEntry<'page> {
    entry: VerifiedAccessAuditSource<'page>,
}

enum VerifiedAccessAuditSource<'page> {
    Page(&'page AccessAuditProjection),
    Folded(&'page FoldedAccessAuditEntry),
}

impl<'page> VerifiedAccessAuditEntry<'page> {
    pub(super) const fn new(entry: &'page AccessAuditProjection) -> Self {
        Self {
            entry: VerifiedAccessAuditSource::Page(entry),
        }
    }

    const fn from_folded(entry: &'page FoldedAccessAuditEntry) -> Self {
        Self {
            entry: VerifiedAccessAuditSource::Folded(entry),
        }
    }

    /// Returns the exact authorization record reference.
    pub const fn authorization_ref(&self) -> &'page AuthorizationRef {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => &entry.authorization_ref,
            VerifiedAccessAuditSource::Folded(entry) => &entry.authorization_ref,
        }
    }

    /// Returns the exact observation record reference when visible at the selected head.
    pub const fn observation_ref(&self) -> Option<&'page ObservationRef> {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => entry.observation_ref.as_ref(),
            VerifiedAccessAuditSource::Folded(entry) => match entry.observation.as_ref() {
                Some((reference, _, _)) => Some(reference),
                None => None,
            },
        }
    }

    /// Returns the physical head containing the authorization.
    pub const fn authorization_journal_head(&self) -> &'page JournalHead {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => &entry.authorization_journal_head,
            VerifiedAccessAuditSource::Folded(entry) => &entry.authorization_journal_head,
        }
    }

    /// Returns the physical head containing the observation when visible at the selected head.
    pub const fn observation_journal_head(&self) -> Option<&'page JournalHead> {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => entry.observation_journal_head.as_ref(),
            VerifiedAccessAuditSource::Folded(entry) => match entry.observation.as_ref() {
                Some((_, _, head)) => Some(head),
                None => None,
            },
        }
    }

    /// Returns the immutable admitted capability binding.
    pub const fn capability_binding_ref(&self) -> &'page CapabilityBindingRef {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => &entry.capability_binding_ref,
            VerifiedAccessAuditSource::Folded(entry) => &entry.capability_binding_ref,
        }
    }

    /// Returns the exact reviewed capability operation.
    pub const fn capability_operation_id(&self) -> &'page StableId {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => &entry.capability_operation_id,
            VerifiedAccessAuditSource::Folded(entry) => &entry.capability_operation_id,
        }
    }

    /// Returns the full retained request authority.
    pub const fn request_ref(&self) -> &'page ValueRef {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => &entry.request_ref,
            VerifiedAccessAuditSource::Folded(entry) => &entry.request_ref,
        }
    }

    /// Returns the closed safe audit status at the selected head.
    pub const fn status(&self) -> AccessAuditStatus {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => entry.status,
            VerifiedAccessAuditSource::Folded(entry) => entry.status,
        }
    }

    /// Returns the full retained result authority for a reviewed return.
    pub const fn result_ref(&self) -> Option<&'page ValueRef> {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => entry.result_ref.as_ref(),
            VerifiedAccessAuditSource::Folded(entry) => entry.result_ref.as_ref(),
        }
    }

    /// Returns the reviewed bounded failure when visible at the selected head.
    pub const fn failure(&self) -> Option<&'page SafeFailure> {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => entry.failure.as_ref(),
            VerifiedAccessAuditSource::Folded(entry) => entry.failure.as_ref(),
        }
    }

    /// Returns the immutable effect key for executor access when present.
    pub const fn effect_key(&self) -> Option<&'page EffectKey> {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => entry.effect_key.as_ref(),
            VerifiedAccessAuditSource::Folded(entry) => entry.effect_key.as_ref(),
        }
    }

    /// Returns the greatest verified executor delivery-audit head when visible.
    pub const fn delivery_audit_ref(&self) -> Option<&'page ValueRef> {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => entry.delivery_audit_ref.as_ref(),
            VerifiedAccessAuditSource::Folded(entry) => entry.delivery_audit_ref.as_ref(),
        }
    }

    /// Returns whether the returned executor delivery audit is terminal.
    ///
    /// `None` identifies an unobserved authorization or a non-executor observation.
    pub const fn delivery_audit_terminal(&self) -> Option<bool> {
        match self.entry {
            VerifiedAccessAuditSource::Page(entry) => entry.delivery_audit_terminal,
            VerifiedAccessAuditSource::Folded(entry) => entry.delivery_audit_terminal,
        }
    }
}

/// Sealed exact access history for one certified external-operation occurrence.
pub struct VerifiedNodeAccessHistory<'view> {
    entries: Vec<&'view FoldedAccessAuditEntry>,
    observed_suffix: Vec<&'view FoldedAccessAuditEntry>,
    authorization_count: u32,
}

impl<'view> VerifiedNodeAccessHistory<'view> {
    /// Returns the number of exact matching committed authorizations, including unobserved gaps.
    pub const fn authorization_count(&self) -> u32 {
        self.authorization_count
    }

    /// Returns all exact attempts in physical authorization order.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = VerifiedAccessAttempt<'view>> + '_ {
        self.entries.iter().copied().map(VerifiedAccessAttempt::new)
    }

    /// Returns the maximal physically observed suffix in forward or reverse order.
    pub fn observed_suffix(
        &self,
    ) -> impl DoubleEndedIterator<Item = VerifiedObservedAccess<'view>> + '_ {
        self.observed_suffix
            .iter()
            .copied()
            .filter_map(VerifiedObservedAccess::from_entry)
    }
}

/// One exact committed external-access authorization and optional observation.
pub struct VerifiedAccessAttempt<'view> {
    entry: &'view FoldedAccessAuditEntry,
}

impl<'view> VerifiedAccessAttempt<'view> {
    const fn new(entry: &'view FoldedAccessAuditEntry) -> Self {
        Self { entry }
    }

    /// Returns the exact authorization record reference.
    pub const fn authorization_ref(&self) -> &'view AuthorizationRef {
        &self.entry.authorization_ref
    }

    /// Returns the complete typed authorization record.
    pub const fn authorization(&self) -> &'view ExternalAccessAuthorized {
        &self.entry.authorization
    }

    /// Returns the physical head containing the authorization.
    pub const fn authorization_journal_head(&self) -> &'view JournalHead {
        &self.entry.authorization_journal_head
    }

    /// Returns the linked typed observation when this attempt is physically observed.
    pub fn observed(&self) -> Option<VerifiedObservedAccess<'view>> {
        VerifiedObservedAccess::from_entry(self.entry)
    }
}

/// One exact committed observation and its redaction-safe audit projection.
pub struct VerifiedObservedAccess<'view> {
    entry: &'view FoldedAccessAuditEntry,
    observation: &'view (ObservationRef, ExternalAccessObserved, JournalHead),
}

impl<'view> VerifiedObservedAccess<'view> {
    fn from_entry(entry: &'view FoldedAccessAuditEntry) -> Option<Self> {
        entry
            .observation
            .as_ref()
            .map(|observation| Self { entry, observation })
    }

    /// Returns the exact observation record reference.
    pub const fn observation_ref(&self) -> &'view ObservationRef {
        &self.observation.0
    }

    /// Returns the complete typed observation record.
    pub const fn observation(&self) -> &'view ExternalAccessObserved {
        &self.observation.1
    }

    /// Returns the physical head containing the observation.
    pub const fn observation_journal_head(&self) -> &'view JournalHead {
        &self.observation.2
    }

    /// Returns the safe typed audit projection derived from the exact records.
    pub const fn audit(&self) -> VerifiedAccessAuditEntry<'view> {
        VerifiedAccessAuditEntry::from_folded(self.entry)
    }
}

#[derive(Clone)]
struct AuthorizationState {
    entry: FoldedAccessAuditEntry,
}

/// Terminal semantic outcome derived for one certified node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTerminalOutcome {
    /// The node committed a successful settlement.
    Succeeded,
    /// The node committed a typed failed settlement.
    Failed,
    /// The node was deterministically skipped by unavailable dependencies.
    Skipped,
}

/// Current audit status of one pending effect's latest ensure authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingEffectStatus {
    /// No ensure authorization has been committed for this pending request.
    NotAuthorized,
    /// The latest authorization has no committed observation.
    AuthorizedUnobserved,
    /// The latest ensure call returned a reviewed result.
    Returned,
    /// The latest ensure call proved that boundary entry did not occur.
    DidNotEnter,
    /// The latest ensure call could not prove whether boundary entry occurred.
    Indeterminate,
}

fn pending_effect_status(status: AccessAuditStatus) -> PendingEffectStatus {
    match status {
        AccessAuditStatus::AuthorizedUnobserved => PendingEffectStatus::AuthorizedUnobserved,
        AccessAuditStatus::Returned => PendingEffectStatus::Returned,
        AccessAuditStatus::DidNotEnter => PendingEffectStatus::DidNotEnter,
        AccessAuditStatus::Indeterminate => PendingEffectStatus::Indeterminate,
    }
}

/// Folded current pending-effect intent and its latest audited executor status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedPendingEffect {
    node_id: NodeId,
    request_transition_ref: TransitionRef,
    input_manifest_ref: InputManifestRef,
    effect_key: mfm_ids::EffectKey,
    semantic_request_ref: ValueRef,
    request_digest: mfm_ids::RequestDigest,
    executor_binding_ref: mfm_journal::v1::CapabilityBindingRef,
    status: PendingEffectStatus,
}

impl FoldedPendingEffect {
    /// Returns the exact certified node occurrence.
    pub const fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the effect-request transition that created this pending state.
    pub const fn request_transition_ref(&self) -> &TransitionRef {
        &self.request_transition_ref
    }

    /// Returns the exact input manifest frozen by the effect request.
    pub const fn input_manifest_ref(&self) -> &InputManifestRef {
        &self.input_manifest_ref
    }

    /// Returns the deterministic effect identity.
    pub const fn effect_key(&self) -> &mfm_ids::EffectKey {
        &self.effect_key
    }

    /// Returns the full retained semantic-request authority.
    pub const fn semantic_request_ref(&self) -> &ValueRef {
        &self.semantic_request_ref
    }

    /// Returns the immutable semantic request digest.
    pub const fn request_digest(&self) -> &mfm_ids::RequestDigest {
        &self.request_digest
    }

    /// Returns the exact certified executor binding.
    pub const fn executor_binding_ref(&self) -> &mfm_journal::v1::CapabilityBindingRef {
        &self.executor_binding_ref
    }

    /// Returns the latest folded ensure status.
    pub const fn status(&self) -> PendingEffectStatus {
        self.status
    }
}

#[derive(Clone)]
struct PendingEffectState {
    request_transition_ref: Option<TransitionRef>,
    input_manifest_ref: InputManifestRef,
    effect_key: mfm_ids::EffectKey,
    semantic_request_ref: ValueRef,
    request_digest: mfm_ids::RequestDigest,
    executor_binding_ref: mfm_journal::v1::CapabilityBindingRef,
}

/// The sole callback-free reducer used for full replay and candidate previews.
struct FoldEngine;

impl FoldEngine {
    fn replay(
        journal: &CommittedRunJournal,
        admission: &RunAdmitted,
        spec: &ExpandedCertifiedSpec,
        capability_binding_manifest: &CapabilityBindingManifest,
    ) -> Result<FoldState> {
        FoldState::rebuild(journal, admission, spec, capability_binding_manifest)
    }

    fn preview_transition(
        current: &FoldState,
        spec: &ExpandedCertifiedSpec,
        node_id: &NodeId,
        body: &TransitionBodyFields,
        binding_delta: &BindingDelta,
        resolve_object: &dyn Fn(&ValueRef) -> Result<Vec<u8>>,
    ) -> Result<FoldState> {
        if current.run_phase == RunPhase::Closed {
            return Err(StoreError::RunClosed);
        }
        let certified_node = certified_node(spec, node_id)?;
        let current_phase = current
            .node_phases
            .get(node_id)
            .copied()
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        let slot = if matches!(body, TransitionBodyFields::EffectRequested { .. }) {
            TransitionSlot::Request
        } else {
            TransitionSlot::Settlement
        };
        let mut next = current.clone();
        if !next.transition_slots.insert((node_id.clone(), slot)) {
            return Err(StoreError::DuplicateLogicalRecord);
        }
        validate_transition_body(
            &mut next,
            node_id,
            current_phase,
            slot,
            body,
            certified_node,
        )?;
        next.apply_binding_delta(node_id, body, binding_delta, spec, None, resolve_object)?;
        next.run_state_digest = next.derive_run_state_digest(spec)?;
        Ok(next)
    }
}

#[derive(Clone)]
struct FoldState {
    journal_head: JournalHead,
    semantic_head: JournalHead,
    semantic_head_record_ref: RecordRef,
    run_phase: RunPhase,
    run_state_digest: RunSemanticStateDigest,
    node_phases: BTreeMap<NodeId, NodePhase>,
    terminal_outcomes: BTreeMap<NodeId, NodeTerminalOutcome>,
    semantic_terminal_outcomes: BTreeMap<NodeId, JournalNodeTerminalOutcome>,
    terminal_transition_refs: BTreeMap<NodeId, TransitionRef>,
    node_outputs: BTreeMap<NodeId, Vec<OutputBinding>>,
    node_facts: BTreeMap<NodeId, Vec<FactEmission>>,
    typed_bindings: Vec<SemanticBinding>,
    pending_effects: BTreeMap<NodeId, PendingEffectState>,
    public_output: Option<ValueRef>,
    transitions: Vec<FoldedTransitionEntry>,
    authorizations: Vec<AuthorizationState>,
    transition_slots: BTreeSet<(NodeId, TransitionSlot)>,
    consumed_observations: BTreeSet<Vec<u8>>,
    semantic_closure: Option<SemanticClosureCoordinate>,
}

struct TransitionApplication<'a> {
    run_id: &'a RunId,
    run_sequence: u64,
    commit: &'a CommittedJournalCommit,
    transition: StateTransitionCommitted,
    closure: Option<mfm_journal::v1::RunClosed>,
    spec: &'a ExpandedCertifiedSpec,
    journal: &'a CommittedRunJournal,
}

struct ObservationApplication<'a> {
    run_id: &'a RunId,
    run_sequence: u64,
    commit: &'a CommittedJournalCommit,
    observation: ExternalAccessObserved,
    spec: &'a ExpandedCertifiedSpec,
    capability_binding_manifest: &'a CapabilityBindingManifest,
    journal: &'a CommittedRunJournal,
}

impl FoldState {
    fn rebuild(
        journal: &CommittedRunJournal,
        admission: &RunAdmitted,
        spec: &ExpandedCertifiedSpec,
        capability_binding_manifest: &CapabilityBindingManifest,
    ) -> Result<Self> {
        let admission_fields = admission.fields()?;
        if spec.spec_hash()? != admission_fields.spec_hash {
            return Err(StoreError::PersistedMismatch {
                field: "certified_spec_hash",
            });
        }
        let first = journal.commits.first().ok_or(StoreError::EmptyJournal)?;
        let first_record = first.records.first().ok_or(StoreError::EmptyJournal)?;
        let semantic_head_record_ref = first_record.record_ref(&admission_fields.run_id, 1)?;
        let mut fold = Self {
            journal_head: first.envelope.journal_head()?,
            semantic_head: first.envelope.journal_head()?,
            semantic_head_record_ref,
            run_phase: RunPhase::Open,
            run_state_digest: admission_fields.initial_run_state_digest.clone(),
            node_phases: spec
                .nodes()
                .iter()
                .map(|node| (node.node_id().clone(), NodePhase::Unstarted))
                .collect(),
            terminal_outcomes: BTreeMap::new(),
            semantic_terminal_outcomes: BTreeMap::new(),
            terminal_transition_refs: BTreeMap::new(),
            node_outputs: BTreeMap::new(),
            node_facts: BTreeMap::new(),
            typed_bindings: Vec::new(),
            pending_effects: BTreeMap::new(),
            public_output: None,
            transitions: Vec::new(),
            authorizations: Vec::new(),
            transition_slots: BTreeSet::new(),
            consumed_observations: BTreeSet::new(),
            semantic_closure: None,
        };
        let derived_initial = fold.derive_run_state_digest(spec)?;
        if derived_initial != admission_fields.initial_run_state_digest {
            return Err(StoreError::TransitionFoldMismatch {
                field: "initial_run_state_digest",
            });
        }

        for (commit, batch) in journal
            .commits
            .iter()
            .zip(&journal.normalized_batches)
            .skip(1)
        {
            fold.apply_commit(commit, batch, spec, capability_binding_manifest, journal)?;
        }
        if fold
            .node_phases
            .values()
            .all(|phase| *phase == NodePhase::Terminal)
            && fold.run_phase != RunPhase::Closed
        {
            return Err(StoreError::InvalidClosure);
        }
        Ok(fold)
    }

    fn derive_run_state_digest(
        &self,
        spec: &ExpandedCertifiedSpec,
    ) -> Result<RunSemanticStateDigest> {
        let ordered_node_states = spec
            .nodes()
            .iter()
            .map(|node| {
                NodeSemanticState::new(
                    node.node_id(),
                    self.node_phases
                        .get(node.node_id())
                        .copied()
                        .unwrap_or(NodePhase::Unstarted),
                    self.semantic_terminal_outcomes.get(node.node_id()),
                )
                .map_err(StoreError::from)
            })
            .collect::<Result<Vec<_>>>()?;
        let pending_effects = spec
            .nodes()
            .iter()
            .filter_map(|node| {
                self.pending_effects
                    .get(node.node_id())
                    .map(|pending| pending_effect_value(node.node_id(), pending))
            })
            .collect::<Result<Vec<_>>>()?;
        RunSemanticStatePreimage::new(
            &spec.spec_hash()?,
            self.run_phase,
            &ordered_node_states,
            &self.typed_bindings,
            &pending_effects,
            self.public_output.as_ref(),
        )?
        .run_state_digest()
        .map_err(Into::into)
    }

    fn apply_commit(
        &mut self,
        commit: &CommittedJournalCommit,
        batch: &NormalizedBatch,
        spec: &ExpandedCertifiedSpec,
        capability_binding_manifest: &CapabilityBindingManifest,
        journal: &CommittedRunJournal,
    ) -> Result<()> {
        let envelope_fields = commit.envelope.fields()?;
        match batch {
            NormalizedBatch::Transition {
                transition,
                closure,
            } => {
                if self.run_phase == RunPhase::Closed {
                    return Err(StoreError::RunClosed);
                }
                self.apply_transition(TransitionApplication {
                    run_id: &envelope_fields.core.run_id,
                    run_sequence: envelope_fields.core.run_sequence,
                    commit,
                    transition: transition.clone(),
                    closure: closure.clone(),
                    spec,
                    journal,
                })?;
            }
            NormalizedBatch::Authorization(authorization) => {
                if self.run_phase == RunPhase::Closed {
                    return Err(StoreError::RunClosed);
                }
                self.apply_authorization(
                    &envelope_fields.core.run_id,
                    envelope_fields.core.run_sequence,
                    commit,
                    authorization.clone(),
                )?;
            }
            NormalizedBatch::Observation(observation) => {
                self.apply_observation(ObservationApplication {
                    run_id: &envelope_fields.core.run_id,
                    run_sequence: envelope_fields.core.run_sequence,
                    commit,
                    observation: observation.clone(),
                    spec,
                    capability_binding_manifest,
                    journal,
                })?;
            }
            NormalizedBatch::Admission(_) => {
                return Err(StoreError::DuplicateLogicalRecord);
            }
        }
        self.journal_head = commit.envelope.journal_head()?;
        Ok(())
    }

    fn apply_transition(&mut self, application: TransitionApplication<'_>) -> Result<()> {
        let TransitionApplication {
            run_id,
            run_sequence,
            commit,
            transition,
            closure,
            spec,
            journal,
        } = application;
        let fields = transition.fields()?;
        let node = certified_node(spec, &fields.node_id)?;
        if fields.spec_hash != spec.spec_hash()?
            || &fields.state_contract_ref != node.state_contract_ref()
        {
            return Err(StoreError::TransitionFoldMismatch {
                field: "certified_node",
            });
        }
        let current_phase = self
            .node_phases
            .get(&fields.node_id)
            .copied()
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        let before = fields.before.fields()?;
        if before.journal_head != self.journal_head {
            return Err(StoreError::TransitionFoldMismatch {
                field: "before_journal_head",
            });
        }
        if before.run_state_digest != self.run_state_digest
            || before.run_phase != self.run_phase
            || before.node_phase != current_phase
        {
            return Err(StoreError::TransitionFoldMismatch {
                field: "before_state",
            });
        }
        if !self
            .transition_slots
            .insert((fields.node_id.clone(), fields.slot))
        {
            return Err(StoreError::DuplicateLogicalRecord);
        }
        let body = fields.body.fields()?;
        validate_transition_body(
            self,
            &fields.node_id,
            current_phase,
            fields.slot,
            &body,
            node,
        )?;
        let after = fields.after.fields()?;

        let transition_record = commit.records.first().ok_or(StoreError::InvalidClosure)?;
        let transition_record_ref = transition_record.record_ref(run_id, run_sequence)?;
        let transition_ref = TransitionRef::new(&transition_record_ref)?;
        self.apply_binding_delta(
            &fields.node_id,
            &body,
            &after.binding_delta,
            spec,
            Some(&transition_ref),
            &|value_ref| {
                let key = ObjectAuthorityKey::from_value_ref(value_ref)?;
                let object = journal.object(&key).ok_or(StoreError::ObjectNotReachable)?;
                if object.value_ref().as_bytes() != value_ref.as_bytes() {
                    return Err(StoreError::InvalidObjectAuthority {
                        message: "retained object metadata disagrees with semantic binding",
                    });
                }
                Ok(object.bytes().to_vec())
            },
        )?;
        let derived_digest = self.derive_run_state_digest(spec)?;
        if after.node_phase
            != self.node_phases.get(&fields.node_id).copied().ok_or(
                StoreError::TransitionFoldMismatch {
                    field: "after_node_phase",
                },
            )?
            || after.run_phase != self.run_phase
            || after.run_state_digest != derived_digest
        {
            return Err(StoreError::TransitionFoldMismatch {
                field: "after_state",
            });
        }
        self.run_state_digest = derived_digest;
        let closure_ref = if commit.records.len() == 2 {
            let closure_record = commit.records.get(1).ok_or(StoreError::InvalidClosure)?;
            Some(ClosureRef::new(
                &closure_record.record_ref(run_id, run_sequence)?,
            )?)
        } else {
            None
        };

        let all_terminal = self
            .node_phases
            .values()
            .all(|phase| *phase == NodePhase::Terminal);
        match (all_terminal, self.run_phase, closure) {
            (true, RunPhase::Closed, Some(closed)) => {
                if commit.records.len() != 2
                    || closed.fields()?.terminal_transition_record_hash
                        != *transition_record.record_hash()
                {
                    return Err(StoreError::InvalidClosure);
                }
                self.semantic_closure = Some(SemanticClosureCoordinate::new(
                    &transition_ref,
                    &commit.envelope.fields()?.commit_digest,
                )?);
            }
            (false, RunPhase::Open, None) => {}
            _ => return Err(StoreError::InvalidClosure),
        }
        self.transitions.push(FoldedTransitionEntry {
            transition_ref: transition_ref.clone(),
            transition,
            containing_journal_head: commit.envelope.journal_head()?,
            closure_ref,
        });
        self.semantic_head_record_ref = transition_record_ref;
        self.semantic_head = commit.envelope.journal_head()?;
        Ok(())
    }

    fn apply_binding_delta(
        &mut self,
        node_id: &NodeId,
        body: &TransitionBodyFields,
        delta: &BindingDelta,
        spec: &ExpandedCertifiedSpec,
        transition_ref: Option<&TransitionRef>,
        resolve_object: &dyn Fn(&ValueRef) -> Result<Vec<u8>>,
    ) -> Result<()> {
        let (expected_phase, terminal_outcome, expected_outputs, expected_facts) = match body {
            TransitionBodyFields::PureSettled { settlement, .. }
            | TransitionBodyFields::ReadSettled { settlement, .. }
            | TransitionBodyFields::EffectSettled { settlement, .. } => {
                match settlement.fields()? {
                    SettlementFields::Succeeded {
                        output_bindings,
                        fact_emissions,
                    } => (
                        NodePhase::Terminal,
                        Some(NodeTerminalOutcome::Succeeded),
                        output_bindings,
                        fact_emissions,
                    ),
                    SettlementFields::Failed { .. } => (
                        NodePhase::Terminal,
                        Some(NodeTerminalOutcome::Failed),
                        Vec::new(),
                        Vec::new(),
                    ),
                }
            }
            TransitionBodyFields::EffectRequested { .. } => {
                (NodePhase::AwaitingEffect, None, Vec::new(), Vec::new())
            }
            TransitionBodyFields::DependencySkipped { .. } => (
                NodePhase::Terminal,
                Some(NodeTerminalOutcome::Skipped),
                Vec::new(),
                Vec::new(),
            ),
        };

        for (expected, binding) in expected_outputs.iter().enumerate() {
            if binding.fields()?.output_ordinal
                != u32::try_from(expected).map_err(|_| StoreError::SequenceOverflow)?
            {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "output_ordinal",
                });
            }
        }
        for (expected, emission) in expected_facts.iter().enumerate() {
            if emission.fields()?.emission_ordinal
                != u32::try_from(expected).map_err(|_| StoreError::SequenceOverflow)?
            {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "fact_emission_ordinal",
                });
            }
        }

        let mut phase_changes = 0_usize;
        let mut outputs = Vec::new();
        let mut facts = Vec::new();
        let mut pending_insert = None;
        let mut pending_remove = None;
        let mut public_output_change = None;
        let mut run_phase_change = None;
        for entry in delta.entries()? {
            match entry.fields()? {
                BindingDeltaEntryFields::NodePhaseChange {
                    node_id: changed,
                    phase,
                } if changed == *node_id && phase == expected_phase => {
                    phase_changes += 1;
                }
                BindingDeltaEntryFields::OutputBinding(binding) => outputs.push(binding),
                BindingDeltaEntryFields::FactBinding(emission) => facts.push(emission),
                BindingDeltaEntryFields::PendingEffectInsert {
                    effect_key,
                    request_digest,
                } => {
                    if pending_insert.is_some() {
                        return Err(StoreError::TransitionFoldMismatch {
                            field: "binding_delta_entry",
                        });
                    }
                    pending_insert = Some((effect_key, request_digest));
                }
                BindingDeltaEntryFields::PendingEffectRemove { effect_key } => {
                    if pending_remove.is_some() {
                        return Err(StoreError::TransitionFoldMismatch {
                            field: "binding_delta_entry",
                        });
                    }
                    pending_remove = Some(effect_key);
                }
                BindingDeltaEntryFields::PublicOutputChange(output) => {
                    if public_output_change.is_some() {
                        return Err(StoreError::TransitionFoldMismatch {
                            field: "binding_delta_entry",
                        });
                    }
                    public_output_change = Some(output);
                }
                BindingDeltaEntryFields::RunPhaseChange(phase) => {
                    if run_phase_change.replace(phase).is_some() {
                        return Err(StoreError::TransitionFoldMismatch {
                            field: "binding_delta_entry",
                        });
                    }
                }
                _ => {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "binding_delta_entry",
                    });
                }
            }
        }
        if phase_changes != 1 || outputs != expected_outputs || facts != expected_facts {
            return Err(StoreError::TransitionFoldMismatch {
                field: "binding_delta_exact_body",
            });
        }

        match body {
            TransitionBodyFields::EffectRequested {
                input_manifest_ref,
                effect_key,
                semantic_request_ref,
                request_digest,
                executor_binding_ref,
            } => {
                if pending_insert.as_ref() != Some(&(effect_key.clone(), request_digest.clone()))
                    || pending_remove.is_some()
                    || self.pending_effects.contains_key(node_id)
                {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "pending_effect_insert",
                    });
                }
                self.pending_effects.insert(
                    node_id.clone(),
                    PendingEffectState {
                        request_transition_ref: transition_ref.cloned(),
                        input_manifest_ref: input_manifest_ref.clone(),
                        effect_key: effect_key.clone(),
                        semantic_request_ref: semantic_request_ref.clone(),
                        request_digest: request_digest.clone(),
                        executor_binding_ref: executor_binding_ref.clone(),
                    },
                );
            }
            TransitionBodyFields::EffectSettled {
                request_transition_ref,
                request_input_manifest_ref,
                ..
            } => {
                let pending = self.pending_effects.get(node_id).ok_or(
                    StoreError::TransitionFoldMismatch {
                        field: "pending_effect_remove",
                    },
                )?;
                if pending.request_transition_ref.as_ref() != Some(request_transition_ref)
                    || pending.input_manifest_ref != *request_input_manifest_ref
                    || pending_remove.as_ref() != Some(&pending.effect_key)
                    || pending_insert.is_some()
                {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "pending_effect_remove",
                    });
                }
                self.pending_effects.remove(node_id);
            }
            TransitionBodyFields::PureSettled { .. }
            | TransitionBodyFields::ReadSettled { .. }
            | TransitionBodyFields::DependencySkipped { .. } => {
                if pending_insert.is_some() || pending_remove.is_some() {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "unexpected_pending_effect_delta",
                    });
                }
            }
        }

        self.node_phases.insert(node_id.clone(), expected_phase);
        if let Some(outcome) = terminal_outcome {
            self.terminal_outcomes.insert(node_id.clone(), outcome);
            let semantic_outcome = match body {
                TransitionBodyFields::PureSettled { settlement, .. }
                | TransitionBodyFields::ReadSettled { settlement, .. }
                | TransitionBodyFields::EffectSettled { settlement, .. } => {
                    match settlement.fields()? {
                        SettlementFields::Succeeded { .. } => {
                            JournalNodeTerminalOutcome::succeeded()?
                        }
                        SettlementFields::Failed { typed_failure_ref } => {
                            JournalNodeTerminalOutcome::failed(&typed_failure_ref)?
                        }
                    }
                }
                TransitionBodyFields::DependencySkipped { blocking_sources } => {
                    JournalNodeTerminalOutcome::skipped(blocking_sources)?
                }
                TransitionBodyFields::EffectRequested { .. } => {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "semantic_terminal_outcome",
                    });
                }
            };
            self.semantic_terminal_outcomes
                .insert(node_id.clone(), semantic_outcome);
            if let Some(reference) = transition_ref {
                self.terminal_transition_refs
                    .insert(node_id.clone(), reference.clone());
            }
        }
        if !outputs.is_empty() {
            self.node_outputs.insert(node_id.clone(), outputs.clone());
        }
        if !facts.is_empty() {
            self.node_facts.insert(node_id.clone(), facts.clone());
        }
        for binding in outputs {
            let fields = binding.fields()?;
            self.typed_bindings.push(SemanticBinding::output(
                node_id,
                fields.output_ordinal,
                &fields.value_ref,
            )?);
        }
        if let Some(input_manifest_ref) = transition_input_manifest(body) {
            let value_ref = input_manifest_ref.value_ref()?;
            let manifest = InputManifest::strict_decode(&resolve_object(&value_ref)?)?;
            for binding in manifest.fields()?.bindings {
                let fields = binding.fields()?;
                self.typed_bindings.push(SemanticBinding::input(
                    node_id,
                    &fields.field_path,
                    &fields.value_ref,
                )?);
            }
        }
        for emission in &facts {
            let fields = emission.fields()?;
            let claim =
                FactClaimEnvelope::strict_decode(&resolve_object(&fields.claim_ref)?)?.fields()?;
            if claim.fact_descriptor_ref != fields.fact_descriptor_ref {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "semantic_fact_binding",
                });
            }
            self.typed_bindings.push(SemanticBinding::fact(
                node_id,
                fields.emission_ordinal,
                &fields.claim_ref,
                &claim.subject_ref,
                &claim.response_ref,
            )?);
        }
        sort_semantic_bindings(&mut self.typed_bindings)?;

        let all_terminal = self
            .node_phases
            .values()
            .all(|phase| *phase == NodePhase::Terminal);
        if all_terminal {
            if run_phase_change != Some(RunPhase::Closed) {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "terminal_run_phase_change",
                });
            }
            self.run_phase = RunPhase::Closed;
            if let Some(change) = public_output_change {
                self.public_output = change;
            }
            let required_success_met = spec
                .run_terminal_contract()
                .required_success_nodes()
                .iter()
                .all(|required| {
                    self.terminal_outcomes.get(required) == Some(&NodeTerminalOutcome::Succeeded)
                });
            if required_success_met
                && spec.run_terminal_contract().requires_public_output()
                && self.public_output.is_none()
            {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "terminal_public_output",
                });
            }
        } else {
            if run_phase_change.is_some() || public_output_change.is_some() {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "premature_terminal_delta",
                });
            }
            self.run_phase = RunPhase::Open;
        }
        Ok(())
    }

    fn apply_authorization(
        &mut self,
        run_id: &RunId,
        run_sequence: u64,
        commit: &CommittedJournalCommit,
        authorization: ExternalAccessAuthorized,
    ) -> Result<()> {
        let fields = authorization.fields()?;
        let anchor = fields.semantic_anchor.fields()?;
        let phase = self
            .node_phases
            .get(&anchor.node_id)
            .copied()
            .ok_or(StoreError::AuthorizationNotEligible)?;
        if anchor.journal_head != self.journal_head
            || anchor.run_state_digest != self.run_state_digest
            || anchor.node_phase != phase
        {
            return Err(StoreError::AuthorizationNotEligible);
        }
        match fields.scope.fields()? {
            AuthorizationScopeFields::Read { .. } if phase == NodePhase::Unstarted => {}
            AuthorizationScopeFields::EnsureEffect {
                effect_request_transition_ref,
            } if phase == NodePhase::AwaitingEffect
                && self
                    .transitions
                    .iter()
                    .any(|entry| entry.transition_ref == effect_request_transition_ref) => {}
            _ => return Err(StoreError::AuthorizationNotEligible),
        }
        let record = commit.records.first().ok_or(StoreError::EmptyJournal)?;
        let authorization_ref = AuthorizationRef::new(&record.record_ref(run_id, run_sequence)?)?;
        let effect_key = authorization_effect_key(self, &authorization)?;
        let audit_entry = AccessAuditEntry::new(
            &authorization_ref,
            None,
            AccessAuditStatus::AuthorizedUnobserved,
            None,
            effect_key.as_ref(),
            None,
        )?;
        audit_entry.validate_unobserved(&authorization_ref)?;
        self.authorizations.push(AuthorizationState {
            entry: FoldedAccessAuditEntry {
                authorization_ref,
                authorization,
                authorization_journal_head: commit.envelope.journal_head()?,
                observation: None,
                capability_binding_ref: fields.capability_binding_ref,
                capability_operation_id: fields.capability_operation_id,
                request_ref: fields.request_ref,
                status: AccessAuditStatus::AuthorizedUnobserved,
                result_ref: None,
                failure: None,
                effect_key,
                delivery_audit_ref: None,
                audit_entry,
                delivery_audit_terminal: None,
            },
        });
        Ok(())
    }

    fn apply_observation(&mut self, application: ObservationApplication<'_>) -> Result<()> {
        let ObservationApplication {
            run_id,
            run_sequence,
            commit,
            observation,
            spec,
            capability_binding_manifest,
            journal,
        } = application;
        let fields = observation.fields()?;
        let key = fields.authorization_ref.as_bytes();
        let authorization_index = self
            .authorizations
            .iter()
            .position(|entry| entry.entry.authorization_ref.as_bytes() == key)
            .ok_or(StoreError::UnknownAuthorization)?;
        if self.authorizations[authorization_index]
            .entry
            .observation
            .is_some()
        {
            return Err(StoreError::ObservationAlreadyCommitted);
        }
        verify_observation_semantics(
            ObservationVerificationContext {
                fold: self,
                resolver: ObservationObjectResolver {
                    journal,
                    prepared: None,
                    visible_run_sequence: run_sequence,
                },
                spec,
            },
            capability_binding_manifest,
            &self.authorizations[authorization_index]
                .entry
                .authorization_ref,
            &self.authorizations[authorization_index].entry.authorization,
            &observation,
        )?;
        let authorization = &mut self.authorizations[authorization_index];
        let record = commit.records.first().ok_or(StoreError::EmptyJournal)?;
        let observation_ref = ObservationRef::new(&record.record_ref(run_id, run_sequence)?)?;
        let (status, failure, returned_ref) = audit_outcome_fields(&observation)?;
        let effect_key = authorization_effect_key_from_entry(&authorization.entry)?;
        let delivery_audit = if effect_key.is_some() {
            returned_ref
                .as_ref()
                .map(|result_ref| executor_delivery_audit(journal, result_ref))
                .transpose()?
        } else {
            None
        };
        let audit_entry = AccessAuditEntry::new(
            &authorization.entry.authorization_ref,
            Some(&observation_ref),
            status,
            failure.as_ref(),
            effect_key.as_ref(),
            delivery_audit
                .as_ref()
                .map(|(delivery_audit_ref, _)| delivery_audit_ref),
        )?;
        audit_entry.validate_observation(&observation_ref, &observation)?;
        let observation_head = commit.envelope.journal_head()?;
        authorization.entry.observation = Some((observation_ref, observation, observation_head));
        authorization.entry.status = status;
        authorization.entry.result_ref = returned_ref;
        authorization.entry.failure = failure;
        authorization.entry.delivery_audit_ref = delivery_audit
            .as_ref()
            .map(|(delivery_audit_ref, _)| delivery_audit_ref.clone());
        authorization.entry.audit_entry = audit_entry;
        authorization.entry.delivery_audit_terminal = delivery_audit.map(|(_, terminal)| terminal);
        Ok(())
    }

    fn validate_transition_candidate(
        &self,
        transition: &StateTransitionCommitted,
        closure: Option<&mfm_journal::v1::RunClosed>,
        transition_hash: &JournalRecordHash,
        spec: &ExpandedCertifiedSpec,
        objects: &super::PreparedObjectGraph,
    ) -> Result<()> {
        if self.run_phase == RunPhase::Closed {
            return Err(StoreError::RunClosed);
        }
        let mut next = self.clone();
        let fields = transition.fields()?;
        let node = certified_node(spec, &fields.node_id)?;
        if fields.spec_hash != spec.spec_hash()?
            || &fields.state_contract_ref != node.state_contract_ref()
        {
            return Err(StoreError::TransitionFoldMismatch {
                field: "certified_node",
            });
        }
        let current_phase = next
            .node_phases
            .get(&fields.node_id)
            .copied()
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        let before = fields.before.fields()?;
        if before.journal_head != next.journal_head
            || before.run_state_digest != next.run_state_digest
            || before.run_phase != next.run_phase
            || before.node_phase != current_phase
        {
            return Err(StoreError::TransitionFoldMismatch {
                field: "before_state",
            });
        }
        if !next
            .transition_slots
            .insert((fields.node_id.clone(), fields.slot))
        {
            return Err(StoreError::DuplicateLogicalRecord);
        }
        let body = fields.body.fields()?;
        validate_transition_body(
            &mut next,
            &fields.node_id,
            current_phase,
            fields.slot,
            &body,
            node,
        )?;
        let after = fields.after.fields()?;
        next.apply_binding_delta(
            &fields.node_id,
            &body,
            &after.binding_delta,
            spec,
            None,
            &|value_ref| Ok(objects.bytes_for(value_ref)?.to_vec()),
        )?;
        let derived_digest = next.derive_run_state_digest(spec)?;
        if after.node_phase
            != next.node_phases.get(&fields.node_id).copied().ok_or(
                StoreError::TransitionFoldMismatch {
                    field: "after_node_phase",
                },
            )?
            || after.run_phase != next.run_phase
            || after.run_state_digest != derived_digest
        {
            return Err(StoreError::TransitionFoldMismatch {
                field: "after_state",
            });
        }
        next.run_state_digest = derived_digest;
        let all_terminal = next
            .node_phases
            .values()
            .all(|phase| *phase == NodePhase::Terminal);
        match (all_terminal, next.run_phase, closure) {
            (true, RunPhase::Closed, Some(closed)) => {
                if &closed.fields()?.terminal_transition_record_hash != transition_hash {
                    return Err(StoreError::InvalidClosure);
                }
            }
            (false, RunPhase::Open, None) => {}
            _ => return Err(StoreError::InvalidClosure),
        }
        Ok(())
    }

    fn validate_authorization_candidate(
        &self,
        authorization: &ExternalAccessAuthorized,
    ) -> Result<()> {
        if self.run_phase == RunPhase::Closed {
            return Err(StoreError::RunClosed);
        }
        let fields = authorization.fields()?;
        let anchor = fields.semantic_anchor.fields()?;
        let phase = self
            .node_phases
            .get(&anchor.node_id)
            .copied()
            .ok_or(StoreError::AuthorizationNotEligible)?;
        if anchor.journal_head != self.journal_head
            || anchor.run_state_digest != self.run_state_digest
            || anchor.node_phase != phase
        {
            return Err(StoreError::AuthorizationNotEligible);
        }
        match fields.scope.fields()? {
            AuthorizationScopeFields::Read { .. } if phase == NodePhase::Unstarted => Ok(()),
            AuthorizationScopeFields::EnsureEffect {
                effect_request_transition_ref,
            } if phase == NodePhase::AwaitingEffect
                && self
                    .transitions
                    .iter()
                    .any(|entry| entry.transition_ref == effect_request_transition_ref) =>
            {
                Ok(())
            }
            _ => Err(StoreError::AuthorizationNotEligible),
        }
    }

    fn validate_observation_candidate(
        &self,
        journal: &CommittedRunJournal,
        objects: &super::PreparedObjectGraph,
        spec: &ExpandedCertifiedSpec,
        capability_binding_manifest: &CapabilityBindingManifest,
        observation: &ExternalAccessObserved,
    ) -> Result<()> {
        let fields = observation.fields()?;
        let entry = self
            .authorizations
            .iter()
            .find(|entry| entry.entry.authorization_ref == fields.authorization_ref)
            .ok_or(StoreError::UnknownAuthorization)?;
        if entry.entry.observation.is_some() {
            return Err(StoreError::ObservationAlreadyCommitted);
        }
        verify_observation_semantics(
            ObservationVerificationContext {
                fold: self,
                resolver: ObservationObjectResolver {
                    journal,
                    prepared: Some(objects),
                    visible_run_sequence: u64::try_from(journal.commits.len())
                        .map_err(|_| StoreError::SequenceOverflow)?,
                },
                spec,
            },
            capability_binding_manifest,
            &entry.entry.authorization_ref,
            &entry.entry.authorization,
            observation,
        )
    }
}

fn pending_effect_value(
    node_id: &NodeId,
    pending: &PendingEffectState,
) -> Result<JournalPendingEffectState> {
    JournalPendingEffectState::new(
        node_id,
        &pending.effect_key,
        &pending.request_digest,
        &pending.executor_binding_ref,
        &pending.input_manifest_ref,
        &pending.semantic_request_ref,
    )
    .map_err(Into::into)
}

fn transition_input_manifest(body: &TransitionBodyFields) -> Option<&InputManifestRef> {
    match body {
        TransitionBodyFields::PureSettled {
            input_manifest_ref, ..
        }
        | TransitionBodyFields::ReadSettled {
            input_manifest_ref, ..
        }
        | TransitionBodyFields::EffectRequested {
            input_manifest_ref, ..
        } => Some(input_manifest_ref),
        TransitionBodyFields::EffectSettled { .. }
        | TransitionBodyFields::DependencySkipped { .. } => None,
    }
}

fn sort_semantic_bindings(values: &mut [SemanticBinding]) -> Result<()> {
    values.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    if values
        .windows(2)
        .any(|pair| pair[0].as_bytes() == pair[1].as_bytes())
    {
        return Err(StoreError::TransitionFoldMismatch {
            field: "duplicate_semantic_binding",
        });
    }
    Ok(())
}

fn validate_transition_body(
    fold: &mut FoldState,
    node_id: &NodeId,
    current_phase: NodePhase,
    slot: TransitionSlot,
    body: &TransitionBodyFields,
    certified_node: &CertifiedNodeContract,
) -> Result<()> {
    let expected_slot = if matches!(body, TransitionBodyFields::EffectRequested { .. }) {
        TransitionSlot::Request
    } else {
        TransitionSlot::Settlement
    };
    if slot != expected_slot {
        return Err(StoreError::TransitionFoldMismatch {
            field: "transition_slot",
        });
    }
    let execution_matches = matches!(
        (body, certified_node.execution()),
        (
            TransitionBodyFields::PureSettled { .. },
            CertifiedStateExecution::Pure
        ) | (
            TransitionBodyFields::ReadSettled { .. },
            CertifiedStateExecution::Read { .. }
        ) | (
            TransitionBodyFields::EffectRequested { .. }
                | TransitionBodyFields::EffectSettled { .. },
            CertifiedStateExecution::Effect { .. },
        ) | (TransitionBodyFields::DependencySkipped { .. }, _)
    );
    if !execution_matches {
        return Err(StoreError::TransitionFoldMismatch {
            field: "certified_execution_kind",
        });
    }
    if current_phase == NodePhase::Unstarted {
        match body {
            TransitionBodyFields::DependencySkipped { blocking_sources } => {
                validate_dependency_skip(fold, certified_node, blocking_sources)?;
            }
            _ if !node_is_ready(fold, certified_node) => {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "node_readiness",
                });
            }
            _ => {}
        }
    }
    match body {
        TransitionBodyFields::PureSettled { .. }
        | TransitionBodyFields::DependencySkipped { .. }
            if current_phase == NodePhase::Unstarted => {}
        TransitionBodyFields::ReadSettled {
            input_manifest_ref,
            request_ref,
            consumed_observation_ref,
            ..
        } if current_phase == NodePhase::Unstarted => {
            consume_observation(
                fold,
                node_id,
                consumed_observation_ref,
                ObservationConsumption::Read {
                    input_manifest_ref,
                    request_ref,
                },
            )?;
        }
        TransitionBodyFields::EffectRequested { .. } if current_phase == NodePhase::Unstarted => {
            let TransitionBodyFields::EffectRequested {
                executor_binding_ref,
                ..
            } = body
            else {
                unreachable!("matched effect request");
            };
            if executor_binding_ref.fields()?
                != certified_node
                    .execution()
                    .executor_binding_ref()
                    .cloned()
                    .ok_or(StoreError::TransitionFoldMismatch {
                        field: "executor_binding_ref",
                    })?
            {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "executor_binding_ref",
                });
            }
        }
        TransitionBodyFields::EffectSettled {
            request_transition_ref,
            consumed_terminal_observation_ref,
            ..
        } if current_phase == NodePhase::AwaitingEffect => {
            if !fold.transitions.iter().any(|entry| {
                &entry.transition_ref == request_transition_ref
                    && entry
                        .transition
                        .fields()
                        .is_ok_and(|fields| fields.node_id == *node_id)
            }) {
                return Err(StoreError::ObservationNotConsumable);
            }
            consume_observation(
                fold,
                node_id,
                consumed_terminal_observation_ref,
                ObservationConsumption::EnsureEffect {
                    request_transition_ref,
                },
            )?;
        }
        _ => {
            return Err(StoreError::TransitionFoldMismatch {
                field: "transition_body",
            });
        }
    }
    Ok(())
}

fn source_is_available(fold: &FoldState, source: &CertifiedSourceSelector) -> bool {
    match source {
        CertifiedSourceSelector::NodeOutput {
            producer_node_id,
            output_ordinal,
            ..
        } => fold
            .node_outputs
            .get(producer_node_id)
            .is_some_and(|outputs| {
                outputs.iter().any(|output| {
                    output
                        .fields()
                        .is_ok_and(|fields| fields.output_ordinal == *output_ordinal)
                })
            }),
        CertifiedSourceSelector::NodeFact {
            producer_node_id,
            emission_ordinal,
        } => fold.node_facts.get(producer_node_id).is_some_and(|facts| {
            facts.iter().any(|fact| {
                fact.fields()
                    .is_ok_and(|fields| fields.emission_ordinal == *emission_ordinal)
            })
        }),
        CertifiedSourceSelector::RunAdmission { .. }
        | CertifiedSourceSelector::Config { .. }
        | CertifiedSourceSelector::QualifiedSupport { .. }
        | CertifiedSourceSelector::Seed { .. }
        | CertifiedSourceSelector::Context { .. }
        | CertifiedSourceSelector::CrossRunEffectiveOutput { .. }
        | CertifiedSourceSelector::CrossRunEvidence { .. } => true,
    }
}

fn node_is_ready(fold: &FoldState, node: &CertifiedNodeContract) -> bool {
    fold.node_phases.get(node.node_id()) == Some(&NodePhase::Unstarted)
        && source_is_available(fold, node.config_binding().source())
        && node
            .context_binding()
            .is_none_or(|binding| source_is_available(fold, binding.source()))
        && node.input_bindings().iter().all(|binding| {
            binding
                .ordered_sources()
                .iter()
                .any(|source| source_is_available(fold, source))
        })
}

fn validate_dependency_skip(
    fold: &FoldState,
    node: &CertifiedNodeContract,
    blocking_sources: &[BlockingSource],
) -> Result<()> {
    let expected = derive_dependency_blocking_sources(fold, node)?
        .into_iter()
        .map(|blocker| blocker.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for blocking in blocking_sources {
        let fields = blocking.fields()?;
        if fold.terminal_transition_refs.get(&fields.producer_node_id)
            != Some(&fields.producer_terminal_transition_ref)
            || !actual.insert(blocking.as_bytes().to_vec())
        {
            return Err(StoreError::TransitionFoldMismatch {
                field: "dependency_skip_evidence",
            });
        }
    }
    if actual != expected {
        return Err(StoreError::TransitionFoldMismatch {
            field: "dependency_skip_blocking_set",
        });
    }
    Ok(())
}

fn derive_dependency_blocking_sources(
    fold: &FoldState,
    node: &CertifiedNodeContract,
) -> Result<Vec<BlockingSource>> {
    let mut expected = BTreeMap::new();

    let config = node.config_binding();
    if !source_is_available(fold, config.source()) {
        insert_terminal_destination(
            fold,
            &BlockingDestination::config()?,
            std::slice::from_ref(config.source()),
            &mut expected,
        )?;
    }

    if let Some(context) = node.context_binding() {
        if !source_is_available(fold, context.source()) {
            insert_terminal_destination(
                fold,
                &BlockingDestination::context()?,
                std::slice::from_ref(context.source()),
                &mut expected,
            )?;
        }
    }

    for binding in node.input_bindings() {
        if binding
            .ordered_sources()
            .iter()
            .any(|source| source_is_available(fold, source))
        {
            continue;
        }
        let destination = BlockingDestination::input(binding.destination_field_path())?;
        insert_terminal_destination(fold, &destination, binding.ordered_sources(), &mut expected)?;
    }
    if expected.is_empty() {
        return Err(StoreError::TransitionFoldMismatch {
            field: "dependency_skip_without_blocker",
        });
    }
    Ok(expected.into_values().collect())
}

fn insert_terminal_destination(
    fold: &FoldState,
    destination: &BlockingDestination,
    sources: &[CertifiedSourceSelector],
    expected: &mut BTreeMap<Vec<u8>, BlockingSource>,
) -> Result<()> {
    let mut blockers = Vec::with_capacity(sources.len());
    for source in sources {
        let Some(blocker) = terminal_blocker(fold, destination, source)? else {
            return Ok(());
        };
        blockers.push(blocker);
    }
    for blocker in blockers {
        if expected
            .insert(blocker.as_bytes().to_vec(), blocker)
            .is_some()
        {
            return Err(StoreError::TransitionFoldMismatch {
                field: "dependency_skip_blocking_set",
            });
        }
    }
    Ok(())
}

fn terminal_blocker(
    fold: &FoldState,
    destination: &BlockingDestination,
    source: &CertifiedSourceSelector,
) -> Result<Option<BlockingSource>> {
    let (producer_node_id, requirement) = match source {
        CertifiedSourceSelector::NodeOutput {
            producer_node_id,
            output_ordinal,
            source_field_path,
        } => (
            producer_node_id,
            BlockingProducerRequirement::output(*output_ordinal, source_field_path.as_ref())?,
        ),
        CertifiedSourceSelector::NodeFact {
            producer_node_id,
            emission_ordinal,
        } => (
            producer_node_id,
            BlockingProducerRequirement::fact(*emission_ordinal)?,
        ),
        CertifiedSourceSelector::RunAdmission { .. }
        | CertifiedSourceSelector::Config { .. }
        | CertifiedSourceSelector::QualifiedSupport { .. }
        | CertifiedSourceSelector::Seed { .. }
        | CertifiedSourceSelector::Context { .. }
        | CertifiedSourceSelector::CrossRunEffectiveOutput { .. }
        | CertifiedSourceSelector::CrossRunEvidence { .. } => {
            return Err(StoreError::TransitionFoldMismatch {
                field: "dependency_skip_not_decidable",
            });
        }
    };
    if fold.node_phases.get(producer_node_id) != Some(&NodePhase::Terminal) {
        return Ok(None);
    }
    let terminal_transition_ref = fold.terminal_transition_refs.get(producer_node_id).ok_or(
        StoreError::TransitionFoldMismatch {
            field: "dependency_skip_not_decidable",
        },
    )?;
    BlockingSource::new(
        producer_node_id,
        terminal_transition_ref,
        destination,
        &requirement,
    )
    .map(Some)
    .map_err(Into::into)
}

enum ObservationConsumption<'a> {
    Read {
        input_manifest_ref: &'a InputManifestRef,
        request_ref: &'a ValueRef,
    },
    EnsureEffect {
        request_transition_ref: &'a TransitionRef,
    },
}

fn consume_observation(
    fold: &mut FoldState,
    node_id: &NodeId,
    observation_ref: &ObservationRef,
    consumption: ObservationConsumption<'_>,
) -> Result<()> {
    let key = observation_ref.as_bytes().to_vec();
    if fold.consumed_observations.contains(&key) {
        return Err(StoreError::ObservationNotConsumable);
    }
    let entry = fold
        .authorizations
        .iter()
        .find(|entry| {
            entry
                .entry
                .observation
                .as_ref()
                .is_some_and(|(reference, _, _)| reference.as_bytes() == key)
        })
        .ok_or(StoreError::ObservationNotConsumable)?;
    let auth_fields = entry.entry.authorization.fields()?;
    if auth_fields.semantic_anchor.fields()?.node_id != *node_id {
        return Err(StoreError::ObservationNotConsumable);
    }
    let matches_scope = match (consumption, auth_fields.scope.fields()?) {
        (
            ObservationConsumption::Read {
                input_manifest_ref,
                request_ref,
            },
            AuthorizationScopeFields::Read {
                input_manifest_ref: authorized_manifest_ref,
            },
        ) => {
            authorized_manifest_ref == *input_manifest_ref
                && auth_fields.request_ref == *request_ref
        }
        (
            ObservationConsumption::EnsureEffect {
                request_transition_ref,
            },
            AuthorizationScopeFields::EnsureEffect {
                effect_request_transition_ref,
            },
        ) => effect_request_transition_ref == *request_transition_ref,
        (ObservationConsumption::Read { .. }, AuthorizationScopeFields::EnsureEffect { .. })
        | (ObservationConsumption::EnsureEffect { .. }, AuthorizationScopeFields::Read { .. }) => {
            false
        }
    };
    if !matches_scope {
        return Err(StoreError::ObservationNotConsumable);
    }
    fold.consumed_observations.insert(key);
    Ok(())
}

const READ_REQUEST_PATH: &str = "request_ref";
const FROZEN_READ_INTENT_PATH: &str = "frozen_read_intent_ref";
const READ_RESULT_PATH: &str = "outcome.result_ref";
const READ_DIAGNOSTIC_PATH: &str = "outcome.safe_failure.diagnostic_ref";
const EFFECT_REQUEST_PATH: &str = "body.semantic_request_ref";
const ENSURE_RESULT_PATH: &str = "executor.ensure_result";
const TERMINAL_EVIDENCE_PATH: &str = "executor.terminal_evidence";
const REQUEST_PREIMAGE_SCHEMA: &str = "mfm.request-digest-preimage.v1";

struct ObservationObjectResolver<'a> {
    journal: &'a CommittedRunJournal,
    prepared: Option<&'a super::PreparedObjectGraph>,
    visible_run_sequence: u64,
}

struct ObservationVerificationContext<'a> {
    fold: &'a FoldState,
    resolver: ObservationObjectResolver<'a>,
    spec: &'a ExpandedCertifiedSpec,
}

impl ObservationObjectResolver<'_> {
    fn value_bytes(&self, value_ref: &ValueRef) -> Result<Vec<u8>> {
        if let Some(prepared) = self.prepared {
            match prepared.bytes_for(value_ref) {
                Ok(bytes) => return Ok(bytes.to_vec()),
                Err(StoreError::ObjectNotReachable) => {}
                Err(error) => return Err(error),
            }
        }
        let object = retained_value_in_journal(self.journal, value_ref)?;
        if !self.committed_value_visible(value_ref)? {
            return Err(StoreError::ObjectNotReachable);
        }
        Ok(object.bytes().to_vec())
    }

    fn content_bytes(&self, content_ref: &ContentRef) -> Result<Vec<u8>> {
        let mut matched: Option<Vec<u8>> = None;
        let mut accept = |schema_id: &mfm_ids::SchemaId,
                          digest: &mfm_ids::ContentDigest,
                          bytes: &[u8]|
         -> Result<()> {
            if schema_id == content_ref.schema_id() && digest == content_ref.content_digest() {
                if matched.as_ref().is_some_and(|existing| existing != bytes) {
                    return Err(observation_mismatch("observation_content_identity"));
                }
                matched = Some(bytes.to_vec());
            }
            Ok(())
        };
        if let Some(prepared) = self.prepared {
            for payload in prepared.payloads() {
                accept(
                    payload.schema_id(),
                    payload.content_digest(),
                    payload.bytes(),
                )?;
            }
        }
        for object in self.journal.objects.values() {
            if !self.committed_value_visible(object.value_ref())? {
                continue;
            }
            let fields = object.value_ref().fields()?;
            accept(&fields.schema_id, &fields.content_digest, object.bytes())?;
        }
        matched.ok_or(StoreError::ObjectNotReachable)
    }

    fn produced_values(
        &self,
        authorization_ref: &AuthorizationRef,
    ) -> Result<Vec<(ValueRef, Vec<u8>)>> {
        let mut exact = BTreeMap::<Vec<u8>, (ValueRef, Vec<u8>)>::new();
        let mut accept = |value_ref: &ValueRef, bytes: &[u8]| -> Result<()> {
            let ProducerBindingFields::ExternalObservation {
                authorization_ref: producer,
                ..
            } = value_ref.fields()?.producer_binding.fields()?
            else {
                return Ok(());
            };
            if &producer != authorization_ref {
                return Ok(());
            }
            match exact.entry(value_ref.as_bytes().to_vec()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((value_ref.clone(), bytes.to_vec()));
                }
                std::collections::btree_map::Entry::Occupied(entry)
                    if entry.get().1.as_slice() == bytes => {}
                std::collections::btree_map::Entry::Occupied(_) => {
                    return Err(observation_mismatch("observation_producer_identity"));
                }
            }
            Ok(())
        };
        if let Some(prepared) = self.prepared {
            for payload in prepared.payloads() {
                for value_ref in payload.value_refs() {
                    accept(value_ref, payload.bytes())?;
                }
            }
        }
        for object in self.journal.objects.values() {
            if self.committed_value_visible(object.value_ref())? {
                accept(object.value_ref(), object.bytes())?;
            }
        }
        Ok(exact.into_values().collect())
    }

    fn committed_value_visible(&self, value_ref: &ValueRef) -> Result<bool> {
        let target = ObjectAuthorityKey::from_value_ref(value_ref)?;
        for commit in self.journal.commits.iter().take(
            usize::try_from(self.visible_run_sequence).map_err(|_| StoreError::SequenceOverflow)?,
        ) {
            let fields = commit.envelope.fields()?;
            for intent in fields.core.artifact_admission_intents {
                if ObjectAuthorityKey::from_value_ref(&intent.fields()?.value_ref)? == target {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

fn verify_observation_semantics(
    context: ObservationVerificationContext<'_>,
    capability_binding_manifest: &CapabilityBindingManifest,
    expected_authorization_ref: &AuthorizationRef,
    authorization: &ExternalAccessAuthorized,
    observation: &ExternalAccessObserved,
) -> Result<()> {
    let authorization_fields = authorization.fields()?;
    let observation_fields = observation.fields()?;
    if &observation_fields.authorization_ref != expected_authorization_ref
        || authorization_fields.capability_operation_id.is_empty()
    {
        return Err(observation_mismatch("observation_authorization"));
    }
    let anchor = authorization_fields.semantic_anchor.fields()?;
    let node = certified_node(context.spec, &anchor.node_id)?;
    let admitted = capability_binding_manifest
        .entries()
        .iter()
        .find(|entry| entry.operation_id == authorization_fields.capability_operation_id)
        .ok_or_else(|| observation_mismatch("observation_capability_manifest"))?;
    let binding_ref = authorization_fields.capability_binding_ref.fields()?;
    if admitted.binding_ref != binding_ref {
        return Err(observation_mismatch("observation_capability_manifest"));
    }

    if authorization_fields.capability_operation_id.as_str() == FACT_SELECTION_OPERATION_ID {
        return verify_fact_selection_observation(
            &context,
            node,
            expected_authorization_ref,
            &authorization_fields,
            &observation_fields,
        );
    }
    if observation_fields
        .fact_selection_scan_attestation_ref
        .is_some()
    {
        return Err(observation_mismatch("observation_scan_attestation"));
    }

    match authorization_fields.scope.fields()? {
        AuthorizationScopeFields::Read { input_manifest_ref } => verify_read_observation(
            &context,
            node,
            &observation_fields.authorization_ref,
            &authorization_fields,
            &input_manifest_ref,
            &observation_fields.outcome,
        ),
        AuthorizationScopeFields::EnsureEffect {
            effect_request_transition_ref,
        } => verify_effect_observation(
            &context,
            node,
            &observation_fields.authorization_ref,
            &authorization_fields,
            &effect_request_transition_ref,
            &observation_fields.outcome,
        ),
    }
}

fn verify_fact_selection_observation(
    context: &ObservationVerificationContext<'_>,
    node: &CertifiedNodeContract,
    authorization_ref: &AuthorizationRef,
    authorization: &mfm_journal::v1::ExternalAccessAuthorizedFields,
    observation: &mfm_journal::v1::ExternalAccessObservedFields,
) -> Result<()> {
    let fold = context.fold;
    let journal = context.resolver.journal;
    let resolver = &context.resolver;
    let spec = context.spec;
    let CertifiedStateExecution::Read {
        capability_operation_id,
        capability_binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
    } = node.execution()
    else {
        return Err(StoreError::FactScanBindingMismatch);
    };
    let AuthorizationScopeFields::Read { input_manifest_ref } = authorization.scope.fields()?
    else {
        return Err(StoreError::FactScanBindingMismatch);
    };
    if capability_operation_id.as_str() != FACT_SELECTION_OPERATION_ID
        || capability_binding_ref != &authorization.capability_binding_ref.fields()?
    {
        return Err(StoreError::FactScanBindingMismatch);
    }

    let frozen_ref = authorization
        .frozen_read_intent_ref
        .as_ref()
        .ok_or(StoreError::FactScanBindingMismatch)?;
    let source_visible_sequence = verify_frozen_read_retry(
        fold,
        journal,
        node,
        authorization_ref,
        authorization,
        frozen_ref,
    )
    .map_err(|_| StoreError::FactScanBindingMismatch)?;
    verify_input_manifest_identity(
        fold,
        spec,
        journal,
        node,
        &input_manifest_ref,
        authorization_ref,
        source_visible_sequence,
    )
    .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let authorization_sequence = authorization_ref.fields()?.run_sequence;
    let authorization_resolver = ObservationObjectResolver {
        journal,
        prepared: None,
        visible_run_sequence: authorization_sequence,
    };

    let binding_bytes = authorization_resolver.content_bytes(capability_binding_ref)?;
    let binding = ReadCapabilityBinding::strict_decode(&binding_bytes)?;
    if binding.content_ref()? != *capability_binding_ref {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let binding = binding.fields()?;
    let scan_contract_bytes =
        authorization_resolver.content_bytes(&binding.capability_contract_ref)?;
    let scan_contract = FactSelectionScanContract::strict_decode(&scan_contract_bytes)?;
    if scan_contract.content_ref()? != binding.capability_contract_ref {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let scan = scan_contract.fields()?;
    if scan.capability_operation_id != *capability_operation_id
        || scan.request_contract != *request_contract
        || scan.response_contract != *returned_contract
    {
        return Err(StoreError::FactScanBindingMismatch);
    }
    for reference in [
        &binding.admitted_implementation_ref,
        &binding.safe_classifier_contract_ref,
        &binding.safe_failure_contract_ref,
        &binding.reviewed_source_scope_ref,
        &binding.routing_catalog_ref,
    ] {
        authorization_resolver
            .content_bytes(reference)
            .map_err(|_| StoreError::FactScanBindingMismatch)?;
    }

    validate_value_contract(
        spec.journal_protocol_contracts()
            .frozen_read_intent_contract(),
        frozen_ref,
    )
    .map_err(|_| StoreError::FactScanBindingMismatch)?;
    validate_this_record_producer(frozen_ref, FROZEN_READ_INTENT_PATH)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let frozen = FrozenReadIntent::strict_decode(&authorization_resolver.value_bytes(frozen_ref)?)?;
    let frozen = frozen.fields()?;
    if frozen.node_id != *node.node_id()
        || frozen.input_manifest_ref != input_manifest_ref
        || frozen.state_contract_ref != *node.state_contract_ref()
        || frozen.capability_binding_ref != authorization.capability_binding_ref
        || frozen.capability_operation_id != authorization.capability_operation_id
        || frozen.routing_generation_ref != binding.routing_catalog_ref
        || frozen.request_ref != authorization.request_ref
        || frozen.request_contract != *request_contract
        || frozen.returned_contract != *returned_contract
        || frozen.safe_failure_contract != *safe_failure_contract
    {
        return Err(StoreError::FactScanBindingMismatch);
    }

    validate_value_contract(request_contract, &authorization.request_ref)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    validate_this_record_producer(&authorization.request_ref, READ_REQUEST_PATH)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let request_bytes = authorization_resolver.value_bytes(&authorization.request_ref)?;
    let request = FactSelectionRequest::from_canonical_json(&request_bytes)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    if request
        .content_ref()
        .map_err(|_| StoreError::FactScanBindingMismatch)?
        != content_ref_for_value(&authorization.request_ref)?
    {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let canonical_request = PlainCanonicalJsonBytes::from_canonical_json_slice(&request_bytes)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    if derive_observation_request_digest(request_contract.schema_id().as_str(), &canonical_request)?
        != frozen.request_digest
    {
        return Err(StoreError::FactScanBindingMismatch);
    }

    let ObservationOutcomeFields::Returned { result_ref } = observation.outcome.fields()? else {
        return Err(StoreError::FactScanBindingMismatch);
    };
    let attestation_ref = observation
        .fact_selection_scan_attestation_ref
        .as_ref()
        .ok_or(StoreError::FactScanBindingMismatch)?;
    validate_external_observation_producer(&result_ref, authorization_ref, READ_RESULT_PATH)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    validate_value_contract(&scan.response_contract, &result_ref)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let response_bytes = resolver.value_bytes(&result_ref)?;
    let response = FactSelectionResponse::strict_decode(&response_bytes)?;
    response
        .validate_request(&request)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let frontier = observation_fact_frontier(journal, authorization_ref)?;
    if response.fields()?.frontier != frontier {
        return Err(StoreError::FactScanBindingMismatch);
    }
    validate_external_observation_producer(
        attestation_ref,
        authorization_ref,
        "fact_selection_scan_attestation_ref",
    )
    .map_err(|_| StoreError::FactScanBindingMismatch)?;
    validate_value_contract(&scan.scan_attestation_contract, attestation_ref)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let attestation =
        FactSelectionScanAttestation::strict_decode(&resolver.value_bytes(attestation_ref)?)?;
    let attestation = attestation.fields()?;
    let request_digest = request
        .request_digest()
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    if attestation.store_scope_id != *journal.store_identity().store_scope_id()
        || attestation.store_epoch != journal.store_identity().store_epoch()
        || attestation.tenant_scope_id != *journal.tenant_scope_id()
        || attestation.authorization_ref != *authorization_ref
        || attestation.request_digest != request_digest
        || attestation.frontier != frontier
        || attestation.response_ref != result_ref
    {
        return Err(StoreError::FactScanBindingMismatch);
    }

    verify_exact_observation_values(
        resolver,
        authorization_ref,
        &[&result_ref, attestation_ref],
        true,
    )
    .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let observation_graph =
        observation_graph_objects(resolver, ArtifactAdmissionMode::RequireExisting)?;
    let consumer_prefix = observation_prefix_objects(journal, authorization_sequence)?;
    let roots = recorded_fact_source_roots(&response, &observation_graph)?;
    let closure_digest = super::fact_scan::derive_recorded_response_closure_digest(
        &result_ref,
        &response_bytes,
        roots,
        &observation_graph,
        &consumer_prefix,
    )?;
    if closure_digest != attestation.response_closure_digest {
        return Err(StoreError::InvalidSourceClosure);
    }
    Ok(())
}

fn observation_fact_frontier(
    journal: &CommittedRunJournal,
    authorization_ref: &AuthorizationRef,
) -> Result<TenantFactFrontier> {
    let sequence = authorization_ref.fields()?.run_sequence;
    let index = usize::try_from(
        sequence
            .checked_sub(1)
            .ok_or(StoreError::SequenceOverflow)?,
    )
    .map_err(|_| StoreError::SequenceOverflow)?;
    let commit = journal
        .commits
        .get(index)
        .ok_or(StoreError::FactScanBindingMismatch)?;
    let coordinate = commit.envelope.fields()?.core.tenant_fact_coordinate;
    let (tenant_scope_id, fact_order) = match coordinate.fields()? {
        TenantFactCoordinateFields::FactSelectionBarrier {
            tenant_scope_id,
            frontier_fact_order,
        } => (tenant_scope_id, frontier_fact_order),
        TenantFactCoordinateFields::None | TenantFactCoordinateFields::FactPublication { .. } => {
            return Err(StoreError::FactScanBindingMismatch);
        }
    };
    if tenant_scope_id != *journal.tenant_scope_id() {
        return Err(StoreError::FactScanBindingMismatch);
    }
    TenantFactFrontier::new(
        journal.store_identity().store_scope_id(),
        journal.store_identity().store_epoch(),
        journal.tenant_scope_id(),
        fact_order,
    )
    .map_err(Into::into)
}

fn observation_graph_objects(
    resolver: &ObservationObjectResolver<'_>,
    expected_mode: ArtifactAdmissionMode,
) -> Result<Vec<CommittedObject>> {
    let intents = if let Some(prepared) = resolver.prepared {
        prepared.admission_intents().to_vec()
    } else {
        let index = usize::try_from(
            resolver
                .visible_run_sequence
                .checked_sub(1)
                .ok_or(StoreError::SequenceOverflow)?,
        )
        .map_err(|_| StoreError::SequenceOverflow)?;
        resolver
            .journal
            .commits
            .get(index)
            .ok_or(StoreError::EmptyJournal)?
            .envelope
            .fields()?
            .core
            .artifact_admission_intents
    };
    intents
        .into_iter()
        .filter_map(|intent| match intent.fields() {
            Ok(fields) if fields.mode == expected_mode => Some(Ok(fields.value_ref)),
            Ok(_) => None,
            Err(error) => Some(Err(StoreError::from(error))),
        })
        .map(|value_ref| {
            let value_ref = value_ref?;
            CommittedObject::from_persisted(value_ref.clone(), resolver.value_bytes(&value_ref)?)
        })
        .collect()
}

fn observation_prefix_objects(
    journal: &CommittedRunJournal,
    visible_run_sequence: u64,
) -> Result<Vec<CommittedObject>> {
    let take = usize::try_from(visible_run_sequence).map_err(|_| StoreError::SequenceOverflow)?;
    let mut keys = BTreeSet::new();
    for commit in journal.commits.iter().take(take) {
        for intent in commit.envelope.fields()?.core.artifact_admission_intents {
            keys.insert(ObjectAuthorityKey::from_value_ref(
                &intent.fields()?.value_ref,
            )?);
        }
    }
    keys.into_iter()
        .map(|key| {
            journal
                .objects
                .get(&key)
                .cloned()
                .ok_or(StoreError::ObjectNotReachable)
        })
        .collect()
}

fn recorded_fact_source_roots(
    response: &FactSelectionResponse,
    observation_graph: &[CommittedObject],
) -> Result<Vec<super::fact_scan::RecordedFactSourceRoot>> {
    let claim_contract = FactClaimEnvelope::retained_contract()?;
    let mut roots = Vec::new();
    for result in response.fields()?.results {
        for selected in result.fields()?.selected {
            let selected = selected.fields()?;
            let fact = selected.fact_ref.fields()?;
            if fact.transition_ref != selected.producing_transition_ref {
                return Err(StoreError::InvalidSourceClosure);
            }
            let producer_run_id = fact.transition_ref.fields()?.run_id;
            let mut claims = observation_graph.iter().filter_map(|object| {
                let producer = object
                    .value_ref()
                    .fields()
                    .ok()?
                    .producer_binding
                    .fields()
                    .ok()?;
                let ProducerBindingFields::TransitionFact {
                    run_id,
                    emission_ordinal,
                    component: FactValueComponent::Claim,
                    ..
                } = producer
                else {
                    return None;
                };
                if run_id != producer_run_id || emission_ordinal != fact.emission_ordinal {
                    return None;
                }
                let claim = FactClaimEnvelope::strict_decode(object.bytes()).ok()?;
                let fields = claim.fields().ok()?;
                if fields.fact_descriptor_ref != selected.descriptor_ref
                    || fields.subject_ref != selected.subject_ref
                    || fields.response_ref != selected.response_ref
                    || validate_value_contract(&claim_contract, object.value_ref()).is_err()
                {
                    return None;
                }
                Some(object.value_ref().clone())
            });
            let claim_ref = claims.next().ok_or(StoreError::InvalidSourceClosure)?;
            if claims.next().is_some()
                || FactContentIdentityPreimage::new(
                    &selected.descriptor_ref,
                    &selected.subject_ref,
                    &selected.response_ref,
                )?
                .fact_content_identity()?
                    != selected.content_identity
            {
                return Err(StoreError::InvalidSourceClosure);
            }
            roots.push(super::fact_scan::RecordedFactSourceRoot {
                descriptor_ref: selected.descriptor_ref,
                claim_ref,
                subject_ref: selected.subject_ref,
                response_ref: selected.response_ref,
            });
        }
    }
    Ok(roots)
}

fn verify_exact_observation_values(
    resolver: &ObservationObjectResolver<'_>,
    authorization_ref: &AuthorizationRef,
    expected: &[&ValueRef],
    allow_preexisting: bool,
) -> Result<()> {
    let expected_count = expected.len();
    let expected = expected
        .iter()
        .map(|value_ref| value_ref.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    let actual = resolver
        .produced_values(authorization_ref)?
        .into_iter()
        .map(|(value_ref, _)| value_ref.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    let graph_produced =
        observation_graph_objects(resolver, ArtifactAdmissionMode::AdmitOrVerifyExact)?
            .into_iter()
            .map(|object| object.value_ref().as_bytes().to_vec())
            .collect::<BTreeSet<_>>();
    let has_preexisting =
        !observation_graph_objects(resolver, ArtifactAdmissionMode::RequireExisting)?.is_empty();
    if expected.len() != expected_count
        || expected != actual
        || expected != graph_produced
        || (!allow_preexisting && has_preexisting)
    {
        return Err(observation_mismatch("observation_produced_values"));
    }
    Ok(())
}

fn verify_read_observation(
    context: &ObservationVerificationContext<'_>,
    node: &CertifiedNodeContract,
    authorization_ref: &AuthorizationRef,
    authorization: &mfm_journal::v1::ExternalAccessAuthorizedFields,
    input_manifest_ref: &InputManifestRef,
    outcome: &ObservationOutcome,
) -> Result<()> {
    let CertifiedStateExecution::Read {
        capability_operation_id,
        capability_binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
    } = node.execution()
    else {
        return Err(observation_mismatch("observation_execution_kind"));
    };
    let outcome = outcome.fields()?;
    if authorization.capability_operation_id != *capability_operation_id
        || authorization.capability_binding_ref.fields()? != *capability_binding_ref
    {
        return Err(observation_mismatch("read_operation_binding"));
    }
    let fold = context.fold;
    let journal = context.resolver.journal;
    let resolver = &context.resolver;
    let spec = context.spec;
    let frozen_ref = authorization
        .frozen_read_intent_ref
        .as_ref()
        .ok_or_else(|| observation_mismatch("read_frozen_intent"))?;
    let source_visible_sequence = verify_frozen_read_retry(
        fold,
        journal,
        node,
        authorization_ref,
        authorization,
        frozen_ref,
    )?;
    verify_input_manifest_identity(
        fold,
        spec,
        journal,
        node,
        input_manifest_ref,
        authorization_ref,
        source_visible_sequence,
    )?;
    validate_value_contract(
        spec.journal_protocol_contracts()
            .frozen_read_intent_contract(),
        frozen_ref,
    )?;
    validate_this_record_producer(frozen_ref, FROZEN_READ_INTENT_PATH)?;
    let frozen =
        mfm_journal::v1::FrozenReadIntent::strict_decode(&resolver.value_bytes(frozen_ref)?)?;
    let frozen = frozen.fields()?;
    if frozen.node_id != *node.node_id()
        || frozen.input_manifest_ref != *input_manifest_ref
        || frozen.state_contract_ref != *node.state_contract_ref()
        || frozen.capability_binding_ref != authorization.capability_binding_ref
        || frozen.capability_operation_id != authorization.capability_operation_id
        || frozen.request_ref != authorization.request_ref
        || frozen.request_contract != *request_contract
        || frozen.returned_contract != *returned_contract
        || frozen.safe_failure_contract != *safe_failure_contract
    {
        return Err(observation_mismatch("read_frozen_intent"));
    }
    validate_value_contract(request_contract, &authorization.request_ref)?;
    validate_this_record_producer(&authorization.request_ref, READ_REQUEST_PATH)?;
    let request_bytes = resolver.value_bytes(&authorization.request_ref)?;
    let request = PlainCanonicalJsonBytes::from_canonical_json_slice(&request_bytes)
        .map_err(|_| observation_mismatch("read_request_bytes"))?;
    if derive_observation_request_digest(request_contract.schema_id().as_str(), &request)?
        != frozen.request_digest
    {
        return Err(observation_mismatch("read_request_digest"));
    }

    let binding =
        ReadCapabilityBinding::strict_decode(&resolver.content_bytes(capability_binding_ref)?)?;
    if binding.content_ref()? != *capability_binding_ref {
        return Err(observation_mismatch("read_binding"));
    }
    let binding = binding.fields()?;
    if binding.safe_classifier_contract_ref.schema_id()
        != &SafeFailureClassifierDescriptor::schema_id()
            .map_err(|_| observation_mismatch("read_classifier_schema"))?
    {
        return Err(observation_mismatch("read_classifier_schema"));
    }
    let classifier = SafeFailureClassifierDescriptor::strict_decode(
        &resolver.content_bytes(&binding.safe_classifier_contract_ref)?,
    )
    .map_err(|_| observation_mismatch("read_classifier"))?;
    let diagnostic_schema_matches = match classifier.diagnostic_schema_identity() {
        Some(identity) => {
            identity
                .schema_id()
                .map_err(|_| observation_mismatch("read_classifier"))?
                == *safe_failure_contract.schema_id()
                && identity.semantic_type_id.as_ref()
                    == Some(safe_failure_contract.semantic_type_id())
                && safe_failure_contract.media_type() == "application/json"
        }
        None => true,
    };
    if classifier
        .content_ref()
        .map_err(|_| observation_mismatch("read_classifier"))?
        != binding.safe_classifier_contract_ref
        || classifier.safe_failure_contract_ref() != &binding.safe_failure_contract_ref
        || !diagnostic_schema_matches
    {
        return Err(observation_mismatch("read_classifier"));
    }
    resolver.content_bytes(&frozen.routing_generation_ref)?;
    if !observation_content_ref_reachable(
        resolver,
        &binding.routing_catalog_ref,
        &frozen.routing_generation_ref,
    )? {
        return Err(observation_mismatch("read_routing_generation"));
    }

    match &outcome {
        ObservationOutcomeFields::Returned { result_ref } => {
            validate_value_contract(returned_contract, result_ref)?;
            validate_external_observation_producer(
                result_ref,
                authorization_ref,
                READ_RESULT_PATH,
            )?;
            PlainCanonicalJsonBytes::from_canonical_json_slice(&resolver.value_bytes(result_ref)?)
                .map_err(|_| observation_mismatch("read_returned_bytes"))?;
            verify_exact_observation_values(
                resolver,
                authorization_ref,
                std::slice::from_ref(&result_ref),
                false,
            )?;
        }
        ObservationOutcomeFields::DidNotEnter { safe_failure } => {
            verify_read_safe_failure(
                resolver,
                authorization_ref,
                safe_failure_contract,
                &classifier,
                safe_failure,
                SafeFailureOutcome::DidNotEnter,
            )?;
        }
        ObservationOutcomeFields::Indeterminate { safe_failure } => {
            verify_read_safe_failure(
                resolver,
                authorization_ref,
                safe_failure_contract,
                &classifier,
                safe_failure,
                SafeFailureOutcome::Indeterminate,
            )?;
        }
    }
    Ok(())
}

fn verify_read_safe_failure(
    resolver: &ObservationObjectResolver<'_>,
    authorization_ref: &AuthorizationRef,
    diagnostic_contract: &RetainedValueContract,
    classifier: &SafeFailureClassifierDescriptor,
    safe_failure: &SafeFailure,
    outcome: SafeFailureOutcome,
) -> Result<()> {
    let fields = safe_failure.fields()?;
    let diagnostic = fields
        .diagnostic_ref
        .as_ref()
        .map(|diagnostic_ref| {
            validate_value_contract(diagnostic_contract, diagnostic_ref)?;
            validate_external_observation_producer(
                diagnostic_ref,
                authorization_ref,
                READ_DIAGNOSTIC_PATH,
            )?;
            let bytes = resolver.value_bytes(diagnostic_ref)?;
            Ok::<_, StoreError>((diagnostic_ref.fields()?.schema_id, bytes))
        })
        .transpose()?;
    classifier
        .verify(
            &fields.safe_failure_contract_ref,
            &fields.stable_code,
            outcome,
            fields.failure_class,
            fields.boundary_stage,
            fields.coarse_size_class,
            diagnostic
                .as_ref()
                .map(|(schema_id, bytes)| (schema_id, bytes.as_slice())),
        )
        .map_err(|_| observation_mismatch("read_safe_failure"))?;
    let expected = fields
        .diagnostic_ref
        .as_ref()
        .into_iter()
        .collect::<Vec<_>>();
    verify_exact_observation_values(resolver, authorization_ref, &expected, false)?;
    Ok(())
}

fn verify_effect_observation(
    context: &ObservationVerificationContext<'_>,
    node: &CertifiedNodeContract,
    authorization_ref: &AuthorizationRef,
    authorization: &mfm_journal::v1::ExternalAccessAuthorizedFields,
    effect_request_transition_ref: &TransitionRef,
    outcome: &ObservationOutcome,
) -> Result<()> {
    let CertifiedStateExecution::Effect {
        executor_operation_id,
        executor_binding_ref,
        request_contract,
        ensure_result_contract,
        terminal_evidence_contract,
        ..
    } = node.execution()
    else {
        return Err(observation_mismatch("observation_execution_kind"));
    };
    let outcome = outcome.fields()?;
    if authorization.capability_operation_id != *executor_operation_id
        || authorization.capability_binding_ref.fields()? != *executor_binding_ref
    {
        return Err(observation_mismatch("effect_operation_binding"));
    }
    let fold = context.fold;
    let journal = context.resolver.journal;
    let resolver = &context.resolver;
    let spec = context.spec;
    let request_transition = fold
        .transitions
        .iter()
        .find(|entry| &entry.transition_ref == effect_request_transition_ref)
        .ok_or_else(|| observation_mismatch("effect_request_transition"))?;
    let request_transition_fields = request_transition.transition.fields()?;
    let TransitionBodyFields::EffectRequested {
        input_manifest_ref,
        effect_key,
        semantic_request_ref,
        request_digest,
        executor_binding_ref: retained_binding_ref,
    } = request_transition_fields.body.fields()?
    else {
        return Err(observation_mismatch("effect_request_transition"));
    };
    if request_transition_fields.node_id != *node.node_id()
        || retained_binding_ref != authorization.capability_binding_ref
        || semantic_request_ref != authorization.request_ref
    {
        return Err(observation_mismatch("effect_request_transition"));
    }
    let source_visible_sequence = request_transition_fields
        .before
        .fields()?
        .journal_head
        .fields()?
        .run_sequence;
    verify_input_manifest_identity(
        fold,
        spec,
        journal,
        node,
        &input_manifest_ref,
        authorization_ref,
        source_visible_sequence,
    )?;

    validate_value_contract(request_contract, &semantic_request_ref)?;
    validate_this_record_producer(&semantic_request_ref, EFFECT_REQUEST_PATH)?;
    let request_bytes = resolver.value_bytes(&semantic_request_ref)?;
    let request_value = SchemaQualifiedCanonicalValue::new(
        semantic_request_ref.fields()?.schema_id,
        &request_bytes,
    )
    .map_err(|_| observation_mismatch("effect_request_bytes"))?;

    let binding_bytes = resolver.content_bytes(executor_binding_ref)?;
    let binding = ExecutorBinding::strict_decode(&binding_bytes)
        .map_err(|_| observation_mismatch("effect_binding"))?;
    let typed_binding_ref = ExecutorBindingRef::from_content_ref(executor_binding_ref.clone())
        .map_err(|_| observation_mismatch("effect_binding"))?;
    if binding
        .reference()
        .map_err(|_| observation_mismatch("effect_binding"))?
        != typed_binding_ref
    {
        return Err(observation_mismatch("effect_binding"));
    }
    let contract = ExecutorContractDescriptor::strict_decode(
        &resolver.content_bytes(binding.executor_contract_ref())?,
    )
    .map_err(|_| observation_mismatch("effect_contract"))?;
    let deployment = ExecutorDeployment::strict_decode(
        &resolver.content_bytes(binding.executor_deployment_ref().as_content_ref())?,
    )
    .map_err(|_| observation_mismatch("effect_deployment"))?;
    let ownership = deployment
        .resource_ownership_ref()
        .map(|reference| {
            ResourceOwnership::strict_decode(&resolver.content_bytes(reference.as_content_ref())?)
                .map_err(|_| observation_mismatch("effect_resource_ownership"))
        })
        .transpose()?;
    let verified_binding = VerifiedExecutorBinding::verify(
        binding,
        contract,
        deployment,
        ownership,
        journal.tenant_scope_id(),
    )
    .map_err(|_| observation_mismatch("effect_binding"))?;
    let closure_contract = verified_binding.contract().retained_closure_contract();
    if verified_binding.contract().semantic_request_contract() != request_contract
        || closure_contract.ensure_result_contract() != ensure_result_contract
        || closure_contract.terminal_evidence_contract() != terminal_evidence_contract
    {
        return Err(observation_mismatch("effect_certified_contracts"));
    }
    let committed_request = CommittedEffectRequest::new(
        typed_binding_ref,
        journal.tenant_scope_id().clone(),
        journal.store_identity().store_scope_id(),
        journal.run_id(),
        node.node_id(),
        request_value,
    )
    .map_err(|_| observation_mismatch("effect_identity"))?;
    let (identity, _) = committed_request.into_parts();
    if identity.effect_key() != &effect_key || identity.request_digest() != &request_digest {
        return Err(observation_mismatch("effect_identity"));
    }

    match &outcome {
        ObservationOutcomeFields::DidNotEnter { safe_failure } => verify_effect_safe_failure(
            resolver,
            authorization_ref,
            &verified_binding,
            safe_failure,
            SafeFailureOutcome::DidNotEnter,
        ),
        ObservationOutcomeFields::Indeterminate { safe_failure } => verify_effect_safe_failure(
            resolver,
            authorization_ref,
            &verified_binding,
            safe_failure,
            SafeFailureOutcome::Indeterminate,
        ),
        ObservationOutcomeFields::Returned { result_ref } => verify_effect_returned(
            resolver,
            authorization_ref,
            &verified_binding,
            identity,
            result_ref,
            ensure_result_contract,
            terminal_evidence_contract,
        ),
    }
}

fn verify_effect_safe_failure(
    resolver: &ObservationObjectResolver<'_>,
    authorization_ref: &AuthorizationRef,
    binding: &VerifiedExecutorBinding,
    safe_failure: &SafeFailure,
    outcome: SafeFailureOutcome,
) -> Result<()> {
    let fields = safe_failure.fields()?;
    if &fields.safe_failure_contract_ref != binding.contract().safe_failure_contract_ref()
        || fields.diagnostic_ref.is_some()
        || fields.coarse_size_class.is_some()
    {
        return Err(observation_mismatch("effect_safe_failure"));
    }
    verify_reference_safe_failure_tuple(
        fields.safe_failure_contract_ref,
        &fields.stable_code,
        outcome,
        fields.failure_class,
        fields.boundary_stage,
        fields.coarse_size_class,
        false,
    )
    .map_err(|_| observation_mismatch("effect_safe_failure"))?;
    verify_exact_observation_values(resolver, authorization_ref, &[], false)?;
    Ok(())
}

fn verify_effect_returned(
    resolver: &ObservationObjectResolver<'_>,
    authorization_ref: &AuthorizationRef,
    binding: &VerifiedExecutorBinding,
    identity: EffectIdentity,
    result_ref: &ValueRef,
    ensure_result_contract: &RetainedValueContract,
    terminal_evidence_contract: &RetainedValueContract,
) -> Result<()> {
    validate_value_contract(ensure_result_contract, result_ref)?;
    validate_external_observation_producer(result_ref, authorization_ref, ENSURE_RESULT_PATH)?;
    let ensure_result = ExecutorEnsureResult::strict_decode(&resolver.value_bytes(result_ref)?)?;
    let (claim, terminal_evidence_ref, required_closure_refs) = match ensure_result.fields()? {
        ExecutorEnsureResultFields::Pending { delivery_audit_ref } => {
            validate_effect_retained_ref(
                &delivery_audit_ref,
                authorization_ref,
                binding
                    .contract()
                    .retained_closure_contract()
                    .delivery_audit_contract(),
                "executor.delivery_audit",
            )?;
            (
                ExecutorEnsureResultClaim::pending(
                    DeliveryAuditFrontierRef::from_content_ref(content_ref_for_value(
                        &delivery_audit_ref,
                    )?)
                    .map_err(|_| observation_mismatch("effect_delivery_audit"))?,
                ),
                None,
                vec![delivery_audit_ref],
            )
        }
        ExecutorEnsureResultFields::Terminal { evidence_ref } => {
            validate_value_contract(terminal_evidence_contract, &evidence_ref)?;
            validate_external_observation_producer(
                &evidence_ref,
                authorization_ref,
                TERMINAL_EVIDENCE_PATH,
            )?;
            let evidence =
                TerminalEffectEvidence::strict_decode(&resolver.value_bytes(&evidence_ref)?)?;
            let fields = evidence.fields()?;
            let contracts = binding.contract().retained_closure_contract();
            validate_effect_retained_ref(
                &fields.delivery_audit_ref,
                authorization_ref,
                contracts.delivery_audit_contract(),
                "executor.delivery_audit",
            )?;
            validate_effect_retained_ref(
                &fields.terminal_tombstone_ref,
                authorization_ref,
                contracts.terminal_tombstone_contract(),
                "executor.terminal_tombstone",
            )?;
            validate_effect_retained_ref(
                &fields.domain_evidence_ref,
                authorization_ref,
                contracts.domain_evidence_contract(),
                "executor.domain_evidence",
            )?;
            if fields.executor_binding_ref.fields()?
                != *identity.executor_binding_ref().as_content_ref()
                || fields.effect_key != *identity.effect_key()
                || fields.request_digest != *identity.request_digest()
            {
                return Err(observation_mismatch("effect_terminal_identity"));
            }
            let proof_basis = match fields.proof_basis.fields()? {
                ExecutorProofBasisFields::SelfAuthenticatingProof => {
                    ProofBasis::SelfAuthenticatingProof
                }
                ExecutorProofBasisFields::ExecutorAttestation {
                    evidence_authority_ref,
                } => ProofBasis::ExecutorAttestation {
                    evidence_authority_ref,
                },
                ExecutorProofBasisFields::TrustedObserver {
                    evidence_authority_ref,
                } => ProofBasis::TrustedObserver {
                    evidence_authority_ref,
                },
            };
            let evidence = ExecutorTerminalEvidenceClaim::new(
                identity.clone(),
                DeliveryAuditFrontierRef::from_content_ref(content_ref_for_value(
                    &fields.delivery_audit_ref,
                )?)
                .map_err(|_| observation_mismatch("effect_delivery_audit"))?,
                TerminalTombstoneRef::from_content_ref(content_ref_for_value(
                    &fields.terminal_tombstone_ref,
                )?)
                .map_err(|_| observation_mismatch("effect_terminal_tombstone"))?,
                fields.external_operation_identity.as_str(),
                fields.terminal_outcome.as_str(),
                fields.assurance_policy_ref,
                proof_basis,
                content_ref_for_value(&fields.domain_evidence_ref)?,
            )
            .map_err(|_| observation_mismatch("effect_terminal_evidence"))?;
            (
                ExecutorEnsureResultClaim::terminal(evidence),
                Some(evidence_ref),
                vec![
                    fields.delivery_audit_ref,
                    fields.terminal_tombstone_ref,
                    fields.domain_evidence_ref,
                ],
            )
        }
    };
    let produced = resolver.produced_values(authorization_ref)?;
    if required_closure_refs.iter().any(|required| {
        !produced
            .iter()
            .any(|(produced_ref, _)| produced_ref == required)
    }) {
        return Err(observation_mismatch("effect_retained_closure"));
    }
    let closure = reconstruct_effect_closure(
        resolver,
        authorization_ref,
        binding,
        result_ref,
        terminal_evidence_ref.as_ref(),
    )?;
    let verified = verify_ensure_result(identity, binding, claim, closure)
        .map_err(|_| observation_mismatch("effect_retained_closure"))?;
    if verified.retained_closure().contract() != binding.contract().retained_closure_contract() {
        return Err(observation_mismatch("effect_retained_closure"));
    }
    let produced = produced
        .into_iter()
        .map(|(value_ref, _)| value_ref)
        .collect::<Vec<_>>();
    let expected = produced.iter().collect::<Vec<_>>();
    verify_exact_observation_values(resolver, authorization_ref, &expected, false)?;
    Ok(())
}

fn validate_effect_retained_ref(
    value_ref: &ValueRef,
    authorization_ref: &AuthorizationRef,
    contract: &RetainedValueContract,
    path_prefix: &str,
) -> Result<()> {
    validate_value_contract(contract, value_ref)?;
    let reference = content_ref_for_value(value_ref)?;
    validate_external_observation_producer(
        value_ref,
        authorization_ref,
        &format!("{path_prefix}.{}", reference.content_digest().digest()),
    )
    .map_err(|_| observation_mismatch("effect_retained_relation"))
}

fn reconstruct_effect_closure(
    resolver: &ObservationObjectResolver<'_>,
    authorization_ref: &AuthorizationRef,
    binding: &VerifiedExecutorBinding,
    result_ref: &ValueRef,
    terminal_evidence_ref: Option<&ValueRef>,
) -> Result<ExecutorRetainedClosureClaim> {
    let contracts = binding.contract().retained_closure_contract();
    let mut members = Vec::new();
    for (value_ref, bytes) in resolver.produced_values(authorization_ref)? {
        if value_ref.as_bytes() == result_ref.as_bytes() {
            continue;
        }
        let ProducerBindingFields::ExternalObservation { field_path, .. } =
            value_ref.fields()?.producer_binding.fields()?
        else {
            return Err(observation_mismatch("effect_retained_producer"));
        };
        if terminal_evidence_ref.is_some_and(|expected| expected.as_bytes() == value_ref.as_bytes())
        {
            continue;
        }
        let (relation, contract, prefix) = effect_closure_relation(field_path.as_str(), contracts)?;
        validate_value_contract(contract, &value_ref)?;
        let reference = content_ref_for_value(&value_ref)?;
        if field_path.as_str() != format!("{prefix}.{}", reference.content_digest().digest()) {
            return Err(observation_mismatch("effect_retained_path"));
        }
        let value = SchemaQualifiedCanonicalValue::new(value_ref.fields()?.schema_id, &bytes)
            .map_err(|_| observation_mismatch("effect_retained_bytes"))?;
        if value
            .reference()
            .map_err(|_| observation_mismatch("effect_retained_bytes"))?
            != reference
        {
            return Err(observation_mismatch("effect_retained_bytes"));
        }
        members.push(
            ExecutorRetainedValue::new(relation, contract.clone(), value)
                .map_err(|_| observation_mismatch("effect_retained_contract"))?,
        );
    }
    ExecutorRetainedClosureClaim::new(members)
        .map_err(|_| observation_mismatch("effect_retained_closure"))
}

fn effect_closure_relation<'a>(
    path: &str,
    contracts: &'a mfm_executor::ExecutorRetainedClosureContract,
) -> Result<(
    ExecutorRetainedValueRelation,
    &'a RetainedValueContract,
    &'static str,
)> {
    let relation = if path.starts_with("executor.delivery_audit.") {
        (
            ExecutorRetainedValueRelation::DeliveryAudit,
            contracts.delivery_audit_contract(),
            "executor.delivery_audit",
        )
    } else if path.starts_with("executor.frontier.") {
        (
            ExecutorRetainedValueRelation::ExecutorFrontier,
            contracts.executor_frontier_contract(),
            "executor.frontier",
        )
    } else if path.starts_with("executor.terminal_tombstone.") {
        (
            ExecutorRetainedValueRelation::TerminalTombstone,
            contracts.terminal_tombstone_contract(),
            "executor.terminal_tombstone",
        )
    } else if path.starts_with("executor.terminal_proof.") {
        (
            ExecutorRetainedValueRelation::TerminalProof,
            contracts.terminal_proof_contract(),
            "executor.terminal_proof",
        )
    } else if path.starts_with("executor.domain_evidence.") {
        (
            ExecutorRetainedValueRelation::DomainEvidence,
            contracts.domain_evidence_contract(),
            "executor.domain_evidence",
        )
    } else {
        return Err(observation_mismatch("effect_retained_path"));
    };
    Ok(relation)
}

fn verify_frozen_read_retry(
    fold: &FoldState,
    journal: &CommittedRunJournal,
    node: &CertifiedNodeContract,
    authorization_ref: &AuthorizationRef,
    authorization: &mfm_journal::v1::ExternalAccessAuthorizedFields,
    frozen_read_intent_ref: &ValueRef,
) -> Result<u64> {
    let current_sequence = authorization_ref.fields()?.run_sequence;
    let current_resolver = ObservationObjectResolver {
        journal,
        prepared: None,
        visible_run_sequence: current_sequence,
    };
    let current_frozen = current_resolver.value_bytes(frozen_read_intent_ref)?;
    let first = fold
        .authorizations
        .iter()
        .filter_map(|candidate| {
            let candidate_ref = candidate.entry.authorization_ref.fields().ok()?;
            if candidate_ref.run_sequence > current_sequence {
                return None;
            }
            let fields = candidate.entry.authorization.fields().ok()?;
            let anchor = fields.semantic_anchor.fields().ok()?;
            if anchor.node_id != *node.node_id()
                || !matches!(
                    fields.scope.fields(),
                    Ok(AuthorizationScopeFields::Read { .. })
                )
            {
                return None;
            }
            Some((candidate_ref.run_sequence, candidate))
        })
        .min_by_key(|(sequence, _)| *sequence)
        .map(|(_, candidate)| candidate)
        .ok_or_else(|| observation_mismatch("read_retry_history"))?;
    let first_fields = first.entry.authorization.fields()?;
    let first_frozen_ref = first_fields
        .frozen_read_intent_ref
        .as_ref()
        .ok_or_else(|| observation_mismatch("read_retry_frozen_intent"))?;
    let first_sequence = first.entry.authorization_ref.fields()?.run_sequence;
    let first_resolver = ObservationObjectResolver {
        journal,
        prepared: None,
        visible_run_sequence: first_sequence,
    };
    if first_fields.request_ref != authorization.request_ref
        || first_resolver.value_bytes(first_frozen_ref)? != current_frozen
    {
        return Err(observation_mismatch("read_retry_frozen_intent"));
    }
    first_fields
        .semantic_anchor
        .fields()?
        .journal_head
        .fields()
        .map(|fields| fields.run_sequence)
        .map_err(Into::into)
}

fn verify_input_manifest_identity(
    fold: &FoldState,
    spec: &ExpandedCertifiedSpec,
    journal: &CommittedRunJournal,
    node: &CertifiedNodeContract,
    input_manifest_ref: &InputManifestRef,
    authorization_ref: &AuthorizationRef,
    source_visible_sequence: u64,
) -> Result<()> {
    let authorization_sequence = authorization_ref.fields()?.run_sequence;
    let resolver = ObservationObjectResolver {
        journal,
        prepared: None,
        visible_run_sequence: authorization_sequence,
    };
    let producer = ProducerBinding::input_assembly(journal.run_id(), node.node_id())?;
    let manifest_ref = input_manifest_ref.value_ref()?;
    let manifest_contract = spec.journal_protocol_contracts().input_manifest_contract();
    validate_value_contract(manifest_contract, &manifest_ref)?;
    validate_input_assembly_producer(&manifest_ref, journal.run_id(), node.node_id())?;
    let manifest_bytes = resolver.value_bytes(&manifest_ref)?;
    let manifest = InputManifest::strict_decode(&manifest_bytes)?;
    if derive_value_ref(manifest_contract, &producer, &manifest_bytes)? != manifest_ref {
        return Err(observation_mismatch("observation_input_manifest"));
    }
    let fields = manifest.fields()?;
    if fields.input_schema_id != *node.input_contract().schema_id()
        || fields.bindings.len() != node.input_bindings().len()
    {
        return Err(observation_mismatch("observation_input_manifest"));
    }

    let config_ref = fields
        .config_ref
        .as_ref()
        .ok_or_else(|| observation_mismatch("observation_input_config"))?;
    verify_frame_binding_value(
        &resolver,
        journal,
        fold,
        source_visible_sequence,
        node.config_binding(),
        config_ref,
        &producer,
    )?;
    match (node.context_binding(), fields.context_ref.as_ref()) {
        (Some(binding), Some(context_ref)) => verify_frame_binding_value(
            &resolver,
            journal,
            fold,
            source_visible_sequence,
            binding,
            context_ref,
            &producer,
        )?,
        (None, None) => {}
        _ => return Err(observation_mismatch("observation_input_context")),
    }

    let mut reconstructed = serde_json::Value::Object(serde_json::Map::new());
    for (recorded, certified) in fields.bindings.iter().zip(node.input_bindings()) {
        let recorded = recorded.fields()?;
        if recorded.field_path != *certified.destination_field_path() {
            return Err(observation_mismatch("observation_input_destination"));
        }
        let selected = certified
            .ordered_sources()
            .iter()
            .map(|selector| {
                resolve_observation_source(
                    &resolver,
                    journal,
                    fold,
                    source_visible_sequence,
                    selector,
                )
            })
            .find_map(|source| match source {
                Ok(Some(source)) => Some(Ok(source)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .transpose()?
            .ok_or_else(|| observation_mismatch("observation_input_source"))?;
        if recorded.source != selected.source
            || recorded.value_ref != selected.root_ref
            || recorded.source_field_path != selected.source_field_path
        {
            return Err(observation_mismatch("observation_input_source"));
        }
        if recorded.source_field_path.is_none() {
            validate_value_contract(certified.value_contract(), &recorded.value_ref)?;
        }
        insert_json_path(
            &mut reconstructed,
            certified.destination_field_path(),
            serde_json::from_slice(selected.selected_bytes.as_bytes())
                .map_err(|_| observation_mismatch("observation_input_source"))?,
        )
        .map_err(|_| observation_mismatch("observation_input_assembly"))?;
    }
    let reconstructed = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&reconstructed)
            .map_err(|_| observation_mismatch("observation_input_assembly"))?,
    )
    .map_err(|_| observation_mismatch("observation_input_assembly"))?;
    let expected_root =
        derive_value_ref(node.input_contract(), &producer, reconstructed.as_bytes())?;
    if fields.root_input_ref != expected_root
        || resolver.value_bytes(&fields.root_input_ref)? != reconstructed.as_bytes()
    {
        return Err(observation_mismatch("observation_input_assembly"));
    }
    Ok(())
}

fn verify_frame_binding_value(
    resolver: &ObservationObjectResolver<'_>,
    journal: &CommittedRunJournal,
    fold: &FoldState,
    source_visible_sequence: u64,
    binding: &CertifiedFrameBinding,
    actual_ref: &ValueRef,
    producer: &ProducerBinding,
) -> Result<()> {
    let source = resolve_observation_source(
        resolver,
        journal,
        fold,
        source_visible_sequence,
        binding.source(),
    )?
    .ok_or_else(|| observation_mismatch("observation_frame_source"))?;
    let expected_ref = if source.source_field_path.is_none() {
        validate_value_contract(binding.value_contract(), &source.root_ref)?;
        source.root_ref
    } else {
        derive_value_ref(
            binding.value_contract(),
            producer,
            source.selected_bytes.as_bytes(),
        )?
    };
    if actual_ref != &expected_ref
        || resolver.value_bytes(actual_ref)? != source.selected_bytes.as_bytes()
    {
        return Err(observation_mismatch("observation_frame_value"));
    }
    Ok(())
}

struct ObservationResolvedSource {
    source: InputSource,
    root_ref: ValueRef,
    selected_bytes: PlainCanonicalJsonBytes,
    source_field_path: Option<FieldPath>,
}

fn resolve_observation_source(
    resolver: &ObservationObjectResolver<'_>,
    journal: &CommittedRunJournal,
    fold: &FoldState,
    visible_run_sequence: u64,
    selector: &CertifiedSourceSelector,
) -> Result<Option<ObservationResolvedSource>> {
    let admission = match journal.normalized_batches.first() {
        Some(NormalizedBatch::Admission(admission)) => admission,
        _ => return Err(StoreError::EmptyJournal),
    };
    let admission_fields = admission.fields()?;
    match selector {
        CertifiedSourceSelector::RunAdmission { source_field_path } => {
            let root_ref = observation_initial_binding(
                &admission_fields.initial_bindings,
                RUN_ADMISSION_INPUT_PATH,
            )?;
            let record_ref = journal
                .commits
                .first()
                .and_then(|commit| commit.records.first())
                .ok_or(StoreError::EmptyJournal)?
                .record_ref(journal.run_id(), 1)?;
            observation_resolved(
                resolver,
                InputSource::run_admission(&record_ref)?,
                root_ref,
                source_field_path.clone(),
            )
            .map(Some)
        }
        CertifiedSourceSelector::Config { source_field_path } => {
            let root_ref = observation_initial_binding(
                &admission_fields.initial_bindings,
                CONFIGURED_VALUE_PATH,
            )?;
            observation_resolved(
                resolver,
                InputSource::config(&root_ref)?,
                root_ref,
                source_field_path.clone(),
            )
            .map(Some)
        }
        CertifiedSourceSelector::QualifiedSupport {
            member_path,
            source_field_path,
        } => {
            let root_ref = observation_initial_binding(
                &admission_fields.initial_bindings,
                member_path.as_str(),
            )?;
            observation_resolved(
                resolver,
                InputSource::qualified_support(member_path, &root_ref)?,
                root_ref,
                source_field_path.clone(),
            )
            .map(Some)
        }
        CertifiedSourceSelector::Seed { source_field_path } => {
            let manifest = SeedManifest::strict_decode(
                &resolver.content_bytes(&admission_fields.seed_manifest_ref)?,
            )?;
            let entries = manifest
                .entries()?
                .into_iter()
                .map(|entry| Ok((entry.field_path()?, entry.value_ref()?)))
                .collect::<std::result::Result<Vec<_>, mfm_journal::v1::JournalError>>()?;
            let (entry_path, root_ref, nested) =
                select_manifest_value(entries, source_field_path.as_ref())?;
            observation_require_initial_binding(
                &admission_fields.initial_bindings,
                &format!("seed.{}", entry_path.as_str()),
                &root_ref,
            )?;
            observation_resolved(resolver, InputSource::seed(&root_ref)?, root_ref, nested)
                .map(Some)
        }
        CertifiedSourceSelector::Context { source_field_path } => {
            let manifest = ContextManifest::strict_decode(
                &resolver.content_bytes(&admission_fields.context_manifest_ref)?,
            )?;
            let entries = manifest
                .entries()?
                .into_iter()
                .map(|entry| Ok((entry.field_path()?, entry.value_ref()?)))
                .collect::<std::result::Result<Vec<_>, mfm_journal::v1::JournalError>>()?;
            let (entry_path, root_ref, nested) =
                select_manifest_value(entries, source_field_path.as_ref())?;
            observation_require_initial_binding(
                &admission_fields.initial_bindings,
                &format!("context.{}", entry_path.as_str()),
                &root_ref,
            )?;
            observation_resolved(resolver, InputSource::context(&root_ref)?, root_ref, nested)
                .map(Some)
        }
        CertifiedSourceSelector::NodeOutput {
            producer_node_id,
            output_ordinal,
            source_field_path,
        } => {
            let Some((transition_ref, value_ref)) = observation_transition_output(
                fold,
                visible_run_sequence,
                producer_node_id,
                *output_ordinal,
            )?
            else {
                return Ok(None);
            };
            let output_ref = OutputRef::new(&transition_ref, *output_ordinal)?;
            observation_resolved(
                resolver,
                InputSource::transition_output(journal.run_id(), &transition_ref, &output_ref)?,
                value_ref,
                source_field_path.clone(),
            )
            .map(Some)
        }
        CertifiedSourceSelector::NodeFact {
            producer_node_id,
            emission_ordinal,
        } => {
            let Some((transition_ref, value_ref)) = observation_transition_fact(
                fold,
                visible_run_sequence,
                producer_node_id,
                *emission_ordinal,
            )?
            else {
                return Ok(None);
            };
            let fact_ref = FactRef::new(&transition_ref, *emission_ordinal)?;
            observation_resolved(
                resolver,
                InputSource::transition_fact(journal.run_id(), &transition_ref, &fact_ref)?,
                value_ref,
                None,
            )
            .map(Some)
        }
        CertifiedSourceSelector::CrossRunEffectiveOutput { source_field_path } => {
            resolve_observation_cross_run(
                resolver,
                &admission_fields,
                source_field_path.as_ref(),
                None,
            )
        }
        CertifiedSourceSelector::CrossRunEvidence {
            source_field_path,
            certified_evidence_role_ref,
        } => resolve_observation_cross_run(
            resolver,
            &admission_fields,
            source_field_path.as_ref(),
            Some(certified_evidence_role_ref),
        ),
    }
}

fn resolve_observation_cross_run(
    resolver: &ObservationObjectResolver<'_>,
    admission: &mfm_journal::v1::RunAdmittedFields,
    selected_path: Option<&FieldPath>,
    evidence_role: Option<&ContentRef>,
) -> Result<Option<ObservationResolvedSource>> {
    let manifest = CrossRunSourceManifest::strict_decode(
        &resolver.content_bytes(&admission.cross_run_source_manifest_ref)?,
    )?;
    let mut entries = Vec::new();
    for entry in manifest.entries()? {
        let source = entry.source()?;
        let role_matches = match (source.fields()?, evidence_role) {
            (CrossRunSourceRefFields::EffectiveOutput { .. }, None) => true,
            (
                CrossRunSourceRefFields::EvidenceOnly {
                    certified_evidence_role_ref,
                    ..
                },
                Some(expected),
            ) => certified_evidence_role_ref == *expected,
            _ => false,
        };
        if role_matches {
            entries.push((entry.field_path()?, source));
        }
    }
    let (entry_path, source, nested) = select_manifest_value(entries, selected_path)?;
    let root_ref = observation_initial_binding(
        &admission.initial_bindings,
        &format!("cross_run.{}", entry_path.as_str()),
    )?;
    observation_resolved(resolver, InputSource::cross_run(&source)?, root_ref, nested).map(Some)
}

fn observation_resolved(
    resolver: &ObservationObjectResolver<'_>,
    source: InputSource,
    root_ref: ValueRef,
    source_field_path: Option<FieldPath>,
) -> Result<ObservationResolvedSource> {
    let selected_bytes = select_canonical(
        &resolver.value_bytes(&root_ref)?,
        source_field_path.as_ref(),
    )?;
    Ok(ObservationResolvedSource {
        source,
        root_ref,
        selected_bytes,
        source_field_path,
    })
}

fn observation_initial_binding(bindings: &[InitialBinding], path: &str) -> Result<ValueRef> {
    let path = FieldPath::new(path)?;
    bindings
        .iter()
        .map(InitialBinding::fields)
        .collect::<std::result::Result<Vec<_>, mfm_journal::v1::JournalError>>()?
        .into_iter()
        .find(|binding| binding.field_path == path)
        .map(|binding| binding.value_ref)
        .ok_or_else(|| observation_mismatch("observation_initial_binding"))
}

fn observation_require_initial_binding(
    bindings: &[InitialBinding],
    path: &str,
    expected: &ValueRef,
) -> Result<()> {
    if observation_initial_binding(bindings, path)? == *expected {
        Ok(())
    } else {
        Err(observation_mismatch("observation_initial_binding"))
    }
}

fn observation_transition_output(
    fold: &FoldState,
    visible_run_sequence: u64,
    producer_node_id: &NodeId,
    output_ordinal: u32,
) -> Result<Option<(TransitionRef, ValueRef)>> {
    for entry in &fold.transitions {
        if entry.containing_journal_head.fields()?.run_sequence > visible_run_sequence {
            continue;
        }
        let transition = entry.transition.fields()?;
        if transition.node_id != *producer_node_id {
            continue;
        }
        let settlement = match transition.body.fields()? {
            TransitionBodyFields::PureSettled { settlement, .. }
            | TransitionBodyFields::ReadSettled { settlement, .. }
            | TransitionBodyFields::EffectSettled { settlement, .. } => settlement,
            TransitionBodyFields::EffectRequested { .. }
            | TransitionBodyFields::DependencySkipped { .. } => continue,
        };
        let SettlementFields::Succeeded {
            output_bindings, ..
        } = settlement.fields()?
        else {
            continue;
        };
        if let Some(output) = output_bindings.iter().find(|output| {
            output
                .fields()
                .is_ok_and(|fields| fields.output_ordinal == output_ordinal)
        }) {
            return Ok(Some((
                entry.transition_ref.clone(),
                output.fields()?.value_ref,
            )));
        }
    }
    Ok(None)
}

fn observation_transition_fact(
    fold: &FoldState,
    visible_run_sequence: u64,
    producer_node_id: &NodeId,
    emission_ordinal: u32,
) -> Result<Option<(TransitionRef, ValueRef)>> {
    for entry in &fold.transitions {
        if entry.containing_journal_head.fields()?.run_sequence > visible_run_sequence {
            continue;
        }
        let transition = entry.transition.fields()?;
        if transition.node_id != *producer_node_id {
            continue;
        }
        let settlement = match transition.body.fields()? {
            TransitionBodyFields::PureSettled { settlement, .. }
            | TransitionBodyFields::ReadSettled { settlement, .. }
            | TransitionBodyFields::EffectSettled { settlement, .. } => settlement,
            TransitionBodyFields::EffectRequested { .. }
            | TransitionBodyFields::DependencySkipped { .. } => continue,
        };
        let SettlementFields::Succeeded { fact_emissions, .. } = settlement.fields()? else {
            continue;
        };
        if let Some(fact) = fact_emissions.iter().find(|fact| {
            fact.fields()
                .is_ok_and(|fields| fields.emission_ordinal == emission_ordinal)
        }) {
            return Ok(Some((
                entry.transition_ref.clone(),
                fact.fields()?.claim_ref,
            )));
        }
    }
    Ok(None)
}

fn validate_this_record_producer(value_ref: &ValueRef, expected_path: &str) -> Result<()> {
    let ProducerBindingFields::ThisRecord { field_path } =
        value_ref.fields()?.producer_binding.fields()?
    else {
        return Err(observation_mismatch("observation_producer"));
    };
    if field_path.as_str() != expected_path {
        return Err(observation_mismatch("observation_producer"));
    }
    Ok(())
}

fn validate_input_assembly_producer(
    value_ref: &ValueRef,
    run_id: &RunId,
    node_id: &NodeId,
) -> Result<()> {
    match value_ref.fields()?.producer_binding.fields()? {
        ProducerBindingFields::InputAssembly {
            run_id: producer_run,
            node_id: producer_node,
        } if producer_run == *run_id && producer_node == *node_id => Ok(()),
        _ => Err(observation_mismatch("observation_input_producer")),
    }
}

fn validate_external_observation_producer(
    value_ref: &ValueRef,
    authorization_ref: &AuthorizationRef,
    expected_path: &str,
) -> Result<()> {
    match value_ref.fields()?.producer_binding.fields()? {
        ProducerBindingFields::ExternalObservation {
            authorization_ref: producer,
            field_path,
        } if producer == *authorization_ref && field_path.as_str() == expected_path => Ok(()),
        _ => Err(observation_mismatch("observation_producer")),
    }
}

fn content_ref_for_value(value_ref: &ValueRef) -> Result<ContentRef> {
    let fields = value_ref.fields()?;
    ContentRef::new(fields.schema_id, fields.content_digest).map_err(Into::into)
}

fn derive_observation_request_digest(
    request_schema_id: &str,
    request: &PlainCanonicalJsonBytes,
) -> Result<mfm_ids::RequestDigest> {
    let request_value: serde_json::Value = serde_json::from_slice(request.as_bytes())
        .map_err(|_| observation_mismatch("observation_request"))?;
    let preimage = serde_json::json!({
        "request_schema_id": request_schema_id,
        "request_value": request_value,
    });
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&preimage)
            .map_err(|_| observation_mismatch("observation_request"))?,
    )
    .map_err(|_| observation_mismatch("observation_request"))?;
    let contract = RecoverabilityContractV1::embedded()?;
    let validated = contract.strict_decode(REQUEST_PREIMAGE_SCHEMA, canonical.as_bytes())?;
    contract
        .derive_request_digest(&validated)
        .map_err(Into::into)
}

fn observation_content_ref_reachable(
    resolver: &ObservationObjectResolver<'_>,
    root: &ContentRef,
    target: &ContentRef,
) -> Result<bool> {
    let mut queue = std::collections::VecDeque::from([root.clone()]);
    let mut visited = BTreeSet::new();
    while let Some(reference) = queue.pop_front() {
        if !visited.insert(reference.clone()) {
            continue;
        }
        if &reference == target {
            return Ok(true);
        }
        if visited.len() > 4_096 {
            return Err(observation_mismatch("read_routing_catalog"));
        }
        let bytes = resolver.content_bytes(&reference)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| observation_mismatch("read_routing_catalog"))?;
        collect_observation_content_refs(&value, &mut queue);
    }
    Ok(false)
}

fn collect_observation_content_refs(
    value: &serde_json::Value,
    output: &mut std::collections::VecDeque<ContentRef>,
) {
    match value {
        serde_json::Value::Object(fields) => {
            if let Ok(reference) = serde_json::from_value::<ContentRef>(value.clone()) {
                output.push_back(reference);
            } else {
                for nested in fields.values() {
                    collect_observation_content_refs(nested, output);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                collect_observation_content_refs(nested, output);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

const fn observation_mismatch(field: &'static str) -> StoreError {
    StoreError::PersistedMismatch { field }
}

fn authorization_effect_key(
    fold: &FoldState,
    authorization: &ExternalAccessAuthorized,
) -> Result<Option<mfm_ids::EffectKey>> {
    let AuthorizationScopeFields::EnsureEffect {
        effect_request_transition_ref,
    } = authorization.fields()?.scope.fields()?
    else {
        return Ok(None);
    };
    let transition = fold
        .transitions
        .iter()
        .find(|entry| entry.transition_ref == effect_request_transition_ref)
        .ok_or(StoreError::AuthorizationNotEligible)?;
    match transition.transition.fields()?.body.fields()? {
        TransitionBodyFields::EffectRequested { effect_key, .. } => Ok(Some(effect_key)),
        _ => Err(StoreError::AuthorizationNotEligible),
    }
}

fn authorization_effect_key_from_entry(
    entry: &FoldedAccessAuditEntry,
) -> Result<Option<mfm_ids::EffectKey>> {
    Ok(entry.audit_entry.fields()?.effect_key)
}

fn audit_outcome_fields(
    observation: &ExternalAccessObserved,
) -> Result<(AccessAuditStatus, Option<SafeFailure>, Option<ValueRef>)> {
    Ok(match observation.fields()?.outcome.fields()? {
        ObservationOutcomeFields::Returned { result_ref } => {
            (AccessAuditStatus::Returned, None, Some(result_ref))
        }
        ObservationOutcomeFields::DidNotEnter { safe_failure } => {
            (AccessAuditStatus::DidNotEnter, Some(safe_failure), None)
        }
        ObservationOutcomeFields::Indeterminate { safe_failure } => {
            (AccessAuditStatus::Indeterminate, Some(safe_failure), None)
        }
    })
}

/// Borrowed complete transition-frame traversal over one verified root run.
pub struct TransitionFrameReader<'view> {
    view: &'view VerifiedRunView,
}

impl<'view> TransitionFrameReader<'view> {
    /// Returns complete transition frames in committed semantic order.
    pub fn frames(&self) -> impl ExactSizeIterator<Item = TransitionFrame<'view>> + '_ {
        self.view
            .fold
            .transitions
            .iter()
            .map(|entry| TransitionFrame {
                view: self.view,
                entry,
            })
    }
}

/// One borrowed complete transition frame and all of its verified store relationships.
pub struct TransitionFrame<'view> {
    view: &'view VerifiedRunView,
    entry: &'view FoldedTransitionEntry,
}

impl<'view> TransitionFrame<'view> {
    /// Returns the exact assigned transition reference.
    pub const fn transition_ref(&self) -> &TransitionRef {
        self.entry.transition_ref()
    }

    /// Returns the complete committed transition.
    pub const fn transition(&self) -> &StateTransitionCommitted {
        self.entry.transition()
    }

    /// Returns the exact certified occurrence, including its canonical expansion path.
    pub fn certified_node(&self) -> Result<&'view CertifiedNodeContract> {
        let node_id = self.entry.transition.fields()?.node_id;
        certified_node(&self.view.certified_spec, &node_id)
    }

    /// Returns the physical head containing this semantic transition.
    pub const fn containing_journal_head(&self) -> &JournalHead {
        self.entry.containing_journal_head()
    }

    /// Returns the inseparable closure reference when this is the terminal transition.
    pub const fn closure_ref(&self) -> Option<&ClosureRef> {
        self.entry.closure_ref()
    }

    /// Returns the exact input-manifest reference used by this transition, when applicable.
    pub fn input_manifest_ref(&self) -> Result<Option<InputManifestRef>> {
        Ok(match self.entry.transition.fields()?.body.fields()? {
            TransitionBodyFields::PureSettled {
                input_manifest_ref, ..
            }
            | TransitionBodyFields::ReadSettled {
                input_manifest_ref, ..
            }
            | TransitionBodyFields::EffectRequested {
                input_manifest_ref, ..
            } => Some(input_manifest_ref),
            TransitionBodyFields::EffectSettled {
                request_input_manifest_ref,
                ..
            } => Some(request_input_manifest_ref),
            TransitionBodyFields::DependencySkipped { .. } => None,
        })
    }

    /// Returns the strictly decoded exact input manifest, when applicable.
    pub fn input_manifest(&self) -> Result<Option<InputManifest>> {
        self.input_manifest_ref()?
            .as_ref()
            .map(|reference| self.view.input_manifest(reference))
            .transpose()
    }

    /// Returns the consumed authorization, observation, and audit projection when settled from IO.
    pub fn consumed_access(&self) -> Result<Option<&'view FoldedAccessAuditEntry>> {
        let consumed = match self.entry.transition.fields()?.body.fields()? {
            TransitionBodyFields::ReadSettled {
                consumed_observation_ref,
                ..
            } => Some(consumed_observation_ref),
            TransitionBodyFields::EffectSettled {
                consumed_terminal_observation_ref,
                ..
            } => Some(consumed_terminal_observation_ref),
            TransitionBodyFields::PureSettled { .. }
            | TransitionBodyFields::EffectRequested { .. }
            | TransitionBodyFields::DependencySkipped { .. } => None,
        };
        let Some(consumed) = consumed else {
            return Ok(None);
        };
        self.view
            .fold
            .authorizations
            .iter()
            .find(|state| {
                state
                    .entry
                    .observation
                    .as_ref()
                    .is_some_and(|(reference, _, _)| reference == &consumed)
            })
            .map(|state| &state.entry)
            .ok_or(StoreError::ObservationNotConsumable)
            .map(Some)
    }
}

/// One exact immutable root required by the run admission.
pub struct VerifiedAdmissionRoot {
    field_path: FieldPath,
    value_ref: ValueRef,
}

impl VerifiedAdmissionRoot {
    /// Returns the stable field within its admission manifest.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns the complete verified retained authority.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }
}

/// One exact registration-qualified support root required by the admission.
pub struct VerifiedQualifiedSupportRoot {
    qualification_scope_id: SemanticTypeId,
    member_path: FieldPath,
    value_ref: ValueRef,
}

impl VerifiedQualifiedSupportRoot {
    /// Returns the exact qualification scope that produced this root.
    pub const fn qualification_scope_id(&self) -> &SemanticTypeId {
        &self.qualification_scope_id
    }

    /// Returns the stable member path in that qualified graph.
    pub const fn member_path(&self) -> &FieldPath {
        &self.member_path
    }

    /// Returns the complete verified retained authority.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }
}

/// One exact closed cross-run source required by the admission.
pub struct VerifiedCrossRunSourceRequirement {
    field_path: FieldPath,
    destination_value_ref: ValueRef,
    source_ref: CrossRunSourceRef,
    source_run_id: RunId,
    source_admission_ref: RecordRef,
    source_closure_ref: ClosureRef,
}

impl VerifiedCrossRunSourceRequirement {
    /// Returns the stable destination in the cross-run source manifest.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns the complete destination authority admitted into this run.
    pub const fn destination_value_ref(&self) -> &ValueRef {
        &self.destination_value_ref
    }

    /// Returns the complete closed source role and lineage.
    pub const fn source_ref(&self) -> &CrossRunSourceRef {
        &self.source_ref
    }

    /// Returns the validated closed source run.
    pub const fn source_run_id(&self) -> &RunId {
        &self.source_run_id
    }

    /// Returns the validated immutable source admission.
    pub const fn source_admission_ref(&self) -> &RecordRef {
        &self.source_admission_ref
    }

    /// Returns the validated semantic source closure.
    pub const fn source_closure_ref(&self) -> &ClosureRef {
        &self.source_closure_ref
    }

    /// Verifies one supplied closed source view against every cached source coordinate.
    ///
    /// This check grants no source access. Callers must separately obtain the source view under
    /// its own purpose authority before presenting it here.
    pub fn verify_closed_source_view(&self, source_view: &VerifiedRunView) -> Result<()> {
        let identity = self.source_ref.identity()?;
        if identity.source_run_id != self.source_run_id
            || identity.source_admission_ref != self.source_admission_ref
            || identity.source_closure_ref != self.source_closure_ref
        {
            return Err(StoreError::PersistedMismatch {
                field: "cross_run_source_view",
            });
        }
        verify_cross_run_source_view(&self.source_ref, &self.destination_value_ref, source_view)
    }
}

pub(super) fn verify_cross_run_source_view(
    source_ref: &CrossRunSourceRef,
    destination_value_ref: &ValueRef,
    source_view: &VerifiedRunView,
) -> Result<()> {
    let identity = source_ref.identity()?;
    let admission_ref = source_view
        .journal()
        .commits()
        .first()
        .and_then(|commit| commit.records().first())
        .ok_or(StoreError::EmptyJournal)?
        .record_ref(source_view.run_id(), 1)?;
    let closure_matches = source_view
        .transition_entries()
        .filter_map(|entry| entry.closure_ref())
        .any(|reference| reference == &identity.source_closure_ref);
    if source_view.store_identity().store_scope_id() != &identity.source_store_scope_id
        || source_view.store_identity().store_epoch() != identity.source_store_epoch
        || source_view.run_id() != &identity.source_run_id
        || admission_ref != identity.source_admission_ref
        || source_view.certified_spec().spec_hash()? != identity.source_spec_hash
        || !closure_matches
        || source_view.node_phase(&identity.source_node_id) != Some(NodePhase::Terminal)
    {
        return Err(StoreError::PersistedMismatch {
            field: "cross_run_source_view",
        });
    }

    let destination = destination_value_ref.fields()?;
    match source_ref.fields()? {
        CrossRunSourceRefFields::EffectiveOutput {
            effective_transition_ref,
            effective_output_ref,
            ..
        } => {
            if source_view.node_terminal_transition_ref(&identity.source_node_id)
                != Some(&effective_transition_ref)
                || effective_output_ref.fields()?.transition_ref != effective_transition_ref
            {
                return Err(StoreError::PersistedMismatch {
                    field: "cross_run_effective_output",
                });
            }
            let source = source_view
                .output_binding(&effective_output_ref)?
                .fields()?
                .value_ref
                .fields()?;
            if !same_retained_value_shape(&destination, &source) {
                return Err(StoreError::PersistedMismatch {
                    field: "cross_run_effective_output",
                });
            }
        }
        CrossRunSourceRefFields::EvidenceOnly {
            raw_transition_ref,
            raw_result_or_evidence_ref,
            ..
        } => {
            let transition_matches = source_view.transition_entries().any(|entry| {
                entry.transition_ref() == &raw_transition_ref
                    && entry
                        .transition()
                        .fields()
                        .is_ok_and(|fields| fields.node_id == identity.source_node_id)
            });
            let expected = raw_result_or_evidence_ref.fields()?;
            let source_matches = source_view.journal().objects().any(|object| {
                object.value_ref().fields().is_ok_and(|source| {
                    source.artifact_id == expected.artifact_id
                        && source.content_digest == expected.content_digest
                        && source.evidence_hash == expected.evidence_hash
                        && source.schema_id == expected.schema_id
                        && source.semantic_type_id == expected.semantic_type_id
                        && source.role == expected.role
                        && source.byte_length == expected.byte_length
                        && source.media_type == expected.media_type
                        && source.evidence_contract_ref == expected.evidence_contract_ref
                        && same_retained_value_shape(&destination, &source)
                })
            });
            if !transition_matches || !source_matches {
                return Err(StoreError::PersistedMismatch {
                    field: "cross_run_evidence",
                });
            }
        }
    }
    Ok(())
}

fn same_retained_value_shape(
    left: &mfm_journal::v1::ValueRefFields,
    right: &mfm_journal::v1::ValueRefFields,
) -> bool {
    left.artifact_id == right.artifact_id
        && left.content_digest == right.content_digest
        && left.evidence_hash == right.evidence_hash
        && left.schema_id == right.schema_id
        && left.semantic_type_id == right.semantic_type_id
        && left.role == right.role
        && left.byte_length == right.byte_length
        && left.media_type == right.media_type
        && left.evidence_contract_ref == right.evidence_contract_ref
}

/// Complete store-verified admission source closure.
///
/// Construction is part of recorded-history verification. This value has no public constructor,
/// clone implementation, or serialization surface.
pub struct VerifiedAdmissionSourceRequirements {
    config_manifest_ref: ContentRef,
    seed_manifest_ref: ContentRef,
    context_manifest_ref: ContentRef,
    cross_run_source_manifest_ref: ContentRef,
    config_roots: Vec<VerifiedAdmissionRoot>,
    seed_roots: Vec<VerifiedAdmissionRoot>,
    context_roots: Vec<VerifiedAdmissionRoot>,
    qualified_support_roots: Vec<VerifiedQualifiedSupportRoot>,
    cross_run_sources: Vec<VerifiedCrossRunSourceRequirement>,
}

/// Exact historical configured value frozen by this admission.
///
/// The projection is created only while verifying the admission closure and cannot be converted
/// into mutable configuration or a fresh admission authority.
pub struct VerifiedRecordedConfiguredValue {
    recorded_binding: ConfiguredValueBinding,
    recorded_entry_point_id: EntryPointId,
    target: StableId,
    value_contract: RetainedValueContract,
    value_ref: ValueRef,
    bytes: Vec<u8>,
}

impl VerifiedRecordedConfiguredValue {
    /// Returns the reconstructed immutable configured-value binding.
    pub const fn recorded_binding(&self) -> &ConfiguredValueBinding {
        &self.recorded_binding
    }

    /// Returns the exact historical entry-point version in the configured key.
    pub const fn recorded_entry_point_id(&self) -> &EntryPointId {
        &self.recorded_entry_point_id
    }

    /// Returns the exact configured target.
    pub const fn target(&self) -> &StableId {
        &self.target
    }

    /// Returns the complete producer-independent historical retained-value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }

    /// Returns the complete producer-bound configured value.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }

    /// Returns the exact verified immutable configured bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl VerifiedAdmissionSourceRequirements {
    /// Returns the exact immutable configuration-manifest reference.
    pub const fn config_manifest_ref(&self) -> &ContentRef {
        &self.config_manifest_ref
    }

    /// Returns the exact immutable seed-manifest reference.
    pub const fn seed_manifest_ref(&self) -> &ContentRef {
        &self.seed_manifest_ref
    }

    /// Returns the exact immutable context-manifest reference.
    pub const fn context_manifest_ref(&self) -> &ContentRef {
        &self.context_manifest_ref
    }

    /// Returns the exact closed cross-run-source-manifest reference.
    pub const fn cross_run_source_manifest_ref(&self) -> &ContentRef {
        &self.cross_run_source_manifest_ref
    }

    /// Returns configuration roots in manifest order.
    pub fn config_roots(&self) -> &[VerifiedAdmissionRoot] {
        &self.config_roots
    }

    /// Returns seed roots in manifest order.
    pub fn seed_roots(&self) -> &[VerifiedAdmissionRoot] {
        &self.seed_roots
    }

    /// Returns context roots in manifest order.
    pub fn context_roots(&self) -> &[VerifiedAdmissionRoot] {
        &self.context_roots
    }

    /// Returns direct qualified-support roots in canonical member-path order.
    pub fn qualified_support_roots(&self) -> &[VerifiedQualifiedSupportRoot] {
        &self.qualified_support_roots
    }

    /// Returns closed cross-run requirements in manifest order.
    pub fn cross_run_sources(&self) -> &[VerifiedCrossRunSourceRequirement] {
        &self.cross_run_sources
    }
}

/// Opaque callback-free structural view over one completely verified run history.
///
/// This value is intentionally non-cloneable and has no public fields or constructor.
pub struct VerifiedRunView {
    journal: CommittedRunJournal,
    admission: RunAdmitted,
    certified_spec: ExpandedCertifiedSpec,
    certificate: Certificate,
    state_implementation_manifest: StateImplementationManifest,
    capability_binding_manifest: CapabilityBindingManifest,
    config_manifest: ConfigManifest,
    seed_manifest: SeedManifest,
    context_manifest: ContextManifest,
    cross_run_source_manifest: CrossRunSourceManifest,
    admission_source_requirements: VerifiedAdmissionSourceRequirements,
    recorded_configured_value: VerifiedRecordedConfiguredValue,
    fold: FoldState,
}

impl VerifiedRunView {
    /// Returns the exact store lineage.
    pub const fn store_identity(&self) -> &StoreIdentity {
        self.journal.store_identity()
    }

    /// Returns the admitted tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        self.journal.tenant_scope_id()
    }

    /// Returns the admitted run.
    pub const fn run_id(&self) -> &RunId {
        self.journal.run_id()
    }

    /// Returns the current physical journal head, including audit-tail commits.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.fold.journal_head
    }

    /// Returns the latest head that changed semantic run state.
    ///
    /// A later authorization or audit-tail observation advances [`Self::journal_head`] without
    /// changing this coordinate.
    pub const fn semantic_head(&self) -> &JournalHead {
        &self.fold.semantic_head
    }

    /// Returns the exact admission or transition record that last changed semantic run state.
    ///
    /// Audit-tail authorizations and observations advance [`Self::journal_head`] without changing
    /// this record coordinate.
    pub const fn semantic_head_record_ref(&self) -> &RecordRef {
        &self.fold.semantic_head_record_ref
    }

    /// Returns the current canonical semantic-state digest.
    pub const fn run_state_digest(&self) -> &RunSemanticStateDigest {
        &self.fold.run_state_digest
    }

    /// Returns whether the semantic run is open or permanently closed.
    pub const fn run_phase(&self) -> RunPhase {
        self.fold.run_phase
    }

    /// Returns the immutable admitted root.
    pub const fn admission(&self) -> &RunAdmitted {
        &self.admission
    }

    /// Returns the exact decoded certified graph.
    pub const fn certified_spec(&self) -> &ExpandedCertifiedSpec {
        &self.certified_spec
    }

    /// Returns the exact decoded certificate bound by admission.
    pub const fn certificate(&self) -> &Certificate {
        &self.certificate
    }

    /// Returns the exact decoded admitted state implementation manifest.
    pub const fn state_implementation_manifest(&self) -> &StateImplementationManifest {
        &self.state_implementation_manifest
    }

    /// Returns the exact decoded admitted capability binding manifest.
    pub const fn capability_binding_manifest(&self) -> &CapabilityBindingManifest {
        &self.capability_binding_manifest
    }

    /// Returns the exact decoded immutable configuration manifest.
    pub const fn config_manifest(&self) -> &ConfigManifest {
        &self.config_manifest
    }

    /// Returns the exact decoded immutable seed manifest.
    pub const fn seed_manifest(&self) -> &SeedManifest {
        &self.seed_manifest
    }

    /// Returns the exact decoded immutable context manifest.
    pub const fn context_manifest(&self) -> &ContextManifest {
        &self.context_manifest
    }

    /// Returns the exact decoded closed cross-run source manifest.
    pub const fn cross_run_source_manifest(&self) -> &CrossRunSourceManifest {
        &self.cross_run_source_manifest
    }

    /// Returns the complete verified immutable source closure fixed at admission.
    pub const fn admission_source_requirements(&self) -> &VerifiedAdmissionSourceRequirements {
        &self.admission_source_requirements
    }

    /// Returns the sole exact configured value historically frozen by admission.
    pub const fn recorded_configured_value(&self) -> &VerifiedRecordedConfiguredValue {
        &self.recorded_configured_value
    }

    /// Returns the physically verified journal used by replay and export.
    pub const fn journal(&self) -> &CommittedRunJournal {
        &self.journal
    }

    /// Returns the current structural phase of one certified node.
    pub fn node_phase(&self, node_id: &NodeId) -> Option<NodePhase> {
        self.fold.node_phases.get(node_id).copied()
    }

    /// Returns the folded terminal outcome of one certified node.
    pub fn node_terminal_outcome(&self, node_id: &NodeId) -> Option<NodeTerminalOutcome> {
        self.fold.terminal_outcomes.get(node_id).copied()
    }

    /// Returns the exact terminal transition of one certified node.
    pub fn node_terminal_transition_ref(&self, node_id: &NodeId) -> Option<&TransitionRef> {
        self.fold.terminal_transition_refs.get(node_id)
    }

    /// Returns successful output bindings for one certified node in ordinal order.
    pub fn node_outputs(&self, node_id: &NodeId) -> &[OutputBinding] {
        self.fold
            .node_outputs
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Returns successful fact emissions for one certified node in ordinal order.
    pub fn node_facts(&self, node_id: &NodeId) -> &[FactEmission] {
        self.fold
            .node_facts
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Returns the exact store-assembled public-output authority after terminal closure.
    pub const fn public_output_value_ref(&self) -> Option<&ValueRef> {
        self.fold.public_output.as_ref()
    }

    /// Resolves one folded output reference to its exact successful binding.
    pub fn output_binding(&self, output_ref: &OutputRef) -> Result<&OutputBinding> {
        let fields = output_ref.fields()?;
        let transition = self
            .fold
            .transitions
            .iter()
            .find(|entry| entry.transition_ref == fields.transition_ref)
            .ok_or(StoreError::TransitionFoldMismatch {
                field: "output_transition_ref",
            })?;
        let node_id = transition.transition.fields()?.node_id;
        let ordinal =
            usize::try_from(fields.output_ordinal).map_err(|_| StoreError::SequenceOverflow)?;
        let binding = self
            .fold
            .node_outputs
            .get(&node_id)
            .and_then(|outputs| outputs.get(ordinal))
            .ok_or(StoreError::TransitionFoldMismatch {
                field: "output_ref",
            })?;
        if binding.fields()?.output_ordinal != fields.output_ordinal {
            return Err(StoreError::TransitionFoldMismatch {
                field: "output_ref",
            });
        }
        Ok(binding)
    }

    /// Returns whether a closed run meets its certified success contract.
    pub fn terminal_succeeded(&self) -> bool {
        self.fold.run_phase == RunPhase::Closed
            && self
                .certified_spec
                .run_terminal_contract()
                .required_success_nodes()
                .iter()
                .all(|node_id| {
                    self.fold.terminal_outcomes.get(node_id)
                        == Some(&NodeTerminalOutcome::Succeeded)
                })
            && (!self
                .certified_spec
                .run_terminal_contract()
                .requires_public_output()
                || self.fold.public_output.is_some())
    }

    /// Returns ready unstarted nodes in certified node order.
    pub fn ready_node_ids(&self) -> Vec<&NodeId> {
        self.certified_spec
            .nodes()
            .iter()
            .filter(|node| self.node_is_ready(node))
            .map(CertifiedNodeContract::node_id)
            .collect()
    }

    /// Returns the first ready node in certified order.
    pub fn next_ready_node(&self) -> Option<&CertifiedNodeContract> {
        self.certified_spec
            .nodes()
            .iter()
            .find(|node| self.node_is_ready(node))
    }

    /// Returns current pending effects in certified node order.
    pub fn pending_effects(&self) -> Result<Vec<FoldedPendingEffect>> {
        self.certified_spec
            .nodes()
            .iter()
            .filter_map(|node| {
                self.fold
                    .pending_effects
                    .get(node.node_id())
                    .map(|pending| (node.node_id(), pending))
            })
            .map(|(node_id, pending)| {
                let request_transition_ref = pending.request_transition_ref.clone().ok_or(
                    StoreError::TransitionFoldMismatch {
                        field: "pending_effect_transition_ref",
                    },
                )?;
                let status = self
                    .fold
                    .authorizations
                    .iter()
                    .rev()
                    .find_map(|state| {
                        let fields = state.entry.authorization.fields().ok()?;
                        match fields.scope.fields().ok()? {
                            AuthorizationScopeFields::EnsureEffect {
                                effect_request_transition_ref,
                            } if effect_request_transition_ref == request_transition_ref => Some(
                                state
                                    .entry
                                    .audit_entry
                                    .fields()
                                    .ok()
                                    .map(|fields| fields.status),
                            ),
                            AuthorizationScopeFields::Read { .. }
                            | AuthorizationScopeFields::EnsureEffect { .. } => None,
                        }
                    })
                    .flatten()
                    .map(pending_effect_status)
                    .unwrap_or(PendingEffectStatus::NotAuthorized);
                Ok(FoldedPendingEffect {
                    node_id: node_id.clone(),
                    request_transition_ref,
                    input_manifest_ref: pending.input_manifest_ref.clone(),
                    effect_key: pending.effect_key.clone(),
                    semantic_request_ref: pending.semantic_request_ref.clone(),
                    request_digest: pending.request_digest.clone(),
                    executor_binding_ref: pending.executor_binding_ref.clone(),
                    status,
                })
            })
            .collect()
    }

    /// Constructs the exact current before-state proof for one certified node.
    pub fn transition_before(&self, node_id: &NodeId) -> Result<TransitionBefore> {
        let node_phase = self
            .node_phase(node_id)
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        TransitionBefore::new(
            self.journal_head(),
            self.run_state_digest(),
            self.run_phase(),
            node_phase,
        )
        .map_err(Into::into)
    }

    pub(super) fn dependency_blocking_sources(
        &self,
        node_id: &NodeId,
    ) -> Result<Vec<BlockingSource>> {
        derive_dependency_blocking_sources(
            &self.fold,
            certified_node(&self.certified_spec, node_id)?,
        )
    }

    /// Derives and validates the exact after-state proof for a proposed transition body and delta.
    ///
    /// This uses the same private reducer as committed-history verification. It performs no IO,
    /// invokes no callback, and does not mutate this view.
    pub fn derive_transition_after(
        &self,
        node_id: &NodeId,
        body: &TransitionBody,
        binding_delta: &BindingDelta,
    ) -> Result<TransitionAfter> {
        let fields = body.fields()?;
        let next = FoldEngine::preview_transition(
            &self.fold,
            &self.certified_spec,
            node_id,
            &fields,
            binding_delta,
            &|value_ref| Ok(self.retained_value(value_ref)?.bytes().to_vec()),
        )?;
        let node_phase = next
            .node_phases
            .get(node_id)
            .copied()
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        TransitionAfter::new(
            &next.run_state_digest,
            next.run_phase,
            node_phase,
            binding_delta,
        )
        .map_err(Into::into)
    }

    pub(super) fn derive_transition_after_with_objects(
        &self,
        node_id: &NodeId,
        body: &TransitionBody,
        binding_delta: &BindingDelta,
        objects: &super::PreparedObjectGraph,
    ) -> Result<TransitionAfter> {
        let fields = body.fields()?;
        let next = FoldEngine::preview_transition(
            &self.fold,
            &self.certified_spec,
            node_id,
            &fields,
            binding_delta,
            &|value_ref| Ok(objects.bytes_for(value_ref)?.to_vec()),
        )?;
        let node_phase = next
            .node_phases
            .get(node_id)
            .copied()
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        TransitionAfter::new(
            &next.run_state_digest,
            next.run_phase,
            node_phase,
            binding_delta,
        )
        .map_err(Into::into)
    }

    /// Returns folded transitions with exact references, containing heads, and closure linkage.
    pub fn transition_entries(&self) -> impl ExactSizeIterator<Item = &FoldedTransitionEntry> {
        self.fold.transitions.iter()
    }

    /// Returns every exact committed transition in append order.
    pub fn transitions(
        &self,
    ) -> impl ExactSizeIterator<Item = (&TransitionRef, &StateTransitionCommitted)> {
        self.fold
            .transitions
            .iter()
            .map(|entry| (&entry.transition_ref, &entry.transition))
    }

    /// Borrows the complete transition-frame reader used by trace, replay, and source discovery.
    pub const fn transition_frames(&self) -> TransitionFrameReader<'_> {
        TransitionFrameReader { view: self }
    }

    /// Returns sealed exact access history for one certified external-operation occurrence.
    pub fn access_history(&self, node_id: &NodeId) -> Result<VerifiedNodeAccessHistory<'_>> {
        let node = self
            .certified_spec
            .nodes()
            .iter()
            .find(|node| node.node_id() == node_id)
            .ok_or(StoreError::AuthorizationNotEligible)?;
        let Some((operation_id, binding_ref)) = node.execution().operation_binding() else {
            return Err(StoreError::AuthorizationNotEligible);
        };
        let binding_ref = CapabilityBindingRef::new(binding_ref)?;
        let mut entries = Vec::new();
        for state in &self.fold.authorizations {
            let fields = state.entry.authorization.fields()?;
            let anchor = fields.semantic_anchor.fields()?;
            if &anchor.node_id != node_id {
                continue;
            }
            if fields.capability_binding_ref != binding_ref
                || &fields.capability_operation_id != operation_id
            {
                return Err(StoreError::PersistedMismatch {
                    field: "node_access_contract",
                });
            }
            entries.push(&state.entry);
        }
        let authorization_count =
            u32::try_from(entries.len()).map_err(|_| StoreError::SequenceOverflow)?;
        let suffix_start = entries
            .iter()
            .rposition(|entry| entry.observation.is_none())
            .map_or(0, |index| index + 1);
        let mut observed_suffix = entries[suffix_start..]
            .iter()
            .copied()
            .map(|entry| {
                let (observation_ref, _, _) =
                    entry
                        .observation
                        .as_ref()
                        .ok_or(StoreError::PersistedMismatch {
                            field: "observed_access_suffix",
                        })?;
                let coordinate = observation_ref.fields()?;
                Ok(((coordinate.run_sequence, coordinate.ordinal), entry))
            })
            .collect::<Result<Vec<_>>>()?;
        observed_suffix.sort_by_key(|(coordinate, _)| *coordinate);
        Ok(VerifiedNodeAccessHistory {
            entries,
            observed_suffix: observed_suffix
                .into_iter()
                .map(|(_, entry)| entry)
                .collect(),
            authorization_count,
        })
    }

    /// Returns folded safe access-audit entries in authorization order.
    pub(crate) fn access_audit_entries(
        &self,
    ) -> impl ExactSizeIterator<Item = &FoldedAccessAuditEntry> {
        self.fold.authorizations.iter().map(|entry| &entry.entry)
    }

    pub(super) fn access_audit_as_of(
        &self,
        at_journal_head: &JournalHead,
    ) -> Result<Vec<AccessAuditProjection>> {
        let at = at_journal_head.fields()?;
        let index = usize::try_from(at.run_sequence.checked_sub(1).ok_or(
            StoreError::PersistedMismatch {
                field: "audit_as_of_head",
            },
        )?)
        .map_err(|_| StoreError::SequenceOverflow)?;
        let exact = self
            .journal
            .commits
            .get(index)
            .map(|commit| commit.envelope.journal_head())
            .transpose()?
            .is_some_and(|head| head == *at_journal_head);
        if !exact {
            return Err(StoreError::PersistedMismatch {
                field: "audit_as_of_head",
            });
        }

        let mut entries = Vec::new();
        for state in &self.fold.authorizations {
            let authorization_sequence = state
                .entry
                .authorization_journal_head
                .fields()?
                .run_sequence;
            if authorization_sequence > at.run_sequence {
                continue;
            }
            let observation_visible = match state.entry.observation.as_ref() {
                Some((_, _, observation_head))
                    if observation_head.fields()?.run_sequence <= at.run_sequence =>
                {
                    true
                }
                Some(_) | None => false,
            };
            let (observation_ref, observation_journal_head) = if observation_visible {
                let (observation_ref, _, observation_head) = state
                    .entry
                    .observation
                    .as_ref()
                    .ok_or(StoreError::PersistedMismatch {
                        field: "audit_observation",
                    })?;
                (
                    Some(observation_ref.clone()),
                    Some(observation_head.clone()),
                )
            } else {
                (None, None)
            };
            entries.push(AccessAuditProjection {
                authorization_ref: state.entry.authorization_ref.clone(),
                observation_ref,
                authorization_journal_head: state.entry.authorization_journal_head.clone(),
                observation_journal_head,
                capability_binding_ref: state.entry.capability_binding_ref.clone(),
                capability_operation_id: state.entry.capability_operation_id.clone(),
                request_ref: state.entry.request_ref.clone(),
                status: if observation_visible {
                    state.entry.status
                } else {
                    AccessAuditStatus::AuthorizedUnobserved
                },
                result_ref: observation_visible
                    .then_some(state.entry.result_ref.clone())
                    .flatten(),
                failure: observation_visible
                    .then_some(state.entry.failure.clone())
                    .flatten(),
                effect_key: state.entry.effect_key.clone(),
                delivery_audit_ref: observation_visible
                    .then_some(state.entry.delivery_audit_ref.clone())
                    .flatten(),
                delivery_audit_terminal: observation_visible
                    .then_some(state.entry.delivery_audit_terminal)
                    .flatten(),
            });
        }
        Ok(entries)
    }

    /// Returns every exact committed authorization in append order.
    pub fn authorizations(
        &self,
    ) -> impl ExactSizeIterator<Item = (&AuthorizationRef, &ExternalAccessAuthorized)> {
        self.fold
            .authorizations
            .iter()
            .map(|entry| (&entry.entry.authorization_ref, &entry.entry.authorization))
    }

    /// Returns every unobserved authorization in committed order.
    pub fn unobserved_authorizations(
        &self,
    ) -> impl Iterator<Item = (&AuthorizationRef, &ExternalAccessAuthorized)> {
        self.fold
            .authorizations
            .iter()
            .filter(|entry| entry.entry.observation.is_none())
            .map(|entry| (&entry.entry.authorization_ref, &entry.entry.authorization))
    }

    /// Returns the semantic closure coordinate, if the run is closed.
    pub const fn semantic_closure(&self) -> Option<&SemanticClosureCoordinate> {
        self.fold.semantic_closure.as_ref()
    }

    /// Returns one exact reachable retained object.
    pub fn object(&self, key: &ObjectAuthorityKey) -> Option<&CommittedObject> {
        self.journal.object(key)
    }

    /// Resolves a full producer-bound retained value to its exact verified bytes.
    pub fn retained_value(&self, value_ref: &ValueRef) -> Result<&CommittedObject> {
        let key = ObjectAuthorityKey::from_value_ref(value_ref)?;
        let object = self
            .journal
            .object(&key)
            .ok_or(StoreError::ObjectNotReachable)?;
        if object.value_ref().as_bytes() != value_ref.as_bytes() {
            return Err(StoreError::InvalidObjectAuthority {
                message: "retained object metadata disagrees with the full value reference",
            });
        }
        Ok(object)
    }

    /// Resolves an exact input-manifest authority to verified retained bytes.
    pub fn input_manifest_object(
        &self,
        input_manifest_ref: &InputManifestRef,
    ) -> Result<&CommittedObject> {
        self.retained_value(&input_manifest_ref.value_ref()?)
    }

    /// Strictly decodes one exact retained input manifest.
    pub fn input_manifest(&self, input_manifest_ref: &InputManifestRef) -> Result<InputManifest> {
        let object = self.input_manifest_object(input_manifest_ref)?;
        InputManifest::strict_decode(object.bytes()).map_err(Into::into)
    }

    /// Resolves the optional configured value frozen into one exact input manifest.
    pub fn resolve_configured_value(
        &self,
        input_manifest_ref: &InputManifestRef,
    ) -> Result<Option<&CommittedObject>> {
        self.input_manifest(input_manifest_ref)?
            .fields()?
            .config_ref
            .as_ref()
            .map(|value_ref| self.retained_value(value_ref))
            .transpose()
    }

    /// Resolves the optional immutable context frozen into one exact input manifest.
    pub fn resolve_context_value(
        &self,
        input_manifest_ref: &InputManifestRef,
    ) -> Result<Option<&CommittedObject>> {
        self.input_manifest(input_manifest_ref)?
            .fields()?
            .context_ref
            .as_ref()
            .map(|value_ref| self.retained_value(value_ref))
            .transpose()
    }

    /// Resolves the complete store-minted root input frozen into one exact input manifest.
    pub fn resolve_root_input(
        &self,
        input_manifest_ref: &InputManifestRef,
    ) -> Result<&CommittedObject> {
        let manifest = self.input_manifest(input_manifest_ref)?;
        self.retained_value(&manifest.fields()?.root_input_ref)
    }

    /// Resolves retained bytes by lightweight schema-and-content identity.
    ///
    /// A `ContentRef` remains non-authoritative outside this already verified journal closure.
    pub fn retained_content(&self, content_ref: &mfm_ids::ContentRef) -> Result<&[u8]> {
        self.journal
            .objects
            .values()
            .find_map(|object| {
                object.value_ref().fields().ok().and_then(|fields| {
                    (fields.schema_id == *content_ref.schema_id()
                        && fields.content_digest == *content_ref.content_digest())
                    .then_some(object.bytes())
                })
            })
            .ok_or(StoreError::ObjectNotReachable)
    }

    fn node_is_ready(&self, node: &CertifiedNodeContract) -> bool {
        node_is_ready(&self.fold, node)
    }

    pub(super) fn validate_prepared_transition(
        &self,
        candidate: &CommitCandidatePreimage,
        objects: &super::PreparedObjectGraph,
    ) -> Result<()> {
        self.validate_candidate_predecessor(candidate)?;
        let fields = candidate.fields()?;
        validate_object_envelope(
            &fields.ordered_object_bindings,
            &fields.artifact_admission_intents,
            fields.ordered_candidate_records.len(),
        )?;
        verify_object_bindings(
            &fields.ordered_candidate_records,
            &fields.ordered_object_bindings,
        )?;
        let NormalizedBatch::Transition {
            transition,
            closure,
        } = normalize_candidates(&fields.ordered_candidate_records)?
        else {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "commit_transition",
                message: "prepared payload is not a transition batch",
            });
        };
        let transition_hash =
            RecordHashPreimage::from_candidate(&fields.ordered_candidate_records[0])?
                .record_hash()?;
        self.fold.validate_transition_candidate(
            &transition,
            closure.as_ref(),
            &transition_hash,
            &self.certified_spec,
            objects,
        )
    }

    pub(super) fn validate_prepared_authorization(
        &self,
        candidate: &CommitCandidatePreimage,
    ) -> Result<()> {
        self.validate_candidate_predecessor(candidate)?;
        let fields = candidate.fields()?;
        validate_object_envelope(
            &fields.ordered_object_bindings,
            &fields.artifact_admission_intents,
            fields.ordered_candidate_records.len(),
        )?;
        verify_object_bindings(
            &fields.ordered_candidate_records,
            &fields.ordered_object_bindings,
        )?;
        let NormalizedBatch::Authorization(authorization) =
            normalize_candidates(&fields.ordered_candidate_records)?
        else {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "authorize_external_access",
                message: "prepared payload is not an authorization batch",
            });
        };
        self.fold.validate_authorization_candidate(&authorization)
    }

    pub(super) fn validate_prepared_observation(
        &self,
        candidate: &CommitCandidatePreimage,
        objects: &super::PreparedObjectGraph,
    ) -> Result<()> {
        self.validate_candidate_predecessor(candidate)?;
        let fields = candidate.fields()?;
        validate_object_envelope(
            &fields.ordered_object_bindings,
            &fields.artifact_admission_intents,
            fields.ordered_candidate_records.len(),
        )?;
        verify_object_bindings(
            &fields.ordered_candidate_records,
            &fields.ordered_object_bindings,
        )?;
        let NormalizedBatch::Observation(observation) =
            normalize_candidates(&fields.ordered_candidate_records)?
        else {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "observe_external_access",
                message: "prepared payload is not an observation batch",
            });
        };
        self.fold.validate_observation_candidate(
            &self.journal,
            objects,
            &self.certified_spec,
            &self.capability_binding_manifest,
            &observation,
        )
    }

    fn validate_candidate_predecessor(&self, candidate: &CommitCandidatePreimage) -> Result<()> {
        let fields = candidate.fields()?;
        if fields.expected_predecessor != JournalPredecessor::journal_head(self.journal_head())? {
            return Err(StoreError::HeadMismatch {
                expected: Box::new(match fields.expected_predecessor.fields()? {
                    JournalPredecessorFields::JournalHead(fields) => {
                        JournalHead::new(fields.run_sequence, &fields.commit_digest)?
                    }
                    JournalPredecessorFields::Genesis { .. } => self.journal_head().clone(),
                }),
                actual: Box::new(self.journal_head().clone()),
            });
        }
        Ok(())
    }
}

fn certified_node<'a>(
    spec: &'a ExpandedCertifiedSpec,
    node_id: &NodeId,
) -> Result<&'a CertifiedNodeContract> {
    spec.nodes()
        .iter()
        .find(|node| node.node_id() == node_id)
        .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })
}

#[derive(Clone)]
enum NormalizedBatch {
    Admission(RunAdmitted),
    Transition {
        transition: StateTransitionCommitted,
        closure: Option<mfm_journal::v1::RunClosed>,
    },
    Authorization(ExternalAccessAuthorized),
    Observation(ExternalAccessObserved),
}

fn normalize_commit(commit: &CommittedJournalCommit) -> Result<NormalizedBatch> {
    let candidates = commit
        .records
        .iter()
        .map(|record| record.candidate.clone())
        .collect::<Vec<_>>();
    normalize_candidates(&candidates)
}

fn normalize_candidates(candidates: &[CandidateRecordEnvelope]) -> Result<NormalizedBatch> {
    let payloads = candidates
        .iter()
        .map(|candidate| candidate.fields()?.payload.fields().map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    match payloads.as_slice() {
        [RunJournalRecordFields::RunAdmitted(value)] => {
            LegalCommitBatch::run_admission(value)?;
            Ok(NormalizedBatch::Admission(value.clone()))
        }
        [RunJournalRecordFields::StateTransitionCommitted(transition)] => {
            LegalCommitBatch::transition(transition, None)?;
            Ok(NormalizedBatch::Transition {
                transition: transition.clone(),
                closure: None,
            })
        }
        [RunJournalRecordFields::StateTransitionCommitted(transition), RunJournalRecordFields::RunClosed(closure)] =>
        {
            LegalCommitBatch::transition(transition, Some(closure))?;
            Ok(NormalizedBatch::Transition {
                transition: transition.clone(),
                closure: Some(closure.clone()),
            })
        }
        [RunJournalRecordFields::ExternalAccessAuthorized(value)] => {
            LegalCommitBatch::authorization(value)?;
            Ok(NormalizedBatch::Authorization(value.clone()))
        }
        [RunJournalRecordFields::ExternalAccessObserved(value)] => {
            LegalCommitBatch::observation(value)?;
            Ok(NormalizedBatch::Observation(value.clone()))
        }
        _ => Err(StoreError::InvalidPreparedAppend {
            purpose: "persisted_batch",
            message: "records do not form one exhaustive legal batch",
        }),
    }
}

fn batch_purpose(payloads: &NormalizedBatch) -> Result<BatchPurpose> {
    Ok(match payloads {
        NormalizedBatch::Admission(_) => BatchPurpose::RunAdmission,
        NormalizedBatch::Authorization(_) => BatchPurpose::ExternalAccessAuthorization,
        NormalizedBatch::Observation(_) => BatchPurpose::ExternalAccessObservation,
        NormalizedBatch::Transition { transition, .. } => {
            match transition.fields()?.body.fields()? {
                TransitionBodyFields::PureSettled { .. } => BatchPurpose::PureSettlement,
                TransitionBodyFields::ReadSettled { .. } => BatchPurpose::ReadSettlement,
                TransitionBodyFields::EffectRequested { .. } => BatchPurpose::EffectRequest,
                TransitionBodyFields::EffectSettled { .. } => BatchPurpose::EffectSettlement,
                TransitionBodyFields::DependencySkipped { .. } => BatchPurpose::DependencySkip,
            }
        }
    })
}

fn verify_physical_journal(
    store_identity: &StoreIdentity,
    tenant_scope_id: &TenantScopeId,
    run_id: &RunId,
    commits: &[CommittedJournalCommit],
    objects: &[CommittedObject],
) -> Result<Vec<NormalizedBatch>> {
    if commits.is_empty() {
        return Err(StoreError::EmptyJournal);
    }
    let object_map = objects
        .iter()
        .map(|object| (object.key().clone(), object))
        .collect::<BTreeMap<_, _>>();
    if object_map.len() != objects.len() {
        return Err(StoreError::InvalidObjectAuthority {
            message: "loaded reachable objects contain duplicate exact authority",
        });
    }
    let mut prior_head: Option<JournalHead> = None;
    let mut seen_objects = BTreeSet::new();
    let mut referenced_objects = BTreeSet::new();
    let mut logical_keys = BTreeSet::new();
    let mut normalized_batches = Vec::with_capacity(commits.len());

    for (index, commit) in commits.iter().enumerate() {
        let envelope = commit.envelope.fields()?;
        let expected_sequence = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(StoreError::SequenceOverflow)?;
        if envelope.core.store_scope_id != *store_identity.store_scope_id()
            || envelope.core.run_id != *run_id
            || envelope.core.run_sequence != expected_sequence
        {
            return Err(StoreError::PersistedMismatch {
                field: "commit_identity",
            });
        }
        match (&prior_head, envelope.core.predecessor.fields()?) {
            (
                None,
                JournalPredecessorFields::Genesis {
                    store_scope_id,
                    store_epoch,
                    run_id: predecessor_run,
                    genesis_digest,
                },
            ) => {
                let derived = mfm_journal::v1::GenesisPreimage::new(
                    store_identity.store_scope_id(),
                    store_identity.store_epoch(),
                    run_id,
                )?
                .genesis_digest()?;
                if store_scope_id != *store_identity.store_scope_id()
                    || store_epoch != store_identity.store_epoch()
                    || predecessor_run != *run_id
                    || genesis_digest != derived
                {
                    return Err(StoreError::PersistedMismatch {
                        field: "genesis_predecessor",
                    });
                }
            }
            (Some(expected), JournalPredecessorFields::JournalHead(actual))
                if expected.fields()? == actual => {}
            _ => {
                return Err(StoreError::PersistedMismatch {
                    field: "journal_predecessor",
                });
            }
        }
        if commit.records.is_empty() || commit.records.len() > 2 {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "persisted_batch",
                message: "commit must contain one or two records",
            });
        }
        let mut candidates = Vec::with_capacity(commit.records.len());
        let mut hashes = Vec::with_capacity(commit.records.len());
        for (ordinal, record) in commit.records.iter().enumerate() {
            let candidate = record.candidate.fields()?;
            if usize::try_from(candidate.ordinal).ok() != Some(ordinal)
                || candidate.schema_id != *candidate.payload.schema_id()
                || candidate.emits_facts != candidate.payload.emits_facts()?
            {
                return Err(StoreError::PersistedMismatch {
                    field: "candidate_record",
                });
            }
            if !logical_keys.insert(record.candidate.fields()?.logical_key.as_bytes().to_vec()) {
                return Err(StoreError::DuplicateLogicalRecord);
            }
            let derived_hash =
                RecordHashPreimage::from_candidate(&record.candidate)?.record_hash()?;
            if derived_hash != record.record_hash {
                return Err(StoreError::PersistedMismatch {
                    field: "record_hash",
                });
            }
            let derived_id = RecordIdPreimage::new(
                store_identity.store_scope_id(),
                run_id,
                expected_sequence,
                candidate.ordinal,
                &record.record_hash,
            )?
            .record_id()?;
            if derived_id != record.record_id {
                return Err(StoreError::PersistedMismatch { field: "record_id" });
            }
            candidates.push(record.candidate.clone());
            hashes.push(record.record_hash.clone());
        }
        if hashes != envelope.core.ordered_record_hashes {
            return Err(StoreError::PersistedMismatch {
                field: "ordered_record_hashes",
            });
        }
        let payloads = normalize_commit(commit)?;
        let purpose = batch_purpose(&payloads)?;
        verify_batch_coordinate(
            tenant_scope_id,
            &payloads,
            &envelope.core.tenant_fact_coordinate,
        )?;
        validate_object_envelope(
            &envelope.core.ordered_object_bindings,
            &envelope.core.artifact_admission_intents,
            candidates.len(),
        )?;
        verify_object_bindings(&candidates, &envelope.core.ordered_object_bindings)?;
        for intent in &envelope.core.artifact_admission_intents {
            let fields = intent.fields()?;
            let key = ObjectAuthorityKey::from_value_ref(&fields.value_ref)?;
            if !object_map.contains_key(&key) {
                return Err(StoreError::MissingObjectAuthority {
                    artifact_id: key.artifact_id().clone(),
                });
            }
            let admission_prerequisite = index == 0
                && matches!(payloads, NormalizedBatch::Admission(_))
                && matches!(
                    fields.value_ref.fields()?.producer_binding.kind()?,
                    ProducerBindingKind::QualifiedSupport | ProducerBindingKind::ConfiguredValue
                );
            match fields.mode {
                ArtifactAdmissionMode::RequireExisting
                    if !seen_objects.contains(&key) && !admission_prerequisite =>
                {
                    return Err(StoreError::MissingObjectAuthority {
                        artifact_id: key.artifact_id().clone(),
                    });
                }
                ArtifactAdmissionMode::RequireExisting
                | ArtifactAdmissionMode::AdmitOrVerifyExact => {
                    seen_objects.insert(key.clone());
                    referenced_objects.insert(key);
                }
            }
        }
        let candidate_preimage = CommitCandidatePreimage::new(
            &envelope.core.predecessor,
            purpose,
            &candidates,
            &envelope.core.ordered_object_bindings,
            &envelope.core.artifact_admission_intents,
        )?;
        if candidate_preimage.candidate_digest()? != envelope.core.candidate_digest {
            return Err(StoreError::PersistedMismatch {
                field: "candidate_digest",
            });
        }
        let commit_preimage = CommitDigestPreimage::new(
            store_identity.store_scope_id(),
            run_id,
            expected_sequence,
            &envelope.core.predecessor,
            &envelope.core.append_request_id,
            &envelope.core.candidate_digest,
            &envelope.core.tenant_fact_coordinate,
            &hashes,
            &envelope.core.ordered_object_bindings,
            &envelope.core.artifact_admission_intents,
        )?;
        if commit_preimage.commit_digest()? != envelope.commit_digest {
            return Err(StoreError::PersistedMismatch {
                field: "commit_digest",
            });
        }
        prior_head = Some(commit.envelope.journal_head()?);
        normalized_batches.push(payloads);
    }
    if referenced_objects != object_map.keys().cloned().collect() {
        return Err(StoreError::InvalidObjectAuthority {
            message: "loaded reachable object set has omissions or extras",
        });
    }
    match normalized_batches.first() {
        Some(NormalizedBatch::Admission(admission)) => {
            let fields = admission.fields()?;
            if fields.run_id != *run_id || fields.tenant_scope_id != *tenant_scope_id {
                return Err(StoreError::PersistedMismatch {
                    field: "admission_scope",
                });
            }
        }
        Some(
            NormalizedBatch::Transition { .. }
            | NormalizedBatch::Authorization(_)
            | NormalizedBatch::Observation(_),
        )
        | None => return Err(StoreError::EmptyJournal),
    }
    Ok(normalized_batches)
}

fn verify_batch_coordinate(
    tenant_scope_id: &TenantScopeId,
    payloads: &NormalizedBatch,
    coordinate: &mfm_journal::v1::TenantFactCoordinate,
) -> Result<()> {
    let emits_facts = matches!(
        payloads,
        NormalizedBatch::Transition { transition, .. }
            if !transition.fact_emissions()?.is_empty()
    );
    let barrier = matches!(
        payloads,
        NormalizedBatch::Authorization(value)
            if value.fields()?.capability_operation_id.as_str() == FACT_SELECTION_OPERATION_ID
    );
    match coordinate.fields()? {
        TenantFactCoordinateFields::None if !emits_facts && !barrier => Ok(()),
        TenantFactCoordinateFields::FactPublication {
            tenant_scope_id: tenant,
            fact_order,
        } if emits_facts && tenant == *tenant_scope_id && fact_order > 0 => Ok(()),
        TenantFactCoordinateFields::FactSelectionBarrier {
            tenant_scope_id: tenant,
            ..
        } if barrier && tenant == *tenant_scope_id => Ok(()),
        _ => Err(StoreError::InvalidFactCoordinate),
    }
}

fn verify_object_bindings(
    candidates: &[CandidateRecordEnvelope],
    bindings: &[ObjectPathBinding],
) -> Result<()> {
    let mut bound_paths = BTreeSet::new();
    for binding in bindings {
        let fields = binding.fields()?;
        bound_paths.insert((fields.record_ordinal, fields.field_path.as_str().to_owned()));
        let candidate = candidates
            .get(usize::try_from(fields.record_ordinal).map_err(|_| {
                StoreError::InvalidObjectAuthority {
                    message: "object binding ordinal cannot be represented",
                }
            })?)
            .ok_or(StoreError::InvalidObjectAuthority {
                message: "object binding ordinal is outside the batch",
            })?;
        let candidate_value = candidate_payload_value(candidate)?;
        let value = value_at_path(&candidate_value, fields.field_path.as_str()).ok_or(
            StoreError::InvalidObjectAuthority {
                message: "object binding field path is absent",
            },
        )?;
        if value != &fields.value_ref.canonical_value()?
            || fields.value_ref.fields()?.evidence_contract_ref != fields.evidence_contract_ref
        {
            return Err(StoreError::InvalidObjectAuthority {
                message: "object binding identity disagrees with its exact field path",
            });
        }
        let producer_kind = fields.value_ref.fields()?.producer_binding.kind()?;
        match (fields.authority_use, producer_kind) {
            (
                AuthorityUse::ProducedHere,
                ProducerBindingKind::ThisAdmission
                | ProducerBindingKind::ThisRecord
                | ProducerBindingKind::InputAssembly
                | ProducerBindingKind::TransitionOutput
                | ProducerBindingKind::TransitionFact
                | ProducerBindingKind::PublicOutputAssembly
                | ProducerBindingKind::ExternalObservation
                | ProducerBindingKind::SourceRun,
            )
            | (
                AuthorityUse::Preexisting,
                ProducerBindingKind::RunAdmission
                | ProducerBindingKind::ThisAdmission
                | ProducerBindingKind::ThisRecord
                | ProducerBindingKind::InputAssembly
                | ProducerBindingKind::TransitionOutput
                | ProducerBindingKind::TransitionFact
                | ProducerBindingKind::PublicOutputAssembly
                | ProducerBindingKind::QualifiedSupport
                | ProducerBindingKind::ConfiguredValue
                | ProducerBindingKind::ExternalObservation
                | ProducerBindingKind::SourceRun,
            ) => {}
            _ => {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "object authority use disagrees with producer binding",
                });
            }
        }
    }

    let mut required_paths = BTreeSet::new();
    for (ordinal, candidate) in candidates.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| StoreError::SequenceOverflow)?;
        collect_full_authority_paths(
            &candidate_payload_value(candidate)?,
            "",
            ordinal,
            &mut required_paths,
        );
    }
    if !required_paths.is_subset(&bound_paths) {
        return Err(StoreError::InvalidObjectAuthority {
            message: "a full retained-object reference has no exact path binding",
        });
    }
    Ok(())
}

fn candidate_payload_value(candidate: &CandidateRecordEnvelope) -> Result<CanonicalValue> {
    match candidate.fields()?.payload.fields()? {
        RunJournalRecordFields::RunAdmitted(value) => value.canonical_value().map_err(Into::into),
        RunJournalRecordFields::StateTransitionCommitted(value) => {
            value.canonical_value().map_err(Into::into)
        }
        RunJournalRecordFields::ExternalAccessAuthorized(value) => {
            value.canonical_value().map_err(Into::into)
        }
        RunJournalRecordFields::ExternalAccessObserved(value) => {
            value.canonical_value().map_err(Into::into)
        }
        RunJournalRecordFields::RunClosed(value) => value.canonical_value().map_err(Into::into),
    }
}

fn collect_full_authority_paths(
    value: &CanonicalValue,
    path: &str,
    record_ordinal: u32,
    paths: &mut BTreeSet<(u32, String)>,
) {
    match value {
        CanonicalValue::Object(object) => {
            let has_full_authority = ["artifact_id", "content_digest", "evidence_hash"]
                .into_iter()
                .all(|field| canonical_object_get(object, field).is_some());
            if has_full_authority {
                if !path.is_empty() {
                    paths.insert((record_ordinal, path.to_owned()));
                }
                return;
            }
            for (field, nested) in object.entries() {
                let nested_path = if path.is_empty() {
                    field.to_owned()
                } else {
                    format!("{path}.{field}")
                };
                collect_full_authority_paths(nested, &nested_path, record_ordinal, paths);
            }
        }
        CanonicalValue::Array(values) => {
            for (index, nested) in values.iter().enumerate() {
                let nested_path = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}.{index}")
                };
                collect_full_authority_paths(nested, &nested_path, record_ordinal, paths);
            }
        }
        CanonicalValue::Null
        | CanonicalValue::Bool(_)
        | CanonicalValue::Bytes(_)
        | CanonicalValue::Signed(_)
        | CanonicalValue::Unsigned(_)
        | CanonicalValue::Decimal(_)
        | CanonicalValue::String(_) => {}
    }
}

fn value_at_path<'a>(root: &'a CanonicalValue, path: &str) -> Option<&'a CanonicalValue> {
    path.split('.')
        .try_fold(root, |value, segment| match value {
            CanonicalValue::Object(fields) => canonical_object_get(fields, segment),
            CanonicalValue::Array(values) => segment
                .parse::<usize>()
                .ok()
                .and_then(|index| values.get(index)),
            _ => None,
        })
}

fn canonical_object_get<'a>(
    object: &'a mfm_canonical::CanonicalObject,
    key: &str,
) -> Option<&'a CanonicalValue> {
    object
        .entries()
        .find_map(|(candidate, value)| (candidate == key).then_some(value))
}

fn load_admission_and_spec(
    journal: &CommittedRunJournal,
) -> Result<(RunAdmitted, ExpandedCertifiedSpec)> {
    let NormalizedBatch::Admission(admission) = journal
        .normalized_batches
        .first()
        .ok_or(StoreError::EmptyJournal)?
    else {
        return Err(StoreError::EmptyJournal);
    };
    let admission = admission.clone();
    let reference = admission.fields()?.certified_spec_ref;
    let spec = ExpandedCertifiedSpec::from_canonical_json(retained_content_in_journal(
        journal, &reference,
    )?)?;
    Ok((admission, spec))
}

fn retained_content_in_journal<'a>(
    journal: &'a CommittedRunJournal,
    reference: &mfm_ids::ContentRef,
) -> Result<&'a [u8]> {
    journal
        .objects
        .values()
        .find_map(|object| {
            object.value_ref().fields().ok().and_then(|fields| {
                (fields.content_digest == *reference.content_digest()
                    && fields.schema_id == *reference.schema_id())
                .then_some(object.bytes())
            })
        })
        .ok_or(StoreError::ObjectNotReachable)
}

fn retained_value_in_journal<'a>(
    journal: &'a CommittedRunJournal,
    value_ref: &ValueRef,
) -> Result<&'a CommittedObject> {
    let key = ObjectAuthorityKey::from_value_ref(value_ref)?;
    let object = journal.object(&key).ok_or(StoreError::ObjectNotReachable)?;
    if object.value_ref().as_bytes() != value_ref.as_bytes() {
        return Err(StoreError::InvalidObjectAuthority {
            message: "retained object disagrees with its full value authority",
        });
    }
    Ok(object)
}

#[allow(clippy::too_many_arguments)]
fn verify_admission_source_requirements(
    journal: &CommittedRunJournal,
    admission: &RunAdmitted,
    spec: &ExpandedCertifiedSpec,
    config_manifest: &ConfigManifest,
    seed_manifest: &SeedManifest,
    context_manifest: &ContextManifest,
    cross_run_manifest: &CrossRunSourceManifest,
) -> Result<(
    VerifiedAdmissionSourceRequirements,
    VerifiedRecordedConfiguredValue,
)> {
    let admitted = admission.fields()?;
    let mut initial = BTreeMap::<String, (ValueRef, ContentRef)>::new();
    for binding in admitted.initial_bindings {
        let fields = binding.fields()?;
        if initial
            .insert(
                fields.field_path.as_str().to_owned(),
                (fields.value_ref, fields.source_role_ref),
            )
            .is_some()
        {
            return Err(StoreError::PersistedMismatch {
                field: "initial_bindings",
            });
        }
    }

    let input = take_initial_binding(&mut initial, "run_admission.input")?;
    retained_value_in_journal(journal, &input)?;
    if !matches!(
        input.fields()?.producer_binding.fields()?,
        ProducerBindingFields::ThisAdmission { .. }
    ) {
        return Err(StoreError::PersistedMismatch {
            field: "admission_input_binding",
        });
    }

    let mut config_roots = Vec::new();
    let mut recorded_configured = None;
    for entry in config_manifest.entries()? {
        let field_path = entry.field_path()?;
        let value_ref = entry.value_ref()?;
        take_matching_initial(
            journal,
            &mut initial,
            &format!("config.{}", field_path.as_str()),
            &value_ref,
        )?;
        let ProducerBindingFields::ConfiguredValue {
            store_scope_id,
            tenant_scope_id,
            entry_point_id,
            target,
        } = value_ref.fields()?.producer_binding.fields()?
        else {
            return Err(StoreError::PersistedMismatch {
                field: "configured_value_producer",
            });
        };
        if store_scope_id != *journal.store_identity().store_scope_id()
            || tenant_scope_id != *journal.tenant_scope_id()
            || recorded_configured.is_some()
        {
            return Err(StoreError::PersistedMismatch {
                field: "configured_value_binding",
            });
        }
        let key =
            ConfiguredValueKey::new(&store_scope_id, &tenant_scope_id, &entry_point_id, &target)?;
        let object = retained_value_in_journal(journal, &value_ref)?;
        let value_contract = verified_recorded_value_contract(&value_ref, object)?;
        recorded_configured = Some(VerifiedRecordedConfiguredValue {
            recorded_binding: ConfiguredValueBinding::new(&key, &value_ref)?,
            recorded_entry_point_id: entry_point_id,
            target,
            value_contract,
            value_ref: value_ref.clone(),
            bytes: object.bytes().to_vec(),
        });
        config_roots.push(VerifiedAdmissionRoot {
            field_path,
            value_ref,
        });
    }
    let recorded_configured_value = recorded_configured.ok_or(StoreError::PersistedMismatch {
        field: "configured_value_binding",
    })?;

    let seed_roots = verify_root_manifest(journal, &mut initial, "seed", seed_manifest.entries()?)?;
    let context_roots = verify_root_manifest(
        journal,
        &mut initial,
        "context",
        context_manifest.entries()?,
    )?;

    let mut cross_run_sources = Vec::new();
    for entry in cross_run_manifest.entries()? {
        let field_path = entry.field_path()?;
        let source_ref = entry.source()?;
        let destination_value_ref =
            take_initial_binding(&mut initial, &format!("cross_run.{}", field_path.as_str()))?;
        retained_value_in_journal(journal, &destination_value_ref)?;
        let ProducerBindingFields::SourceRun {
            source_ref: produced_source,
        } = destination_value_ref.fields()?.producer_binding.fields()?
        else {
            return Err(StoreError::PersistedMismatch {
                field: "cross_run_source_binding",
            });
        };
        if produced_source != source_ref {
            return Err(StoreError::PersistedMismatch {
                field: "cross_run_source_binding",
            });
        }
        let identity = source_ref.identity()?;
        cross_run_sources.push(VerifiedCrossRunSourceRequirement {
            field_path,
            destination_value_ref,
            source_ref,
            source_run_id: identity.source_run_id,
            source_admission_ref: identity.source_admission_ref,
            source_closure_ref: identity.source_closure_ref,
        });
    }

    let mut qualified_support_roots = Vec::new();
    for (initial_path, (value_ref, _source_role_ref)) in initial {
        retained_value_in_journal(journal, &value_ref)?;
        let ProducerBindingFields::QualifiedSupport {
            qualification_scope_id,
            field_path,
        } = value_ref.fields()?.producer_binding.fields()?
        else {
            return Err(StoreError::PersistedMismatch {
                field: "initial_binding_source",
            });
        };
        if initial_path != field_path.as_str() {
            return Err(StoreError::PersistedMismatch {
                field: "qualified_support_binding",
            });
        }
        qualified_support_roots.push(VerifiedQualifiedSupportRoot {
            qualification_scope_id,
            member_path: field_path,
            value_ref,
        });
    }
    qualified_support_roots.sort_by(|left, right| left.member_path.cmp(&right.member_path));

    validate_admission_selectors(
        spec,
        &config_roots,
        &seed_roots,
        &context_roots,
        &qualified_support_roots,
        &cross_run_sources,
    )?;

    Ok((
        VerifiedAdmissionSourceRequirements {
            config_manifest_ref: admitted.config_manifest_ref,
            seed_manifest_ref: admitted.seed_manifest_ref,
            context_manifest_ref: admitted.context_manifest_ref,
            cross_run_source_manifest_ref: admitted.cross_run_source_manifest_ref,
            config_roots,
            seed_roots,
            context_roots,
            qualified_support_roots,
            cross_run_sources,
        },
        recorded_configured_value,
    ))
}

fn verified_recorded_value_contract(
    value_ref: &ValueRef,
    object: &CommittedObject,
) -> Result<RetainedValueContract> {
    if object.value_ref().as_bytes() != value_ref.as_bytes() {
        return Err(StoreError::PersistedMismatch {
            field: "configured_value_contract",
        });
    }
    let fields = value_ref.fields()?;
    let value_contract = RetainedValueContract::new(
        fields.schema_id,
        fields.semantic_type_id,
        fields.role,
        fields.media_type,
        fields.evidence_contract_ref,
    )
    .map_err(|_| StoreError::PersistedMismatch {
        field: "configured_value_contract",
    })?;
    value_ref.validate_contract(&value_contract)?;
    RecoverabilityContractV1::embedded()?
        .strict_decode_schema_id(value_contract.schema_id(), object.bytes())
        .map_err(|_| StoreError::PersistedMismatch {
            field: "configured_value_contract",
        })?;
    Ok(value_contract)
}

fn verify_root_manifest(
    journal: &CommittedRunJournal,
    initial: &mut BTreeMap<String, (ValueRef, ContentRef)>,
    prefix: &str,
    entries: Vec<mfm_journal::v1::RootManifestEntry>,
) -> Result<Vec<VerifiedAdmissionRoot>> {
    let mut roots = Vec::with_capacity(entries.len());
    for entry in entries {
        let field_path = entry.field_path()?;
        let value_ref = entry.value_ref()?;
        take_matching_initial(
            journal,
            initial,
            &format!("{prefix}.{}", field_path.as_str()),
            &value_ref,
        )?;
        let ProducerBindingFields::QualifiedSupport {
            field_path: member_path,
            ..
        } = value_ref.fields()?.producer_binding.fields()?
        else {
            return Err(StoreError::PersistedMismatch {
                field: "admission_root_producer",
            });
        };
        if member_path != field_path {
            return Err(StoreError::PersistedMismatch {
                field: "admission_root_producer",
            });
        }
        roots.push(VerifiedAdmissionRoot {
            field_path,
            value_ref,
        });
    }
    Ok(roots)
}

fn take_matching_initial(
    journal: &CommittedRunJournal,
    initial: &mut BTreeMap<String, (ValueRef, ContentRef)>,
    path: &str,
    expected: &ValueRef,
) -> Result<()> {
    let actual = take_initial_binding(initial, path)?;
    if &actual != expected {
        return Err(StoreError::PersistedMismatch {
            field: "initial_binding_manifest",
        });
    }
    retained_value_in_journal(journal, expected)?;
    Ok(())
}

fn take_initial_binding(
    initial: &mut BTreeMap<String, (ValueRef, ContentRef)>,
    path: &str,
) -> Result<ValueRef> {
    initial
        .remove(path)
        .map(|(value_ref, _)| value_ref)
        .ok_or(StoreError::PersistedMismatch {
            field: "initial_binding_manifest",
        })
}

fn validate_admission_selectors(
    spec: &ExpandedCertifiedSpec,
    config: &[VerifiedAdmissionRoot],
    seed: &[VerifiedAdmissionRoot],
    context: &[VerifiedAdmissionRoot],
    support: &[VerifiedQualifiedSupportRoot],
    cross_run: &[VerifiedCrossRunSourceRequirement],
) -> Result<()> {
    for selector in spec.nodes().iter().flat_map(|node| {
        std::iter::once(node.config_binding().source())
            .chain(node.context_binding().map(|binding| binding.source()))
            .chain(
                node.input_bindings()
                    .iter()
                    .flat_map(|binding| binding.ordered_sources()),
            )
    }) {
        let available = match selector {
            CertifiedSourceSelector::RunAdmission { .. }
            | CertifiedSourceSelector::NodeOutput { .. }
            | CertifiedSourceSelector::NodeFact { .. } => true,
            CertifiedSourceSelector::Config { .. } => !config.is_empty(),
            CertifiedSourceSelector::Seed { .. } => !seed.is_empty(),
            CertifiedSourceSelector::Context { .. } => !context.is_empty(),
            CertifiedSourceSelector::QualifiedSupport { member_path, .. } => {
                support.iter().any(|root| &root.member_path == member_path)
            }
            CertifiedSourceSelector::CrossRunEffectiveOutput { .. } => {
                cross_run.iter().any(|required| {
                    matches!(
                        required.source_ref.fields(),
                        Ok(CrossRunSourceRefFields::EffectiveOutput { .. })
                    )
                })
            }
            CertifiedSourceSelector::CrossRunEvidence {
                certified_evidence_role_ref,
                ..
            } => cross_run.iter().any(|required| {
                matches!(
                    required.source_ref.fields(),
                    Ok(CrossRunSourceRefFields::EvidenceOnly {
                        certified_evidence_role_ref: ref actual,
                        ..
                    }) if actual == certified_evidence_role_ref
                )
            }),
        };
        if !available {
            return Err(StoreError::PersistedMismatch {
                field: "certified_admission_source",
            });
        }
    }
    Ok(())
}

fn executor_delivery_audit(
    journal: &CommittedRunJournal,
    result_ref: &ValueRef,
) -> Result<(ValueRef, bool)> {
    let result = ExecutorEnsureResult::strict_decode(
        retained_value_in_journal(journal, result_ref)?.bytes(),
    )?;
    Ok(match result.fields()? {
        ExecutorEnsureResultFields::Pending { delivery_audit_ref } => (delivery_audit_ref, false),
        ExecutorEnsureResultFields::Terminal { evidence_ref } => {
            let delivery_audit_ref = TerminalEffectEvidence::strict_decode(
                retained_value_in_journal(journal, &evidence_ref)?.bytes(),
            )?
            .fields()?
            .delivery_audit_ref;
            (delivery_audit_ref, true)
        }
    })
}

fn validate_state_manifest(
    spec: &ExpandedCertifiedSpec,
    manifest: &StateImplementationManifest,
) -> Result<()> {
    let expected = spec
        .nodes()
        .iter()
        .map(|node| node.state_contract_ref())
        .collect::<BTreeSet<_>>();
    let actual = manifest
        .entries()
        .iter()
        .map(|entry| &entry.state_contract_ref)
        .collect::<BTreeSet<_>>();
    if expected != actual {
        return Err(StoreError::PersistedMismatch {
            field: "state_implementation_manifest",
        });
    }
    Ok(())
}
