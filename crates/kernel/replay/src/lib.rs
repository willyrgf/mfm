#![warn(missing_docs)]
//! Typed replay brokers and verifier contracts for MFM.
//!
//! Replay is intentionally evidence-only. A [`v1::ReplayBroker`] is built from a
//! certified typed execution spec, the authoritative store-owned run stream, and
//! retained artifact evidence. It never constructs transports, SDK clients,
//! or live capability handles.

/// Versioned v1 typed replay contracts.
pub mod v1;
