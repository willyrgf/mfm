#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
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
//! use mfm_machine::ids::{ContextKey, StateId};
//! use mfm_state_common::states::proof::ProofReadState;
//!
//! let state = ProofReadState {
//!     state_id: StateId::must_new("proof.main.read".to_string()),
//!     purpose: "proof_read",
//!     output_key: ContextKey("result".to_string()),
//!     io_error_code: "read_failed",
//!     io_error_message: "proof read failed",
//! };
//!
//! assert_eq!(state.purpose, "proof_read");
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
