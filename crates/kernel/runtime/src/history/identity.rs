//! Immutable store lineage identity visible to Runtime.

use mfm_ids::{ContentDigest, StoreEpoch, StoreScopeId};
use mfm_program_derive::PersistedSchema;
use serde::{Deserialize, Serialize};

/// Exact physical target state qualified for one store lineage.
///
/// This value is retained only when deployment has supplied a target-bound
/// store identity. Offline verification may validate lineage without knowing the
/// physical target and therefore carry `None` in [`StructuredStoreIdentity`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
pub struct PhysicalTargetIdentity {
    /// Stable deployment target key.
    pub target_key: String,
    /// PostgreSQL database OID (or the equivalent physical database identity).
    pub database_oid: u32,
    /// Non-rollback fence generation.
    pub fence_generation: u64,
    /// Qualified certificate-release epoch.
    pub release_epoch: u64,
    /// Content digest of the exact current target incarnation.
    pub current_incarnation_ref: ContentDigest,
}

/// Immutable qualified identity of one structured-history writer lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredStoreIdentity {
    /// Qualified store lineage.
    pub store_scope_id: StoreScopeId,
    /// Monotonic authoritative writer epoch.
    pub store_epoch: StoreEpoch,
    /// Exact physical target fixation, when this identity came from a
    /// deployment-qualified backend.
    pub physical_target: Option<PhysicalTargetIdentity>,
}
