#[test]
fn root_exposes_one_selective_api_without_role_modules_or_globs() {
    let root = include_str!("lib.rs");
    for role in ["model", "state", "operation"] {
        assert!(root.contains(&format!("mod {role};")));
        assert!(!root.contains(&format!("pub mod {role};")));
    }
    assert!(!root.contains("pub use model::*"));
    assert!(!root.contains("pub use state::*"));
    assert!(!root.contains("pub use operation::*"));
}

#[test]
fn pure_package_has_no_live_or_platform_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "mfm-bitcoin-live",
        "mfm-evm-live",
        "mfm-runtime",
        "mfm-replay",
        "mfm-store",
        "mfm-app",
        "mfm-keystore",
        "mfm-storage-postgres",
        "reqwest",
        "tokio",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "pure portfolio manifest must not depend on {forbidden}"
        );
    }
}

#[test]
fn private_roles_follow_model_then_state_then_operation() {
    let model = concat!(
        include_str!("model/mod.rs"),
        include_str!("model/domain_key.rs"),
        include_str!("model/ids.rs"),
        include_str!("model/metadata.rs"),
        include_str!("model/portfolio.rs"),
        include_str!("model/portfolio/portfolio_snapshot.rs"),
        include_str!("model/symbol.rs"),
        include_str!("model/wallet.rs"),
    );
    for forbidden in [
        "crate::state",
        "crate::operation",
        "mfm_runtime",
        "mfm_store",
    ] {
        assert!(!model.contains(forbidden), "model named {forbidden}");
    }

    let state = concat!(
        include_str!("state/mod.rs"),
        include_str!("state/collection_receipt.rs"),
        include_str!("state/config.rs"),
        include_str!("state/holding_read.rs"),
        include_str!("state/selection.rs"),
        include_str!("state/states.rs"),
        include_str!("state/transforms.rs"),
    );
    for forbidden in ["crate::operation", "mfm_runtime", "mfm_store"] {
        assert!(!state.contains(forbidden), "state named {forbidden}");
    }
}
