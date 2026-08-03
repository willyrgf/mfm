#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! Encrypted key storage and keystore-backed signing for MFM.
//!
//! Raw private-key access stays inside this crate. Callers may import keys, inspect public
//! metadata, delete entries, or bind the generation-guarded wallet signer.
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
//!
//! Administration operations are deliberately absent from the current public surface:
//!
//! ```compile_fail
//! use mfm_keystore::Keystore;
//!
//! fn explicit_lock(keystore: &mut Keystore) {
//!     keystore.lock();
//! }
//! ```
//!
//! ```compile_fail
//! use mfm_keystore::Keystore;
//!
//! fn rotate_password(keystore: &mut Keystore) {
//!     keystore.change_password("old password", "new password");
//! }
//! ```

mod crypto;
mod keystore;
mod signer;

pub use self::keystore::{KeyInfo, KeyType, Keystore, KeystoreConfig, KeystoreError};
pub use self::signer::{
    KeystoreSignerProvider, QualifiedKeystoreSigner, KEYSTORE_SIGNING_IMPLEMENTATION_ID,
};
