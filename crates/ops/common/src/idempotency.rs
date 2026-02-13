use mfm_machine::ids::{OpPath, StateId};

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
}
