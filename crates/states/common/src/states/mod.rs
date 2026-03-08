//! Reusable state implementations and metadata helpers shared across ops.
//!
//! This module is the main docs.rs entry point for shared execution-time building blocks.
//! Op crates typically pull states from here when they need:
//!
//! - namespace-routed read-only IO via [`io`]
//! - stable machine metadata tags via [`meta`]
//! - Nix-backed execution via [`nix`]
//! - idempotent side-effect handling via [`side_effect`]
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::ids::{ContextKey, FactKey};
//! use mfm_state_common::states::io::NamespaceReadState;
//!
//! let state = NamespaceReadState {
//!     namespace: "local.fs.read_text".to_string(),
//!     request: serde_json::json!({ "path_hex": "2f746d702f64656d6f" }),
//!     fact_key: FactKey("mfm:local|state:demo.main.read|purpose:input|req:abc123".to_string()),
//!     output_key: ContextKey("input_text".to_string()),
//!     io_error_code: "fs_read_failed",
//!     io_error_message: "failed to read input file",
//! };
//!
//! assert_eq!(state.namespace, "local.fs.read_text");
//! assert_eq!(state.output_key.0, "input_text");
//! ```

/// Generic state that reads from a namespace-backed IO provider and writes to context.
pub mod io;
/// Helpers for constructing consistent shared-state metadata.
pub mod meta;
/// Reusable state for pre-resolved or flake-resolved nix program execution.
pub mod nix;
/// Reusable state for idempotent side-effect application.
pub mod side_effect;
