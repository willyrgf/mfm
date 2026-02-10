mod default_impls;

use default_impls::{ComputePrice, Report, Setup};
use mfm_machine_legacy::state::safe_context::create_default_safe_context;
use mfm_machine_legacy::state_machine::StateMachine;
use std::sync::Arc;

use crate::default_impls::{Config, ConfigState, OnChainValuesState, CONFIG};

#[tokio::test]
async fn test_retry_workflow_state_machine() {
    let setup = Setup::new();
    let compute_price = ComputePrice::new();
    let report = Report::new();
    let config_state = ConfigState::new();
    let onchain_values = OnChainValuesState::new();

    let states = vec![
        Box::new(setup) as Box<dyn mfm_machine_legacy::state::StateHandler>,
        Box::new(compute_price) as Box<dyn mfm_machine_legacy::state::StateHandler>,
        Box::new(report) as Box<dyn mfm_machine_legacy::state::StateHandler>,
        Box::new(config_state) as Box<dyn mfm_machine_legacy::state::StateHandler>,
        Box::new(onchain_values) as Box<dyn mfm_machine_legacy::state::StateHandler>,
    ];
    let states = Arc::from(states);

    let mut state_machine = StateMachine::new(states).unwrap();
    let context = create_default_safe_context();

    let result = state_machine.execute(context).await;
    assert!(result.is_ok());

    // Verify we can read the configuration
    let final_context = result.unwrap();
    let setup_data: Config = final_context.read_typed(CONFIG).unwrap();
    assert_eq!(setup_data.a, "setup_b");
    assert_eq!(setup_data.b, 1);

    // Verify we can read the final report
    let report_data: Config = final_context.read_typed("report").unwrap();
    assert_eq!(report_data.a, "setup_b_compute_price_report");
    assert_eq!(report_data.b, 4);
}
