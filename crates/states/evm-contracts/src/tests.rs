use super::*;
use mfm_capabilities::CapabilitySet;
use mfm_values::MfmValue;

fn validated_config<T: mfm_values::MfmConfig>(config: T) -> mfm_program::ValidatedConfig<T> {
    mfm_program::ValidatedConfig::new(config).expect("valid config")
}

fn network_json() -> serde_json::Value {
    serde_json::json!({
        "network_id": "ethereum-mainnet",
        "expected_chain_id": 1,
    })
}

fn signer_json() -> serde_json::Value {
    serde_json::json!({
        "signer_ref": "deployer",
        "expected_signer_address": "0x000000000000000000000000000000000000dead",
    })
}

fn deploy_config() -> DeployPhaseConfig {
    serde_json::from_value(serde_json::json!({
        "network": network_json(),
        "signer": signer_json(),
    }))
    .expect("deploy config")
}

fn configure_config() -> ConfigurePhaseConfig {
    serde_json::from_value(serde_json::json!({
        "network": network_json(),
        "signer": signer_json(),
        "calls": [{
            "function": "configure",
            "args": [],
        }],
    }))
    .expect("configure config")
}

fn validate_config() -> ValidatePhaseConfig {
    serde_json::from_value(serde_json::json!({
        "network": network_json(),
    }))
    .expect("validate config")
}

fn deployed_contract() -> DeployedContract {
    DeployedContract {
        lifecycle_version: 1,
        network_id: "ethereum-mainnet".to_owned(),
        expected_chain_id: 1,
        contract_address: "0x000000000000000000000000000000000000dead".to_owned(),
        deploy_tx_hash: "0x01".to_owned(),
        deploy_receipt_evidence: None,
        deployed_block_number: Some(1),
    }
}

fn deployed_contract_on_chain(chain_id: u64) -> DeployedContract {
    DeployedContract {
        expected_chain_id: chain_id,
        ..deployed_contract()
    }
}

fn configured_contract() -> ConfiguredContract {
    ConfiguredContract {
        lifecycle_version: 1,
        deployed: deployed_contract(),
        configure_calls: Vec::new(),
        confirmation_read_assertions: Vec::new(),
        confirmation_event_assertions: Vec::new(),
        configure_tx_hashes: Vec::new(),
        configure_receipt_evidence: Vec::new(),
        configured_block_number: Some(2),
    }
}

fn configured_contract_on_chain(chain_id: u64) -> ConfiguredContract {
    ConfiguredContract {
        deployed: deployed_contract_on_chain(chain_id),
        ..configured_contract()
    }
}

#[test]
fn state_schemas_and_names_use_contract_lifecycle_namespace() {
    let state_names = [
        DeployContractState::name(),
        ConfigureContractState::name(),
        ValidateContractState::name(),
        ProjectConfiguredContractRefState::name(),
    ];
    assert!(state_names
        .iter()
        .all(|name| name.starts_with("mfm.evm.contract.")));

    let schema_ids = [
        ContractTransactionIntent::schema_id()
            .expect("schema")
            .to_string(),
        ContractDeployIntent::schema_id()
            .expect("schema")
            .to_string(),
        ContractConfigureIntent::schema_id()
            .expect("schema")
            .to_string(),
        ContractDeployReceipt::schema_id()
            .expect("schema")
            .to_string(),
        ContractConfigureReceipt::schema_id()
            .expect("schema")
            .to_string(),
        ContractValidationReadRequest::schema_id()
            .expect("schema")
            .to_string(),
        ContractValidationReadResponse::schema_id()
            .expect("schema")
            .to_string(),
    ];
    assert!(schema_ids
        .iter()
        .all(|schema_id| schema_id.contains("mfm.evm.contract.")));
}

#[test]
fn mutation_states_use_lifecycle_adapter_binding() {
    let expected = evm_contract_lifecycle_adapter_binding().expect("binding");

    for bindings in [
        DeployContractState::adapter_bindings().expect("bindings"),
        ConfigureContractState::adapter_bindings().expect("bindings"),
        ValidateContractState::adapter_bindings().expect("bindings"),
    ] {
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].adapter_kind, *expected.adapter_kind());
        assert_eq!(bindings[0].adapter_version, *expected.adapter_version());
    }
}

#[test]
fn capability_sets_validate_for_their_effects() {
    <ContractMutationCaps as CapabilitySet>::descriptor()
        .expect("mutation caps")
        .validate_for_effect::<ApplySideEffect>()
        .expect("side effect caps");
    <ContractValidationReadCaps as CapabilitySet>::descriptor()
        .expect("read caps")
        .validate_for_effect::<ReadExternal>()
        .expect("read caps");
}

#[test]
fn idempotency_digest_uses_canonical_json() {
    #[derive(Serialize)]
    struct First {
        b: u64,
        a: u64,
    }

    #[derive(Serialize)]
    struct Second {
        a: u64,
        b: u64,
    }

    let first = idempotency_from_intent(&First { b: 2, a: 1 }).expect("first");
    let second = idempotency_from_intent(&Second { a: 1, b: 2 }).expect("second");

    assert_eq!(first.key, second.key);
}

#[test]
fn deploy_intent_contains_no_signed_payload_or_secret_material() {
    let state = DeployContractState::new(validated_config(deploy_config())).expect("state");
    let context = mfm_program::CertifiedContext::no_context();
    let intent = state.prepare_intent(&(), &context).expect("intent");
    let json = serde_json::to_string(&intent).expect("intent json");

    for forbidden in [
        ["raw", "_transaction"].concat(),
        "signature".to_owned(),
        ["private", "_key"].concat(),
        ["pass", "word"].concat(),
        ["rpc", "_url"].concat(),
        ["keystore", "_path"].concat(),
    ] {
        assert!(!json.contains(&forbidden), "{json} contains {forbidden}");
    }
}

#[test]
fn deploy_receipt_projects_deployed_typestate() {
    let state = DeployContractState::new(validated_config(deploy_config())).expect("state");
    let context = mfm_program::CertifiedContext::no_context();
    let intent = state.prepare_intent(&(), &context).expect("intent");
    let output = state
        .output_from_receipt(
            &(),
            &intent,
            &ContractDeployReceipt {
                receipt_version: 1,
                contract_address: "0x000000000000000000000000000000000000beef".to_owned(),
                receipt: ContractTransactionReceipt {
                    receipt_version: 1,
                    transaction_hash: "0x01".to_owned(),
                    block_number: 3,
                    status: true,
                    receipt_evidence: None,
                },
            },
            &context,
        )
        .expect("deployed");

    assert_eq!(
        output.contract_address,
        "0x000000000000000000000000000000000000beef"
    );
    assert_eq!(output.deploy_tx_hash, "0x01");
    assert_eq!(output.deployed_block_number, Some(3));
}

#[test]
fn configure_receipt_projects_configured_typestate() {
    let state = ConfigureContractState::new(validated_config(configure_config())).expect("state");
    let input = ConfigureContractInput {
        deployed: deployed_contract(),
    };
    let context = mfm_program::CertifiedContext::no_context();
    let intent = state.prepare_intent(&input, &context).expect("intent");
    let output = state
        .output_from_receipt(
            &input,
            &intent,
            &ContractConfigureReceipt {
                receipt_version: 1,
                receipts: vec![ContractTransactionReceipt {
                    receipt_version: 1,
                    transaction_hash: "0x02".to_owned(),
                    block_number: 3,
                    status: true,
                    receipt_evidence: None,
                }],
                configured_block_number: None,
            },
            &context,
        )
        .expect("configured");

    assert_eq!(output.configure_tx_hashes, vec!["0x02"]);
    assert_eq!(output.configured_block_number, Some(3));
    assert_eq!(intent.transactions.len(), 1);
}

#[test]
fn configure_confirmation_projects_configured_typestate() {
    let state = ConfigureContractState::new(validated_config(configure_config())).expect("state");
    let input = ConfigureContractInput {
        deployed: deployed_contract(),
    };
    let context = mfm_program::CertifiedContext::no_context();
    let intent = state.prepare_intent(&input, &context).expect("intent");
    let output = state
        .output_from_confirmation(
            &input,
            &intent,
            &ContractConfigureConfirmation {
                confirmation_version: 1,
                confirmations: 1,
                receipts: vec![ContractTransactionReceipt {
                    receipt_version: 1,
                    transaction_hash: "0x02".to_owned(),
                    block_number: 3,
                    status: true,
                    receipt_evidence: None,
                }],
                configured_block_number: None,
            },
            &context,
        )
        .expect("configured");

    assert_eq!(output.configure_tx_hashes, vec!["0x02"]);
    assert_eq!(output.configured_block_number, Some(3));
    assert_eq!(intent.transactions.len(), 1);
}

#[test]
fn configure_rejects_typestate_network_mismatch() {
    let state = ConfigureContractState::new(validated_config(configure_config())).expect("state");
    let input = ConfigureContractInput {
        deployed: deployed_contract_on_chain(2),
    };
    let context = mfm_program::CertifiedContext::no_context();

    assert!(state.prepare_intent(&input, &context).is_err());
    assert!(state
        .output_from_confirmation(
            &input,
            &ContractConfigureIntent {
                intent_version: 1,
                deployed: input.deployed.clone(),
                transactions: Vec::new(),
                has_inline_artifact: false,
            },
            &ContractConfigureConfirmation {
                confirmation_version: 1,
                confirmations: 1,
                receipts: Vec::new(),
                configured_block_number: None,
            },
            &context,
        )
        .is_err());
}

#[test]
fn validate_report_projection_fails_closed_on_wrong_chain() {
    let state = ValidateContractState::new(validated_config(validate_config())).expect("state");
    let input = ValidateContractInput {
        configured: configured_contract(),
    };
    let response = ContractValidationReadResponse {
        response_version: 1,
        observed_chain_id: 2,
        client_version: "redacted-client".to_owned(),
        configuration_read_results: Vec::new(),
        configuration_event_results: Vec::new(),
        read_results: Vec::new(),
        event_results: Vec::new(),
    };
    let report = state
        .report_from_response(&input, response)
        .expect("validation report");

    assert!(!report.valid);
    assert_eq!(
        state
            .read_request(&input)
            .expect("read request")
            .expected_chain_id,
        1
    );
}

#[test]
fn validate_rejects_typestate_network_mismatch() {
    let state = ValidateContractState::new(validated_config(validate_config())).expect("state");
    let input = ValidateContractInput {
        configured: configured_contract_on_chain(2),
    };
    let response = ContractValidationReadResponse {
        response_version: 1,
        observed_chain_id: 1,
        client_version: "redacted-client".to_owned(),
        configuration_read_results: Vec::new(),
        configuration_event_results: Vec::new(),
        read_results: Vec::new(),
        event_results: Vec::new(),
    };

    assert!(state.read_request(&input).is_err());
    assert!(state.report_from_response(&input, response).is_err());
}
