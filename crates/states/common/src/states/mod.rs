//! Reusable state implementations and metadata helpers shared across ops.
//!
//! This module is the main docs.rs entry point for shared execution-time building blocks.
//! Op crates typically pull states from here when they need:
//!
//! - stable machine metadata tags via [`meta`]
//! - Nix-backed execution via [`nix`]
//! - deterministic context/artifact publication via [`publish`]

/// Helpers for constructing consistent shared-state metadata.
pub mod meta;
/// Reusable state for pre-resolved or flake-resolved nix program execution.
pub mod nix;
/// Shared states for deterministic JSON/context publication and artifact emission.
pub mod publish;
