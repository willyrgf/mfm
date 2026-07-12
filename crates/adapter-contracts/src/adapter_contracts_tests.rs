use super::*;

#[test]
fn evm_contract_lifecycle_ids_are_stable_and_not_workflow_named() {
    let kind = evm_contract_lifecycle_adapter_kind().expect("kind");
    let version = evm_contract_lifecycle_adapter_version().expect("version");

    assert_eq!(kind.canonical_name(), Some("mfm.evm/contract_lifecycle"));
    assert_eq!(version.as_str(), EVM_CONTRACT_LIFECYCLE_ADAPTER_VERSION);

    let rendered = format!("{kind} {version}");
    for term in stale_recipe_terms() {
        assert!(!rendered.contains(&term));
    }
}

#[test]
fn evm_contract_lifecycle_binding_records_capability_contracts_only() {
    let descriptor = evm_contract_lifecycle_adapter_binding().expect("binding");

    descriptor
        .validate_no_live_io()
        .expect("no live io binding");
    assert_eq!(
        descriptor.adapter_kind(),
        &evm_contract_lifecycle_adapter_kind().expect("kind")
    );
    assert!(descriptor
        .required_capabilities()
        .iter()
        .any(|capability| capability.canonical_name() == Some("mfm.evm/transaction.submit")));
    assert!(descriptor
        .required_capabilities()
        .iter()
        .any(|capability| { capability.canonical_name() == Some("mfm.evm/nonce_occupancy.read") }));
}

#[test]
fn binding_descriptor_rejects_duplicate_capabilities() {
    let capability = capability_kind::<EvmChainIdentityCapability>().expect("capability");
    let error = AdapterBindingDescriptor::new(
        evm_contract_lifecycle_adapter_kind().expect("kind"),
        evm_contract_lifecycle_adapter_version().expect("version"),
        vec![capability.clone(), capability.clone()],
    )
    .expect_err("duplicate capability");

    assert_eq!(
        error,
        AdapterContractError::DuplicateCapability { capability }
    );
}

#[test]
fn binding_descriptor_rejects_empty_capability_sets() {
    let error = AdapterBindingDescriptor::new(
        evm_contract_lifecycle_adapter_kind().expect("kind"),
        evm_contract_lifecycle_adapter_version().expect("version"),
        Vec::new(),
    )
    .expect_err("empty capabilities");

    assert_eq!(error, AdapterContractError::EmptyCapabilitySet);
}

fn stale_recipe_terms() -> [String; 3] {
    [
        ["d", "c", "v"].concat(),
        ["deploy", "_configure", "_validate"].concat(),
        ["deploy", "-configure", "-validate"].concat(),
    ]
}
