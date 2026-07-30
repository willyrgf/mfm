use std::future::Future;
use std::pin::Pin;

use mfm_ids::RunId;
use mfm_journal::JournalHead;

use super::{
    journal::AccessAuditProjection, AppendOutcome, CommittedRunJournal, JournalAppendVerifier,
    JournalLoadVerifier, StoreAuthorityContext, StoreError, StoreErrorInspection,
    VerifiedAccessAuditEntry,
};

/// Boxed asynchronous store operation.
pub type AsyncStoreFuture<'a, T, E> =
    Pin<Box<dyn Future<Output = std::result::Result<T, E>> + Send + 'a>>;

/// One store-verified, head-fixed access-audit page.
pub struct VerifiedAccessAuditPage {
    run_id: RunId,
    complete_as_of_journal_head: JournalHead,
    entries: Vec<AccessAuditProjection>,
    has_more: bool,
    next_index: Option<u32>,
}

impl VerifiedAccessAuditPage {
    pub(super) fn new(
        run_id: RunId,
        complete_as_of_journal_head: JournalHead,
        entries: Vec<AccessAuditProjection>,
        has_more: bool,
        next_index: Option<u32>,
        start: u32,
    ) -> Result<Self, StoreError> {
        let expected_next = if has_more {
            let length =
                u32::try_from(entries.len()).map_err(|_| StoreError::InvalidAuditPage {
                    field: "next_index",
                })?;
            Some(
                start
                    .checked_add(length)
                    .ok_or(StoreError::InvalidAuditPage {
                        field: "next_index",
                    })?,
            )
        } else {
            None
        };
        if next_index != expected_next {
            return Err(StoreError::InvalidAuditPage {
                field: "next_index",
            });
        }
        Ok(Self {
            run_id,
            complete_as_of_journal_head,
            entries,
            has_more,
            next_index,
        })
    }

    /// Returns the inspected run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact physical head fixed by the first page.
    pub const fn complete_as_of_journal_head(&self) -> &JournalHead {
        &self.complete_as_of_journal_head
    }

    /// Returns safe entries in authorization order.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = VerifiedAccessAuditEntry<'_>> + '_ {
        self.entries.iter().map(VerifiedAccessAuditEntry::new)
    }

    /// Returns whether another entry exists after this page.
    pub const fn has_more(&self) -> bool {
        self.has_more
    }

    /// Returns the next zero-based authorization index when another page exists.
    pub const fn next_index(&self) -> Option<u32> {
        self.next_index
    }
}

/// Trusted durable-backend seam behind the sealed journal contract.
///
/// Implementations receive only store-created verifiers. They must perform compare-and-swap,
/// idempotency, object admission, tenant-coordinate assignment, and immutable-row publication in
/// one transaction. There is no raw append method.
pub trait RunJournalBackend: Send + Sync {
    /// Backend-specific error preserving typed store-error inspection.
    type Error: std::error::Error + StoreErrorInspection + From<StoreError> + Send + Sync + 'static;

    /// Returns the private context paired with this exact store's authority issuer.
    fn store_authority_context(&self) -> &StoreAuthorityContext;

    /// Atomically validates, assigns, and publishes one store-created append verifier.
    fn backend_append<'a>(
        &'a self,
        verifier: JournalAppendVerifier,
    ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error>;

    /// Loads immutable rows and completes one store-created exact-run verifier.
    fn backend_load<'a>(
        &'a self,
        verifier: JournalLoadVerifier,
    ) -> AsyncStoreFuture<'a, CommittedRunJournal, Self::Error>;
}
