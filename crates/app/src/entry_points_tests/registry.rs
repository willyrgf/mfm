use super::*;

#[test]
fn production_registry_resolves_portfolio_snapshot_latest() {
    let registry = production_entry_point_op_registry().expect("registry");
    let op = portfolio_op(&registry);

    assert_eq!(op.descriptor().version, 1);
    assert_eq!(
        registry
            .resolve_version(&portfolio_public_op_name(), OpVersion::new(1).unwrap())
            .expect("portfolio snapshot v1")
            .descriptor()
            .version,
        1
    );
    assert_eq!(
        op.descriptor().accepted_config_formats,
        &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json]
    );
}

#[test]
fn production_registry_rejects_deleted_portfolio_snapshot_v2() {
    let registry = production_entry_point_op_registry().expect("registry");
    let error =
        match registry.resolve_version(&portfolio_public_op_name(), OpVersion::new(2).unwrap()) {
            Ok(_) => panic!("portfolio snapshot v2 was deleted"),
            Err(error) => error,
        };

    assert_eq!(error.code(), "EntryPointOpVersionNotFound");
}

#[test]
fn production_registry_resolves_balance_collector_entry_points() {
    let registry = production_entry_point_op_registry().expect("registry");
    let btc = PublicOpName::new(mfm_op_btc_collectors::BTC_ADDRESS_BALANCE_ENTRY_POINT.public_name)
        .expect("btc name");
    let evm = PublicOpName::new(mfm_op_evm_collectors::EVM_NATIVE_BALANCE_ENTRY_POINT.public_name)
        .expect("evm name");

    let btc_op = registry.resolve_latest(&btc).expect("btc address balance");
    let evm_op = registry.resolve_latest(&evm).expect("evm native balance");
    assert_eq!(btc_op.descriptor().version, 1);
    assert_eq!(evm_op.descriptor().version, 1);
    assert_eq!(
        btc_op.descriptor().accepted_config_formats,
        &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json]
    );
    assert_eq!(
        evm_op.descriptor().accepted_config_formats,
        &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json]
    );
}

#[test]
fn production_registry_does_not_expose_btc_chain_head_as_public_entry_point() {
    let registry = production_entry_point_op_registry().expect("registry");
    let public_name = PublicOpName::new("btc_chain_head_collector").expect("name");

    let error = match registry.resolve_latest(&public_name) {
        Ok(_) => panic!("chain-head collector is not a public balance entry point"),
        Err(error) => error,
    };

    assert_eq!(error.code(), "EntryPointOpNotFound");
}

#[test]
fn btc_address_balance_entry_point_plans_from_json_config() {
    let registry = production_entry_point_op_registry().expect("registry");
    let public_name =
        PublicOpName::new(mfm_op_btc_collectors::BTC_ADDRESS_BALANCE_ENTRY_POINT.public_name)
            .expect("name");
    let op = registry.resolve_latest(&public_name).expect("btc op");
    let authored = AuthoredConfig::new(
        AuthoredConfigFormat::Json,
        r#"{
            "network": "bitcoin-mainnet",
            "bitcoin_network": "main",
            "semantic_source_identity": "public-bitcoin-core",
            "addresses": ["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"],
            "coverage": "configured_only",
            "max_source_reads": 1
        }"#,
    )
    .expect("authored");
    let plan = op.plan(authored).expect("btc plan");
    assert!(!plan.draft.state_nodes().is_empty());
    assert!(!plan.config_material.is_empty());
}

#[test]
fn btc_address_balance_entry_point_rejects_removed_head_selection_fields() {
    let registry = production_entry_point_op_registry().expect("registry");
    let public_name =
        PublicOpName::new(mfm_op_btc_collectors::BTC_ADDRESS_BALANCE_ENTRY_POINT.public_name)
            .expect("name");
    let op = registry.resolve_latest(&public_name).expect("btc op");
    let authored = AuthoredConfig::new(
        AuthoredConfigFormat::Json,
        r#"{
            "network": "bitcoin-mainnet",
            "bitcoin_network": "main",
            "semantic_source_identity": "public-bitcoin-core",
            "addresses": ["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"],
            "head_kind": "best",
            "coverage": "configured_only",
            "max_source_reads": 1
        }"#,
    )
    .expect("authored");

    let error = op.plan(authored).expect_err("removed field must reject");

    assert_eq!(error.code(), "AuthoredConfigUnknownField");
}

#[test]
fn evm_native_balance_entry_point_plans_from_json_config() {
    let registry = production_entry_point_op_registry().expect("registry");
    let public_name =
        PublicOpName::new(mfm_op_evm_collectors::EVM_NATIVE_BALANCE_ENTRY_POINT.public_name)
            .expect("name");
    let op = registry.resolve_latest(&public_name).expect("evm op");
    let authored = AuthoredConfig::new(
        AuthoredConfigFormat::Json,
        r#"{
            "network": "ethereum-mainnet",
            "chain_id": 1,
            "accounts": ["0x0000000000000000000000000000000000000001"],
            "coverage": "configured_only",
            "decimals": 18,
            "max_source_reads": 1
        }"#,
    )
    .expect("authored");
    let plan = op.plan(authored).expect("evm plan");
    assert!(!plan.draft.state_nodes().is_empty());
    assert!(!plan.config_material.is_empty());
}

#[test]
fn production_registry_plans_tracked_collect_then_report_configs() {
    let registry = production_entry_point_op_registry().expect("registry");
    for (name, raw) in [
        (
            "btc_address_balance",
            include_str!("../../../../examples/configs/btc-address-balance.toml"),
        ),
        (
            "evm_native_balance",
            include_str!("../../../../examples/configs/evm-native-balance.toml"),
        ),
        (
            "portfolio_snapshot",
            include_str!("../../../../examples/configs/portfolio-dual-mainnet.toml"),
        ),
    ] {
        let public_name = PublicOpName::new(name).expect("public name");
        let op = registry.resolve_latest(&public_name).expect("tracked op");
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Toml, raw).expect("tracked authored config");
        let plan = op.plan(authored).expect("tracked config plan");

        assert!(!plan.draft.state_nodes().is_empty(), "{name}");
        assert!(!plan.config_material.is_empty(), "{name}");
    }
}

#[test]
fn portfolio_snapshot_entry_point_plans_from_json_config() {
    let registry = production_entry_point_op_registry().expect("registry");
    let op = portfolio_op(&registry);
    let authored = AuthoredConfig::new(AuthoredConfigFormat::Json, sample_portfolio_config_json())
        .expect("authored config");

    let plan = op.plan(authored).expect("portfolio plan");

    assert!(!plan.draft.state_nodes().is_empty());
    assert!(!plan.config_material.is_empty());
    assert!(plan.seed_material.is_empty());
}

#[test]
fn portfolio_snapshot_entry_point_preserves_config_validation_error_code() {
    let registry = production_entry_point_op_registry().expect("registry");
    let op = portfolio_op(&registry);
    let mut config: serde_json::Value =
        serde_json::from_str(&sample_portfolio_config_json()).expect("portfolio json");
    config["portfolio"]["wallets"][0]["network_id"] = serde_json::json!("missing-network");
    let authored = AuthoredConfig::new(AuthoredConfigFormat::Json, config.to_string())
        .expect("authored config");

    let error = op.plan(authored).expect_err("invalid portfolio config");

    assert_eq!(error.code(), "PortfolioSnapshotConfigInvalid");
}

#[test]
fn evm_contract_entry_points_resolve_and_plan_without_seeds() {
    let registry = production_entry_point_op_registry().expect("registry");

    for (name, config, expects_import_node) in [
        ("evm_contract_deploy", deploy_config_json(), false),
        (
            "evm_contract_configure",
            configure_entry_config_json(),
            true,
        ),
        ("evm_contract_validate", validate_entry_config_json(), true),
        ("evm_contract_lifecycle", lifecycle_config_json(), false),
    ] {
        let op = registry
            .resolve_latest(&PublicOpName::new(name).expect("name"))
            .expect("EVM contract op");
        assert_eq!(op.descriptor().version, 1);
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Json, config.to_string()).expect("authored");
        let plan = op.plan(authored).expect("EVM contract plan");

        assert!(!plan.config_material.is_empty());
        assert!(plan.seed_material.is_empty());
        assert!(plan.draft.seeds().is_empty());
        if expects_import_node {
            assert!(
                plan.draft
                    .state_nodes()
                    .iter()
                    .any(|node| node.state_descriptor_name.contains("import_")),
                "{name} should include an import state"
            );
        }
        assert!(
            !plan.draft.state_nodes().is_empty(),
            "{name} should plan state nodes"
        );
    }
}
