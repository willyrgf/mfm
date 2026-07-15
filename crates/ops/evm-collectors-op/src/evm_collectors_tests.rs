use std::collections::BTreeMap;

use super::*;

fn fixture() -> EvmNativeBalanceConfig {
    EvmNativeBalanceConfig {
        network: NetworkConfig::new(
            "ethereum-mainnet".to_owned(),
            NetworkFamilyConfig::Evm,
            Some(1),
            Some(18),
            None,
            None,
            BTreeMap::new(),
        )
        .expect("EVM network"),
        accounts: vec!["0x0000000000000000000000000000000000000001".to_owned()],
    }
}

#[test]
fn fixture_config_derives_closed_child_policy() {
    let config = fixture();
    validate_evm_native_balance_config(&config).expect("fixture");

    let joint_tip = config.joint_tip_config().expect("joint tip config");
    assert_eq!(joint_tip.network, "ethereum-mainnet");
    assert_eq!(joint_tip.chain_id, 1);
    assert_eq!(joint_tip.max_source_reads.get(), EVM_JOINT_TIP_SOURCE_READS);

    let observe = config
        .observe_config_for_account(&config.accounts[0])
        .expect("observe config");
    assert_eq!(observe.coverage, EVM_NATIVE_BALANCE_COVERAGE);
    assert_eq!(
        observe.max_source_reads.get(),
        EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS
    );
    assert_eq!(
        observe.evm_network_parts().expect("EVM network parts"),
        ("ethereum-mainnet", 1, 18)
    );
}

#[test]
fn native_balance_config_rejects_noncanonical_accounts() {
    let config = EvmNativeBalanceConfig {
        accounts: vec!["0x00000000000000000000000000000000000000AA".to_owned()],
        ..fixture()
    };

    assert!(validate_evm_native_balance_config(&config)
        .expect_err("mixed-case account must be rejected")
        .contains("normalized lowercase"));
}

#[test]
fn native_balance_config_rejects_non_evm_networks() {
    let bitcoin_network = NetworkConfig::new(
        "bitcoin-mainnet".to_owned(),
        NetworkFamilyConfig::Bitcoin,
        None,
        None,
        Some("main".to_owned()),
        Some("public-bitcoin-core".to_owned()),
        BTreeMap::new(),
    )
    .expect("Bitcoin network");
    let config = EvmNativeBalanceConfig {
        network: bitcoin_network,
        ..fixture()
    };

    assert!(validate_evm_native_balance_config(&config)
        .expect_err("Bitcoin network must be rejected")
        .contains("requires an EVM network"));
}
