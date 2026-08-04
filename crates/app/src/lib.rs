#![warn(missing_docs)]
//! Purpose-authorized application facade for recoverable MFM runs.
//!
//! The run-facing surface admits one selected published entry point per request, advances at most
//! one action per call, renders reviewed public evidence, performs callback-free replay, and
//! exports portable evidence. Authentication policy, store authority issuance, live execution,
//! and export policy remain private composition concerns.

mod access;
mod application;
mod errors;
mod keystore_services;
#[path = "production_structured.rs"]
mod production;
mod render;
mod runtime_config;
mod stream_spool;
mod surface;

pub use self::access::{
    AccessPolicyError, AccessTarget, AuthorizedTenant, RunAccessGrant, RunAccessPolicy,
    SecretCredential, SecretCredentialError, MAX_SECRET_CREDENTIAL_BYTES,
};
#[cfg(any(test, feature = "test-support"))]
pub use self::application::{
    application_for_test, application_with_export_for_test, TestApplicationMode,
};
pub use self::application::{
    connect_production_application, Application, EvmWalletDeployment,
    EvmWalletDeploymentAssemblyInput, EvmWalletDeploymentReleaseMaterial,
};
pub use self::errors::{
    ErrorClass, PublicError, PublicRuntimeFaultAttribution, PublicRuntimeFaultPhase,
    PublicRuntimeFaultSubject, MAX_PUBLIC_ERROR_CODE_BYTES, MAX_PUBLIC_ERROR_MESSAGE_BYTES,
};
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

/// Shared observability configuration used by typed binaries.
pub mod observability;
