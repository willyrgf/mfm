use super::*;
use mfm_capabilities::CapabilitySet;
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, NodeId};
use mfm_program::StateContext;
use mfm_values::{ContextBoundOutput, MfmValue};

fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest_str(byte: u8) -> String {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

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

fn context_json() -> serde_json::Value {
    serde_json::json!({
        "lifecycle_key": "example-lifecycle",
        "network": {
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "chain_fingerprint": null,
            "finality_or_observation_policy": null,
        },
        "contract_profile": {
            "profile_id": "example-profile",
            "artifact_digest": content_digest_str(0x20),
            "interface_digest": content_digest_str(0x21),
            "creation_bytecode_digest": null,
            "deployed_code_hash": null,
            "selector_event_policy_digest": null,
        },
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

fn deploy_action() -> DeployAction {
    serde_json::from_value(serde_json::json!({
        "signer": signer_json(),
    }))
    .expect("deploy action")
}

fn configure_action() -> ConfigureAction {
    serde_json::from_value(serde_json::json!({
        "signer": signer_json(),
        "calls": [{
            "function": "configure",
            "args": [],
        }],
    }))
    .expect("configure action")
}

fn validate_action() -> ValidateAction {
    serde_json::from_value(serde_json::json!({})).expect("validate action")
}

fn import_deployed_spec() -> ImportDeployedSpec {
    serde_json::from_value(serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000beef",
            "provenance_label": "audited-external",
        },
    }))
    .expect("import deployed")
}

fn import_configured_spec() -> ImportConfiguredSpec {
    serde_json::from_value(serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000beef",
            "provenance_label": "audited-external",
        },
    }))
    .expect("import configured")
}

fn certified_contract_context() -> mfm_program::CertifiedContext<EvmContractContext> {
    let value: EvmContractContext = serde_json::from_value(context_json()).expect("context");
    let mfm_program::StateContextDescriptorSpec::Required(requirement) =
        <EvmContractContext as StateContext>::descriptor().expect("descriptor")
    else {
        panic!("EVM contract context must require context");
    };
    let json = serde_json::to_string(&value).expect("context json");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical context json");
    let context_ref = mfm_program::CertifiedContextSpec::derive_context_ref(
        &requirement.context_descriptor_id,
        &requirement.schema_id,
        &requirement.semantic_type_id,
        &requirement.canonicalizer_identity,
        &canonical,
    )
    .expect("context ref");
    let spec = mfm_program::CertifiedContextSpec {
        context_ref,
        context_descriptor_id: requirement.context_descriptor_id,
        schema_id: requirement.schema_id,
        semantic_type_id: requirement.semantic_type_id,
        canonicalizer_identity: requirement.canonicalizer_identity,
        canonical_context_digest: canonical.content_digest(),
        canonical_context_byte_len: canonical.as_bytes().len() as u64,
        canonical_context: canonical,
    };
    mfm_program::CertifiedContext::from_certified_spec(&spec).expect("certified context")
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

fn deployed_instance(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> DeployedContractInstance {
    DeployedContractInstance {
        lifecycle_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        address: ContractAddress::new("0x000000000000000000000000000000000000beef")
            .expect("address"),
        deploy_provenance: DeployProvenance::MfmDeploy {
            deploy_tx_hash: "0x01".to_owned(),
        },
        deploy_evidence: Vec::new(),
        deployed_block_number: Some(1),
    }
}

fn configured_instance(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> ConfiguredContractInstance {
    let deployed = deployed_instance(context);
    ConfiguredContractInstance {
        lifecycle_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        address: deployed.address.clone(),
        configured_from: ConfiguredFrom {
            deployed_context_ref: ContextRefValue::from(context.context_ref().clone()),
            deployed_address: deployed.address,
        },
        configuration_claim: ConfigurationClaim::MfmConfigured {
            configure_node: LifecycleNodeIdRef::from(node_id(0x41)),
            configure_action_digest: ContractProfileDigestRef::from(ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_with(0x42),
            )),
            call_evidence_refs: Vec::new(),
            confirmation_evidence_refs: Vec::new(),
        },
        configure_or_import_evidence: Vec::new(),
        configured_block_number: Some(2),
        asserted_configuration_snapshot: None,
    }
}

#[test]
fn state_schemas_and_names_use_contract_lifecycle_namespace() {
    let state_names = [
        DeployContractState::name(),
        ConfigureContractState::name(),
        ValidateContractState::name(),
        ProjectConfiguredContractRefState::name(),
        ContextBoundDeployContractState::name(),
        ContextBoundConfigureContractState::name(),
        ContextBoundValidateContractState::name(),
        ImportDeployedContractState::name(),
        ImportConfiguredContractState::name(),
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
        ContextContractTransactionIntent::schema_id()
            .expect("schema")
            .to_string(),
        ContextContractDeployIntent::schema_id()
            .expect("schema")
            .to_string(),
        ContextContractConfigureIntent::schema_id()
            .expect("schema")
            .to_string(),
        ContextContractConfigureReceipt::schema_id()
            .expect("schema")
            .to_string(),
        ContextContractConfigureConfirmation::schema_id()
            .expect("schema")
            .to_string(),
        ContextContractValidationReadRequest::schema_id()
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
        ContextBoundDeployContractState::adapter_bindings().expect("bindings"),
        ContextBoundConfigureContractState::adapter_bindings().expect("bindings"),
        ContextBoundValidateContractState::adapter_bindings().expect("bindings"),
        ImportDeployedContractState::adapter_bindings().expect("bindings"),
        ImportConfiguredContractState::adapter_bindings().expect("bindings"),
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
    <ContractImportReadCaps as CapabilitySet>::descriptor()
        .expect("import caps")
        .validate_for_effect::<ReadExternal>()
        .expect("import caps");
}

#[test]
fn context_bound_state_descriptors_certify_resource_contracts() {
    let deploy_id =
        descriptor_id_for_state::<ContextBoundDeployContractState>().expect("deploy descriptor");
    let import_deployed_id =
        descriptor_id_for_state::<ImportDeployedContractState>().expect("import descriptor");
    let configure_id = descriptor_id_for_state::<ContextBoundConfigureContractState>()
        .expect("configure descriptor");
    let import_configured_id =
        descriptor_id_for_state::<ImportConfiguredContractState>().expect("import descriptor");

    let output = ContextBoundDeployContractState::output_context_contract().expect("output");
    assert!(matches!(
        output,
        mfm_program::StateOutputContextContractSpec::Produces {
            ref resource_kind,
            ref stage,
        } if resource_kind == contract_instance_resource_kind()
            && stage == deployed_contract_stage()
    ));

    let input = ContextBoundConfigureContractState::input_context_contract().expect("input");
    let expected = sorted_descriptor_ids(vec![deploy_id, import_deployed_id]);
    assert!(matches!(
        input,
        mfm_program::StateInputContextContractSpec::Required {
            ref resource_kind,
            ref stage,
            ref producer,
        } if resource_kind == contract_instance_resource_kind()
            && stage == deployed_contract_stage()
            && producer.producer_descriptor_ids.as_slice() == expected.as_slice()
            && !producer.seed_producers_allowed
    ));

    let input = ContextBoundValidateContractState::input_context_contract().expect("input");
    let expected = sorted_descriptor_ids(vec![configure_id, import_configured_id]);
    assert!(matches!(
        input,
        mfm_program::StateInputContextContractSpec::Required {
            ref resource_kind,
            ref stage,
            ref producer,
        } if resource_kind == contract_instance_resource_kind()
            && stage == configured_contract_stage()
            && producer.producer_descriptor_ids.as_slice() == expected.as_slice()
            && !producer.seed_producers_allowed
    ));
}

#[test]
fn context_deploy_intent_and_output_use_certified_context() {
    let state =
        ContextBoundDeployContractState::new(validated_config(deploy_action())).expect("state");
    let context = certified_contract_context();
    let intent = state.prepare_intent(&(), &context).expect("intent");

    assert_eq!(
        intent.transaction.context_ref.as_context_ref(),
        context.context_ref()
    );
    assert_eq!(intent.transaction.network_id, "ethereum-mainnet");
    assert_eq!(intent.transaction.expected_chain_id, 1);
    assert_eq!(intent.constructor_args_len, 0);
    assert_eq!(intent.contract_profile_id, "example-profile");

    let output = state
        .output_from_receipt(
            &(),
            &intent,
            &ContractDeployReceipt {
                receipt_version: 1,
                contract_address: "0X000000000000000000000000000000000000BEEF".to_owned(),
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

    assert_eq!(output.context_ref(), context.context_ref());
    assert_eq!(
        output.address.as_str(),
        "0x000000000000000000000000000000000000beef"
    );
    assert!(matches!(
        output.deploy_provenance,
        DeployProvenance::MfmDeploy { .. }
    ));
}

#[test]
fn context_configure_intent_and_output_use_context_bound_input() {
    let state = ContextBoundConfigureContractState::new(validated_config(configure_action()))
        .expect("state");
    let context = certified_contract_context();
    let input = ContextConfigureContractInput {
        deployed: deployed_instance(&context),
    };
    let intent = state.prepare_intent(&input, &context).expect("intent");

    assert_eq!(intent.transactions.len(), 1);
    assert_eq!(
        intent.transactions[0].context_ref.as_context_ref(),
        context.context_ref()
    );
    assert_eq!(
        intent.transactions[0].to_address.as_deref(),
        Some("0x000000000000000000000000000000000000beef")
    );

    let output = state
        .output_from_receipt(
            &input,
            &intent,
            &ContextContractConfigureReceipt {
                receipt_version: 1,
                configure_node: LifecycleNodeIdRef::from(node_id(0x51)),
                receipts: vec![ContractTransactionReceipt {
                    receipt_version: 1,
                    transaction_hash: "0x02".to_owned(),
                    block_number: 4,
                    status: true,
                    receipt_evidence: None,
                }],
                call_evidence_refs: Vec::new(),
                confirmation_evidence_refs: Vec::new(),
                configured_block_number: None,
            },
            &context,
        )
        .expect("configured");

    assert_eq!(output.context_ref(), context.context_ref());
    assert_eq!(
        output.configured_from.deployed_context_ref.as_context_ref(),
        context.context_ref()
    );
    assert_eq!(output.configured_block_number, Some(4));
    assert!(matches!(
        output.configuration_claim,
        ConfigurationClaim::MfmConfigured { .. }
    ));
}

#[test]
fn context_validate_request_and_report_use_certified_context() {
    let state =
        ContextBoundValidateContractState::new(validated_config(validate_action())).expect("state");
    let context = certified_contract_context();
    let input = ContextValidateContractInput {
        configured: configured_instance(&context),
    };
    let request = state.read_request(&input, &context).expect("request");

    assert_eq!(request.context_ref.as_context_ref(), context.context_ref());
    assert_eq!(request.network_id, "ethereum-mainnet");
    assert_eq!(request.expected_chain_id, 1);

    let report = state
        .report_from_response(
            &input,
            ContractValidationReadResponse {
                response_version: 1,
                observed_chain_id: 2,
                client_version: "redacted-client".to_owned(),
                configuration_read_results: Vec::new(),
                configuration_event_results: Vec::new(),
                read_results: Vec::new(),
                event_results: Vec::new(),
            },
            &context,
        )
        .expect("report");

    assert_eq!(report.context_ref(), context.context_ref());
    assert!(!report.valid);
}

#[test]
fn import_states_fail_closed_without_verified_evidence() {
    let deployed = ImportDeployedContractState::new(validated_config(import_deployed_spec()))
        .expect("deployed import state");
    let configured = ImportConfiguredContractState::new(validated_config(import_configured_spec()))
        .expect("configured import state");

    assert!(deployed.reject_without_verified_evidence().is_err());
    assert!(configured.reject_without_verified_evidence().is_err());
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
