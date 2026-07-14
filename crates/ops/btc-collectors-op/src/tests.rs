use super::*;
use mfm_program::StateSpec;

fn balance_fixture() -> BtcAddressBalanceConfig {
    BtcAddressBalanceConfig {
        network: "bitcoin-mainnet".to_owned(),
        bitcoin_network: "main".to_owned(),
        semantic_source_identity: "public-bitcoin-core".to_owned(),
        addresses: vec!["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned()],
        coverage: "configured_only".to_owned(),
        max_source_reads: NonZeroU64::new(1).expect("non-zero"),
    }
}

#[test]
fn default_chain_head_config_is_valid() {
    validate_btc_chain_head_collector_config(&BtcChainHeadCollectorConfig::default())
        .expect("default config");
}

#[test]
fn fixture_balance_config_is_valid() {
    validate_btc_address_balance_config(&balance_fixture()).expect("fixture balance config");
}

#[test]
fn multi_address_draft_shares_one_joint_tip_node() {
    let config = BtcAddressBalanceConfig {
        addresses: vec![
            "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned(),
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_owned(),
        ],
        ..balance_fixture()
    };
    let draft = btc_address_balance_program_draft(config).expect("draft");
    let nodes = draft.state_nodes();
    let joint_tip_nodes = nodes
        .iter()
        .filter(|node| node.key.as_str() == "resolve_joint_tip")
        .count();
    assert_eq!(joint_tip_nodes, 1, "must resolve joint tip exactly once");
    // tip + 2*(observe+record) + assemble = 1 + 4 + 1 = 6
    assert_eq!(nodes.len(), 6);
    let observe_count = nodes
        .iter()
        .filter(|node| node.key.as_str().starts_with("observe_address_balance_"))
        .count();
    assert_eq!(observe_count, 2);
    // Both observe nodes take the same joint tip cell as input.
    let tip_cell = nodes
        .iter()
        .find(|node| node.key.as_str() == "resolve_joint_tip")
        .expect("tip")
        .output_cell_id
        .as_str();
    for node in nodes
        .iter()
        .filter(|node| node.key.as_str().starts_with("observe_address_balance_"))
    {
        let rendered = format!("{:?}", node.input);
        assert!(
            rendered.contains(tip_cell),
            "observe must depend on shared joint tip cell: {rendered}"
        );
    }
}

#[test]
fn chain_head_record_states_advertise_exact_fact_descriptor_allow_lists() {
    let chain_head =
        RecordBtcChainHeadFactState::emitted_fact_descriptors().expect("chain-head descriptors");
    let checkpoint =
        RecordCollectorCheckpointState::emitted_fact_descriptors().expect("checkpoint descriptors");
    let balance =
        RecordBtcAddressBalanceFactState::emitted_fact_descriptors().expect("balance descriptors");

    assert_eq!(chain_head.len(), 1);
    assert_eq!(checkpoint.len(), 1);
    assert_eq!(balance.len(), 1);
    assert_ne!(chain_head[0].descriptor_hash, balance[0].descriptor_hash);
}
