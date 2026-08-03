//! Immutable store lineage identity visible to Runtime.

use mfm_ids::{StoreEpoch, StoreScopeId};

/// Immutable qualified identity of one structured-history writer lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredStoreIdentity {
    /// Qualified store lineage.
    pub store_scope_id: StoreScopeId,
    /// Monotonic authoritative writer epoch.
    pub store_epoch: StoreEpoch,
}
