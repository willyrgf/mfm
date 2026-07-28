#![warn(missing_docs)]
//! Purpose-authorized application facade for recoverable MFM runs.
//!
//! The run-facing surface admits one published entry point, advances at most one action per call,
//! renders reviewed public evidence, performs callback-free replay, and exports portable evidence.
//! Authentication policy, store authority issuance, live execution, and reproduction policy
//! remain private composition concerns.

mod access;
mod application;
mod errors;
mod executable_identity;
mod keystore_services;
mod production;
mod render;
mod runtime_config;
mod surface;
mod transaction_signing;

pub use self::access::{
    AccessPolicyError, AccessTarget, AuthorizedTenant, RunAccessGrant, RunAccessPolicy,
    SecretCredential, SecretCredentialError, MAX_SECRET_CREDENTIAL_BYTES,
};
#[cfg(any(test, feature = "test-support"))]
pub use self::application::{application_for_test, TestApplicationMode};
pub use self::application::{connect_production_application, Application};
pub use self::errors::{ErrorClass, PublicError};
#[cfg(any(test, feature = "test-support"))]
pub use self::keystore_services::initialize_insecure_keystore_for_test;
pub use self::keystore_services::{
    delete_keystore_key, import_keystore_key, list_keystore_keys, prepare_existing_keystore_access,
    prepare_import_keystore_access, KeystoreAccess, KeystoreCredentialRequirement,
    KeystoreImportRequest, KeystoreKeyMetadata, KeystoreKeySelector, KeystoreKeyType,
    KeystoreSelection, PreparedKeystoreAccess, SecretInput,
};
pub use self::render::PublicJsonResponse;
pub use self::surface::*;
pub use self::transaction_signing::{
    sign_evm_transaction_command, EvmTransactionSigningEnvelopeInput,
    EvmTransactionSigningMetadata, EvmTransactionSigningRequest, SignedEvmTransaction,
};

/// Shared observability configuration used by typed binaries.
pub mod observability;

pub(crate) fn production_database_url(database_url: Option<&str>) -> Result<String, PublicError> {
    match database_url {
        Some(database_url) => Ok(database_url.to_owned()),
        None => std::env::var("DATABASE_URL").map_err(|_| {
            PublicError::bad_request(
                "MissingDatabaseUrl",
                "Missing DATABASE_URL (or pass --database-url)",
            )
        }),
    }
}
