//! Reusable state implementations and metadata helpers shared across ops.

/// Helpers for constructing consistent shared-state metadata.
pub mod meta;
/// Reusable state for pre-resolved or flake-resolved nix program execution.
pub mod nix;
/// Typed proof states that avoid raw proof namespace strings in op planners.
pub mod proof;
/// Shared failpoint helpers used by proof recovery tests.
pub mod side_effect;
