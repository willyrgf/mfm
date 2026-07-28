use std::collections::BTreeSet;

use mfm_evm::{
    evm_balance_collection_callback_surfaces, AggregateEvmBalancesState, BootstrapEvmSourceState,
    ConfirmEvmAnchorState, ReadEvmInitialAnchorState, ReadEvmNativeBalanceState,
    ReadEvmTokenBalanceState, ReadEvmTokenDecimalsState, EVM_READ_OPERATION_IDS,
    EVM_STATE_CALLBACK_SURFACE_VERSION,
};
use mfm_program::State as _;

#[test]
fn evm_collection_qualification_is_seven_distinct_audited_states() {
    let surfaces =
        evm_balance_collection_callback_surfaces().expect("qualified EVM callback surfaces");
    let ordered = surfaces.ordered();
    let state_contracts = [
        BootstrapEvmSourceState::state_contract_ref().expect("bootstrap state"),
        ReadEvmInitialAnchorState::state_contract_ref().expect("initial-anchor state"),
        ReadEvmTokenDecimalsState::state_contract_ref().expect("token-decimals state"),
        ReadEvmNativeBalanceState::state_contract_ref().expect("native-balance state"),
        ReadEvmTokenBalanceState::state_contract_ref().expect("token-balance state"),
        ConfirmEvmAnchorState::state_contract_ref().expect("confirmation state"),
        AggregateEvmBalancesState::state_contract_ref().expect("aggregate state"),
    ];

    assert_eq!(
        ordered
            .iter()
            .map(|surface| surface.content_ref())
            .collect::<BTreeSet<_>>()
            .len(),
        ordered.len()
    );

    for (index, (surface, state_contract)) in ordered.iter().zip(&state_contracts).enumerate() {
        assert_eq!(surface.state_contract_ref(), state_contract);
        let descriptor: serde_json::Value = serde_json::from_slice(surface.canonical().as_bytes())
            .expect("canonical callback descriptor");
        assert_eq!(
            descriptor
                .get("version")
                .and_then(serde_json::Value::as_str),
            Some(EVM_STATE_CALLBACK_SURFACE_VERSION)
        );
        assert_eq!(
            descriptor
                .get("external_operation_id")
                .and_then(serde_json::Value::as_str),
            EVM_READ_OPERATION_IDS.get(index).copied()
        );
        let callbacks = descriptor
            .get("callbacks")
            .and_then(serde_json::Value::as_array)
            .expect("closed callback list")
            .iter()
            .map(|callback| callback.as_str().expect("callback name"))
            .collect::<Vec<_>>();
        if index < EVM_READ_OPERATION_IDS.len() {
            assert_eq!(callbacks, ["apply", "request"]);
        } else {
            assert_eq!(callbacks, ["apply"]);
        }
    }
}

#[test]
fn production_live_adapter_is_exactly_the_six_read_operations() {
    let canonical =
        mfm_evm_live::evm_adapter_callback_surface_canonical().expect("adapter descriptor");
    let descriptor: serde_json::Value =
        serde_json::from_slice(canonical.as_bytes()).expect("canonical adapter descriptor");
    let callbacks = descriptor
        .get("callbacks")
        .and_then(serde_json::Value::as_array)
        .expect("adapter callbacks")
        .iter()
        .map(|callback| callback.as_str().expect("callback name"))
        .collect::<Vec<_>>();
    let mut expected = EVM_READ_OPERATION_IDS.to_vec();
    expected.sort_unstable();
    assert_eq!(callbacks, expected);

    for disabled in [
        "arbitrary_methods",
        "batching",
        "failover",
        "redirects",
        "reselection",
        "retries",
    ] {
        assert_eq!(
            descriptor
                .get(disabled)
                .and_then(serde_json::Value::as_bool),
            Some(false),
            "{disabled} must remain disabled"
        );
    }
    assert!(!canonical.as_str().contains("eth_send"));
    assert!(mfm_evm_live::evm_adapter_callback_surface_ref().is_ok());
}
