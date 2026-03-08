//! Reusable state implementations and metadata helpers shared across ops.

/// Generic state that reads from a namespace-backed IO provider and writes to context.
pub mod io;
/// Helpers for constructing consistent shared-state metadata.
pub mod meta;
/// Reusable state for pre-resolved or flake-resolved nix program execution.
pub mod nix;
/// Typed proof states that avoid raw proof namespace strings in op planners.
pub mod proof;
/// Reusable state for idempotent side-effect application.
pub mod side_effect;
