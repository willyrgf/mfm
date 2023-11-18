mod default_impls;

use default_impls::{ComputePrice, ContextA, Report, Setup};
use mfm_machine::state::context::Context;
use mfm_machine::state::StateHandler;
use mfm_machine::state_machine::StateMachine;
use std::sync::Arc;

#[test]
fn test_retry_workflow_state_machine() {
    let setup_state = Box::new(Setup::new());
    let compute_price_state = Box::new(ComputePrice::new());
    let report_state = Box::new(Report::new());

    let context: &mut dyn Context = &mut ContextA::new(String::from("hello"), 7);

    let initial_states: Arc<[Box<dyn StateHandler>]> = Arc::new([
        setup_state.clone(),
        compute_price_state.clone(),
        report_state.clone(),
    ]);

    let mut state_machine = StateMachine::new(initial_states);

    let result = state_machine.execute(context);

    assert!(result.is_ok());
}
