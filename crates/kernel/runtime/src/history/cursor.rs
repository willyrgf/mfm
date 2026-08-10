//! Fold-derived cursor and frontier types owned by the Runtime history surface.

use mfm_ids::{AccessAttemptId, ContentRef, OccurrenceId};
use mfm_journal::structured::{LexicalValueRef, RecordRef};
use mfm_spec::structured::{StructuralPath, StructuredExecutionKind};

/// Folded state of one current executable occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateLeaf {
    /// No callback or access has yet committed for this occurrence.
    Ready,
    /// One exact Effect access authorization is outstanding.
    Authorized {
        /// Immutable attempt identity.
        access_attempt_id: AccessAttemptId,
    },
    /// One normal observation is committed and awaits settlement.
    ObservedForSettlement {
        /// Immutable attempt identity.
        access_attempt_id: AccessAttemptId,
        /// Exact committed observation.
        observation_ref: RecordRef,
    },
    /// A refreshable Effect may authorize exactly this next ordinal.
    Refreshable {
        /// Next attempt ordinal.
        next_attempt_ordinal: u64,
        /// Monotonic lower-bound lineage head.
        public_lineage_head_ref: ContentRef,
    },
    /// Effect entry may have happened and no successor is legal.
    EntryUnknown {
        /// Immutable ambiguous attempt identity.
        access_attempt_id: AccessAttemptId,
    },
    /// Committed integrity evidence blocks semantic progress.
    BlockedIntegrity {
        /// Exact integrity observation.
        observation_ref: RecordRef,
    },
    /// A crashed Effect attempt on an absorbing capability with budget
    /// remaining. Runtime must commit its closing observation before any
    /// re-assertion. Authorizes nothing.
    EntryClosable {
        /// Immutable identity of the attempt whose invoker authority is lost.
        access_attempt_id: AccessAttemptId,
    },
    /// The parked attempt is resolved and a repeat is safe. The next ordinal
    /// re-asserts the byte-identical committed request.
    ///
    /// One leaf serves both access kinds: it carries only an ordinal, and the
    /// access kind is in scope wherever it is read. What differs between the
    /// kinds is which rule admits the successor, and that lives in one place.
    Reassertable {
        /// Next attempt ordinal.
        next_attempt_ordinal: u64,
    },
}

/// One current or completed fan-out lane cursor.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneCursor {
    /// One exact executable state inside this lane.
    AtState(ActionableState),
    /// One nested depth-two fan-out.
    InFanOut {
        /// Exact nested group path.
        group_path: StructuralPath,
        /// Lane cursors in declaration order.
        lanes: Vec<LaneCursor>,
    },
    /// Exact completed nominal lane outcome.
    Completed {
        /// Content-addressed lane outcome value.
        outcome_ref: ContentRef,
    },
}

/// Exact current executable state and its access leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionableState {
    /// Exact normalized occurrence identity.
    pub occurrence_id: OccurrenceId,
    /// Exact normalized occurrence path.
    pub occurrence_path: StructuralPath,
    /// Stable authored or injected semantic call identity.
    pub semantic_call_id: mfm_ids::SemanticCallId,
    /// Exact semantic state contract selected by certification.
    pub state_contract_ref: ContentRef,
    /// Exact current producer-bound input passed to the state callback.
    pub input: LexicalValueRef,
    /// Exact semantic capability contract for Read or Effect.
    pub capability_contract_ref: Option<ContentRef>,
    /// Exact admitted stable Resource lineage for a refreshable Effect.
    pub stable_resource_lineage_contract_ref: Option<ContentRef>,
    /// Certified Pure, Read, or Effect execution kind.
    pub execution_kind: StructuredExecutionKind,
    /// Folded local/access state.
    pub leaf: StateLeaf,
}

/// Sole recursive cursor for an open or closed structured program.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramCursor {
    /// One current sequential state.
    AtState(ActionableState),
    /// One active collect-all fan-out.
    InFanOut {
        /// Exact group path.
        group_path: StructuralPath,
        /// Lane cursors in declaration order.
        lanes: Vec<LaneCursor>,
    },
    /// One exact terminal operation outcome.
    Closed {
        /// Content-addressed nominal operation outcome.
        outcome_ref: ContentRef,
    },
}

/// Exact identity of the occurrence whose unresolved Effect entry blocks a run.
///
/// Every field is already exposed by the access-audit projection, so carrying
/// the subject past the barrier is redaction-safe by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntrySubject {
    /// Exact normalized occurrence identity.
    pub occurrence_id: OccurrenceId,
    /// Canonical normalized occurrence path reference.
    pub occurrence_path_ref: ContentRef,
    /// Immutable identity of the attempt whose entry is unresolved.
    pub access_attempt_id: AccessAttemptId,
    /// Exact semantic Effect capability contract.
    pub capability_contract_ref: ContentRef,
}

/// Closed action frontier derived only from the verified cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredFrontier {
    /// Declaration-ordered actionable state paths.
    Actions(Vec<ActionableState>),
    /// Possible Effect entry of one exact occurrence blocks all later work.
    ///
    /// The subject is boxed: it is four identities wide and this frontier is
    /// returned through the recursive fan-out walk.
    PossibleEntry(Box<EffectEntrySubject>),
    /// Committed integrity evidence blocks all later work.
    BlockedIntegrity,
    /// Root outcome is closed.
    Complete,
}

/// Store-owned preflight of one invoked completion before Runtime freezes its
/// pending observation bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationQualification {
    /// The proposed completion is valid and has not already committed.
    Ready,
    /// The exact logical observation already committed with identical bytes.
    ExistingSame,
    /// Proposed Effect supersession evidence failed its purpose-limited
    /// physical-lineage verification.
    InvalidSupersessionEvidence,
}
