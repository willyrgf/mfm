#![warn(missing_docs)]
//! Real PostgreSQL chain-registry provider, activation registry, and wallet-nonce authority.
//!
//! The provider authenticates complete chain attestations and route-membership
//! catalogs against its pinned registry lineage and exact current head,
//! then returns an opaque, non-serializable qualification value. Cloning the
//! client copies only endpoint configuration and its public trust anchor;
//! provider signing authority and target inventory remain in the separate
//! provider process. Pools are consumed by role-specific constructors and
//! never escape the returned administrative, public-read, or sealed
//! target-bound authority. Normal wallet execution verifies immutable
//! activation evidence offline and performs no registry query.

mod authority;
mod error;
mod provider;
mod registry;
mod schema;
mod support;

pub use authority::{open_wallet_nonce_authority, PostgresWalletNonceAuthority};
pub use error::{PostgresEvmWalletError, Result};
pub use provider::{
    DeploymentAssemblyBinding, DeploymentAssemblyRouteChallenge, DeploymentAssemblyRouteProof,
    FinishedDeploymentAssembly, PendingDeploymentAssembly, QualifiedEvmRoutingCatalog,
    WalletAuthorityProviderClient, WalletAuthorityProviderTrust,
};
#[cfg(feature = "parity-tests")]
pub use registry::ActivationIssuanceFaultPoint;
pub use registry::{
    open_activation_registry_admin, open_activation_registry_public,
    PostgresWalletActivationRegistryAdmin, PostgresWalletActivationRegistryPublic,
};
pub use schema::PostgresEvmWalletSchema;
