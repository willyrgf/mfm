#![warn(missing_docs)]
//! Append-only authority contracts for EVM transaction Effects.
//!
//! These records are deliberately outside Program values and serde. They retain nonce, prepared
//! wire, and settlement facts without acquiring provider, signer, or workflow semantics.

use std::future::Future;
use std::pin::Pin;

use mfm_evm::{EvmAddress, EvmAuthorityEpoch, EvmChainInstance, EvmHash, EvmTransactionSettlement};
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

/// Exact nonce domain, excluding endpoint and custody-provider dimensions.
#[derive(Clone, PartialEq, Eq)]
pub struct NonceDomain {
    authority_epoch: EvmAuthorityEpoch,
    chain_instance: EvmChainInstance,
    sender: EvmAddress,
}

impl NonceDomain {
    /// Constructs a nonce domain from checked public identities.
    pub fn new(
        authority_epoch: EvmAuthorityEpoch,
        chain_instance: EvmChainInstance,
        sender: EvmAddress,
    ) -> Self {
        Self {
            authority_epoch,
            chain_instance,
            sender,
        }
    }

    /// Returns the authority epoch that separates fresh installations.
    pub const fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.authority_epoch
    }

    /// Returns the endpoint-independent chain instance.
    pub const fn chain_instance(&self) -> &EvmChainInstance {
        &self.chain_instance
    }

    /// Returns the exact sender address.
    pub const fn sender(&self) -> &EvmAddress {
        &self.sender
    }
}

/// Immutable nonce reservation for one Effect and command.
#[derive(Clone, PartialEq, Eq)]
pub struct Reservation {
    effect_id: EffectId,
    command_ref: ContentRef,
    domain: NonceDomain,
    nonce: u64,
}

impl Reservation {
    /// Constructs one checked reservation from checked public identities.
    pub fn new(
        effect_id: EffectId,
        command_ref: ContentRef,
        domain: NonceDomain,
        nonce: u64,
    ) -> Self {
        Self {
            effect_id,
            command_ref,
            domain,
            nonce,
        }
    }

    /// Returns the reserved Effect identity.
    pub const fn effect_id(&self) -> &EffectId {
        &self.effect_id
    }

    /// Returns the exact prepared-command reference.
    pub const fn command_ref(&self) -> &ContentRef {
        &self.command_ref
    }

    /// Returns the complete nonce domain.
    pub const fn domain(&self) -> &NonceDomain {
        &self.domain
    }

    /// Returns the reserved nonce.
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }
}

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

/// Immutable prepared transaction fact nested over its reservation.
#[derive(Clone, PartialEq, Eq)]
pub struct PreparedRecord {
    reservation: Reservation,
    transaction_hash: EvmHash,
    raw_transaction: ExactRawTransaction,
}

impl PreparedRecord {
    /// Constructs one prepared transaction record.
    pub fn new(
        reservation: Reservation,
        transaction_hash: EvmHash,
        raw_transaction: ExactRawTransaction,
    ) -> Self {
        Self {
            reservation,
            transaction_hash,
            raw_transaction,
        }
    }

    /// Returns the exact predecessor reservation.
    pub const fn reservation(&self) -> &Reservation {
        &self.reservation
    }

    /// Returns the exact signed transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.transaction_hash
    }

    /// Returns the exact signed transaction custody value.
    pub const fn raw_transaction(&self) -> &ExactRawTransaction {
        &self.raw_transaction
    }
}

/// Immutable terminal settlement fact nested over its prepared transaction.
#[derive(Clone, PartialEq, Eq)]
pub struct SettledRecord {
    prepared: PreparedRecord,
    evidence: EvmTransactionSettlement,
}

impl SettledRecord {
    /// Constructs a settlement only when its exact predecessor facts agree.
    pub fn new(prepared: PreparedRecord, evidence: EvmTransactionSettlement) -> Result<Self> {
        let reservation = prepared.reservation();
        if evidence.effect_id() != reservation.effect_id()
            || evidence.nonce() != reservation.nonce()
            || evidence.transaction_hash() != prepared.transaction_hash()
        {
            return Err(AuthorityError::Internal);
        }
        Ok(Self { prepared, evidence })
    }

    /// Returns the exact nested prepared transaction.
    pub const fn prepared(&self) -> &PreparedRecord {
        &self.prepared
    }

    /// Returns the exact typed terminal evidence.
    pub const fn evidence(&self) -> &EvmTransactionSettlement {
        &self.evidence
    }
}

/// Complete append-only state retained for one Effect.
#[derive(Clone, PartialEq, Eq)]
pub enum AuthorityState {
    /// A nonce has been reserved.
    Reserved(Reservation),
    /// Exact signed bytes have been prepared.
    Prepared(PreparedRecord),
    /// Terminal typed evidence has been settled.
    Settled(SettledRecord),
}

/// Object-safe append-only EVM transaction authority.
pub trait EvmTransactionAuthority: Send + Sync {
    /// Returns the immutable epoch captured during concrete authority construction.
    fn authority_epoch(&self) -> &EvmAuthorityEpoch;

    /// Loads and qualifies one Effect under its expected command identity.
    fn load<'a>(
        &'a self,
        effect_id: &'a EffectId,
        expected_command_ref: &'a ContentRef,
    ) -> AuthorityFuture<'a, Option<AuthorityState>>;

    /// Atomically creates or exactly compares one nonce reservation.
    fn reserve_or_compare<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        domain: &'a NonceDomain,
        observed_pending_nonce: u64,
    ) -> AuthorityFuture<'a, Reservation>;

    /// Appends or exactly compares one prepared transaction fact.
    fn retain_prepared<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        transaction_hash: &'a EvmHash,
        raw_transaction: &'a ExactRawTransaction,
    ) -> AuthorityFuture<'a, PreparedRecord>;

    /// Appends or exactly compares one terminal settlement fact.
    fn retain_settlement<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        evidence: &'a EvmTransactionSettlement,
    ) -> AuthorityFuture<'a, SettledRecord>;
}

#[cfg(test)]
mod tests;
