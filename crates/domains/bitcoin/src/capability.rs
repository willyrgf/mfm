//! Aggregate Bitcoin balance-read capability contract.

use std::future::Future;
use std::pin::Pin;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{CapabilityError, CapabilitySpec, ReadExternalRole};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm};

use crate::model::{
    BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionResponse, BitcoinCapabilityError,
    BitcoinSourceBinding,
};

/// Stable implementation identity of the strict Bitcoin Core JSON-RPC session.
pub const BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID: &str =
    "mfm.bitcoin.jsonrpc.balance_collection.v1";

/// Boxed future returned by a Bitcoin balance session.
pub type BitcoinSessionFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, BitcoinCapabilityError>> + Send + 'a>>;

/// Aggregate Bitcoin balance-read authority.
pub struct BitcoinBalanceCollectionReadCapability;

impl CapabilitySpec for BitcoinBalanceCollectionReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.bitcoin",
            "balance_collection.read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.bitcoin.capability:balance_collection.read"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.bitcoin.balance_collection.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.balance_collection.read"
    }
}

/// Route-aware aggregate Bitcoin balance session.
pub trait BitcoinBalanceSession: Send + Sync {
    /// Returns the stable implementation identity installed in the runtime registry.
    fn implementation_id(&self) -> &'static str;

    /// Validates that the semantic binding can be served without live network IO.
    fn validate_binding<'a>(
        &'a self,
        binding: &'a BitcoinSourceBinding,
    ) -> BitcoinSessionFuture<'a, ()>;

    /// Executes one complete aggregate balance read for the checked request.
    fn collect_balances<'a>(
        &'a self,
        request: &'a BitcoinBalanceCollectionRequest,
    ) -> BitcoinSessionFuture<'a, BitcoinBalanceCollectionResponse>;
}
