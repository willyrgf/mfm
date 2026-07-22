#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! Encrypted key storage and keystore-backed signing for MFM.
//!
//! Raw private-key access stays inside this crate. Callers may import keys, inspect public
//! metadata, delete entries, or bind the generic signing provider.
//!
//! # Examples
//!
//! ```rust,no_run
//! use mfm_keystore::{Keystore, KeystoreConfig};
//!
//! let path = std::env::temp_dir().join("mfm-keystore-doc-example.json");
//! let mut keystore =
//!     Keystore::new_with_config(&path, KeystoreConfig::insecure_integration_test())?;
//! keystore.unlock("correct horse battery staple")?;
//! # Ok::<(), mfm_keystore::KeystoreError>(())
//! ```
//!
//! Raw key wrappers and retrieval are deliberately not public:
//!
//! ```compile_fail
//! use mfm_keystore::SecureKey;
//! ```
//!
//! ```compile_fail
//! use mfm_keystore::Keystore;
//! use uuid::Uuid;
//!
//! fn raw_key(keystore: &mut Keystore, id: Uuid) {
//!     let _ = keystore.get_private_key(id);
//! }
//! ```

mod crypto;
mod keystore;
mod signer;

pub use self::keystore::{KeyInfo, KeyType, Keystore, KeystoreConfig, KeystoreError};
pub use self::signer::{KeystoreSignerProvider, KEYSTORE_SIGNING_IMPLEMENTATION_ID};
