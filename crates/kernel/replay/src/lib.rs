#![warn(missing_docs)]
//! Typed replay brokers and verifier contracts for MFM.
//!
//! Replay is intentionally evidence-only. A [`v1::ReplayBroker`] is built from a
//! the authoritative store-owned [`mfm_store::v1::VerifiedRunView`] plus explicit
//! cross-run source evidence. It never constructs transports, SDK clients,
//! or live capability handles.

/// Versioned v1 typed replay contracts.
pub mod v1;
