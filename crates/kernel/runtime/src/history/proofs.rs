//! Sealed append and authorization proof types.

use std::future::Future;
use std::pin::Pin;

use mfm_facts::FactSelectionRequest;
use mfm_ids::{AccessAttemptId, AppendRequestId};
use mfm_journal::structured::{CommittedBatch, ExternalAccessAuthorized, JournalHead, RecordRef};

pub use mfm_certify::structured::{CertifiedAccessAuthorization, PriorRunFactScanCompletion};

type FactScanFuture = Pin<Box<dyn Future<Output = PriorRunFactScanCompletion> + Send + 'static>>;

/// Runtime-owned affine proof of one exact committed access authorization.
/// Certification supplies only deterministic data; this value is minted by
/// the store and consumed by Runtime exactly once.
pub struct CommittedAccessAuthorization {
    inner: mfm_certify::structured::CertifiedAccessAuthorization,
    predecessor_head: Option<JournalHead>,
    successor_head: JournalHead,
}

impl std::fmt::Debug for CommittedAccessAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommittedAccessAuthorization")
            .field("authorization_ref", &self.authorization_ref())
            .field("access_attempt_id", &self.access_attempt_id())
            .finish_non_exhaustive()
    }
}

impl CommittedAccessAuthorization {
    /// Store-only mint after an exact newly committed authorization append.
    #[doc(hidden)]
    #[cfg(feature = "store-authority")]
    pub fn from_committed_successor(
        authorization_ref: RecordRef,
        authorization: ExternalAccessAuthorized,
        fact_scan: Option<Box<dyn FnOnce(FactSelectionRequest) -> FactScanFuture + Send>>,
        predecessor_head: Option<JournalHead>,
        successor_head: JournalHead,
    ) -> Self {
        Self {
            inner: mfm_certify::structured::CertifiedAccessAuthorization::from_committed_successor(
                authorization_ref,
                authorization,
                fact_scan,
            ),
            predecessor_head,
            successor_head,
        }
    }

    /// Exact assigned authorization record reference.
    pub const fn authorization_ref(&self) -> &RecordRef {
        self.inner.authorization_ref()
    }
    /// Kernel-derived access-attempt identity.
    pub const fn access_attempt_id(&self) -> &AccessAttemptId {
        self.inner.access_attempt_id()
    }

    /// Exact predecessor head used by the append that minted this proof.
    pub const fn predecessor_head(&self) -> Option<&JournalHead> {
        self.predecessor_head.as_ref()
    }

    /// Exact successor head committed by the append that minted this proof.
    pub const fn successor_head(&self) -> &JournalHead {
        &self.successor_head
    }
    /// Complete immutable committed authorization payload.
    pub const fn authorization(&self) -> &ExternalAccessAuthorized {
        self.inner.authorization()
    }
    /// Consumes this proof for the store-owned fact scanner.
    pub fn invoke_prior_run_fact_scan(
        self,
        request: FactSelectionRequest,
    ) -> Option<FactScanFuture> {
        self.inner.invoke_prior_run_fact_scan(request)
    }

    pub(crate) fn into_certified(self) -> mfm_certify::structured::CertifiedAccessAuthorization {
        self.inner
    }
}

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
    outcome: HistoryAppendOutcome,
    authorization: Option<CommittedAccessAuthorization>,
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
    /// Store-adapter construction after one reducer-validated append attempt.
    #[doc(hidden)]
    #[cfg(feature = "store-authority")]
    pub fn from_store_attempt(
        append_request_id: AppendRequestId,
        outcome: HistoryAppendOutcome,
        authorization: Option<CommittedAccessAuthorization>,
        closed: bool,
    ) -> Self {
        Self {
            append_request_id,
            outcome,
            authorization,
            closed,
        }
    }

    /// Returns the stable physical append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
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

    /// Consumes a directly acknowledged new authorization append into its
    /// one-use invocation permit.
    pub fn into_committed_access_authorization(self) -> Option<CommittedAccessAuthorization> {
        if !matches!(self.outcome, HistoryAppendOutcome::NewlyCommitted(_)) {
            return None;
        }
        self.authorization
    }
}
