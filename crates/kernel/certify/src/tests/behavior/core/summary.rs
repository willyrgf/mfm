use super::*;

#[test]
fn certification_summary_keys_are_stable() {
    let keys = [
        ProblemClass::InvalidTopology.summary_key(),
        ProblemClass::InvalidInterfaceWiring.summary_key(),
        ProblemClass::InvalidSemanticTransition.summary_key(),
        ProblemClass::InvalidDataShape.summary_key(),
        ProblemClass::InvalidDataMeaning.summary_key(),
        ProblemClass::InvalidTerminalShape.summary_key(),
    ];
    assert_eq!(
        keys,
        [
            "invalid_topology_rejected",
            "invalid_interface_wiring_rejected",
            "invalid_semantic_transition_rejected",
            "invalid_data_shape_rejected",
            "invalid_data_meaning_rejected",
            "invalid_terminal_shape_rejected"
        ]
    );
}
