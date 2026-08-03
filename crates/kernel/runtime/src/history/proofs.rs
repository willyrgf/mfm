//! Sealed append and authorization proof types.

use mfm_ids::{AppendRequestId, ContentDigest};
use mfm_journal::structured::CommittedBatch;

pub use mfm_certify::structured::{NewlyAppendedAuthorization, PriorRunFactScanCompletion};

/// Result of one backend exact-head transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryAppendOutcome {
    /// This transaction atomically committed the batch.
    NewlyCommitted(CommittedBatch),
    /// The same append identity already committed byte-identical content.
    ExistingSame(CommittedBatch),
    /// The locked run head differs from the exact expected predecessor.
    StaleHead,
    /// Commit acknowledgement is unavailable and must be resolved unchanged.
    AcknowledgementUnknown,
}

/// Result of one exact physical append attempt, including ambiguity identity.
///
/// Production construction is limited to the store adapter.
pub struct StructuredAppendAttempt {
    append_request_id: AppendRequestId,
    candidate_digest: ContentDigest,
    candidate: CommittedBatch,
    outcome: HistoryAppendOutcome,
    authorization: Option<NewlyAppendedAuthorization>,
    closed: bool,
}

impl std::fmt::Debug for StructuredAppendAttempt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StructuredAppendAttempt")
            .field("append_request_id", &self.append_request_id)
            .field("outcome", &self.outcome)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl StructuredAppendAttempt {
    /// Store-adapter construction after one fold-validated append attempt.
    #[doc(hidden)]
    pub fn from_store_attempt(
        append_request_id: AppendRequestId,
        candidate_digest: ContentDigest,
        candidate: CommittedBatch,
        outcome: HistoryAppendOutcome,
        authorization: Option<NewlyAppendedAuthorization>,
        closed: bool,
    ) -> Self {
        Self {
            append_request_id,
            candidate_digest,
            candidate,
            outcome,
            authorization,
            closed,
        }
    }

    /// Returns the stable physical append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the exact candidate digest required for ambiguity resolution.
    pub const fn candidate_digest(&self) -> &ContentDigest {
        &self.candidate_digest
    }

    /// Returns the exact backend disposition.
    pub const fn outcome(&self) -> &HistoryAppendOutcome {
        &self.outcome
    }

    /// Returns whether a root-close record was part of a committed batch.
    pub const fn closed(&self) -> bool {
        self.closed
    }

    /// Returns a positive committed batch proof, when acknowledgement is known.
    pub const fn committed(&self) -> Option<&CommittedBatch> {
        match &self.outcome {
            HistoryAppendOutcome::NewlyCommitted(batch)
            | HistoryAppendOutcome::ExistingSame(batch) => Some(batch),
            HistoryAppendOutcome::StaleHead | HistoryAppendOutcome::AcknowledgementUnknown => None,
        }
    }

    /// Returns the exact candidate envelope (store resolve path).
    #[doc(hidden)]
    pub const fn candidate(&self) -> &CommittedBatch {
        &self.candidate
    }

    /// Consumes a directly acknowledged new authorization append into its
    /// one-use invocation permit.
    pub fn into_newly_appended_authorization(self) -> Option<NewlyAppendedAuthorization> {
        if !matches!(self.outcome, HistoryAppendOutcome::NewlyCommitted(_)) {
            return None;
        }
        self.authorization
    }

    /// Confirms an unresolved attempt resolved to existing identical content.
    #[doc(hidden)]
    pub fn confirm_existing_same(&mut self) {
        self.outcome = HistoryAppendOutcome::ExistingSame(self.candidate.clone());
        self.authorization = None;
    }
}

/// Store-owned resolution of one stable pending observation.
#[derive(Debug)]
pub enum ObservationCommit {
    /// The exact logical observation already exists with identical content.
    ExistingSame,
    /// One predecessor-bound physical append was attempted.
    Attempt(Box<StructuredAppendAttempt>),
}
