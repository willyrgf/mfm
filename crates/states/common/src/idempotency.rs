use mfm_machine::errors::StateError;
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{OpPath, StateId};

use crate::errors::state_unknown_msg;

pub fn state_scope(scope: impl AsRef<str>, state_id: &StateId) -> String {
    format!("{}|state:{}", scope.as_ref(), state_id.0)
}

pub fn op_scope(scope: impl AsRef<str>, op_path: &OpPath) -> String {
    format!("{}|op:{}", scope.as_ref(), op_path.0)
}

pub fn canonical(op: impl AsRef<str>, state: impl AsRef<str>, purpose: impl AsRef<str>) -> String {
    format!(
        "mfm:{}|state:{}|purpose:{}",
        op.as_ref(),
        state.as_ref(),
        purpose.as_ref()
    )
}

pub fn state_purpose(op: impl AsRef<str>, state_id: &StateId, purpose: impl AsRef<str>) -> String {
    canonical(op, &state_id.0, purpose)
}

pub fn op_purpose(op: impl AsRef<str>, op_path: &OpPath, purpose: impl AsRef<str>) -> String {
    format!(
        "mfm:{}|op:{}|purpose:{}",
        op.as_ref(),
        op_path.0,
        purpose.as_ref()
    )
}

pub fn idempotency_key_for_value(v: &serde_json::Value) -> Result<String, StateError> {
    let id = artifact_id_for_json(v).map_err(|_| {
        state_unknown_msg(
            "idempotency_key_not_canonical",
            "value was not canonical-json-hashable",
        )
    })?;
    Ok(id.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_scope_shape_is_stable() {
        let out = state_scope("mfm:exec", &StateId("m.main.run".to_string()));
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
        let out = state_purpose("nix_app", &StateId("nix_app.main.run".to_string()), "exec");
        assert_eq!(out, "mfm:nix_app|state:nix_app.main.run|purpose:exec");
    }

    #[test]
    fn op_purpose_shape_is_stable() {
        let out = op_purpose("proof", &OpPath("proof.main".to_string()), "side_effect");
        assert_eq!(out, "mfm:proof|op:proof.main|purpose:side_effect");
    }
}
