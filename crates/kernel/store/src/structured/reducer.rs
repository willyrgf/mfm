//! Capability-free interpretation of one already-qualified semantic event.

use std::collections::BTreeMap;

use mfm_facts::FactSet;
use mfm_ids::{
    AccessAttemptId, ContentRef, OccurrenceId, RequestDigest, RunId, RunSemanticStateDigest,
    SemanticCallId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    derive_access_attempt_id, derive_semantic_state_digest, AccessAttemptIdentityPreimage,
    AccessKind, CommittedFactRef, ExternalAccessAuthorized, ExternalAccessObserved, JournalHead,
    LexicalValueRef, ObservationOutcome, RecordRef, SemanticHead, SemanticStatePreimage,
    SemanticTransitionPreimage, StateOutcomeRef, StateTransitionCommitted, StructuralValueOrigin,
    TypedValueRef,
};
use mfm_runtime::history::{
    ActionableState, EffectEntryAttentionResolution, EffectEntrySubject, LaneCursor, ProgramCursor,
    StateLeaf, StructuredFrontier,
};
use mfm_spec::structured::{
    BlockTail, CertifiedFailureBoundary, ExpandedBlock, ExpandedDeclaration, ExpandedFanOut,
    ExpandedFragment, ExpandedMatch, ExpandedStateBinding, FailurePlan, HandlerContinuation,
    LexicalProducer, LexicalSlot, StructuralPath, StructuredCapabilityProtocolContract,
    StructuredComponentKind, StructuredEffectEntryContract, StructuredEffectRefreshContract,
    StructuredExecutionKind,
};
use mfm_spec::CanonicalJsonValue;
use mfm_values::CanonicalJsonPersistedSchema;

use super::compiler::{address_artifact, request_digest, AddressedArtifactIntent};
use super::qualification::{
    invalid, QualifiedRecordedEvent, QualifiedRunContext, StructuredStoreError,
};

/// One reducer-owned current Effect attention item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntryAttention {
    subject: EffectEntrySubject,
    resolution: EffectEntryAttentionResolution,
}

impl EffectEntryAttention {
    pub(super) fn new(
        subject: EffectEntrySubject,
        resolution: EffectEntryAttentionResolution,
    ) -> Self {
        Self {
            subject,
            resolution,
        }
    }

    /// Returns the exact unresolved Effect subject.
    pub const fn subject(&self) -> &EffectEntrySubject {
        &self.subject
    }

    /// Returns the required operator resolution.
    pub const fn resolution(&self) -> EffectEntryAttentionResolution {
        self.resolution
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordHandle {
    Existing(RecordRef),
    PendingPrimary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransitionEntry {
    handle: RecordHandle,
    intent: TransitionIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthorizationEntry {
    handle: RecordHandle,
    intent: AuthorizationIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservationEntry {
    handle: RecordHandle,
    intent: ObservationIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkingSemanticHead {
    Genesis {
        admission: RecordHandle,
        digest: RunSemanticStateDigest,
    },
    Transition {
        transition: RecordHandle,
        digest: RunSemanticStateDigest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkingState {
    run_id: RunId,
    admitted: bool,
    admission_handle: Option<RecordHandle>,
    transitions: BTreeMap<OccurrenceId, TransitionEntry>,
    authorizations: BTreeMap<AccessAttemptId, AuthorizationEntry>,
    occurrence_attempts: BTreeMap<OccurrenceId, Vec<AccessAttemptId>>,
    observations: BTreeMap<AccessAttemptId, ObservationEntry>,
    semantic_head: Option<WorkingSemanticHead>,
    closed_outcome_ref: Option<ContentRef>,
}

impl WorkingState {
    fn empty(run_id: RunId) -> Self {
        Self {
            run_id,
            admitted: false,
            admission_handle: None,
            transitions: BTreeMap::new(),
            authorizations: BTreeMap::new(),
            occurrence_attempts: BTreeMap::new(),
            observations: BTreeMap::new(),
            semantic_head: None,
            closed_outcome_ref: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkLeaf {
    Ready,
    Authorized(AccessAttemptId),
    ObservedForSettlement {
        access_attempt_id: AccessAttemptId,
        observation: RecordHandle,
    },
    Refreshable {
        next_attempt_ordinal: u64,
        public_lineage_head_ref: ContentRef,
    },
    EntryUnknown(AccessAttemptId),
    BlockedIntegrity(RecordHandle),
    EntryClosable(AccessAttemptId),
    Reassertable {
        access_attempt_id: AccessAttemptId,
        next_attempt_ordinal: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkAction {
    occurrence_id: OccurrenceId,
    occurrence_path: StructuralPath,
    semantic_call_id: mfm_ids::SemanticCallId,
    state_contract_ref: ContentRef,
    input: LexicalValueRef,
    capability_contract_ref: Option<ContentRef>,
    stable_resource_lineage_contract_ref: Option<ContentRef>,
    execution_kind: StructuredExecutionKind,
    leaf: WorkLeaf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Work cursors stay inline so the deterministic walk does not allocate per state.
#[allow(clippy::large_enum_variant)]
enum WorkCursor {
    AtState(WorkAction),
    InFanOut {
        group_path: StructuralPath,
        lanes: Vec<WorkCursor>,
    },
    Completed {
        outcome_ref: ContentRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReducedRunState {
    working: WorkingState,
    cursor: Option<ProgramCursor>,
    frontier: Option<StructuredFrontier>,
    bindings: BTreeMap<ContentRef, LexicalValueRef>,
    journal_head: Option<JournalHead>,
    semantic_head: Option<SemanticHead>,
    effect_entry_attention: Option<EffectEntryAttention>,
}

impl ReducedRunState {
    /// Returns the sole reducer seed for one qualified run context.
    pub(super) fn empty(context: &QualifiedRunContext) -> Self {
        Self {
            working: WorkingState::empty(context.admission.run_id.clone()),
            cursor: None,
            frontier: None,
            bindings: BTreeMap::new(),
            journal_head: None,
            semantic_head: None,
            effect_entry_attention: None,
        }
    }

    pub(super) fn run_id(&self) -> &RunId {
        &self.working.run_id
    }

    pub(super) fn journal_head(&self) -> Result<&JournalHead, StructuredStoreError> {
        self.journal_head.as_ref().ok_or_else(invalid)
    }

    pub(super) fn semantic_head(&self) -> Result<&SemanticHead, StructuredStoreError> {
        self.semantic_head.as_ref().ok_or_else(invalid)
    }

    pub(super) fn frontier(&self) -> Result<&StructuredFrontier, StructuredStoreError> {
        self.frontier.as_ref().ok_or_else(invalid)
    }

    fn action(&self) -> Result<&ActionableState, StructuredStoreError> {
        match self.frontier()? {
            StructuredFrontier::Actions(actions) => actions.first().ok_or_else(invalid),
            StructuredFrontier::PossibleEntry(_) => Err(invalid()),
            StructuredFrontier::BlockedIntegrity => Err(invalid()),
            StructuredFrontier::Complete => Err(invalid()),
        }
    }

    pub(super) fn cursor(&self) -> Result<&ProgramCursor, StructuredStoreError> {
        self.cursor.as_ref().ok_or_else(invalid)
    }

    pub(super) const fn effect_entry_attention(&self) -> Option<&EffectEntryAttention> {
        self.effect_entry_attention.as_ref()
    }

    pub(super) fn authorization_ref(&self, attempt: &AccessAttemptId) -> Option<&RecordRef> {
        let entry = self.working.authorizations.get(attempt)?;
        let RecordHandle::Existing(reference) = &entry.handle else {
            return None;
        };
        Some(reference)
    }

    pub(super) fn observation_ref(&self, attempt: &AccessAttemptId) -> Option<&RecordRef> {
        let entry = self.working.observations.get(attempt)?;
        let RecordHandle::Existing(reference) = &entry.handle else {
            return None;
        };
        Some(reference)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TransitionValueIntent {
    Success {
        value: CanonicalJsonValue,
        facts: FactSet,
    },
    Failure(CanonicalJsonValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ObservationValueIntent {
    Returned(CanonicalJsonValue),
    SafeFailure(CanonicalJsonValue),
    SupersededBeforeEntry {
        public_lineage_head_ref: ContentRef,
        evidence: CanonicalJsonValue,
    },
    EntryUnknown {
        fault_code: StableId,
    },
    IntegrityFault {
        fault_code: StableId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Qualified values stay inline across the reducer's single consuming path.
#[allow(clippy::large_enum_variant)]
pub(super) enum QualifiedIntentEvent {
    Admission,
    Transition {
        value: TransitionValueIntent,
    },
    Authorization {
        state_input_ref: LexicalValueRef,
        request: CanonicalJsonValue,
        physical_binding_ref: ContentRef,
    },
    Observation {
        authorization_ref: RecordRef,
        outcome: ObservationValueIntent,
    },
}

pub(super) enum QualifiedEvent<'a> {
    Intent(&'a QualifiedIntentEvent),
    Recorded(&'a QualifiedRecordedEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdmissionIntent {
    pub(super) genesis_semantic_state_digest: RunSemanticStateDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StateOutcomeIntent {
    Success(LexicalValueRef),
    Failure(LexicalValueRef),
}

impl StateOutcomeIntent {
    fn from_recorded(value: &StateOutcomeRef) -> Self {
        match value {
            StateOutcomeRef::Success(value) => Self::Success(value.clone()),
            StateOutcomeRef::Failure(value) => Self::Failure(value.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FactIntent {
    pub(super) emission_ordinal: u32,
    pub(super) fact_slot_ordinal: u32,
    pub(super) descriptor_ref: ContentRef,
    pub(super) subject: TypedValueRef,
    pub(super) response: TypedValueRef,
    pub(super) claim_ref: ContentRef,
}

impl From<&CommittedFactRef> for FactIntent {
    fn from(value: &CommittedFactRef) -> Self {
        Self {
            emission_ordinal: value.emission_ordinal,
            fact_slot_ordinal: value.fact_slot_ordinal,
            descriptor_ref: value.descriptor_ref.clone(),
            subject: value.subject.clone(),
            response: value.response.clone(),
            claim_ref: value.claim_ref.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TransitionIntent {
    pub(super) occurrence_id: OccurrenceId,
    pub(super) occurrence_path_ref: ContentRef,
    pub(super) semantic_call_id: SemanticCallId,
    pub(super) input: LexicalValueRef,
    pub(super) consumed_observation_ref: Option<RecordRef>,
    pub(super) outcome_ref: ContentRef,
    pub(super) outcome: StateOutcomeIntent,
    pub(super) facts: Vec<FactIntent>,
    pub(super) before_semantic_state_digest: RunSemanticStateDigest,
    pub(super) after_semantic_state_digest: RunSemanticStateDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AuthorizationIntent {
    pub(super) access_attempt_id: AccessAttemptId,
    pub(super) attempt_ordinal: u64,
    pub(super) occurrence_id: OccurrenceId,
    pub(super) occurrence_path_ref: ContentRef,
    pub(super) semantic_call_id: SemanticCallId,
    pub(super) state_input_ref: LexicalValueRef,
    pub(super) access_kind: AccessKind,
    pub(super) semantic_head: SemanticHead,
    pub(super) store_scope_id: StoreScopeId,
    pub(super) store_epoch: StoreEpoch,
    pub(super) tenant_scope_id: TenantScopeId,
    pub(super) admitted_routing_policy_ref: ContentRef,
    pub(super) minimum_lineage_head_ref: Option<ContentRef>,
    pub(super) capability_contract_ref: ContentRef,
    pub(super) capability_implementation_ref: ContentRef,
    pub(super) adapter_contract_ref: ContentRef,
    pub(super) adapter_implementation_ref: ContentRef,
    pub(super) request: TypedValueRef,
    pub(super) request_digest: RequestDigest,
    pub(super) physical_binding_ref: ContentRef,
    pub(super) stable_resource_lineage_contract_ref: Option<ContentRef>,
}

impl AuthorizationIntent {
    fn from_recorded(value: &ExternalAccessAuthorized) -> Self {
        Self {
            access_attempt_id: value.access_attempt_id.clone(),
            attempt_ordinal: value.attempt_ordinal,
            occurrence_id: value.occurrence_id.clone(),
            occurrence_path_ref: value.occurrence_path_ref.clone(),
            semantic_call_id: value.semantic_call_id.clone(),
            state_input_ref: value.state_input_ref.clone(),
            access_kind: value.access_kind,
            semantic_head: value.semantic_head.clone(),
            store_scope_id: value.store_scope_id.clone(),
            store_epoch: value.store_epoch,
            tenant_scope_id: value.tenant_scope_id.clone(),
            admitted_routing_policy_ref: value.admitted_routing_policy_ref.clone(),
            minimum_lineage_head_ref: value.minimum_lineage_head_ref.clone(),
            capability_contract_ref: value.capability_contract_ref.clone(),
            capability_implementation_ref: value.capability_implementation_ref.clone(),
            adapter_contract_ref: value.adapter_contract_ref.clone(),
            adapter_implementation_ref: value.adapter_implementation_ref.clone(),
            request: value.request.clone(),
            request_digest: value.request_digest.clone(),
            physical_binding_ref: value.physical_binding_ref.clone(),
            stable_resource_lineage_contract_ref: value
                .stable_resource_lineage_contract_ref
                .clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ObservationOutcomeIntent {
    Returned(TypedValueRef),
    SafeFailure(TypedValueRef),
    SupersededBeforeEntry {
        public_lineage_head_ref: ContentRef,
        evidence_ref: ContentRef,
    },
    EntryUnknown {
        fault_code: StableId,
    },
    IntegrityFault {
        fault_code: StableId,
    },
}

impl ObservationOutcomeIntent {
    fn from_recorded(value: &ObservationOutcome) -> Self {
        match value {
            ObservationOutcome::Returned { value } => Self::Returned(value.clone()),
            ObservationOutcome::SafeFailure { value } => Self::SafeFailure(value.clone()),
            ObservationOutcome::SupersededBeforeEntry {
                public_lineage_head_ref,
                evidence_ref,
            } => Self::SupersededBeforeEntry {
                public_lineage_head_ref: public_lineage_head_ref.clone(),
                evidence_ref: evidence_ref.clone(),
            },
            ObservationOutcome::EntryUnknown { fault_code } => Self::EntryUnknown {
                fault_code: fault_code.clone(),
            },
            ObservationOutcome::IntegrityFault { fault_code } => Self::IntegrityFault {
                fault_code: fault_code.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservationIntent {
    pub(super) authorization_ref: RecordRef,
    pub(super) access_attempt_id: AccessAttemptId,
    pub(super) outcome: ObservationOutcomeIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub(super) enum PrimaryIntent {
    Admission(AdmissionIntent),
    Transition(TransitionIntent),
    Authorization(AuthorizationIntent),
    Observation(ObservationIntent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ArtifactIntent {
    TypedValue {
        schema_id: mfm_ids::SchemaId,
        value: CanonicalJsonValue,
    },
    StateOutcome(StateOutcomeIntent),
    OperationOutcome(StateOutcomeIntent),
    FactClaim {
        descriptor_ref: ContentRef,
        subject: TypedValueRef,
        response: TypedValueRef,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ArtifactDrafts {
    authored: Vec<AddressedArtifactIntent>,
    retained: Vec<ContentRef>,
}

impl ArtifactDrafts {
    fn author(&mut self, intent: ArtifactIntent) -> Result<ContentRef, StructuredStoreError> {
        let addressed = address_artifact(intent)?;
        let reference = addressed.content_ref().clone();
        self.authored.push(addressed);
        Ok(reference)
    }

    fn retain(&mut self, reference: ContentRef) {
        self.retained.push(reference);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TenantFactRequirement {
    None,
    Barrier,
    Publish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Obligations stay inline as one closed, allocation-free validation set.
#[allow(clippy::large_enum_variant)]
pub(super) enum SemanticObligation {
    PhysicalAuthorization {
        access_kind: AccessKind,
        capability_contract_ref: ContentRef,
        capability_implementation_ref: ContentRef,
        adapter_contract_ref: ContentRef,
        adapter_implementation_ref: ContentRef,
        admitted_routing_policy_ref: ContentRef,
        stable_resource_lineage_contract_ref: Option<ContentRef>,
        minimum_lineage_head_ref: Option<ContentRef>,
        previous_physical_binding_ref: Option<ContentRef>,
        certificate_ref: ContentRef,
    },
    PhysicalSupersession {
        capability_contract_ref: ContentRef,
        adapter_contract_ref: ContentRef,
        adapter_implementation_ref: ContentRef,
        authorized_binding_ref: ContentRef,
        stable_resource_lineage_contract_ref: ContentRef,
        public_lineage_head_ref: ContentRef,
        evidence_ref: ContentRef,
    },
    PriorRunSelection {
        request: TypedValueRef,
        certificate_ref: ContentRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UnboundSuccessor {
    working: WorkingState,
    cursor: WorkCursor,
    bindings: BTreeMap<ContentRef, LexicalValueRef>,
    attention: Option<EffectEntryAttention>,
}

impl UnboundSuccessor {
    fn bind(
        mut self,
        primary_assignment: &RecordRef,
        journal_head: JournalHead,
    ) -> Result<ReducedRunState, StructuredStoreError> {
        bind_working_state(&mut self.working, primary_assignment)?;
        let cursor = bind_cursor(self.cursor, primary_assignment)?;
        let frontier = frontier(&cursor)?;
        let semantic_head = bind_semantic_head(
            self.working.semantic_head.as_ref().ok_or_else(invalid)?,
            primary_assignment,
        )?;
        Ok(ReducedRunState {
            working: self.working,
            cursor: Some(cursor),
            frontier: Some(frontier),
            bindings: self.bindings,
            journal_head: Some(journal_head),
            semantic_head: Some(semantic_head),
            effect_entry_attention: self.attention,
        })
    }
}

pub(super) struct PendingSemanticStep {
    successor: Box<UnboundSuccessor>,
    primary: PrimaryIntent,
    closure: Option<ContentRef>,
    artifacts: Vec<AddressedArtifactIntent>,
    retained_objects: Vec<ContentRef>,
    tenant_fact_requirement: TenantFactRequirement,
    obligations: Vec<SemanticObligation>,
}

impl PendingSemanticStep {
    pub(super) fn semantic_eq(&self, other: &Self) -> bool {
        self.successor == other.successor
            && self.primary == other.primary
            && self.closure == other.closure
            && self.artifacts == other.artifacts
            && self.retained_objects == other.retained_objects
            && self.tenant_fact_requirement == other.tenant_fact_requirement
            && self.obligations == other.obligations
    }

    pub(super) fn has_effect_entry_attention(&self) -> bool {
        self.successor.attention.is_some()
    }

    pub(super) const fn primary(&self) -> &PrimaryIntent {
        &self.primary
    }

    pub(super) fn closure(&self) -> Option<&ContentRef> {
        self.closure.as_ref()
    }

    pub(super) fn artifacts(&self) -> &[AddressedArtifactIntent] {
        &self.artifacts
    }

    pub(super) fn retained_objects(&self) -> &[ContentRef] {
        &self.retained_objects
    }

    pub(super) const fn tenant_fact_requirement(&self) -> &TenantFactRequirement {
        &self.tenant_fact_requirement
    }

    pub(super) fn bind_after_comparison(
        self,
        _passed: super::compiler::ComparisonPassed,
        primary_assignment: &RecordRef,
        journal_head: JournalHead,
    ) -> Result<(Box<ReducedRunState>, Vec<SemanticObligation>), StructuredStoreError> {
        let reduced = self.successor.bind(primary_assignment, journal_head)?;
        Ok((Box::new(reduced), self.obligations))
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_step(
    working: WorkingState,
    cursor: WorkCursor,
    bindings: BTreeMap<ContentRef, LexicalValueRef>,
    primary: PrimaryIntent,
    closure: Option<ContentRef>,
    artifacts: ArtifactDrafts,
    tenant_fact_requirement: TenantFactRequirement,
    obligations: Vec<SemanticObligation>,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    let attention = attention(&cursor)?;
    Ok(PendingSemanticStep {
        successor: Box::new(UnboundSuccessor {
            working,
            cursor,
            bindings,
            attention,
        }),
        primary,
        closure,
        artifacts: artifacts.authored,
        retained_objects: artifacts.retained,
        tenant_fact_requirement,
        obligations,
    })
}

pub(super) fn reduce_event(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    event: &QualifiedEvent<'_>,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    match event {
        QualifiedEvent::Intent(QualifiedIntentEvent::Admission) => {
            reduce_admission(context, previous)
        }
        QualifiedEvent::Recorded(QualifiedRecordedEvent::Admission { .. }) => {
            reduce_admission(context, previous)
        }
        QualifiedEvent::Intent(QualifiedIntentEvent::Transition { value }) => {
            reduce_transition(context, previous, value)
        }
        QualifiedEvent::Recorded(QualifiedRecordedEvent::Transition { transition, .. }) => {
            reduce_recorded_transition(context, previous, transition)
        }
        QualifiedEvent::Intent(QualifiedIntentEvent::Authorization {
            state_input_ref,
            request,
            physical_binding_ref,
        }) => reduce_authorization_intent(
            context,
            previous,
            state_input_ref,
            request,
            physical_binding_ref,
        ),
        QualifiedEvent::Recorded(QualifiedRecordedEvent::Authorization(authorization)) => {
            reduce_recorded_authorization(context, previous, authorization)
        }
        QualifiedEvent::Intent(QualifiedIntentEvent::Observation {
            authorization_ref,
            outcome,
        }) => reduce_observation_intent(context, previous, authorization_ref, outcome),
        QualifiedEvent::Recorded(QualifiedRecordedEvent::Observation(observation)) => {
            reduce_recorded_observation(context, previous, observation)
        }
    }
}

fn reduce_admission(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    if previous.working.admitted || previous.run_id() != &context.admission.run_id {
        return Err(invalid());
    }
    let mut working = previous.working.clone();
    let (cursor, bindings, mut artifacts) = {
        let mut engine = Engine::new(context, &working);
        for (slot, binding) in context
            .program
            .expanded()
            .input_roots
            .iter()
            .zip(&context.admission.initial_bindings)
        {
            engine.bind_exact(slot, binding.clone())?;
        }
        let cursor = engine.derive_root()?;
        (cursor, engine.bindings, engine.artifacts)
    };
    let digest = semantic_digest(
        &context.admission.certified_program_ref,
        &working.transitions,
        &bindings,
    )?;
    if context
        .admission
        .recorded_genesis
        .as_ref()
        .is_some_and(|value| value != &digest)
    {
        return Err(invalid());
    }
    working.admitted = true;
    working.admission_handle = Some(RecordHandle::PendingPrimary);
    working.semantic_head = Some(WorkingSemanticHead::Genesis {
        admission: RecordHandle::PendingPrimary,
        digest: digest.clone(),
    });
    let closure = match &cursor {
        WorkCursor::Completed { outcome_ref } => Some(outcome_ref.clone()),
        _ => None,
    };
    if let Some(outcome_ref) = &closure {
        working.closed_outcome_ref = Some(outcome_ref.clone());
    }
    for reference in &context.admission_object_refs {
        artifacts.retain(reference.clone());
    }
    finish_step(
        working,
        cursor,
        bindings,
        PrimaryIntent::Admission(AdmissionIntent {
            genesis_semantic_state_digest: digest,
        }),
        closure,
        artifacts,
        TenantFactRequirement::None,
        Vec::new(),
    )
}

fn reduce_transition(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    value: &TransitionValueIntent,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    require_open(previous)?;
    let action = previous.action()?.clone();
    let state = occurrence(context, &action.occurrence_id)?;
    validate_settleable(previous, &action)?;
    let mut artifacts = ArtifactDrafts::default();
    let (outcome, facts) = match value {
        TransitionValueIntent::Success { value, facts } => {
            let binding = typed_binding(
                context,
                &state.output_slot,
                value.clone(),
                &mut artifacts,
                None,
            )?;
            let facts = prepare_fact_intents(context, state, facts, &mut artifacts)?;
            (StateOutcomeIntent::Success(binding), facts)
        }
        TransitionValueIntent::Failure(value) => {
            let CertifiedFailureBoundary::Typed { source_slot, .. } = &state.failure_boundary
            else {
                return Err(invalid());
            };
            let binding = typed_binding(context, source_slot, value.clone(), &mut artifacts, None)?;
            (StateOutcomeIntent::Failure(binding), Vec::new())
        }
    };
    reduce_transition_parts(context, previous, action, state, outcome, facts, artifacts)
}

fn reduce_recorded_transition(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    transition: &StateTransitionCommitted,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    require_open(previous)?;
    let action = previous.action()?.clone();
    let state = occurrence(context, &action.occurrence_id)?;
    validate_settleable(previous, &action)?;
    let mut artifacts = ArtifactDrafts::default();
    match &transition.outcome {
        StateOutcomeRef::Success(binding) => {
            require_binding(context, &state.output_slot, binding, &mut artifacts)?;
        }
        StateOutcomeRef::Failure(binding) => {
            let CertifiedFailureBoundary::Typed { source_slot, .. } = &state.failure_boundary
            else {
                return Err(invalid());
            };
            require_binding(context, source_slot, binding, &mut artifacts)?;
        }
    }
    validate_recorded_facts(context, state, &transition.facts, &mut artifacts)?;
    reduce_transition_parts(
        context,
        previous,
        action,
        state,
        StateOutcomeIntent::from_recorded(&transition.outcome),
        transition.facts.iter().map(FactIntent::from).collect(),
        artifacts,
    )
}

#[allow(clippy::too_many_arguments)]
fn reduce_transition_parts(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    action: ActionableState,
    state: &ExpandedStateBinding,
    outcome: StateOutcomeIntent,
    facts: Vec<FactIntent>,
    mut artifacts: ArtifactDrafts,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    let [input_slot] = state.inputs.as_slice() else {
        return Err(invalid());
    };
    let input = previous
        .bindings
        .get(&action.input.slot_ref)
        .cloned()
        .ok_or_else(invalid)?;
    if action.input != input || input.value.contract_ref != input_slot.contract_ref {
        return Err(invalid());
    }
    let consumed_observation_ref = match &action.leaf {
        StateLeaf::Ready => None,
        StateLeaf::ObservedForSettlement {
            observation_ref, ..
        } => Some(observation_ref.clone()),
        _ => return Err(invalid()),
    };
    let outcome_ref = artifacts.author(ArtifactIntent::StateOutcome(outcome.clone()))?;
    let before = previous.semantic_head()?.semantic_state_digest().clone();
    let mut transition = TransitionIntent {
        occurrence_id: action.occurrence_id.clone(),
        occurrence_path_ref: action
            .occurrence_path
            .content_ref()
            .map_err(|_| invalid())?,
        semantic_call_id: state.semantic_call_id.clone(),
        input,
        consumed_observation_ref,
        outcome_ref,
        outcome,
        facts,
        before_semantic_state_digest: before,
        after_semantic_state_digest: previous.semantic_head()?.semantic_state_digest().clone(),
    };
    let mut working = previous.working.clone();
    if working.transitions.contains_key(&transition.occurrence_id) {
        return Err(invalid());
    }
    working.transitions.insert(
        transition.occurrence_id.clone(),
        TransitionEntry {
            handle: RecordHandle::PendingPrimary,
            intent: transition.clone(),
        },
    );
    let (cursor, bindings, artifacts) = {
        let mut engine = Engine::from_previous(context, &working, &previous.bindings, artifacts)?;
        let cursor = engine.derive_root()?;
        (cursor, engine.bindings, engine.artifacts)
    };
    transition.after_semantic_state_digest = semantic_digest(
        &context.admission.certified_program_ref,
        &working.transitions,
        &bindings,
    )?;
    working
        .transitions
        .get_mut(&transition.occurrence_id)
        .ok_or_else(invalid)?
        .intent = transition.clone();
    working.semantic_head = Some(WorkingSemanticHead::Transition {
        transition: RecordHandle::PendingPrimary,
        digest: transition.after_semantic_state_digest.clone(),
    });
    let closure = match &cursor {
        WorkCursor::Completed { outcome_ref } => Some(outcome_ref.clone()),
        _ => None,
    };
    if let Some(outcome_ref) = &closure {
        working.closed_outcome_ref = Some(outcome_ref.clone());
    }
    let tenant_fact_requirement = if transition.facts.is_empty() {
        TenantFactRequirement::None
    } else {
        TenantFactRequirement::Publish
    };
    finish_step(
        working,
        cursor,
        bindings,
        PrimaryIntent::Transition(transition),
        closure,
        artifacts,
        tenant_fact_requirement,
        Vec::new(),
    )
}

fn reduce_authorization_intent(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    state_input_ref: &LexicalValueRef,
    request: &CanonicalJsonValue,
    physical_binding_ref: &ContentRef,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    let action = previous.action()?.clone();
    let state = occurrence(context, &action.occurrence_id)?;
    let capability_ref = state
        .contract
        .execution
        .capability_contract_ref()
        .ok_or_else(invalid)?;
    let capability = context.live_component(capability_ref)?;
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(invalid)?;
    let mut artifacts = ArtifactDrafts::default();
    let authored_request_digest = request_digest(request)?;
    let request = typed_value(
        context,
        protocol.request_contract_ref(),
        request.clone(),
        &mut artifacts,
    )?;
    let authorization = author_authorization(
        context,
        previous,
        &action,
        state,
        state_input_ref,
        request,
        authored_request_digest,
        physical_binding_ref.clone(),
    )?;
    artifacts.retain(physical_binding_ref.clone());
    reduce_authorization_parts(context, previous, authorization, artifacts)
}

fn reduce_recorded_authorization(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    authorization: &ExternalAccessAuthorized,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    let mut artifacts = ArtifactDrafts::default();
    require_typed_value(
        context,
        &authorization.request,
        &authorization.request.contract_ref,
        &mut artifacts,
    )?;
    let retained_request = context
        .value(&authorization.request.value_ref)
        .ok_or_else(invalid)?;
    if authorization.request_digest != request_digest(retained_request)? {
        return Err(invalid());
    }
    artifacts.retain(authorization.physical_binding_ref.clone());
    reduce_authorization_parts(
        context,
        previous,
        AuthorizationIntent::from_recorded(authorization),
        artifacts,
    )
}

fn reduce_authorization_parts(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    authorization: AuthorizationIntent,
    artifacts: ArtifactDrafts,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    require_open(previous)?;
    let action = previous.action()?.clone();
    let state = occurrence(context, &action.occurrence_id)?;
    validate_authorization(context, previous, &action, state, &authorization)?;
    let capability = context.live_component(&authorization.capability_contract_ref)?;
    let is_fact_selection = capability
        == &mfm_spec::structured::prior_run_fact_selection_capability_contract()
            .map_err(|_| invalid())?;
    let minimum_lineage_head_ref = match &action.leaf {
        StateLeaf::Refreshable {
            public_lineage_head_ref,
            ..
        } => Some(public_lineage_head_ref.clone()),
        _ => None,
    };
    let previous_physical_binding_ref = if minimum_lineage_head_ref.is_some() {
        previous
            .working
            .occurrence_attempts
            .get(&authorization.occurrence_id)
            .and_then(|attempts| attempts.last())
            .and_then(|attempt| previous.working.authorizations.get(attempt))
            .map(|entry| entry.intent.physical_binding_ref.clone())
    } else {
        None
    };
    let obligation = if is_fact_selection {
        SemanticObligation::PriorRunSelection {
            request: authorization.request.clone(),
            certificate_ref: authorization.physical_binding_ref.clone(),
        }
    } else {
        SemanticObligation::PhysicalAuthorization {
            access_kind: authorization.access_kind,
            capability_contract_ref: authorization.capability_contract_ref.clone(),
            capability_implementation_ref: authorization.capability_implementation_ref.clone(),
            adapter_contract_ref: authorization.adapter_contract_ref.clone(),
            adapter_implementation_ref: authorization.adapter_implementation_ref.clone(),
            admitted_routing_policy_ref: authorization.admitted_routing_policy_ref.clone(),
            stable_resource_lineage_contract_ref: authorization
                .stable_resource_lineage_contract_ref
                .clone(),
            minimum_lineage_head_ref,
            previous_physical_binding_ref,
            certificate_ref: authorization.physical_binding_ref.clone(),
        }
    };
    let mut working = previous.working.clone();
    if working
        .authorizations
        .contains_key(&authorization.access_attempt_id)
    {
        return Err(invalid());
    }
    working
        .occurrence_attempts
        .entry(authorization.occurrence_id.clone())
        .or_default()
        .push(authorization.access_attempt_id.clone());
    working.authorizations.insert(
        authorization.access_attempt_id.clone(),
        AuthorizationEntry {
            handle: RecordHandle::PendingPrimary,
            intent: authorization.clone(),
        },
    );
    let mut engine = Engine::from_previous(context, &working, &previous.bindings, artifacts)?;
    let cursor = engine.derive_root()?;
    let bindings = engine.bindings;
    let artifacts = engine.artifacts;
    finish_step(
        working,
        cursor,
        bindings,
        PrimaryIntent::Authorization(authorization),
        None,
        artifacts,
        if is_fact_selection {
            TenantFactRequirement::Barrier
        } else {
            TenantFactRequirement::None
        },
        vec![obligation],
    )
}

fn reduce_observation_intent(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    authorization_ref: &RecordRef,
    outcome: &ObservationValueIntent,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    let authorization = previous
        .working
        .authorizations
        .values()
        .find(|entry| matches!(&entry.handle, RecordHandle::Existing(reference) if reference == authorization_ref))
        .ok_or_else(invalid)?;
    let capability = context.live_component(&authorization.intent.capability_contract_ref)?;
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(invalid)?;
    let mut artifacts = ArtifactDrafts::default();
    let outcome = match outcome {
        ObservationValueIntent::Returned(value) => ObservationOutcomeIntent::Returned(typed_value(
            context,
            protocol.returned_contract_ref(),
            value.clone(),
            &mut artifacts,
        )?),
        ObservationValueIntent::SafeFailure(value) => {
            ObservationOutcomeIntent::SafeFailure(typed_value(
                context,
                protocol.safe_failure_contract_ref(),
                value.clone(),
                &mut artifacts,
            )?)
        }
        ObservationValueIntent::SupersededBeforeEntry {
            public_lineage_head_ref,
            evidence,
        } => {
            let StructuredCapabilityProtocolContract::Effect {
                refresh_contract:
                    StructuredEffectRefreshContract::Refreshable {
                        refresh_evidence_contract_ref,
                        ..
                    },
                ..
            } = protocol
            else {
                return Err(invalid());
            };
            let evidence = typed_value(
                context,
                refresh_evidence_contract_ref,
                evidence.clone(),
                &mut artifacts,
            )?;
            artifacts.retain(public_lineage_head_ref.clone());
            ObservationOutcomeIntent::SupersededBeforeEntry {
                public_lineage_head_ref: public_lineage_head_ref.clone(),
                evidence_ref: evidence.value_ref,
            }
        }
        ObservationValueIntent::EntryUnknown { fault_code } => {
            ObservationOutcomeIntent::EntryUnknown {
                fault_code: fault_code.clone(),
            }
        }
        ObservationValueIntent::IntegrityFault { fault_code } => {
            ObservationOutcomeIntent::IntegrityFault {
                fault_code: fault_code.clone(),
            }
        }
    };
    let observation = ObservationIntent {
        authorization_ref: authorization_ref.clone(),
        access_attempt_id: authorization.intent.access_attempt_id.clone(),
        outcome,
    };
    reduce_observation_parts(context, previous, observation, artifacts)
}

fn reduce_recorded_observation(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    observation: &ExternalAccessObserved,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    let authorization = previous
        .working
        .authorizations
        .get(&observation.access_attempt_id)
        .ok_or_else(invalid)?;
    let capability = context.live_component(&authorization.intent.capability_contract_ref)?;
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(invalid)?;
    let mut artifacts = ArtifactDrafts::default();
    match &observation.outcome {
        ObservationOutcome::Returned { value } => require_typed_value(
            context,
            value,
            protocol.returned_contract_ref(),
            &mut artifacts,
        )?,
        ObservationOutcome::SafeFailure { value } => require_typed_value(
            context,
            value,
            protocol.safe_failure_contract_ref(),
            &mut artifacts,
        )?,
        ObservationOutcome::SupersededBeforeEntry { evidence_ref, .. } => {
            let StructuredCapabilityProtocolContract::Effect {
                refresh_contract:
                    StructuredEffectRefreshContract::Refreshable {
                        refresh_evidence_contract_ref,
                        ..
                    },
                ..
            } = protocol
            else {
                return Err(invalid());
            };
            require_typed_value(
                context,
                &TypedValueRef {
                    contract_ref: refresh_evidence_contract_ref.clone(),
                    value_ref: evidence_ref.clone(),
                },
                refresh_evidence_contract_ref,
                &mut artifacts,
            )?;
            if let ObservationOutcome::SupersededBeforeEntry {
                public_lineage_head_ref,
                ..
            } = &observation.outcome
            {
                artifacts.retain(public_lineage_head_ref.clone());
            }
        }
        ObservationOutcome::EntryUnknown { .. } | ObservationOutcome::IntegrityFault { .. } => {}
    }
    reduce_observation_parts(
        context,
        previous,
        ObservationIntent {
            authorization_ref: observation.authorization_ref.clone(),
            access_attempt_id: observation.access_attempt_id.clone(),
            outcome: ObservationOutcomeIntent::from_recorded(&observation.outcome),
        },
        artifacts,
    )
}

fn reduce_observation_parts(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    observation: ObservationIntent,
    artifacts: ArtifactDrafts,
) -> Result<PendingSemanticStep, StructuredStoreError> {
    require_open(previous)?;
    if previous
        .working
        .observations
        .contains_key(&observation.access_attempt_id)
    {
        return Err(invalid());
    }
    let authorization = previous
        .working
        .authorizations
        .get(&observation.access_attempt_id)
        .ok_or_else(invalid)?;
    let RecordHandle::Existing(authorization_ref) = &authorization.handle else {
        return Err(invalid());
    };
    if observation.authorization_ref != *authorization_ref
        || observation.access_attempt_id != authorization.intent.access_attempt_id
    {
        return Err(invalid());
    }
    if matches!(
        observation.outcome,
        ObservationOutcomeIntent::EntryUnknown { .. }
    ) && authorization.intent.access_kind != AccessKind::Effect
    {
        return Err(invalid());
    }
    let mut obligations = Vec::new();
    if let ObservationOutcomeIntent::SupersededBeforeEntry {
        public_lineage_head_ref,
        evidence_ref,
    } = &observation.outcome
    {
        let lineage = authorization
            .intent
            .stable_resource_lineage_contract_ref
            .clone()
            .ok_or_else(invalid)?;
        obligations.push(SemanticObligation::PhysicalSupersession {
            capability_contract_ref: authorization.intent.capability_contract_ref.clone(),
            adapter_contract_ref: authorization.intent.adapter_contract_ref.clone(),
            adapter_implementation_ref: authorization.intent.adapter_implementation_ref.clone(),
            authorized_binding_ref: authorization.intent.physical_binding_ref.clone(),
            stable_resource_lineage_contract_ref: lineage,
            public_lineage_head_ref: public_lineage_head_ref.clone(),
            evidence_ref: evidence_ref.clone(),
        });
    }
    let mut working = previous.working.clone();
    working.observations.insert(
        observation.access_attempt_id.clone(),
        ObservationEntry {
            handle: RecordHandle::PendingPrimary,
            intent: observation.clone(),
        },
    );
    let mut engine = Engine::from_previous(context, &working, &previous.bindings, artifacts)?;
    let cursor = engine.derive_root()?;
    let bindings = engine.bindings;
    let artifacts = engine.artifacts;
    finish_step(
        working,
        cursor,
        bindings,
        PrimaryIntent::Observation(observation),
        None,
        artifacts,
        TenantFactRequirement::None,
        obligations,
    )
}

fn require_open(previous: &ReducedRunState) -> Result<(), StructuredStoreError> {
    if !previous.working.admitted || previous.working.closed_outcome_ref.is_some() {
        return Err(invalid());
    }
    Ok(())
}

fn validate_settleable(
    previous: &ReducedRunState,
    action: &ActionableState,
) -> Result<(), StructuredStoreError> {
    match (&action.execution_kind, &action.leaf) {
        (StructuredExecutionKind::Pure, StateLeaf::Ready) => Ok(()),
        (
            StructuredExecutionKind::Read | StructuredExecutionKind::Effect,
            StateLeaf::ObservedForSettlement {
                access_attempt_id,
                observation_ref,
            },
        ) => {
            let authorization = previous
                .working
                .authorizations
                .get(access_attempt_id)
                .ok_or_else(invalid)?;
            let observation = previous
                .working
                .observations
                .get(access_attempt_id)
                .ok_or_else(invalid)?;
            let RecordHandle::Existing(actual_ref) = &observation.handle else {
                return Err(invalid());
            };
            let expected_kind = match action.execution_kind {
                StructuredExecutionKind::Read => AccessKind::Read,
                StructuredExecutionKind::Effect => AccessKind::Effect,
                StructuredExecutionKind::Pure => {
                    return Err(invalid());
                }
            };
            if actual_ref != observation_ref || authorization.intent.access_kind != expected_kind {
                return Err(invalid());
            }
            Ok(())
        }
        _ => Err(invalid()),
    }
}

// The explicit arguments are the complete authorization identity preimage.
#[allow(clippy::too_many_arguments)]
fn author_authorization(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    action: &ActionableState,
    state: &ExpandedStateBinding,
    state_input_ref: &LexicalValueRef,
    request: TypedValueRef,
    request_digest: RequestDigest,
    physical_binding_ref: ContentRef,
) -> Result<AuthorizationIntent, StructuredStoreError> {
    let capability_contract_ref = state
        .contract
        .execution
        .capability_contract_ref()
        .cloned()
        .ok_or_else(invalid)?;
    let access_kind = match state.contract.execution.kind() {
        StructuredExecutionKind::Read => AccessKind::Read,
        StructuredExecutionKind::Effect => AccessKind::Effect,
        StructuredExecutionKind::Pure => {
            return Err(invalid());
        }
    };
    let attempt_ordinal = next_attempt_ordinal(&action.leaf, access_kind)?;
    let capability = context.live_component(&capability_contract_ref)?;
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(invalid)?;
    if state_input_ref != &action.input || &request.contract_ref != protocol.request_contract_ref()
    {
        return Err(invalid());
    }
    let [adapter] = capability.dependencies.as_slice() else {
        return Err(invalid());
    };
    if adapter.component_kind != StructuredComponentKind::Adapter {
        return Err(invalid());
    }
    let capability_implementation_ref = context
        .implementation_ref(
            StructuredComponentKind::Capability,
            &capability_contract_ref,
        )?
        .clone();
    let adapter_implementation_ref = context
        .implementation_ref(StructuredComponentKind::Adapter, &adapter.contract_ref)?
        .clone();
    let stable_resource_lineage_contract_ref = state_lineage(context, state)?;
    let minimum_lineage_head_ref = match &action.leaf {
        StateLeaf::Refreshable {
            public_lineage_head_ref,
            ..
        } => Some(public_lineage_head_ref.clone()),
        _ => None,
    };
    let mut authorization = AuthorizationIntent {
        access_attempt_id: AccessAttemptId::from_digest(mfm_canonical::sha256_digest_bytes(&[])),
        attempt_ordinal,
        occurrence_id: action.occurrence_id.clone(),
        occurrence_path_ref: action
            .occurrence_path
            .content_ref()
            .map_err(|_| invalid())?,
        semantic_call_id: state.semantic_call_id.clone(),
        state_input_ref: state_input_ref.clone(),
        access_kind,
        semantic_head: previous.semantic_head()?.clone(),
        store_scope_id: context.admission.store_scope_id.clone(),
        store_epoch: context.admission.store_epoch,
        tenant_scope_id: context.admission.tenant_scope_id.clone(),
        admitted_routing_policy_ref: context
            .admission
            .admission_material_refs
            .routing_policy_ref
            .clone(),
        minimum_lineage_head_ref,
        capability_contract_ref,
        capability_implementation_ref,
        adapter_contract_ref: adapter.contract_ref.clone(),
        adapter_implementation_ref,
        request,
        request_digest,
        physical_binding_ref,
        stable_resource_lineage_contract_ref,
    };
    authorization.access_attempt_id = derive_access_attempt_id(&AccessAttemptIdentityPreimage {
        run_id: previous.run_id(),
        occurrence_id: &authorization.occurrence_id,
        occurrence_path_ref: &authorization.occurrence_path_ref,
        semantic_call_id: &authorization.semantic_call_id,
        state_input_ref: &authorization.state_input_ref,
        attempt_ordinal: authorization.attempt_ordinal,
        access_kind: authorization.access_kind,
        semantic_head: &authorization.semantic_head,
        capability_contract_ref: &authorization.capability_contract_ref,
        capability_implementation_ref: &authorization.capability_implementation_ref,
        adapter_contract_ref: &authorization.adapter_contract_ref,
        adapter_implementation_ref: &authorization.adapter_implementation_ref,
        request: &authorization.request,
        request_digest: &authorization.request_digest,
        physical_binding_ref: &authorization.physical_binding_ref,
        stable_resource_lineage_contract_ref: &authorization.stable_resource_lineage_contract_ref,
    })
    .map_err(|_| invalid())?;
    Ok(authorization)
}

fn validate_authorization(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    action: &ActionableState,
    state: &ExpandedStateBinding,
    authorization: &AuthorizationIntent,
) -> Result<(), StructuredStoreError> {
    let expected = author_authorization(
        context,
        previous,
        action,
        state,
        &authorization.state_input_ref,
        authorization.request.clone(),
        authorization.request_digest.clone(),
        authorization.physical_binding_ref.clone(),
    )?;
    if &expected != authorization {
        return Err(invalid());
    }
    if matches!(action.leaf, StateLeaf::Reassertable { .. }) {
        let predecessor = previous
            .working
            .occurrence_attempts
            .get(&authorization.occurrence_id)
            .and_then(|attempts| attempts.last())
            .and_then(|attempt| previous.working.authorizations.get(attempt))
            .ok_or_else(invalid)?;
        if predecessor.intent.occurrence_id != authorization.occurrence_id
            || predecessor.intent.state_input_ref != authorization.state_input_ref
            || predecessor.intent.request.contract_ref != authorization.request.contract_ref
            || predecessor.intent.request_digest != authorization.request_digest
        {
            return Err(invalid());
        }
    }
    if previous
        .working
        .occurrence_attempts
        .get(&authorization.occurrence_id)
        .is_some_and(|attempts| {
            attempts.iter().any(|attempt| {
                previous.working.authorizations.contains_key(attempt)
                    && !previous.working.observations.contains_key(attempt)
            })
        })
        && !matches!(
            (authorization.access_kind, &action.leaf),
            (AccessKind::Read, StateLeaf::Reassertable { .. })
        )
    {
        return Err(invalid());
    }
    Ok(())
}

fn next_attempt_ordinal(
    leaf: &StateLeaf,
    access_kind: AccessKind,
) -> Result<u64, StructuredStoreError> {
    match (leaf, access_kind) {
        (StateLeaf::Ready, _) => Ok(0),
        (
            StateLeaf::Refreshable {
                next_attempt_ordinal,
                ..
            },
            AccessKind::Effect,
        )
        | (
            StateLeaf::Reassertable {
                next_attempt_ordinal,
                ..
            },
            _,
        ) => Ok(*next_attempt_ordinal),
        _ => Err(invalid()),
    }
}

fn state_lineage(
    context: &QualifiedRunContext,
    state: &ExpandedStateBinding,
) -> Result<Option<ContentRef>, StructuredStoreError> {
    let Some(capability_ref) = state.contract.execution.capability_contract_ref() else {
        return Ok(None);
    };
    let protocol = context
        .live_component(capability_ref)?
        .capability_protocol
        .as_ref()
        .ok_or_else(invalid)?;
    match (state.contract.execution.kind(), protocol) {
        (StructuredExecutionKind::Read, StructuredCapabilityProtocolContract::Read { .. })
        | (
            StructuredExecutionKind::Effect,
            StructuredCapabilityProtocolContract::Effect {
                refresh_contract: StructuredEffectRefreshContract::NoRefresh {},
                ..
            },
        ) => Ok(None),
        (
            StructuredExecutionKind::Effect,
            StructuredCapabilityProtocolContract::Effect {
                refresh_contract:
                    StructuredEffectRefreshContract::Refreshable {
                        resource_lineage_contract_ref,
                        ..
                    },
                ..
            },
        ) => Ok(Some(resource_lineage_contract_ref.as_ref().clone())),
        _ => Err(invalid()),
    }
}

fn typed_value(
    context: &QualifiedRunContext,
    contract_ref: &ContentRef,
    value: CanonicalJsonValue,
    artifacts: &mut ArtifactDrafts,
) -> Result<TypedValueRef, StructuredStoreError> {
    context.validate_value(contract_ref, &value)?;
    let intent = ArtifactIntent::TypedValue {
        schema_id: context.value_schema_id(contract_ref)?,
        value,
    };
    let value_ref = artifacts.author(intent)?;
    Ok(TypedValueRef {
        contract_ref: contract_ref.clone(),
        value_ref,
    })
}

fn typed_binding(
    context: &QualifiedRunContext,
    slot: &LexicalSlot,
    value: CanonicalJsonValue,
    artifacts: &mut ArtifactDrafts,
    structural_origin: Option<StructuralValueOrigin>,
) -> Result<LexicalValueRef, StructuredStoreError> {
    Ok(LexicalValueRef {
        slot_ref: slot.content_ref().map_err(|_| invalid())?,
        value: typed_value(context, &slot.contract_ref, value, artifacts)?,
        structural_origin,
    })
}

fn require_typed_value(
    context: &QualifiedRunContext,
    value: &TypedValueRef,
    expected_contract: &ContentRef,
    artifacts: &mut ArtifactDrafts,
) -> Result<(), StructuredStoreError> {
    require_typed_value_ref(context, value, expected_contract)?;
    let canonical = context
        .value(&value.value_ref)
        .cloned()
        .ok_or_else(invalid)?;
    let artifact = ArtifactIntent::TypedValue {
        schema_id: context.value_schema_id(expected_contract)?,
        value: canonical,
    };
    if artifacts.author(artifact)? != value.value_ref {
        return Err(invalid());
    }
    Ok(())
}

fn require_typed_value_ref(
    context: &QualifiedRunContext,
    value: &TypedValueRef,
    expected_contract: &ContentRef,
) -> Result<(), StructuredStoreError> {
    if &value.contract_ref != expected_contract
        || value.value_ref.schema_id() != &context.value_schema_id(expected_contract)?
    {
        return Err(invalid());
    }
    let canonical = context.value(&value.value_ref).ok_or_else(invalid)?;
    context.validate_value(expected_contract, canonical)
}

fn require_binding(
    context: &QualifiedRunContext,
    slot: &LexicalSlot,
    binding: &LexicalValueRef,
    artifacts: &mut ArtifactDrafts,
) -> Result<(), StructuredStoreError> {
    if binding.slot_ref != slot.content_ref().map_err(|_| invalid())? {
        return Err(invalid());
    }
    require_typed_value(context, &binding.value, &slot.contract_ref, artifacts)
}

fn prepare_fact_intents(
    context: &QualifiedRunContext,
    state: &ExpandedStateBinding,
    facts: &FactSet,
    artifacts: &mut ArtifactDrafts,
) -> Result<Vec<FactIntent>, StructuredStoreError> {
    let mut proposals = facts.as_slice().iter().peekable();
    let mut committed = Vec::new();
    for slot in &state.contract.fact_slots {
        let mut count = 0_u32;
        while proposals
            .peek()
            .is_some_and(|proposal| proposal.fact_slot_ordinal() == slot.fact_slot_ordinal())
        {
            if count == slot.maximum_emissions() {
                return Err(invalid());
            }
            let proposal = proposals.next().ok_or_else(invalid)?;
            if proposal.descriptor_ref() != slot.fact_descriptor_ref() {
                return Err(invalid());
            }
            let subject_contract_ref = slot
                .subject_contract()
                .content_ref()
                .map_err(|_| invalid())?;
            let response_contract_ref = slot
                .response_contract()
                .content_ref()
                .map_err(|_| invalid())?;
            validate_fact_component(proposal.subject(), slot.subject_contract())?;
            validate_fact_component(proposal.response(), slot.response_contract())?;
            let subject_value =
                CanonicalJsonValue::from_exact_bytes(proposal.subject().canonical().as_bytes())
                    .map_err(|_| invalid())?;
            let response_value =
                CanonicalJsonValue::from_exact_bytes(proposal.response().canonical().as_bytes())
                    .map_err(|_| invalid())?;
            let subject = typed_value(context, &subject_contract_ref, subject_value, artifacts)?;
            let response = typed_value(context, &response_contract_ref, response_value, artifacts)?;
            if subject.value_ref != *proposal.subject().content_ref()
                || response.value_ref != *proposal.response().content_ref()
            {
                return Err(invalid());
            }
            let claim_ref = artifacts.author(ArtifactIntent::FactClaim {
                descriptor_ref: proposal.descriptor_ref().clone(),
                subject: subject.clone(),
                response: response.clone(),
            })?;
            committed.push(FactIntent {
                emission_ordinal: u32::try_from(committed.len()).map_err(|_| invalid())?,
                fact_slot_ordinal: slot.fact_slot_ordinal(),
                descriptor_ref: proposal.descriptor_ref().clone(),
                subject,
                response,
                claim_ref,
            });
            count = count.checked_add(1).ok_or_else(invalid)?;
        }
        if count < slot.minimum_emissions() {
            return Err(invalid());
        }
    }
    if proposals.next().is_some() {
        return Err(invalid());
    }
    Ok(committed)
}

fn validate_fact_component(
    proposed: &mfm_facts::ProposedFactValue,
    contract: &mfm_values::RetainedValueContract,
) -> Result<(), StructuredStoreError> {
    if proposed.schema_id() != contract.schema_id()
        || proposed.semantic_type_id() != contract.semantic_type_id()
        || proposed.role() != contract.role()
        || proposed.media_type() != contract.media_type()
        || proposed.evidence_contract_ref() != contract.evidence_contract_ref()
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_recorded_facts(
    context: &QualifiedRunContext,
    state: &ExpandedStateBinding,
    facts: &[CommittedFactRef],
    artifacts: &mut ArtifactDrafts,
) -> Result<(), StructuredStoreError> {
    let mut values = facts.iter().peekable();
    let mut ordinal = 0_u32;
    for slot in &state.contract.fact_slots {
        let mut count = 0_u32;
        while values
            .peek()
            .is_some_and(|fact| fact.fact_slot_ordinal == slot.fact_slot_ordinal())
        {
            let fact = values.next().ok_or_else(invalid)?;
            if fact.emission_ordinal != ordinal
                || fact.descriptor_ref != *slot.fact_descriptor_ref()
                || count == slot.maximum_emissions()
            {
                return Err(invalid());
            }
            let subject_contract_ref = slot
                .subject_contract()
                .content_ref()
                .map_err(|_| invalid())?;
            let response_contract_ref = slot
                .response_contract()
                .content_ref()
                .map_err(|_| invalid())?;
            require_typed_value(context, &fact.subject, &subject_contract_ref, artifacts)?;
            require_typed_value(context, &fact.response, &response_contract_ref, artifacts)?;
            let claim_ref = artifacts.author(ArtifactIntent::FactClaim {
                descriptor_ref: fact.descriptor_ref.clone(),
                subject: fact.subject.clone(),
                response: fact.response.clone(),
            })?;
            if claim_ref != fact.claim_ref {
                return Err(invalid());
            }
            ordinal = ordinal.checked_add(1).ok_or_else(invalid)?;
            count = count.checked_add(1).ok_or_else(invalid)?;
        }
        if count < slot.minimum_emissions() {
            return Err(invalid());
        }
    }
    if values.next().is_some() {
        return Err(invalid());
    }
    Ok(())
}

fn occurrence<'a>(
    context: &'a QualifiedRunContext,
    occurrence_id: &OccurrenceId,
) -> Result<&'a ExpandedStateBinding, StructuredStoreError> {
    occurrence_in_block(&context.program.expanded().root, occurrence_id).ok_or_else(invalid)
}

fn occurrence_in_block<'a>(
    block: &'a ExpandedBlock,
    occurrence_id: &OccurrenceId,
) -> Option<&'a ExpandedStateBinding> {
    block
        .declarations
        .iter()
        .find_map(|declaration| match declaration {
            ExpandedDeclaration::State(state) => (&state.occurrence_id == occurrence_id)
                .then_some(state.as_ref())
                .or_else(|| occurrence_in_boundary(&state.failure_boundary, occurrence_id)),
            ExpandedDeclaration::Match(binding) => binding
                .arms
                .iter()
                .find_map(|arm| occurrence_in_block(&arm.body, occurrence_id)),
            ExpandedDeclaration::FanOut(group) => group
                .lanes
                .iter()
                .find_map(|lane| occurrence_in_block(&lane.body, occurrence_id)),
            ExpandedDeclaration::Fragment(fragment) => {
                occurrence_in_block(&fragment.body, occurrence_id)
                    .or_else(|| occurrence_in_boundary(&fragment.failure_boundary, occurrence_id))
            }
        })
}

fn occurrence_in_boundary<'a>(
    boundary: &'a CertifiedFailureBoundary,
    occurrence_id: &OccurrenceId,
) -> Option<&'a ExpandedStateBinding> {
    let CertifiedFailureBoundary::Typed { plan, .. } = boundary else {
        return None;
    };
    match plan.as_ref() {
        FailurePlan::Propagate {
            before_boundary,
            mapping_chain,
            ..
        } => occurrence_in_block(before_boundary, occurrence_id).or_else(|| {
            mapping_chain.iter().find_map(|link| {
                (&link.mapper.occurrence_id == occurrence_id)
                    .then_some(link.mapper.as_ref())
                    .or_else(|| {
                        occurrence_in_boundary(&link.mapper.failure_boundary, occurrence_id)
                    })
            })
        }),
        FailurePlan::Handled {
            before_handler,
            handler,
            continuation,
            ..
        } => occurrence_in_block(before_handler, occurrence_id)
            .or_else(|| (&handler.occurrence_id == occurrence_id).then_some(handler.as_ref()))
            .or_else(|| occurrence_in_boundary(&handler.failure_boundary, occurrence_id))
            .or_else(|| match continuation.as_ref() {
                HandlerContinuation::DefaultPropagation { .. } => None,
                HandlerContinuation::CustomRecovery { arms, .. } => arms
                    .iter()
                    .find_map(|arm| occurrence_in_block(&arm.body, occurrence_id)),
            }),
    }
}

#[derive(Clone)]
struct BoundValue {
    reference: LexicalValueRef,
    value: CanonicalJsonValue,
}

enum FlowValue {
    Normal(Box<BoundValue>),
    ScopeFailure(Box<BoundValue>),
}

enum WalkResult {
    Pending(Box<WorkCursor>),
    Complete(FlowValue),
}

enum DeclarationResult {
    Continue,
    Pending(Box<WorkCursor>),
    ScopeFailure(Box<BoundValue>),
}

enum StateWalk {
    Pending(Box<WorkCursor>),
    Success(Box<BoundValue>),
    Failure(Box<BoundValue>),
}

enum FailureResolution {
    Pending(Box<WorkCursor>),
    Recovered(Box<BoundValue>),
    ScopeFailure(Box<BoundValue>),
}

struct Engine<'a> {
    context: &'a QualifiedRunContext,
    working: &'a WorkingState,
    bindings: BTreeMap<ContentRef, LexicalValueRef>,
    generated: BTreeMap<ContentRef, CanonicalJsonValue>,
    artifacts: ArtifactDrafts,
}

impl<'a> Engine<'a> {
    fn new(context: &'a QualifiedRunContext, working: &'a WorkingState) -> Self {
        Self {
            context,
            working,
            bindings: BTreeMap::new(),
            generated: BTreeMap::new(),
            artifacts: ArtifactDrafts::default(),
        }
    }

    fn from_previous(
        context: &'a QualifiedRunContext,
        working: &'a WorkingState,
        bindings: &BTreeMap<ContentRef, LexicalValueRef>,
        artifacts: ArtifactDrafts,
    ) -> Result<Self, StructuredStoreError> {
        let mut generated = BTreeMap::new();
        for artifact in &artifacts.authored {
            if let Some(value) = artifact.typed_value() {
                generated.insert(artifact.content_ref().clone(), value.clone());
            }
        }
        Ok(Self {
            context,
            working,
            bindings: bindings.clone(),
            generated,
            artifacts,
        })
    }

    fn value(&self, reference: &ContentRef) -> Option<&CanonicalJsonValue> {
        self.generated
            .get(reference)
            .or_else(|| self.context.value(reference))
    }

    fn derive_root(&mut self) -> Result<WorkCursor, StructuredStoreError> {
        match walk_block(self, &self.context.program.expanded().root)? {
            WalkResult::Pending(cursor) => Ok(*cursor),
            WalkResult::Complete(flow) => {
                let artifact = match flow {
                    FlowValue::Normal(value) => {
                        StateOutcomeIntent::Success(value.reference.clone())
                    }
                    FlowValue::ScopeFailure(value) => {
                        StateOutcomeIntent::Failure(value.reference.clone())
                    }
                };
                let outcome_ref = self
                    .artifacts
                    .author(ArtifactIntent::OperationOutcome(artifact))?;
                Ok(WorkCursor::Completed { outcome_ref })
            }
        }
    }

    fn bind_exact(
        &mut self,
        slot: &LexicalSlot,
        binding: LexicalValueRef,
    ) -> Result<(), StructuredStoreError> {
        let slot_ref = self.context.lexical_slot_ref(slot)?.clone();
        if binding.slot_ref != slot_ref || binding.value.contract_ref != slot.contract_ref {
            return Err(invalid());
        }
        let value = self.value(&binding.value.value_ref).ok_or_else(invalid)?;
        self.context.validate_value(&slot.contract_ref, value)?;
        match self.bindings.get(&slot_ref) {
            Some(existing) if existing == &binding => Ok(()),
            Some(_) => Err(invalid()),
            None => {
                self.bindings.insert(slot_ref, binding);
                Ok(())
            }
        }
    }

    fn bind_alias(
        &mut self,
        slot: &LexicalSlot,
        source: &BoundValue,
    ) -> Result<BoundValue, StructuredStoreError> {
        self.bind_alias_with_origin(slot, source, None)
    }

    fn bind_alias_with_origin(
        &mut self,
        slot: &LexicalSlot,
        source: &BoundValue,
        structural_origin: Option<StructuralValueOrigin>,
    ) -> Result<BoundValue, StructuredStoreError> {
        let binding = LexicalValueRef {
            slot_ref: self.context.lexical_slot_ref(slot)?.clone(),
            value: TypedValueRef {
                contract_ref: slot.contract_ref.clone(),
                value_ref: source.reference.value.value_ref.clone(),
            },
            structural_origin,
        };
        self.bind_exact(slot, binding.clone())?;
        Ok(BoundValue {
            reference: binding,
            value: source.value.clone(),
        })
    }

    fn bind_derived(
        &mut self,
        slot: &LexicalSlot,
        value: CanonicalJsonValue,
    ) -> Result<BoundValue, StructuredStoreError> {
        self.bind_derived_with_origin(slot, value, None)
    }

    fn bind_derived_with_origin(
        &mut self,
        slot: &LexicalSlot,
        value: CanonicalJsonValue,
        structural_origin: Option<StructuralValueOrigin>,
    ) -> Result<BoundValue, StructuredStoreError> {
        self.context.validate_value(&slot.contract_ref, &value)?;
        let artifact = ArtifactIntent::TypedValue {
            schema_id: self.context.value_schema_id(&slot.contract_ref)?,
            value: value.clone(),
        };
        let value_ref = self.artifacts.author(artifact)?;
        self.generated.insert(value_ref.clone(), value.clone());
        let binding = LexicalValueRef {
            slot_ref: self.context.lexical_slot_ref(slot)?.clone(),
            value: TypedValueRef {
                contract_ref: slot.contract_ref.clone(),
                value_ref,
            },
            structural_origin,
        };
        self.bind_exact(slot, binding.clone())?;
        Ok(BoundValue {
            reference: binding,
            value,
        })
    }

    fn resolve_slot(&mut self, slot: &LexicalSlot) -> Result<BoundValue, StructuredStoreError> {
        let mut aliases = Vec::new();
        let mut current = slot;
        let mut resolved = loop {
            let slot_ref = self.context.lexical_slot_ref(current)?.clone();
            if let Some(binding) = self.bindings.get(&slot_ref).cloned() {
                let value = self
                    .value(&binding.value.value_ref)
                    .cloned()
                    .ok_or_else(invalid)?;
                break BoundValue {
                    reference: binding,
                    value,
                };
            }
            match &current.producer {
                LexicalProducer::VariantPayload {
                    selector,
                    canonical_tag,
                    payload_path,
                } => {
                    let selector = self.resolve_slot(selector)?;
                    if selector.value.string_field("kind") != Some(canonical_tag.as_str()) {
                        return Err(invalid());
                    }
                    let selected = selector
                        .value
                        .select_path(payload_path.iter().map(StableId::as_str))
                        .map_err(|_| invalid())?;
                    break self.bind_derived(current, selected)?;
                }
                LexicalProducer::FragmentInput { source, .. }
                | LexicalProducer::FragmentBoundary { source, .. }
                | LexicalProducer::ArmValue { source, .. } => {
                    aliases.push(current);
                    current = source;
                }
                LexicalProducer::AdmissionRoot { .. }
                | LexicalProducer::StateOutput { .. }
                | LexicalProducer::MatchMerge { .. }
                | LexicalProducer::ScopeFailureMerge { .. }
                | LexicalProducer::LaneOutcome { .. }
                | LexicalProducer::FanOutJoin { .. }
                | LexicalProducer::AuthoredCallOutput { .. } => {
                    return Err(invalid());
                }
            }
        };
        for alias in aliases.into_iter().rev() {
            resolved = self.bind_alias(alias, &resolved)?;
        }
        Ok(resolved)
    }
}

fn walk_block(
    engine: &mut Engine<'_>,
    block: &ExpandedBlock,
) -> Result<WalkResult, StructuredStoreError> {
    for declaration in &block.declarations {
        match walk_declaration(engine, declaration)? {
            DeclarationResult::Continue => {}
            DeclarationResult::Pending(cursor) => return Ok(WalkResult::Pending(cursor)),
            DeclarationResult::ScopeFailure(value) => {
                return Ok(WalkResult::Complete(FlowValue::ScopeFailure(value)));
            }
        }
    }
    match &block.tail {
        BlockTail::Normal(slot) => Ok(WalkResult::Complete(FlowValue::Normal(Box::new(
            engine.resolve_slot(slot)?,
        )))),
        BlockTail::ScopeFailure(slot) => Ok(WalkResult::Complete(FlowValue::ScopeFailure(
            Box::new(engine.resolve_slot(slot)?),
        ))),
    }
}

fn walk_declaration(
    engine: &mut Engine<'_>,
    declaration: &ExpandedDeclaration,
) -> Result<DeclarationResult, StructuredStoreError> {
    match declaration {
        ExpandedDeclaration::State(state) => match walk_state(engine, state)? {
            StateWalk::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
            StateWalk::Success(value) => {
                engine.bind_exact(&state.output_slot, value.reference)?;
                Ok(DeclarationResult::Continue)
            }
            StateWalk::Failure(failure) => {
                match resolve_failure(engine, &state.failure_boundary, *failure)? {
                    FailureResolution::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
                    FailureResolution::Recovered(value) => {
                        engine.bind_alias(&state.output_slot, &value)?;
                        Ok(DeclarationResult::Continue)
                    }
                    FailureResolution::ScopeFailure(value) => {
                        Ok(DeclarationResult::ScopeFailure(value))
                    }
                }
            }
        },
        ExpandedDeclaration::Match(binding) => walk_match(engine, binding),
        ExpandedDeclaration::FanOut(group) => walk_fan_out(engine, group),
        ExpandedDeclaration::Fragment(fragment) => walk_fragment(engine, fragment),
    }
}

fn walk_state(
    engine: &mut Engine<'_>,
    state: &ExpandedStateBinding,
) -> Result<StateWalk, StructuredStoreError> {
    let [input_slot] = state.inputs.as_slice() else {
        return Err(invalid());
    };
    let input = engine.resolve_slot(input_slot)?;
    if let Some(transition) = engine.working.transitions.get(&state.occurrence_id) {
        return walk_committed_state(engine, state, input, &transition.intent);
    }
    walk_pending_state(engine, state, input)
}

fn walk_committed_state(
    engine: &mut Engine<'_>,
    state: &ExpandedStateBinding,
    input: BoundValue,
    transition: &TransitionIntent,
) -> Result<StateWalk, StructuredStoreError> {
    if transition.input != input.reference
        || transition.semantic_call_id != state.semantic_call_id
        || transition.occurrence_path_ref
            != state.occurrence_path.content_ref().map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    match &transition.outcome {
        StateOutcomeIntent::Success(value) => {
            engine.bind_exact(&state.output_slot, value.clone())?;
            Ok(StateWalk::Success(Box::new(BoundValue {
                value: engine
                    .value(&value.value.value_ref)
                    .cloned()
                    .ok_or_else(invalid)?,
                reference: value.clone(),
            })))
        }
        StateOutcomeIntent::Failure(value) => {
            let CertifiedFailureBoundary::Typed { source_slot, .. } = &state.failure_boundary
            else {
                return Err(invalid());
            };
            engine.bind_exact(source_slot, value.clone())?;
            Ok(StateWalk::Failure(Box::new(BoundValue {
                value: engine
                    .value(&value.value.value_ref)
                    .cloned()
                    .ok_or_else(invalid)?,
                reference: value.clone(),
            })))
        }
    }
}

fn walk_pending_state(
    engine: &mut Engine<'_>,
    state: &ExpandedStateBinding,
    input: BoundValue,
) -> Result<StateWalk, StructuredStoreError> {
    let protocol = state_protocol(engine.context, state)?;
    let leaf = state_leaf(engine.working, state, protocol.as_ref())?;
    Ok(StateWalk::Pending(Box::new(WorkCursor::AtState(
        WorkAction {
            occurrence_id: state.occurrence_id.clone(),
            occurrence_path: state.occurrence_path.clone(),
            semantic_call_id: state.semantic_call_id.clone(),
            state_contract_ref: state.contract.content_ref().map_err(|_| invalid())?,
            input: input.reference,
            capability_contract_ref: state.contract.execution.capability_contract_ref().cloned(),
            stable_resource_lineage_contract_ref: state_lineage(engine.context, state)?,
            execution_kind: state.contract.execution.kind(),
            leaf,
        },
    ))))
}

fn walk_match(
    engine: &mut Engine<'_>,
    binding: &ExpandedMatch,
) -> Result<DeclarationResult, StructuredStoreError> {
    let selector = engine.resolve_slot(&binding.selector)?;
    let tag = selector.value.string_field("kind").ok_or_else(invalid)?;
    let (arm_ordinal, arm) = binding
        .arms
        .iter()
        .enumerate()
        .find(|(_, arm)| arm.canonical_tag == tag)
        .ok_or_else(invalid)?;
    match walk_block(engine, &arm.body)? {
        WalkResult::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
        WalkResult::Complete(FlowValue::Normal(value)) => {
            let origin = StructuralValueOrigin::MatchArm {
                match_path_ref: binding.path.content_ref().map_err(|_| invalid())?,
                arm_ordinal: u32::try_from(arm_ordinal).map_err(|_| invalid())?,
                arm_key: arm.label.clone(),
                value_contract_ref: binding.output_slot.contract_ref.clone(),
                source_slot_ref: value.reference.slot_ref.clone(),
                source_value_ref: value.reference.value.value_ref.clone(),
            };
            engine.bind_alias_with_origin(&binding.output_slot, &value, Some(origin))?;
            Ok(DeclarationResult::Continue)
        }
        WalkResult::Complete(FlowValue::ScopeFailure(value)) => {
            Ok(DeclarationResult::ScopeFailure(value))
        }
    }
}

fn walk_fragment(
    engine: &mut Engine<'_>,
    fragment: &ExpandedFragment,
) -> Result<DeclarationResult, StructuredStoreError> {
    for input in &fragment.input_bindings {
        let source = engine.resolve_slot(&input.caller_slot)?;
        let rebound = LexicalSlot {
            lexical_path: fragment.path.clone(),
            contract_ref: input.child_contract_ref.clone(),
            producer: LexicalProducer::FragmentInput {
                boundary_id: fragment.boundary_id.clone(),
                child_root_id: input.child_root_id.clone(),
                source: Box::new(input.caller_slot.clone()),
            },
        };
        engine.bind_alias(&rebound, &source)?;
    }
    match walk_block(engine, &fragment.body)? {
        WalkResult::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
        WalkResult::Complete(FlowValue::Normal(value)) => {
            engine.bind_alias(&fragment.success_slot, &value)?;
            Ok(DeclarationResult::Continue)
        }
        WalkResult::Complete(FlowValue::ScopeFailure(failure)) => {
            match resolve_failure(engine, &fragment.failure_boundary, *failure)? {
                FailureResolution::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
                FailureResolution::Recovered(value) => {
                    engine.bind_alias(&fragment.success_slot, &value)?;
                    Ok(DeclarationResult::Continue)
                }
                FailureResolution::ScopeFailure(value) => {
                    Ok(DeclarationResult::ScopeFailure(value))
                }
            }
        }
    }
}

fn walk_fan_out(
    engine: &mut Engine<'_>,
    group: &ExpandedFanOut,
) -> Result<DeclarationResult, StructuredStoreError> {
    let parent = engine.bindings.clone();
    let mut joined = parent.clone();
    let mut cursors = Vec::with_capacity(group.lanes.len());
    let mut values = Vec::with_capacity(group.lanes.len());
    let mut pending = false;
    let group_path_ref = group.path.content_ref().map_err(|_| invalid())?;
    for (lane_ordinal, lane) in group.lanes.iter().enumerate() {
        engine.bindings = parent.clone();
        match walk_block(engine, &lane.body)? {
            WalkResult::Pending(cursor) => {
                pending = true;
                cursors.push(*cursor);
                values.push(None);
            }
            WalkResult::Complete(flow) => {
                let (tag, value) = match flow {
                    FlowValue::Normal(value) => ("Success", value),
                    FlowValue::ScopeFailure(value) => ("Failure", value),
                };
                let origin = StructuralValueOrigin::FanOutLane {
                    group_path_ref: group_path_ref.clone(),
                    lane_ordinal: u32::try_from(lane_ordinal).map_err(|_| invalid())?,
                    lane_key: lane.key.clone(),
                    outcome_contract_ref: lane.outcome_slot.contract_ref.clone(),
                    source_slot_ref: value.reference.slot_ref.clone(),
                    source_value_ref: value.reference.value.value_ref.clone(),
                };
                let tagged = CanonicalJsonValue::tagged(tag, value.value).map_err(|_| invalid())?;
                let value =
                    engine.bind_derived_with_origin(&lane.outcome_slot, tagged, Some(origin))?;
                cursors.push(WorkCursor::Completed {
                    outcome_ref: value.reference.value.value_ref.clone(),
                });
                values.push(Some(value));
            }
        }
        merge_bindings(&mut joined, &engine.bindings)?;
    }
    engine.bindings = joined;
    if pending {
        return Ok(DeclarationResult::Pending(Box::new(WorkCursor::InFanOut {
            group_path: group.path.clone(),
            lanes: cursors,
        })));
    }
    let mut values = values
        .into_iter()
        .map(|value| value.ok_or_else(invalid))
        .collect::<Result<Vec<_>, _>>()?;
    let head = values.first().cloned().ok_or_else(invalid)?;
    values.remove(0);
    let joined_value = CanonicalJsonValue::head_tail(
        head.value,
        values.into_iter().map(|value| value.value).collect(),
    )
    .map_err(|_| invalid())?;
    engine.bind_derived(&group.output_slot, joined_value)?;
    Ok(DeclarationResult::Continue)
}

fn merge_bindings(
    joined: &mut BTreeMap<ContentRef, LexicalValueRef>,
    lane: &BTreeMap<ContentRef, LexicalValueRef>,
) -> Result<(), StructuredStoreError> {
    for (slot, binding) in lane {
        match joined.get(slot) {
            Some(existing) if existing == binding => {}
            Some(_) => return Err(invalid()),
            None => {
                joined.insert(slot.clone(), binding.clone());
            }
        }
    }
    Ok(())
}

fn resolve_failure(
    engine: &mut Engine<'_>,
    boundary: &CertifiedFailureBoundary,
    failure: BoundValue,
) -> Result<FailureResolution, StructuredStoreError> {
    let CertifiedFailureBoundary::Typed {
        source_slot, plan, ..
    } = boundary
    else {
        return Err(invalid());
    };
    engine.bind_alias(source_slot, &failure)?;
    match plan.as_ref() {
        FailurePlan::Propagate {
            before_boundary,
            mapping_chain,
            boundary_slot,
            source_slot,
            ..
        } => {
            let mut prefix = before_boundary.as_ref().clone();
            prefix.tail = BlockTail::Normal(source_slot.clone());
            let mut current = match walk_block(engine, &prefix)? {
                WalkResult::Pending(cursor) => return Ok(FailureResolution::Pending(cursor)),
                WalkResult::Complete(FlowValue::Normal(value)) => *value,
                WalkResult::Complete(FlowValue::ScopeFailure(value)) => {
                    return Ok(FailureResolution::ScopeFailure(value));
                }
            };
            for link in mapping_chain {
                match walk_state(engine, &link.mapper)? {
                    StateWalk::Pending(cursor) => return Ok(FailureResolution::Pending(cursor)),
                    StateWalk::Success(value) => {
                        engine.bind_exact(&link.output_slot, value.reference.clone())?;
                        current = *value;
                    }
                    StateWalk::Failure(_) => {
                        return Err(invalid());
                    }
                }
            }
            let source = if mapping_chain.is_empty() {
                current
            } else {
                engine.resolve_slot(&mapping_chain.last().ok_or_else(invalid)?.output_slot)?
            };
            Ok(FailureResolution::ScopeFailure(Box::new(
                engine.bind_alias(boundary_slot, &source)?,
            )))
        }
        FailurePlan::Handled {
            before_handler,
            handler,
            continuation,
            ..
        } => {
            match walk_block(engine, before_handler)? {
                WalkResult::Pending(cursor) => return Ok(FailureResolution::Pending(cursor)),
                WalkResult::Complete(FlowValue::ScopeFailure(value)) => {
                    return Ok(FailureResolution::ScopeFailure(value));
                }
                WalkResult::Complete(FlowValue::Normal(_)) => {}
            }
            let handled = match walk_state(engine, handler)? {
                StateWalk::Pending(cursor) => return Ok(FailureResolution::Pending(cursor)),
                StateWalk::Success(value) => *value,
                StateWalk::Failure(_) => {
                    return Err(invalid());
                }
            };
            engine.bind_exact(&handler.output_slot, handled.reference.clone())?;
            match continuation.as_ref() {
                HandlerContinuation::DefaultPropagation {
                    payload_slot,
                    failure_tail,
                    ..
                } => {
                    let payload = engine.resolve_slot(payload_slot)?;
                    Ok(FailureResolution::ScopeFailure(Box::new(
                        engine.bind_alias(failure_tail, &payload)?,
                    )))
                }
                HandlerContinuation::CustomRecovery { arms, .. } => {
                    let tag = handled.value.string_field("kind").ok_or_else(invalid)?;
                    let arm = arms
                        .iter()
                        .find(|arm| arm.canonical_tag == tag)
                        .ok_or_else(invalid)?;
                    match walk_block(engine, &arm.body)? {
                        WalkResult::Pending(cursor) => Ok(FailureResolution::Pending(cursor)),
                        WalkResult::Complete(FlowValue::Normal(value)) => {
                            Ok(FailureResolution::Recovered(value))
                        }
                        WalkResult::Complete(FlowValue::ScopeFailure(value)) => {
                            Ok(FailureResolution::ScopeFailure(value))
                        }
                    }
                }
            }
        }
    }
}

fn state_protocol(
    context: &QualifiedRunContext,
    state: &ExpandedStateBinding,
) -> Result<Option<StructuredCapabilityProtocolContract>, StructuredStoreError> {
    let Some(reference) = state.contract.execution.capability_contract_ref() else {
        return Ok(None);
    };
    context
        .live_component(reference)?
        .capability_protocol
        .clone()
        .ok_or_else(invalid)
        .map(Some)
}

fn state_leaf(
    working: &WorkingState,
    state: &ExpandedStateBinding,
    protocol: Option<&StructuredCapabilityProtocolContract>,
) -> Result<WorkLeaf, StructuredStoreError> {
    let Some(attempt_id) = working
        .occurrence_attempts
        .get(&state.occurrence_id)
        .and_then(|attempts| attempts.last())
    else {
        return Ok(WorkLeaf::Ready);
    };
    let authorization = working.authorizations.get(attempt_id).ok_or_else(invalid)?;
    let Some(observation) = working.observations.get(attempt_id) else {
        return match authorization.intent.access_kind {
            AccessKind::Read => Ok(WorkLeaf::Reassertable {
                access_attempt_id: attempt_id.clone(),
                next_attempt_ordinal: authorization
                    .intent
                    .attempt_ordinal
                    .checked_add(1)
                    .ok_or_else(invalid)?,
            }),
            AccessKind::Effect => parked_effect_leaf(
                effect_entry_contract(protocol),
                attempt_id,
                authorization.intent.attempt_ordinal,
                false,
            ),
        };
    };
    match &observation.intent.outcome {
        ObservationOutcomeIntent::Returned(_) | ObservationOutcomeIntent::SafeFailure(_) => {
            Ok(WorkLeaf::ObservedForSettlement {
                access_attempt_id: attempt_id.clone(),
                observation: observation.handle.clone(),
            })
        }
        ObservationOutcomeIntent::SupersededBeforeEntry {
            public_lineage_head_ref,
            ..
        } => Ok(WorkLeaf::Refreshable {
            next_attempt_ordinal: authorization
                .intent
                .attempt_ordinal
                .checked_add(1)
                .ok_or_else(invalid)?,
            public_lineage_head_ref: public_lineage_head_ref.clone(),
        }),
        ObservationOutcomeIntent::EntryUnknown { .. } => parked_effect_leaf(
            effect_entry_contract(protocol),
            attempt_id,
            authorization.intent.attempt_ordinal,
            true,
        ),
        ObservationOutcomeIntent::IntegrityFault { .. } => {
            Ok(WorkLeaf::BlockedIntegrity(observation.handle.clone()))
        }
    }
}

fn effect_entry_contract(
    protocol: Option<&StructuredCapabilityProtocolContract>,
) -> Option<&StructuredEffectEntryContract> {
    match protocol {
        Some(StructuredCapabilityProtocolContract::Effect { entry_contract, .. }) => {
            Some(entry_contract)
        }
        _ => None,
    }
}

fn parked_effect_leaf(
    entry: Option<&StructuredEffectEntryContract>,
    attempt_id: &AccessAttemptId,
    attempt_ordinal: u64,
    observed_entry_unknown: bool,
) -> Result<WorkLeaf, StructuredStoreError> {
    let budget_remaining = match entry {
        Some(StructuredEffectEntryContract::EntryAbsorbing { max_entries, .. }) => {
            attempt_ordinal.saturating_add(1) < u64::from(max_entries.get())
        }
        Some(StructuredEffectEntryContract::EntryOnce {}) | None => false,
    };
    if !budget_remaining {
        return Ok(if observed_entry_unknown {
            WorkLeaf::EntryUnknown(attempt_id.clone())
        } else {
            WorkLeaf::Authorized(attempt_id.clone())
        });
    }
    Ok(if observed_entry_unknown {
        WorkLeaf::Reassertable {
            access_attempt_id: attempt_id.clone(),
            next_attempt_ordinal: attempt_ordinal.checked_add(1).ok_or_else(invalid)?,
        }
    } else {
        WorkLeaf::EntryClosable(attempt_id.clone())
    })
}

fn semantic_digest(
    certified_program_ref: &ContentRef,
    transitions: &BTreeMap<OccurrenceId, TransitionEntry>,
    bindings: &BTreeMap<ContentRef, LexicalValueRef>,
) -> Result<RunSemanticStateDigest, StructuredStoreError> {
    derive_semantic_state_digest(&SemanticStatePreimage {
        certified_program_ref,
        transitions: transitions
            .values()
            .map(|transition| SemanticTransitionPreimage {
                occurrence_id: &transition.intent.occurrence_id,
                outcome_ref: &transition.intent.outcome_ref,
                facts: transition
                    .intent
                    .facts
                    .iter()
                    .map(|fact| mfm_journal::structured::SemanticFactPreimage {
                        emission_ordinal: fact.emission_ordinal,
                        fact_slot_ordinal: fact.fact_slot_ordinal,
                        descriptor_ref: &fact.descriptor_ref,
                        subject: &fact.subject,
                        response: &fact.response,
                        claim_ref: &fact.claim_ref,
                    })
                    .collect(),
            })
            .collect(),
        live_bindings: bindings.values().collect(),
    })
    .map_err(|_| invalid())
}

fn bind_handle(
    handle: &RecordHandle,
    primary_assignment: &RecordRef,
) -> Result<RecordRef, StructuredStoreError> {
    match handle {
        RecordHandle::Existing(reference) => Ok(reference.clone()),
        RecordHandle::PendingPrimary => Ok(primary_assignment.clone()),
    }
}

fn bind_working_state(
    working: &mut WorkingState,
    primary_assignment: &RecordRef,
) -> Result<(), StructuredStoreError> {
    if let Some(handle) = &mut working.admission_handle {
        *handle = RecordHandle::Existing(bind_handle(handle, primary_assignment)?);
    }
    for transition in working.transitions.values_mut() {
        transition.handle =
            RecordHandle::Existing(bind_handle(&transition.handle, primary_assignment)?);
    }
    for authorization in working.authorizations.values_mut() {
        authorization.handle =
            RecordHandle::Existing(bind_handle(&authorization.handle, primary_assignment)?);
    }
    for observation in working.observations.values_mut() {
        observation.handle =
            RecordHandle::Existing(bind_handle(&observation.handle, primary_assignment)?);
    }
    if let Some(head) = &mut working.semantic_head {
        match head {
            WorkingSemanticHead::Genesis { admission, .. } => {
                *admission = RecordHandle::Existing(bind_handle(admission, primary_assignment)?);
            }
            WorkingSemanticHead::Transition { transition, .. } => {
                *transition = RecordHandle::Existing(bind_handle(transition, primary_assignment)?);
            }
        }
    }
    Ok(())
}

fn bind_semantic_head(
    head: &WorkingSemanticHead,
    primary_assignment: &RecordRef,
) -> Result<SemanticHead, StructuredStoreError> {
    match head {
        WorkingSemanticHead::Genesis { admission, digest } => Ok(SemanticHead::Genesis {
            admission_ref: bind_handle(admission, primary_assignment)?,
            semantic_state_digest: digest.clone(),
        }),
        WorkingSemanticHead::Transition { transition, digest } => Ok(SemanticHead::Transition {
            transition_ref: bind_handle(transition, primary_assignment)?,
            semantic_state_digest: digest.clone(),
        }),
    }
}

fn bind_leaf(
    leaf: WorkLeaf,
    primary_assignment: &RecordRef,
) -> Result<StateLeaf, StructuredStoreError> {
    Ok(match leaf {
        WorkLeaf::Ready => StateLeaf::Ready,
        WorkLeaf::Authorized(access_attempt_id) => StateLeaf::Authorized { access_attempt_id },
        WorkLeaf::ObservedForSettlement {
            access_attempt_id,
            observation,
        } => StateLeaf::ObservedForSettlement {
            access_attempt_id,
            observation_ref: bind_handle(&observation, primary_assignment)?,
        },
        WorkLeaf::Refreshable {
            next_attempt_ordinal,
            public_lineage_head_ref,
        } => StateLeaf::Refreshable {
            next_attempt_ordinal,
            public_lineage_head_ref,
        },
        WorkLeaf::EntryUnknown(access_attempt_id) => StateLeaf::EntryUnknown { access_attempt_id },
        WorkLeaf::BlockedIntegrity(observation) => StateLeaf::BlockedIntegrity {
            observation_ref: bind_handle(&observation, primary_assignment)?,
        },
        WorkLeaf::EntryClosable(access_attempt_id) => {
            StateLeaf::EntryClosable { access_attempt_id }
        }
        WorkLeaf::Reassertable {
            access_attempt_id,
            next_attempt_ordinal,
        } => StateLeaf::Reassertable {
            access_attempt_id,
            next_attempt_ordinal,
        },
    })
}

fn bind_action(
    action: WorkAction,
    primary_assignment: &RecordRef,
) -> Result<ActionableState, StructuredStoreError> {
    Ok(ActionableState {
        occurrence_id: action.occurrence_id,
        occurrence_path: action.occurrence_path,
        semantic_call_id: action.semantic_call_id,
        state_contract_ref: action.state_contract_ref,
        input: action.input,
        capability_contract_ref: action.capability_contract_ref,
        stable_resource_lineage_contract_ref: action.stable_resource_lineage_contract_ref,
        execution_kind: action.execution_kind,
        leaf: bind_leaf(action.leaf, primary_assignment)?,
    })
}

fn bind_lane(
    lane: WorkCursor,
    primary_assignment: &RecordRef,
) -> Result<LaneCursor, StructuredStoreError> {
    Ok(match lane {
        WorkCursor::AtState(action) => {
            LaneCursor::AtState(bind_action(action, primary_assignment)?)
        }
        WorkCursor::InFanOut { group_path, lanes } => LaneCursor::InFanOut {
            group_path,
            lanes: lanes
                .into_iter()
                .map(|lane| bind_lane(lane, primary_assignment))
                .collect::<Result<Vec<_>, _>>()?,
        },
        WorkCursor::Completed { outcome_ref } => LaneCursor::Completed { outcome_ref },
    })
}

fn bind_cursor(
    cursor: WorkCursor,
    primary_assignment: &RecordRef,
) -> Result<ProgramCursor, StructuredStoreError> {
    Ok(match cursor {
        WorkCursor::AtState(action) => {
            ProgramCursor::AtState(bind_action(action, primary_assignment)?)
        }
        WorkCursor::InFanOut { group_path, lanes } => ProgramCursor::InFanOut {
            group_path,
            lanes: lanes
                .into_iter()
                .map(|lane| bind_lane(lane, primary_assignment))
                .collect::<Result<Vec<_>, _>>()?,
        },
        WorkCursor::Completed { outcome_ref } => ProgramCursor::Closed { outcome_ref },
    })
}

fn frontier(cursor: &ProgramCursor) -> Result<StructuredFrontier, StructuredStoreError> {
    match cursor {
        ProgramCursor::Closed { .. } => Ok(StructuredFrontier::Complete),
        ProgramCursor::AtState(action) => frontier_actions(std::slice::from_ref(action)),
        ProgramCursor::InFanOut { lanes, .. } => {
            let mut actions = Vec::new();
            collect_lane_actions(lanes, &mut actions)?;
            frontier_actions(&actions)
        }
    }
}

fn collect_lane_actions<'a>(
    lanes: &'a [LaneCursor],
    actions: &mut Vec<&'a ActionableState>,
) -> Result<(), StructuredStoreError> {
    for lane in lanes {
        match lane {
            LaneCursor::AtState(action) => actions.push(action),
            LaneCursor::InFanOut { lanes, .. } => collect_lane_actions(lanes, actions)?,
            LaneCursor::Completed { .. } => {}
        }
    }
    Ok(())
}

fn frontier_actions(
    actions: &[impl std::borrow::Borrow<ActionableState>],
) -> Result<StructuredFrontier, StructuredStoreError> {
    let mut actionable = Vec::new();
    for action in actions {
        let action = action.borrow();
        match (&action.execution_kind, &action.leaf) {
            (_, StateLeaf::BlockedIntegrity { .. }) => {
                return Ok(StructuredFrontier::BlockedIntegrity);
            }
            (
                StructuredExecutionKind::Effect,
                StateLeaf::Authorized { access_attempt_id }
                | StateLeaf::EntryUnknown { access_attempt_id },
            ) => {
                return Ok(StructuredFrontier::PossibleEntry(Box::new(effect_subject(
                    action,
                    access_attempt_id,
                )?)));
            }
            _ => actionable.push(action.clone()),
        }
    }
    Ok(StructuredFrontier::Actions(actionable))
}

fn effect_subject(
    action: &ActionableState,
    access_attempt_id: &AccessAttemptId,
) -> Result<EffectEntrySubject, StructuredStoreError> {
    Ok(EffectEntrySubject {
        occurrence_id: action.occurrence_id.clone(),
        occurrence_path_ref: action
            .occurrence_path
            .content_ref()
            .map_err(|_| invalid())?,
        access_attempt_id: access_attempt_id.clone(),
        capability_contract_ref: action.capability_contract_ref.clone().ok_or_else(invalid)?,
    })
}

fn attention(cursor: &WorkCursor) -> Result<Option<EffectEntryAttention>, StructuredStoreError> {
    let mut actions = Vec::new();
    collect_work_actions(cursor, &mut actions);
    for action in actions {
        if action.execution_kind != StructuredExecutionKind::Effect {
            continue;
        }
        let (attempt, resolution) = match &action.leaf {
            WorkLeaf::Authorized(attempt) | WorkLeaf::EntryUnknown(attempt) => {
                (attempt, EffectEntryAttentionResolution::Manual)
            }
            WorkLeaf::EntryClosable(attempt) => {
                (attempt, EffectEntryAttentionResolution::CloseThenReassert)
            }
            WorkLeaf::Reassertable {
                access_attempt_id, ..
            } => (access_attempt_id, EffectEntryAttentionResolution::Reassert),
            _ => continue,
        };
        return Ok(Some(EffectEntryAttention::new(
            EffectEntrySubject {
                occurrence_id: action.occurrence_id.clone(),
                occurrence_path_ref: action
                    .occurrence_path
                    .content_ref()
                    .map_err(|_| invalid())?,
                access_attempt_id: attempt.clone(),
                capability_contract_ref: action
                    .capability_contract_ref
                    .clone()
                    .ok_or_else(invalid)?,
            },
            resolution,
        )));
    }
    Ok(None)
}

fn collect_work_actions<'a>(cursor: &'a WorkCursor, actions: &mut Vec<&'a WorkAction>) {
    match cursor {
        WorkCursor::AtState(action) => actions.push(action),
        WorkCursor::InFanOut { lanes, .. } => {
            for lane in lanes {
                collect_work_actions(lane, actions);
            }
        }
        WorkCursor::Completed { .. } => {}
    }
}
