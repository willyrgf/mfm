#![warn(missing_docs)]
//! Workspace-private marker traits for authority-bearing integration seams.
//!
//! The package is intentionally not publishable. Production authority traits in
//! public crates inherit one of these markers, so an ordinary downstream crate
//! cannot implement a store, provider, or physical-binding authority merely by
//! implementing the visible methods. Concrete adapters in the MFM workspace opt
//! in explicitly.

/// Marker required by the Runtime history port.
pub trait RuntimeHistoryPortSeal: Send + Sync {}

/// Marker required by the store's physical-binding verifier.
pub trait PhysicalBindingVerifierSeal: Send + Sync {}

/// Marker required by the EVM wallet authority.
pub trait WalletNonceAuthoritySeal: Send + Sync {}

/// Marker required by the PostgreSQL checkpoint authority.
pub trait ExternalCheckpointAuthoritySeal: Send + Sync {}

/// Marker required by the deployment credential sink.
pub trait DeploymentCredentialSinkSeal {}

/// Marker required by the deployment credential broker.
pub trait DeploymentCredentialBrokerSeal: Send {}

/// Marker carried by the one workspace-owned Runtime assembly cutover.
pub trait RuntimeAssemblyConsumerSeal {}
