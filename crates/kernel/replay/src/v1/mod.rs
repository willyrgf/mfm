use mfm_events::v1::{self as events, side_effect, ArtifactRole};
use mfm_ids::{
    ArtifactId, AttemptId, ContentDigest, NodeId, SchemaId, SeedId, SemanticTypeId,
    SideEffectPairId, StateKind, StateVersion,
};
use mfm_spec::v1::{self as spec, HashedSpecEnvelope};
use mfm_spec::SpecError;
use mfm_store::v1::{self as store, ArtifactEvidenceRef as StoredArtifactEvidenceRef};

#[path = "verification_helpers.rs"]
mod verification_helpers;
use self::verification_helpers::*;
#[path = "value_read.rs"]
mod value_read;
pub use self::value_read::{
    canonical_value_bytes, decode_produced_value, external_read_evidence,
    fact_query_evidence_for_attempt, load_node_config, load_node_context, load_node_input,
    produced_input_frames, single_state_output_frame, verify_external_read_state,
    verify_pure_state, verify_recorded_fact_batch_evidence, verify_recorded_fact_evidence,
};
#[path = "broker.rs"]
mod broker;
#[path = "evidence.rs"]
mod evidence;
pub use self::evidence::{
    AmbiguityReplayEvidence, ArtifactReplayEvidence, ArtifactReplayRequest,
    CertifiedSideEffectContext, ConfirmationReplayEvidence, NotSubmittedReplayEvidence,
    PreparedInvocationReplayEvidence, ProducedCellReplayFrame, ReceiptReplayEvidence,
    RetainedSourceFactReplayEvent, SideEffectConfirmationReplayInput,
    SideEffectEvidenceReplayRequest, SideEffectIntentReplayEvidence, SideEffectReceiptReplayInput,
    SideEffectReplayFrame, SideEffectReplayVerifier, SideEffectSubmissionReplayInput,
    SubmissionReplayEvidence, SubmissionUnknownReplayEvidence,
};

/// Result type for replay broker operations.
pub type Result<T> = std::result::Result<T, ReplayError>;

/// Typed replay validation error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", .kind.code())]
pub struct ReplayError {
    /// Stable replay error category.
    pub kind: ReplayErrorKind,
    /// Redaction-safe diagnostic.
    pub message: String,
}

impl ReplayError {
    /// Creates a replay error with a stable category.
    pub fn new(kind: ReplayErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Returns the stable machine-readable replay error code.
    pub const fn code(&self) -> &'static str {
        self.kind.code()
    }
}

impl From<SpecError> for ReplayError {
    fn from(error: SpecError) -> Self {
        Self::new(
            ReplayErrorKind::CertifiedEvidenceMismatch,
            error.to_string(),
        )
    }
}

impl From<store::StoreError> for ReplayError {
    fn from(error: store::StoreError) -> Self {
        Self::new(ReplayErrorKind::InvalidRunJournal, error.to_string())
    }
}

/// Stable typed replay error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReplayErrorKind {
    /// The committed journal cannot support a replay query.
    InvalidRunJournal,
    /// Replay attempted to request live capability access.
    LiveCapabilityRequest,
    /// A requested recorded fact was absent.
    FactMissing,
    /// A recorded fact did not match the replay request.
    FactMismatch,
    /// Required retained artifact evidence was absent.
    ArtifactMissing,
    /// Retained artifact evidence disagreed with typed event evidence.
    ArtifactMismatch,
    /// Required side-effect evidence was absent.
    SideEffectMissing,
    /// Recorded side-effect evidence did not match the replay request.
    SideEffectMismatch,
    /// A committed record references evidence outside the certified spec.
    CertifiedEvidenceMismatch,
    /// A side-effect replay verifier identity did not match.
    ReplayVerifierMismatch,
}

impl ReplayErrorKind {
    /// Returns the stable replay error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRunJournal => "MFM_REPLAY_JOURNAL_INVALID",
            Self::LiveCapabilityRequest => "MFM_REPLAY_LIVE_CAPABILITY_REQUEST",
            Self::FactMissing => "MFM_REPLAY_FACT_MISSING",
            Self::FactMismatch => "MFM_REPLAY_FACT_MISMATCH",
            Self::ArtifactMissing => "MFM_REPLAY_ARTIFACT_MISSING",
            Self::ArtifactMismatch => "MFM_REPLAY_ARTIFACT_MISMATCH",
            Self::SideEffectMissing => "MFM_REPLAY_SIDE_EFFECT_MISSING",
            Self::SideEffectMismatch => "MFM_REPLAY_SIDE_EFFECT_MISMATCH",
            Self::CertifiedEvidenceMismatch => "MFM_REPLAY_CERTIFIED_EVIDENCE_MISMATCH",
            Self::ReplayVerifierMismatch => "MFM_REPLAY_VERIFIER_MISMATCH",
        }
    }
}

/// Borrowing replay authority over one store-verified committed run view.
///
/// The primary journal, certified spec, lifecycle fold, and retained objects remain owned by
/// [`store::VerifiedRunView`]. This envelope owns only explicit evidence that is not part of the
/// primary run: cross-run source facts and additional already-verified retained artifacts.
#[derive(Debug)]
pub struct ReplayReadAuthority<'view> {
    view: &'view store::VerifiedRunView,
    additional_artifacts: Vec<store::VerifiedRetainedArtifactBytes>,
    source_fact_events: Vec<RetainedSourceFactReplayEvent>,
}

impl<'view> ReplayReadAuthority<'view> {
    /// Borrows verified primary history and adds explicit cross-run replay evidence.
    pub fn from_verified_run_view_with_source_facts_and_artifacts(
        view: &'view store::VerifiedRunView,
        source_fact_events: Vec<RetainedSourceFactReplayEvent>,
        additional_artifacts: Vec<store::VerifiedRetainedArtifactBytes>,
    ) -> Result<Self> {
        Ok(Self {
            view,
            additional_artifacts,
            source_fact_events,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct ArtifactEvidenceExpectation<'a> {
    artifact_id: &'a ArtifactId,
    evidence_hash: &'a ContentDigest,
    digest: &'a ContentDigest,
    schema_id: Option<&'a SchemaId>,
    semantic_type_id: Option<&'a SemanticTypeId>,
    role: ArtifactRole,
    producer_node_id: Option<&'a NodeId>,
    producer_seed_id: Option<&'a SeedId>,
}

type ReplayArtifactKey = (ArtifactId, ContentDigest);

trait SideEffectReplayArtifact {
    fn artifact_id(&self) -> &ArtifactId;
    fn evidence_hash(&self) -> &ContentDigest;
    fn artifact_evidence_hash(&self) -> &ContentDigest;
    fn evidence_schema_id(&self) -> &SchemaId;
    fn artifact_role(&self) -> ArtifactRole;
    fn producer_node_id(&self) -> &NodeId;
    fn mismatch_message(&self) -> &'static str;
    fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
        None
    }
}

impl SideEffectReplayArtifact for side_effect::SubmissionObserved {
    fn artifact_id(&self) -> &ArtifactId {
        &self.submission_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.submission_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.submission_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.submission_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::Submission
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "submission evidence mismatch"
    }
}

impl SideEffectReplayArtifact for side_effect::SubmissionUnknown {
    fn artifact_id(&self) -> &ArtifactId {
        &self.evidence_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.evidence_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.evidence_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::SubmissionUnknownEvidence
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "submission-unknown evidence mismatch"
    }
}

impl SideEffectReplayArtifact for side_effect::NotSubmittedProven {
    fn artifact_id(&self) -> &ArtifactId {
        &self.proof_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.proof_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.proof_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.proof_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::NotSubmittedProof
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "not-submitted proof evidence mismatch"
    }
}

impl SideEffectReplayArtifact for side_effect::ReceiptObserved {
    fn artifact_id(&self) -> &ArtifactId {
        &self.receipt_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.receipt_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.receipt_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.receipt_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::Receipt
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "receipt evidence mismatch"
    }

    fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
        Some(&self.replay_verifier_id)
    }
}

impl SideEffectReplayArtifact for side_effect::ConfirmationObserved {
    fn artifact_id(&self) -> &ArtifactId {
        &self.confirmation_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.confirmation_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.confirmation_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.confirmation_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::Confirmation
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "confirmation evidence mismatch"
    }

    fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
        Some(&self.replay_verifier_id)
    }
}

impl SideEffectReplayArtifact for side_effect::Ambiguous {
    fn artifact_id(&self) -> &ArtifactId {
        &self.evidence_artifact_id
    }

    fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
    }

    fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.evidence_artifact_evidence_hash
    }

    fn evidence_schema_id(&self) -> &SchemaId {
        &self.evidence_schema_id
    }

    fn artifact_role(&self) -> ArtifactRole {
        ArtifactRole::AmbiguityEvidence
    }

    fn producer_node_id(&self) -> &NodeId {
        &self.node_id
    }

    fn mismatch_message(&self) -> &'static str {
        "ambiguity evidence mismatch"
    }
}

/// Evidence-only broker borrowing one store-owned verified run view.
///
/// The broker deliberately carries no copied journal, rebuilt projection, or lifecycle index.
/// Point queries borrow the store-owned current lifecycle fold; exact-order evidence checks use
/// its controlled tagged-record visitor.
#[derive(Debug)]
pub struct ReplayBroker<'view> {
    view: &'view store::VerifiedRunView,
    additional_artifacts: Vec<store::VerifiedRetainedArtifactBytes>,
    source_fact_events: Vec<RetainedSourceFactReplayEvent>,
}

#[cfg(test)]
mod tests;
