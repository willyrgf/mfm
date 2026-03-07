#![warn(missing_docs)]
//! Shared state-layer building blocks used across operation crates.
//!
//! `mfm-state-common` sits between the machine/runtime layer and the domain-specific op crates.
//! It centralizes reusable `State` implementations, error mappers, context helpers, and test
//! wiring so operation planners can stay thin.
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::ids::{ContextKey, FactKey};
//! use mfm_state_common::states::io::NamespaceReadState;
//!
//! let state = NamespaceReadState {
//!     namespace: "exec".to_string(),
//!     request: serde_json::json!({ "program_path": "/nix/store/example/bin/tool" }),
//!     fact_key: FactKey("mfm:local|state:demo.main.read|purpose:probe|req:deadbeef".to_string()),
//!     output_key: ContextKey("result".to_string()),
//!     io_error_code: "exec_failed",
//!     io_error_message: "exec call failed",
//! };
//!
//! assert_eq!(state.namespace, "exec");
//! ```

/// Context read/write helpers that map low-level context failures into stable state errors.
pub mod ctx;
/// Error constructors shared by reusable state implementations and op helpers.
pub mod errors;
/// Helpers for generating stable idempotency scopes and content-addressed keys.
pub mod idempotency;
/// Helpers for local IO calls and domain-event emission from shared states.
pub mod local_io_helpers;
/// Helpers for writing output artifacts and associated domain events.
pub mod output;
/// Small JSON/RPC validation helpers used by EVM-facing states.
pub mod rpc;
/// Reusable `State` implementations and metadata helpers.
pub mod states;
/// Reusable in-memory test wiring for op and state crate tests.
pub mod test_support;
