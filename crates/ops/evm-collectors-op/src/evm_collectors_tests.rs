use super::*;

#[test]
fn default_config_is_valid() {
    validate_evm_native_balance_config(&EvmNativeBalanceConfig::default()).expect("default");
}

#[test]
fn multi_account_draft_shares_one_joint_tip_node() {
    let config = EvmNativeBalanceConfig {
        accounts: vec![
            "0x0000000000000000000000000000000000000001".to_owned(),
            "0x0000000000000000000000000000000000000002".to_owned(),
        ],
        ..EvmNativeBalanceConfig::default()
    };
    let draft = evm_native_balance_program_draft(config).expect("draft");
    let nodes = draft.state_nodes();
    let joint_tip_nodes = nodes
        .iter()
        .filter(|node| node.key.as_str() == "resolve_joint_tip")
        .count();
    assert_eq!(joint_tip_nodes, 1);
    // tip + 2*(observe+record) + assemble = 6
    assert_eq!(nodes.len(), 6);
    let tip_cell = nodes
        .iter()
        .find(|node| node.key.as_str() == "resolve_joint_tip")
        .expect("tip")
        .output_cell_id
        .as_str();
    for node in nodes
        .iter()
        .filter(|node| node.key.as_str().starts_with("observe_native_balance_"))
    {
        let rendered = format!("{:?}", node.input);
        assert!(
            rendered.contains(tip_cell),
            "observe must depend on shared joint tip: {rendered}"
        );
    }
}

#[test]
fn native_balance_config_rejects_noncanonical_accounts() {
    let config = EvmNativeBalanceConfig {
        accounts: vec!["0x00000000000000000000000000000000000000AA".to_owned()],
        ..EvmNativeBalanceConfig::default()
    };

    assert!(validate_evm_native_balance_config(&config)
        .expect_err("mixed-case account must be rejected")
        .contains("normalized lowercase"));
}
