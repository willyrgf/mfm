#[test]
fn private_role_modules_only_point_downward() {
    let model = include_str!("model.rs");
    let capability = include_str!("capability.rs");
    let state = include_str!("state.rs");

    for forbidden in ["crate::capability", "crate::state", "crate::operation"] {
        assert!(!model.contains(forbidden), "model imports {forbidden}");
    }
    for forbidden in ["crate::state", "crate::operation"] {
        assert!(
            !capability.contains(forbidden),
            "capability imports {forbidden}"
        );
    }
    assert!(
        !state.contains("crate::operation"),
        "state imports operation"
    );
}

#[test]
fn root_exposes_one_explicit_api_without_module_or_glob_bridges() {
    let root = include_str!("lib.rs");
    assert!(!root.contains("pub mod "));
    assert!(!root.contains("::*"));
    for superseded_package in [
        concat!("mfm_btc", "_capabilities"),
        concat!("mfm_states", "_btc"),
        concat!("mfm_op_btc", "_collectors"),
    ] {
        assert!(!root.contains(superseded_package));
    }
}
