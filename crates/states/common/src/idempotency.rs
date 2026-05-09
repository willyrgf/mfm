//! Helpers for constructing stable idempotency scopes and keys.
//!
//! Shared states use these helpers to keep fact keys and idempotency scopes deterministic across
//! planning, live execution, and replay.
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::ids::StateId;
//! use mfm_state_common::idempotency::{canonical, state_purpose, state_scope};
//!
//! let state_id = StateId::must_new("demo.main.read".to_string());
//!
//! assert_eq!(state_scope("mfm:exec", &state_id), "mfm:exec|state:demo.main.read");
//! assert_eq!(
//!     state_purpose("demo", &state_id, "read"),
//!     "mfm:demo|state:demo.main.read|purpose:read"
//! );
//! assert_eq!(
//!     canonical("demo", "demo.main.read", "read"),
//!     "mfm:demo|state:demo.main.read|purpose:read"
//! );
//! ```

use mfm_machine::errors::StateError;
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{OpPath, StateId};

use crate::errors::state_unknown_msg;

/// Builds a stable idempotency scope string for a specific state.
pub fn state_scope(scope: impl AsRef<str>, state_id: &StateId) -> String {
    format!("{}|state:{}", scope.as_ref(), state_id.as_str())
}

/// Builds a stable idempotency scope string for an operation path.
pub fn op_scope(scope: impl AsRef<str>, op_path: &OpPath) -> String {
    format!("{}|op:{}", scope.as_ref(), op_path.0)
}

/// Builds the canonical `mfm:` scope string used by shared states.
pub fn canonical(op: impl AsRef<str>, state: impl AsRef<str>, purpose: impl AsRef<str>) -> String {
    format!(
        "mfm:{}|state:{}|purpose:{}",
        op.as_ref(),
        state.as_ref(),
        purpose.as_ref()
    )
}

/// Builds the canonical idempotency purpose string for a state id.
pub fn state_purpose(op: impl AsRef<str>, state_id: &StateId, purpose: impl AsRef<str>) -> String {
    canonical(op, state_id.as_str(), purpose)
}

/// Builds the canonical idempotency purpose string for an operation path.
pub fn op_purpose(op: impl AsRef<str>, op_path: &OpPath, purpose: impl AsRef<str>) -> String {
    format!(
        "mfm:{}|op:{}|purpose:{}",
        op.as_ref(),
        op_path.0,
        purpose.as_ref()
    )
}

/// Hashes a canonical JSON value into a stable idempotency key.
///
/// Non-canonical JSON values, including floats, map to a stable state error.
pub fn idempotency_key_for_value(v: &serde_json::Value) -> Result<String, StateError> {
    let id = artifact_id_for_json(v).map_err(|_| {
        state_unknown_msg(
            "idempotency_key_not_canonical",
            "value was not canonical-json-hashable",
        )
    })?;
    Ok(id.into_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_scope_shape_is_stable() {
        let out = state_scope("mfm:exec", &StateId::must_new("m.main.run".to_string()));
        assert_eq!(out, "mfm:exec|state:m.main.run");
    }

    #[test]
    fn op_scope_shape_is_stable() {
        let out = op_scope("proof:side_effect", &OpPath("proof.main".to_string()));
        assert_eq!(out, "proof:side_effect|op:proof.main");
    }

    #[test]
    fn canonical_shape_is_stable() {
        let out = canonical("evm_validate", "m.main.validate", "assertions");
        assert_eq!(
            out,
            "mfm:evm_validate|state:m.main.validate|purpose:assertions"
        );
    }

    #[test]
    fn state_purpose_shape_is_stable() {
        let out = state_purpose(
            "nix_app",
            &StateId::must_new("nix_app.main.run".to_string()),
            "exec",
        );
        assert_eq!(out, "mfm:nix_app|state:nix_app.main.run|purpose:exec");
    }

    #[test]
    fn op_purpose_shape_is_stable() {
        let out = op_purpose("proof", &OpPath("proof.main".to_string()), "side_effect");
        assert_eq!(out, "mfm:proof|op:proof.main|purpose:side_effect");
    }
}
