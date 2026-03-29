//! Reusable state implementations and metadata helpers shared across ops.
//!
//! This module is the main docs.rs entry point for shared execution-time building blocks.
//! Op crates typically pull states from here when they need:
//!
//! - stable machine metadata tags via [`meta`]
//! - Nix-backed execution via [`nix`]
//! - typed proof flows via [`proof`]
//! - deterministic context/artifact publication via [`publish`]
//! - shared failpoint helpers via [`side_effect`]

/// Helpers for constructing consistent shared-state metadata.
pub mod meta;
/// Reusable state for pre-resolved or flake-resolved nix program execution.
pub mod nix;
/// Typed proof states that avoid raw proof namespace strings in op planners.
pub mod proof;
/// Shared states for deterministic JSON/context publication and artifact emission.
pub mod publish;
/// Shared failpoint helpers used by proof recovery tests.
pub mod side_effect;
