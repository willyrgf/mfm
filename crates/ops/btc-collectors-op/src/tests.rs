use super::*;
use mfm_program::StateSpec;

fn network_fixture() -> BtcNetworkCollectionConfig {
    BtcNetworkCollectionConfig {
        native_balances: BtcNativeBalancesAtAnchorConfig {
            network: "bitcoin-mainnet".to_owned(),
            bitcoin_network: "main".to_owned(),
            semantic_source_identity: "public-bitcoin-core".to_owned(),
            addresses: vec!["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned()],
        },
    }
}

#[test]
fn default_chain_head_config_is_valid() {
    validate_btc_chain_head_collector_config(&BtcChainHeadCollectorConfig::default())
        .expect("default config");
}

#[test]
fn network_collection_derives_closed_state_policy() {
    let config = network_fixture();
    validate_btc_network_collection_config(&config).expect("network config");
    let native = &config.native_balances;
    assert_eq!(
        native.joint_tip_config().max_source_reads.get(),
        BTC_JOINT_TIP_SOURCE_READS
    );
    let observe = native.observe_config_for_address(&native.addresses[0]);
    assert_eq!(observe.coverage, BTC_NATIVE_BALANCE_COVERAGE);
    assert_eq!(
        observe.max_source_reads.get(),
        BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS
    );
}

#[test]
fn network_collection_rejects_unsorted_or_duplicate_addresses() {
    let mut unsorted = network_fixture();
    unsorted.native_balances.addresses = vec![
        "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned(),
        "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_owned(),
    ];
    assert!(validate_btc_network_collection_config(&unsorted).is_err());

    let mut duplicate = network_fixture();
    duplicate
        .native_balances
        .addresses
        .push("bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_owned());
    assert!(validate_btc_network_collection_config(&duplicate).is_err());
}

#[test]
fn fact_record_states_advertise_exact_fact_descriptor_allow_lists() {
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
