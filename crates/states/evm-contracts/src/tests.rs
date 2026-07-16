use super::*;
use mfm_capabilities::CapabilitySet;

#[test]
fn retained_state_descriptors_use_the_direct_state_adapter_binding() {
    let expected_kind = contract_states_adapter_kind().expect("adapter kind");
    let expected_version = contract_states_adapter_version().expect("adapter version");

    for bindings in [
        ContextBoundDeployContractState::adapter_bindings().expect("deploy bindings"),
        ContextBoundConfigureContractState::adapter_bindings().expect("configure bindings"),
        ContextBoundValidateContractState::adapter_bindings().expect("validate bindings"),
    ] {
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].adapter_kind, expected_kind);
        assert_eq!(bindings[0].adapter_version, expected_version);
    }

    <ContractMutationCaps as CapabilitySet>::descriptor()
        .expect("mutation caps")
        .validate_for_effect::<ApplySideEffect>()
        .expect("mutation effect caps");
    <ContractValidationReadCaps as CapabilitySet>::descriptor()
        .expect("validation caps")
        .validate_for_effect::<ReadExternal>()
        .expect("read effect caps");
}

#[test]
fn direct_state_producer_contracts_allow_only_the_preceding_state() {
    let deploy = descriptor_id_for_state::<ContextBoundDeployContractState>().expect("deploy");
    let configure =
        descriptor_id_for_state::<ContextBoundConfigureContractState>().expect("configure");

    let mfm_program::StateInputContextContractSpec::Required {
        producer: configure_producer,
        ..
    } = ContextBoundConfigureContractState::input_context_contract().expect("configure input")
    else {
        panic!("configure must require a deployed value");
    };
    assert_eq!(configure_producer.producer_descriptor_ids, vec![deploy]);
    assert!(!configure_producer.seed_producers_allowed);

    let mfm_program::StateInputContextContractSpec::Required {
        producer: validate_producer,
        ..
    } = ContextBoundValidateContractState::input_context_contract().expect("validate input")
    else {
        panic!("validate must require a configured value");
    };
    assert_eq!(validate_producer.producer_descriptor_ids, vec![configure]);
    assert!(!validate_producer.seed_producers_allowed);
}
