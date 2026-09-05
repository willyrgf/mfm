//! Atomic nonce reservation and exact signed-byte custody contracts.
//! Concrete persistence and all signer/provider IO remain in downstream adapters.

use std::future::Future;
use std::pin::Pin;

use crate::{EvmAuthorityEpoch, EvmHash};
pub use crate::{NonceDomain, Reservation};
use mfm_ids::{ContentRef, EffectId};

/// Maximum exact signed transaction bytes retained by authority.
pub const MAX_EXACT_RAW_TRANSACTION_BYTES: usize = 132_096;

/// Redaction-safe transaction-authority failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthorityError {
    /// The operation could not safely make progress or its commit acknowledgement was ambiguous.
    #[error("EVM transaction authority is unavailable")]
    Unavailable,
    /// Caller identity or retained authority facts violated the current contract.
    #[error("EVM transaction authority is internally inconsistent")]
    Internal,
}

/// Result returned by transaction-authority operations.
pub type Result<T> = std::result::Result<T, AuthorityError>;

/// Boxed object-safe asynchronous authority operation.
pub type AuthorityFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Bounded exact signed transaction bytes.
///
/// This custody wrapper intentionally implements neither text/debug rendering nor serde.
#[derive(Clone, PartialEq, Eq)]
pub struct ExactRawTransaction {
    bytes: Vec<u8>,
}

impl ExactRawTransaction {
    /// Checks and owns one nonempty bounded signed transaction.
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_EXACT_RAW_TRANSACTION_BYTES {
            return Err(AuthorityError::Internal);
        }
        Ok(Self { bytes })
    }

    /// Borrows the exact retained bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Exact prepared wire retained for one reservation.
#[derive(Clone, PartialEq, Eq)]
pub struct PreparedRecord {
    transaction_hash: EvmHash,
    raw_transaction: ExactRawTransaction,
}
impl PreparedRecord {
    /// Combines the public hash with bounded opaque wire; the live adapter qualifies their relationship.
    pub const fn new(transaction_hash: EvmHash, raw_transaction: ExactRawTransaction) -> Self {
        Self {
            transaction_hash,
            raw_transaction,
        }
    }
    /// Returns the retained transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.transaction_hash
    }
    /// Borrows the exact signed bytes.
    pub const fn raw_transaction(&self) -> &ExactRawTransaction {
        &self.raw_transaction
    }
}
/// One complete snapshot of a reservation and its optional prepared bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct LoadedTransaction {
    /// Immutable public reservation.
    pub reservation: Reservation,
    /// Optional exact prepared wire.
    pub prepared: Option<PreparedRecord>,
}
/// Object-safe append-only custody for EVM transaction stages.
pub trait EvmTransactionAuthority: Send + Sync {
    /// Returns the immutable epoch captured during admission.
    fn authority_epoch(&self) -> &EvmAuthorityEpoch;
    /// Loads the complete reservation and optional prepared bytes in one snapshot.
    fn load<'a>(
        &'a self,
        reservation_effect_id: &'a EffectId,
    ) -> AuthorityFuture<'a, Option<LoadedTransaction>>;
    /// Atomically reserves max(observed pending, highest retained nonce + 1), or returns the exact existing reservation.
    fn reserve_or_compare<'a>(
        &'a self,
        reservation_effect_id: &'a EffectId,
        command_value_ref: &'a ContentRef,
        domain: &'a NonceDomain,
        observed_pending_nonce: u64,
    ) -> AuthorityFuture<'a, Reservation>;
    /// Retains or returns the immutable first winner; the caller qualifies its wire before use.
    fn retain_prepared<'a>(
        &'a self,
        reservation: &'a Reservation,
        candidate: &'a PreparedRecord,
    ) -> AuthorityFuture<'a, PreparedRecord>;
}
