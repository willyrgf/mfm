//! Proc-macro support for the `mfm-machine` runtime.
//!
//! This crate reserves the proc-macro integration point for compile-time helpers that mirror the
//! stable runtime contract described in `docs/redesign.md`.
//!
//! The crate currently exposes no public derive or attribute macros. Downstream crates typically
//! do not need to depend on it directly until that surface exists.
//!
//! # Examples
//!
//! ```rust
//! // `mfm-machine-derive` currently reserves the proc-macro crate slot for future derives.
//! use mfm_machine_derive as _;
//! ```
#![warn(missing_docs)]
