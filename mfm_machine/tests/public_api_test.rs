mod default_impls;

use default_impls::{ContextA, Report, Setup};
use mfm_machine::state::context::wrap_context;
use mfm_machine::state::DependencyStrategy;
use mfm_machine::state::Label;
use mfm_machine::state::States;
use mfm_machine::state::Tag;
use mfm_machine::state_machine::StateMachine;

#[test]
fn test_state_machine_execute() {
    use std::sync::Arc;

    let setup_state = Box::new(Setup::new());
    let report_state = Box::new(Report::new());

    let initial_states: States = Arc::new([setup_state.clone(), report_state.clone()]);
    let initial_states_cloned = initial_states.clone();

    let iss: Vec<(Label, Vec<Tag>, Vec<Tag>, DependencyStrategy)> = initial_states_cloned
        .iter()
        .map(|is| {
            (
                is.label().clone(),
                is.tags(),
                is.depends_on(),
                is.depends_on_strategy().clone(),
            )
        })
        .collect();

    let mut state_machine = StateMachine::new(initial_states);

    let context = wrap_context(ContextA::new(String::from("hello"), 7));
    let result = state_machine.execute(context.clone());
    let last_ctx_message = context.lock().unwrap().read_input().unwrap();

    assert_eq!(state_machine.states.len(), iss.len());

    state_machine.states.iter().zip(iss.iter()).for_each(
        |(s, (label, tags, depends_on, depends_on_strategy))| {
            assert_eq!(s.label(), *label);
            assert_eq!(s.tags(), *tags);
            assert_eq!(s.depends_on(), *depends_on);
            assert_eq!(s.depends_on_strategy(), *depends_on_strategy);
        },
    );

    let last_ctx_data: ContextA = serde_json::from_value(last_ctx_message).unwrap();

    assert!(result.is_ok());
    assert_eq!(
        last_ctx_data.a,
        String::from("some new data reported: setting up")
    );
}
