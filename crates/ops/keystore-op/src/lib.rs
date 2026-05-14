#![warn(missing_docs)]
//! Keystore operation planners.
//!
//! This crate owns the thin planning layer for keystore administration and transaction signing.
//! Execution and secret-adjacent behavior remain in `mfm-state-keystore` and
//! `mfm-transports-local-keystore`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_keystore::{KeystoreImportOp, KEYSTORE_IMPORT_OP_ID};
//! use mfm_sdk::op::Operation;
//!
//! let op = KeystoreImportOp;
//! assert_eq!(op.op_id().as_str(), KEYSTORE_IMPORT_OP_ID);
//! ```

/// Keystore administration planners.
pub mod admin;
/// Keystore transaction signing planners.
pub mod tx;

pub use admin::{
    keystore_delete_report_context_key, keystore_import_report_context_key,
    keystore_list_report_context_key, KeystoreDeleteOp, KeystoreDeleteOpConfig,
    KeystoreDeleteReport, KeystoreImportOp, KeystoreImportOpConfig, KeystoreImportReport,
    KeystoreImportType, KeystoreListKey, KeystoreListOp, KeystoreListOpConfig, KeystoreListReport,
    KeystoreListSortBy, KEYSTORE_ADMIN_OP_VERSION, KEYSTORE_DELETE_OP_ID, KEYSTORE_IMPORT_OP_ID,
    KEYSTORE_LIST_OP_ID,
};
pub use tx::{
    tx_sign_report_context_key, KeystoreTxSignOp, LocalFileWriteMode, TxSignOpConfig, TxSignReport,
    TX_OP_VERSION, TX_SIGN_OP_ID,
};
