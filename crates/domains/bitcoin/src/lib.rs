#![warn(missing_docs)]
//! Runtime-independent Bitcoin read qualification models.
//!
//! Bitcoin balance collection is intentionally not a registered MFM operation or capability.
//! The crate retains only reusable checked values needed by independent one-RPC transport
//! qualification.

mod model;

pub use model::{
    BitcoinAddress, BitcoinBlockchainInfo, BitcoinInvalidRequest, BitcoinModelError,
    BitcoinNetworkId, BitcoinNetworkTag, BitcoinRoutingGenerationRef, BitcoinScanRequest,
    BitcoinScanResult, BitcoinScannedBalance, BitcoinSourceBinding, BitcoinSourceIdentity,
    BITCOIN_SCAN_ADDRESS_LIMIT,
};
