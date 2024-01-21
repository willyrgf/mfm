use std::sync::Arc;

use default_impls::{ConfigState, OnChainValuesState};
use mfm_machine::{
    state::{
        context::{wrap_context, Local},
        States,
    },
    state_machine::StateMachine,
};
use serde_json::json;

use crate::default_impls::{Config, CONFIG};

mod default_impls;

#[test]
fn test_n_states_with_ctxs() {
    let config_state = Box::new(ConfigState::new());
    let onchain_value_state = Box::new(OnChainValuesState::new());

    let config = Config {
        a: "zero".to_string(),
        b: "zero".to_string(),
    };

    // starting with a useless context
    // TODO: add an empty context impl
    let context = wrap_context(Local::default());

    context
        .lock()
        .unwrap()
        .write(CONFIG.to_string(), &json!(config))
        .unwrap();

    let initial_states: States = Arc::new([config_state.clone(), onchain_value_state.clone()]);

    let mut state_machine = StateMachine::new(initial_states);

    let result = state_machine.execute(context);

    println!(
        "state machine execution history: \n{:?}",
        state_machine.track_history()
    );

    assert!(result.is_ok());
}
