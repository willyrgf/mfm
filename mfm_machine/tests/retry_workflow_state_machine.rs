mod default_impls;

use default_impls::{ComputePrice, Report, Setup};
use mfm_machine::state::context::{wrap_context, Local};
use mfm_machine::state::States;
use mfm_machine::state_machine::StateMachine;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

#[test]
fn test_retry_workflow_state_machine() {
    let setup_state = Box::new(Setup::new());
    let compute_price_state = Box::new(ComputePrice::new());
    let report_state = Box::new(Report::new());

    let context = wrap_context(Local::new(HashMap::from([(
        "zero_ctx".to_string(),
        json!(0),
    )])));

    let initial_states: States = Arc::new([
        setup_state.clone(),
        compute_price_state.clone(),
        report_state.clone(),
    ]);

    let mut state_machine = StateMachine::new(initial_states);

    let result = state_machine.execute(context);

    println!(
        "state machine execution history: \n{:?}",
        state_machine.track_history()
    );

    println!("result: {:?}", result);

    assert!(result.is_ok());
}
