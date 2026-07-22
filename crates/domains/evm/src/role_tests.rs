#[test]
fn private_role_modules_only_point_downward() {
    let model = include_str!("model.rs");
    let capability = include_str!("capability.rs");
    let signing = include_str!("signing.rs");
    let state = concat!(
        include_str!("state.rs"),
        include_str!("state/balance_collection.rs"),
        include_str!("state/canonical.rs"),
        include_str!("state/contract_validation.rs"),
        include_str!("state/identity.rs"),
        include_str!("state/transaction.rs"),
    );

    for forbidden in [
        "crate::capability",
        "crate::signing",
        "crate::state",
        "crate::operation",
    ] {
        assert!(!model.contains(forbidden), "model imports {forbidden}");
    }
    for forbidden in ["crate::signing", "crate::state", "crate::operation"] {
        assert!(
            !capability.contains(forbidden),
            "capability imports {forbidden}"
        );
    }
    for forbidden in ["crate::state", "crate::operation"] {
        assert!(!signing.contains(forbidden), "signing imports {forbidden}");
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
        concat!("mfm_evm", "_capabilities"),
        concat!("mfm_evm", "_signing"),
        concat!("mfm_states", "_evm"),
        concat!("mfm_op_evm", "_collectors"),
    ] {
        assert!(!root.contains(superseded_package));
    }
}

#[test]
fn production_dependencies_are_pure_domain_only() {
    let manifest = include_str!("../Cargo.toml");
    let dependencies = manifest
        .split_once("[dependencies]")
        .expect("dependencies section")
        .1
        .split_once("[dev-dependencies]")
        .expect("dev-dependencies section")
        .0;

    for forbidden in [
        "mfm-runtime",
        "mfm-replay",
        "mfm-store",
        "mfm-app",
        "mfm-keystore",
        "mfm-storage-postgres",
        "reqwest",
        "hyper",
        "url =",
        "tokio =",
    ] {
        assert!(
            !dependencies.contains(forbidden),
            "pure domain depends on {forbidden}"
        );
    }
}
