mod default_impls;

use crate::default_impls::{ComputePrice, ConfigState, OnChainValuesState};
use default_impls::{Report, Setup};
use mfm_machine::state::safe_context::create_default_safe_context;
use mfm_machine::state::DependencyStrategy;
use mfm_machine::state::Label;
use mfm_machine::state::States;
use mfm_machine::state::Tag;
use mfm_machine::state::{StateHandler, StateMetadata};
use mfm_machine::state_machine::StateMachine;
use std::sync::Arc;

#[test]
fn test_state_machine_execute() {
    let setup_state = Box::new(Setup::new());
    let compute_price = Box::new(ComputePrice::new());
    let report_state = Box::new(Report::new());

    let initial_states: States =
        Arc::new([setup_state.clone(), compute_price, report_state.clone()]);
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

    let context = create_default_safe_context();
    let result = state_machine.execute(context);
    println!("Execute result: {result:?}");

    assert_eq!(state_machine.states.len(), iss.len());
    state_machine.states.iter().zip(iss.iter()).for_each(
        |(s, (label, tags, depends_on, depends_on_strategy))| {
            assert_eq!(s.label(), *label);
            assert_eq!(s.tags(), *tags);
            assert_eq!(s.depends_on(), *depends_on);
            assert_eq!(s.depends_on_strategy(), *depends_on_strategy);
        },
    );

    assert!(result.is_ok());
}

#[test]
fn test_public_api() {
    let setup = Setup::new();
    let compute_price = ComputePrice::new();
    let report = Report::new();
    let config_state = ConfigState::new();
    let onchain_values = OnChainValuesState::new();

    assert_eq!(setup.label().as_str(), "setup_state");
    assert!(setup.tags().iter().any(|t| t.as_str() == "setup"));
    assert_eq!(compute_price.label().as_str(), "compute_price_state");
    assert!(compute_price
        .tags()
        .iter()
        .any(|t| t.as_str() == "compute_price"));
    assert_eq!(report.label().as_str(), "report_state");
    assert!(report.tags().iter().any(|t| t.as_str() == "report"));

    let states: Vec<Box<dyn StateHandler>> = vec![
        Box::new(setup) as Box<dyn StateHandler>,
        Box::new(compute_price) as Box<dyn StateHandler>,
        Box::new(report) as Box<dyn StateHandler>,
        Box::new(config_state) as Box<dyn StateHandler>,
        Box::new(onchain_values) as Box<dyn StateHandler>,
    ];
    let states = Arc::from(states);

    let mut state_machine = StateMachine::new(states);
    let context = create_default_safe_context();

    let result = state_machine.execute(context);
    assert!(result.is_ok());
}
