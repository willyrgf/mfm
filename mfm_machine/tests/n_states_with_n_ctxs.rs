use std::sync::Arc;

use default_impls::{
    AnalyticsState, ComputePrice, ConfigState, FinalizeState, NotificationState,
    OnChainValuesState, Report, Setup, ValidationState,
};
use mfm_machine::{
    state::{safe_context::create_default_safe_context, States},
    state_machine::StateMachine,
};

use crate::default_impls::Config;

mod default_impls;

#[test]
fn test_n_states_with_ctxs() {
    let config_state = Box::new(ConfigState::new());
    let onchain_value_state = Box::new(OnChainValuesState::new());

    let _config = Config {
        a: "zero".to_string(),
        b: 0,
    };

    // Create a default SafeContext
    let safe_context = create_default_safe_context();

    // Verify we can use the context
    let dump = safe_context.dump().unwrap();
    assert!(dump.is_object());

    let initial_states: States = Arc::new([config_state.clone(), onchain_value_state.clone()]);

    let mut state_machine = StateMachine::new(initial_states);

    // Execute state machine
    println!("Executing state machine...");
    let result = state_machine.execute(safe_context);

    println!(
        "state machine execution history: \n{:?}",
        state_machine.track_history()
    );

    assert!(result.is_ok());
}

#[test]
fn test_linear_state_transition() {
    // Create states with a linear dependency chain
    // Setup -> ComputePrice -> Report
    let setup_state = Box::new(Setup::new());
    let compute_price_state = Box::new(ComputePrice::new());
    let report_state = Box::new(Report::new());

    // Create a default SafeContext
    let safe_context = create_default_safe_context();

    // Configure initial states
    let initial_states: States = Arc::new([
        setup_state.clone(),
        compute_price_state.clone(),
        report_state.clone(),
    ]);

    let mut state_machine = StateMachine::new(initial_states);

    // Execute state machine with the linear chain
    println!("Executing linear state transition...");
    let result = state_machine.execute(safe_context);

    println!(
        "Linear state machine execution history: \n{:?}",
        state_machine.track_history()
    );

    assert!(result.is_ok());

    // Verify execution order through the tracked history
    let history = state_machine.track_history();
    assert_eq!(history.len(), 3);

    // Convert history to a vector for easier testing
    let history_vec: Vec<_> = history.into_iter().collect();

    // The order should be: Setup -> ComputePrice -> Report
    assert_eq!(history_vec[0].1.state_label.as_str(), "setup_state");
    assert_eq!(history_vec[1].1.state_label.as_str(), "compute_price_state");
    assert_eq!(history_vec[2].1.state_label.as_str(), "report_state");
}

#[test]
fn test_complex_state_transition() {
    // Create a complex workflow with multiple dependencies
    // Setup -> ComputePrice -> [Report, Validation] -> Notification -> AnalyticsState -> FinalizeState
    let setup_state = Box::new(Setup::new());
    let compute_price_state = Box::new(ComputePrice::new());
    let report_state = Box::new(Report::new());
    let validation_state = Box::new(ValidationState::new());
    let notification_state = Box::new(NotificationState::new());
    let analytics_state = Box::new(AnalyticsState::new());
    let finalize_state = Box::new(FinalizeState::new());

    // Create a default SafeContext
    let safe_context = create_default_safe_context();

    // Configure initial states
    let initial_states: States = Arc::new([
        setup_state.clone(),
        compute_price_state.clone(),
        report_state.clone(),
        validation_state.clone(),
        notification_state.clone(),
        analytics_state.clone(),
        finalize_state.clone(),
    ]);

    let mut state_machine = StateMachine::new(initial_states);

    // Execute state machine with the complex workflow
    println!("Executing complex state transition...");
    let result = state_machine.execute(safe_context);

    println!(
        "Complex state machine execution history: \n{:?}",
        state_machine.track_history()
    );

    assert!(result.is_ok());

    // Verify the execution completed with all states
    let history = state_machine.track_history();
    assert_eq!(history.len(), 7);

    // Check that the context contains the final data
    let result_context = result.unwrap();
    let finalized_data = result_context.read_typed::<Config>("finalized");
    assert!(finalized_data.is_ok());
}

#[test]
fn test_parallel_state_transitions() {
    // Create states for parallel execution (states with the same dependencies)
    let setup_state = Box::new(Setup::new());

    // These three will run after setup but can run in parallel
    let config_state = Box::new(ConfigState::new());
    let onchain_value_state = Box::new(OnChainValuesState::new());
    let compute_price_state = Box::new(ComputePrice::new());

    // These two depend on compute_price and can run in parallel
    let report_state = Box::new(Report::new());
    let validation_state = Box::new(ValidationState::new());

    // Create a default SafeContext
    let safe_context = create_default_safe_context();

    // Configure initial states
    let initial_states: States = Arc::new([
        setup_state.clone(),
        config_state.clone(),
        onchain_value_state.clone(),
        compute_price_state.clone(),
        report_state.clone(),
        validation_state.clone(),
    ]);

    let mut state_machine = StateMachine::new(initial_states);

    // Execute state machine with parallel flows
    println!("Executing parallel state transitions...");
    let result = state_machine.execute(safe_context);

    println!(
        "Parallel state machine execution history: \n{:?}",
        state_machine.track_history()
    );

    assert!(result.is_ok());

    // Verify all expected states were executed
    let history = state_machine.track_history();
    assert_eq!(history.len(), 6);

    // Convert history to a vector for easier testing
    let history_vec: Vec<_> = history.into_iter().collect();

    // Setup should be first
    assert_eq!(history_vec[0].1.state_label.as_str(), "setup_state");

    // Verify Report and Validation both executed (depends on compute_price)
    let mut found_report = false;
    let mut found_validation = false;

    for (_step, index, _value) in history_vec {
        match index.state_label.as_str() {
            "report_state" => found_report = true,
            "validation_state" => found_validation = true,
            _ => {}
        }
    }

    assert!(found_report);
    assert!(found_validation);
}

#[test]
fn test_dependency_strategy_all() {
    // Test a state that requires ALL dependencies to be satisfied
    let setup_state = Box::new(Setup::new());
    let compute_price_state = Box::new(ComputePrice::new());
    let report_state = Box::new(Report::new());
    let validation_state = Box::new(ValidationState::new());

    // Notification requires BOTH report and validation to run first
    let notification_state = Box::new(NotificationState::new());

    // Create a default SafeContext
    let safe_context = create_default_safe_context();

    // Configure initial states
    let initial_states: States = Arc::new([
        setup_state.clone(),
        compute_price_state.clone(),
        report_state.clone(),
        validation_state.clone(),
        notification_state.clone(),
    ]);

    let mut state_machine = StateMachine::new(initial_states);

    // Execute state machine
    println!("Executing state machine with ALL dependency strategy...");
    let result = state_machine.execute(safe_context);

    println!(
        "ALL dependency state machine execution history: \n{:?}",
        state_machine.track_history()
    );

    assert!(result.is_ok());

    // Verify notification ran only after both dependencies completed
    let history_vec: Vec<_> = state_machine.track_history().into_iter().collect();

    // Find positions of each state in the history
    let mut report_pos = 0;
    let mut validation_pos = 0;
    let mut notification_pos = 0;

    for (i, (_step, index, _value)) in history_vec.iter().enumerate() {
        match index.state_label.as_str() {
            "report_state" => report_pos = i,
            "validation_state" => validation_pos = i,
            "notification_state" => notification_pos = i,
            _ => {}
        }
    }

    // Notification must come after both report and validation
    assert!(notification_pos > report_pos);
    assert!(notification_pos > validation_pos);
}

#[test]
fn test_dependency_strategy_any() {
    // Test a state that requires ANY dependency to be satisfied
    let setup_state = Box::new(Setup::new());
    let compute_price_state = Box::new(ComputePrice::new());
    let report_state = Box::new(Report::new());

    // Analytics can run after EITHER report or notification
    // We'll exclude notification in this test to confirm it runs after report
    let analytics_state = Box::new(AnalyticsState::new());

    // Create a default SafeContext
    let safe_context = create_default_safe_context();

    // Configure initial states
    let initial_states: States = Arc::new([
        setup_state.clone(),
        compute_price_state.clone(),
        report_state.clone(),
        analytics_state.clone(),
    ]);

    let mut state_machine = StateMachine::new(initial_states);

    // Execute state machine
    println!("Executing state machine with ANY dependency strategy...");
    let result = state_machine.execute(safe_context);

    println!(
        "ANY dependency state machine execution history: \n{:?}",
        state_machine.track_history()
    );

    assert!(result.is_ok());

    // Verify analytics ran after report
    let history_vec: Vec<_> = state_machine.track_history().into_iter().collect();

    // Find positions of each state in the history
    let mut report_pos = 0;
    let mut analytics_pos = 0;

    for (i, (_step, index, _value)) in history_vec.iter().enumerate() {
        match index.state_label.as_str() {
            "report_state" => report_pos = i,
            "analytics_state" => analytics_pos = i,
            _ => {}
        }
    }

    // Analytics must come after report
    assert!(analytics_pos > report_pos);
}
