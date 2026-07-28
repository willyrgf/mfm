use mfm_evm::{
    evm_read_capability_contract_ref, AggregateEvmBalancesState, BootstrapEvmSourceState,
    ConfirmEvmAnchorState, ReadEvmInitialAnchorState, ReadEvmNativeBalanceState,
    ReadEvmTokenBalanceState, ReadEvmTokenDecimalsState, SubmitEvmTransactionState,
    EVM_READ_OPERATION_IDS,
};
use mfm_program::{State as _, UnitConfig};

#[test]
fn public_production_state_surface_has_decomposed_reads_and_one_effect() {
    assert_eq!(
        EVM_READ_OPERATION_IDS,
        [
            "eth_chain_id",
            "eth_get_block_by_number_latest",
            "eth_call_erc20_decimals",
            "eth_get_balance",
            "eth_call_erc20_balance_of",
            "eth_get_block_by_number_confirm",
        ]
    );
    assert_unit_config::<BootstrapEvmSourceState>();
    assert_unit_config::<ReadEvmInitialAnchorState>();
    assert_unit_config::<ReadEvmTokenDecimalsState>();
    assert_unit_config::<ReadEvmNativeBalanceState>();
    assert_unit_config::<ReadEvmTokenBalanceState>();
    assert_unit_config::<ConfirmEvmAnchorState>();
    assert_unit_config::<AggregateEvmBalancesState>();
    assert_unit_config::<SubmitEvmTransactionState>();

    let mut state_contracts = [
        BootstrapEvmSourceState::state_contract_ref().expect("bootstrap state"),
        ReadEvmInitialAnchorState::state_contract_ref().expect("initial-anchor state"),
        ReadEvmTokenDecimalsState::state_contract_ref().expect("token-decimals state"),
        ReadEvmNativeBalanceState::state_contract_ref().expect("native-balance state"),
        ReadEvmTokenBalanceState::state_contract_ref().expect("token-balance state"),
        ConfirmEvmAnchorState::state_contract_ref().expect("confirmation state"),
        AggregateEvmBalancesState::state_contract_ref().expect("aggregate state"),
        SubmitEvmTransactionState::state_contract_ref().expect("transaction effect state"),
    ];
    state_contracts.sort();
    assert!(state_contracts.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(evm_read_capability_contract_ref().is_ok());
}

fn assert_unit_config<S: mfm_program::State<Config = UnitConfig>>() {}

#[test]
fn legacy_registration_surfaces_are_absent_and_qualified_effect_is_public() {
    let public_surface = include_str!("../src/lib.rs");
    for removed in [
        "register_evm_transaction_runner",
        "EvmTransactionCapability",
        "EvmReadSessionSet",
        "CollectEvmBalancesState",
        "reduce_evm_balance_collection",
        "verify_evm_transaction_replay",
    ] {
        assert!(
            !public_surface.contains(removed),
            "removed production surface remained public: {removed}"
        );
    }
    for qualified in [
        "SubmitEvmTransactionState",
        "evm_submit_transaction_entry_point_registration",
        "qualify_evm_submit_transaction_state",
        "evm_submit_transaction_leaf_expansion",
    ] {
        assert!(
            public_surface.contains(qualified),
            "qualified production effect surface is not public: {qualified}"
        );
    }
}
