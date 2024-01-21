mod default_impls;

use default_impls::{Report, Setup};
use mfm_machine::state::context::wrap_context;
use mfm_machine::state::context::Local;
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
                is.label(),
                is.tags(),
                is.depends_on(),
                is.depends_on_strategy(),
            )
        })
        .collect();

    let mut state_machine = StateMachine::new(initial_states);

    let context = wrap_context(Local::default());
    let result = state_machine.execute(context.clone());
    //let last_ctx_message = context.lock().unwrap().dump().unwrap();

    assert_eq!(state_machine.states.len(), iss.len());
    state_machine.states.iter().zip(iss.iter()).for_each(
        |(s, (label, tags, depends_on, depends_on_strategy))| {
            assert_eq!(s.label(), *label);
            assert_eq!(s.tags(), *tags);
            assert_eq!(s.depends_on(), *depends_on);
            assert_eq!(s.depends_on_strategy(), *depends_on_strategy);
        },
    );

    // let last_ctx_data: Local = serde_json::from_value(last_ctx_message).unwrap();
    // let report_ctx: ReportCtx =
    //     serde_json::from_value(last_ctx_data.read("report".to_string()).unwrap()).unwrap();
    //
    // println!("report_msg: {}", report_ctx.report_msg);

    assert!(result.is_ok());
    // assert_eq!(
    //     report_ctx.report_msg,
    //     String::from("some new data reported: setting up")
    // );
}
