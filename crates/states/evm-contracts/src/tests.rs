use super::*;
use mfm_capabilities::CapabilitySet;
use mfm_evm_contract_model::{
    BlockSelector, EventName, ExpectedValue, ExternalAdoptionEvidencePolicy, FunctionName,
};
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EventId, NodeId,
    RunId, SpecHash,
};
use mfm_program::StateContext;
use mfm_values::{ContextBoundOutput, MfmValue};

fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest_str(byte: u8) -> String {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn artifact_id_str(byte: u8) -> String {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn lifecycle_artifact_ref(byte: u8) -> LifecycleArtifactEvidenceRef {
    let digest = ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte));
    let evidence_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(byte.wrapping_add(1)),
    );
    LifecycleArtifactEvidenceRef::new(
        ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        evidence_hash,
        32,
        None,
        None,
    )
}

fn contract_profile_digest_ref(byte: u8) -> ContractProfileDigestRef {
    ContractProfileDigestRef::from(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(byte),
    ))
}

fn cell_id_str(byte: u8) -> String {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn descriptor_id_str(byte: u8) -> String {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn event_id_str(byte: u8) -> String {
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn run_id_str(byte: u8) -> String {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn spec_hash_str(byte: u8) -> String {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn validated_config<T: mfm_values::MfmConfig>(config: T) -> mfm_program::ValidatedConfig<T> {
    mfm_program::ValidatedConfig::new(config).expect("valid config")
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

fn import_configured_claimed_spec() -> ImportConfiguredSpec {
    serde_json::from_value(serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000beef",
            "provenance_label": "audited-external",
            "evidence_policy": {
                "allow_external_claimed_configured": true
            }
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

fn test_evm_network_context_ref(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> String {
    digest_for_config(&context.value().network)
        .expect("network digest")
        .to_string()
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
        external_adoption_evidence: None,
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
        external_adoption_evidence: None,
        configured_block_number: Some(2),
        asserted_configuration_snapshot: None,
    }
}

fn external_adoption_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    policy: &mfm_evm_contract_model::ExternalAdoptionEvidencePolicy,
    stage: ContractLifecycleStage,
) -> ExternalAdoptionEvidence {
    ExternalAdoptionEvidence {
        evidence_policy_digest: digest_for_config(policy).expect("policy digest"),
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        evm_network_context_ref: test_evm_network_context_ref(context),
        resource_stage: stage,
        observed_chain_id: context.value().network.expected_chain_id(),
        code_read_evidence: Some(mfm_evm_contract_model::ExternalCodeReadEvidence {
            address: ContractAddress::new("0x000000000000000000000000000000000000beef")
                .expect("address"),
            block: mfm_evm_contract_model::BlockSelector::Tag {
                tag: mfm_evm_contract_model::BlockTag::Latest,
            },
            source: mfm_evm_contract_model::ExternalEvmSourceEvidence {
                network_id: context.value().network.network_id.to_string(),
                expected_chain_id: context.value().network.expected_chain_id(),
                observed_chain_id: context.value().network.expected_chain_id(),
                source_ref: "test-source".to_owned(),
                policy_id: "test-policy".to_owned(),
            },
            observed_code_hash: mfm_evm_contract_model::EvmCodeHash::new(format!(
                "0x{}",
                "11".repeat(32)
            ))
            .expect("code hash"),
            observed_code_byte_len: 1,
        }),
        read_assertion_evidence: Vec::new(),
        event_assertion_evidence: Vec::new(),
    }
}

fn expected_value(value: serde_json::Value) -> ExpectedValue {
    ExpectedValue::from_json_value(&value).expect("expected value")
}

fn read_assertion(function: &str, expected: ExpectedValue) -> ReadAssertionConfig {
    ReadAssertionConfig {
        function: FunctionName::new(function).expect("function"),
        args: Vec::new(),
        expected,
    }
}

fn event_assertion(
    event: &str,
    min_count: u64,
    from_block: Option<BlockSelector>,
) -> EventAssertionConfig {
    EventAssertionConfig {
        event: EventName::new(event).expect("event"),
        min_count,
        from_block,
        to_block: None,
    }
}

fn external_adoption_policy(
    reads: Vec<ReadAssertionConfig>,
    events: Vec<EventAssertionConfig>,
) -> ExternalAdoptionEvidencePolicy {
    ExternalAdoptionEvidencePolicy {
        initial_read_assertions: reads,
        initial_event_assertions: events,
        ..Default::default()
    }
}

fn import_configured_with_policy(policy: &ExternalAdoptionEvidencePolicy) -> ImportConfiguredSpec {
    serde_json::from_value(serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000beef",
            "provenance_label": "audited-external",
            "evidence_policy": policy,
        },
    }))
    .expect("import configured")
}

fn external_source_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> ExternalEvmSourceEvidence {
    ExternalEvmSourceEvidence {
        network_id: context.value().network.network_id.to_string(),
        expected_chain_id: context.value().network.expected_chain_id(),
        observed_chain_id: context.value().network.expected_chain_id(),
        source_ref: "test-source".to_owned(),
        policy_id: "test-policy".to_owned(),
    }
}

fn external_read_assertion_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    function: &str,
    expected: ExpectedValue,
    actual: ExpectedValue,
    passed: bool,
) -> ExternalReadAssertionEvidence {
    ExternalReadAssertionEvidence {
        source: external_source_evidence(context),
        result: ValidationReadResult {
            function: function.to_owned(),
            args: Vec::new(),
            expected,
            actual,
            passed,
        },
    }
}

fn external_event_assertion_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    event: &str,
    min_count: u64,
    observed_count: u64,
    passed: bool,
) -> ExternalEventAssertionEvidence {
    ExternalEventAssertionEvidence {
        source: external_source_evidence(context),
        result: ValidationEventResult {
            event: event.to_owned(),
            min_count,
            observed_count,
            passed,
        },
    }
}

fn snapshot_from_external_evidence(evidence: &ExternalAdoptionEvidence) -> ConfigurationSnapshot {
    ConfigurationSnapshot {
        read_results: evidence
            .read_assertion_evidence
            .iter()
            .map(|record| record.result.clone())
            .collect(),
        event_results: evidence
            .event_assertion_evidence
            .iter()
            .map(|record| record.result.clone())
            .collect(),
    }
}

fn validation_read_response(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    input: &ContextValidateContractInput,
) -> ContractValidationReadResponse {
    ContractValidationReadResponse {
        response_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        configured_input_digest: digest_for_config(&input.configured).expect("configured digest"),
        evm_network_context_ref: test_evm_network_context_ref(context),
        resource_stage: ContractLifecycleStage::Configured,
        observed_chain_id: context.value().network.expected_chain_id(),
        client_version: "redacted-client".to_owned(),
        configuration_read_results: Vec::new(),
        configuration_event_results: Vec::new(),
        read_results: Vec::new(),
        event_results: Vec::new(),
        validation_read_evidence: Vec::new(),
        validation_event_evidence: Vec::new(),
        evidence_refs: input.configured.configure_or_import_evidence.clone(),
    }
}

fn artifact_ref_json(
    byte: u8,
    content_digest: serde_json::Value,
    schema_id: Option<String>,
    semantic_type_id: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": artifact_id_str(byte),
        "content_digest": content_digest,
        "evidence_hash": ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_with(byte.wrapping_add(0x80))
        )
        .to_string(),
        "byte_len": 128,
        "schema_id": schema_id,
        "semantic_type_id": semantic_type_id,
    })
}

fn source_run_import_spec_json(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: &str,
    value_digest: &ContractProfileDigestRef,
) -> serde_json::Value {
    serde_json::json!({
        "source_run_id": run_id_str(0x30),
        "source_spec_hash": spec_hash_str(0x31),
        "source_cell_or_output_id": {
            "kind": "cell",
            "cell_id": cell_id_str(0x32),
        },
        "source_value_digest": value_digest,
        "source_context_ref": context.context_ref().to_string(),
        "required_stage": required_stage,
    })
}

fn source_run_import_evidence_json<T>(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    source: &ImportFromMfmRun,
    required_stage: &str,
    import_policy_digest: &ContractProfileDigestRef,
) -> serde_json::Value
where
    T: MfmValue,
{
    let schema_id = T::schema_id().expect("schema").to_string();
    let semantic_type_id = T::semantic_id().expect("semantic").to_string();
    serde_json::json!({
        "source_spec_hash": &source.source_spec_hash,
        "source_spec_artifact_ref": artifact_ref_json(
            0x4e,
            serde_json::json!(content_digest_str(0x4f)),
            None,
            None,
        ),
        "source_spec_certificate_ref": artifact_ref_json(
            0x50,
            serde_json::json!(content_digest_str(0x51)),
            None,
            None,
        ),
        "source_run_stream_ref": artifact_ref_json(
            0x52,
            serde_json::json!(content_digest_str(0x53)),
            None,
            None,
        ),
        "source_cell_or_output_id": &source.source_cell_or_output_id,
        "source_cell_schema_id": schema_id,
        "source_cell_semantic_type_id": semantic_type_id,
        "source_producer_descriptor_id": descriptor_id_str(0x54),
        "source_stage": required_stage,
        "source_context_ref": &source.source_context_ref,
        "source_context_descriptor_id": context.context_descriptor_id().to_string(),
        "source_value_digest": &source.source_value_digest,
        "source_value_artifact_ref_or_inline_canonical_value": artifact_ref_json(
            0x56,
            serde_json::to_value(&source.source_value_digest).expect("digest json"),
            Some(schema_id),
            Some(semantic_type_id),
        ),
        "source_terminal_cell_or_output_event_ref": event_id_str(0x57),
        "import_policy_digest": import_policy_digest,
    })
}

fn source_run_import_json<T>(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: &str,
    value_digest: &ContractProfileDigestRef,
) -> serde_json::Value
where
    T: MfmValue,
{
    let source_json = source_run_import_spec_json(context, required_stage, value_digest);
    let source: ImportFromMfmRun = serde_json::from_value(source_json).expect("source");
    let import_policy_digest = digest_for_config(&source).expect("policy digest");
    let evidence_json = source_run_import_evidence_json::<T>(
        context,
        &source,
        required_stage,
        &import_policy_digest,
    );
    serde_json::json!({
        "kind": "from_mfm_run",
        "source": source,
        "evidence": evidence_json,
    })
}

#[test]
fn state_schemas_and_names_use_contract_lifecycle_namespace() {
    let state_names = [
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
        ContractDeployReceipt::schema_id()
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
                context_ref: ContextRefValue::from(context.context_ref().clone()),
                evm_network_context_ref: test_evm_network_context_ref(&context),
                resource_stage: ContractLifecycleStage::Deployed,
                contract_address: "0X000000000000000000000000000000000000BEEF".to_owned(),
                receipt: ContractTransactionReceipt {
                    receipt_version: 1,
                    context_ref: ContextRefValue::from(context.context_ref().clone()),
                    evm_network_context_ref: test_evm_network_context_ref(&context),
                    resource_stage: ContractLifecycleStage::Deployed,
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
                context_ref: ContextRefValue::from(context.context_ref().clone()),
                evm_network_context_ref: test_evm_network_context_ref(&context),
                resource_stage: ContractLifecycleStage::Configured,
                configure_node: LifecycleNodeIdRef::from(node_id(0x51)),
                receipts: vec![ContractTransactionReceipt {
                    receipt_version: 1,
                    context_ref: ContextRefValue::from(context.context_ref().clone()),
                    evm_network_context_ref: test_evm_network_context_ref(&context),
                    resource_stage: ContractLifecycleStage::Configured,
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
    let evidence_ref = lifecycle_artifact_ref(0x88);
    let mut configured = configured_instance(&context);
    configured.configure_or_import_evidence = vec![evidence_ref.clone()];
    let input = ContextValidateContractInput { configured };
    let request = state.read_request(&input, &context).expect("request");

    assert_eq!(request.context_ref.as_context_ref(), context.context_ref());
    assert_eq!(
        request.configured_input_digest,
        digest_for_config(&input.configured).expect("configured digest")
    );
    assert_eq!(request.network_id, "ethereum-mainnet");
    assert_eq!(request.expected_chain_id, 1);

    let mut response = validation_read_response(&context, &input);
    response.observed_chain_id = 2;
    let report = state
        .report_from_response(&input, response, &context)
        .expect("report");

    assert_eq!(report.context_ref(), context.context_ref());
    assert!(!report.valid);
    assert_eq!(report.evidence_refs, vec![evidence_ref]);
}

#[test]
fn context_validate_report_rejects_invalid_response_shapes() {
    enum InvalidResponseShape {
        ConfiguredInputDigestMismatch,
        ForgedReadPassedFlag,
        ForgedEventPassedFlag,
    }

    let state =
        ContextBoundValidateContractState::new(validated_config(validate_action())).expect("state");
    let context = certified_contract_context();
    let input = ContextValidateContractInput {
        configured: configured_instance(&context),
    };

    for (name, shape, expected_fragment) in [
        (
            "configured input digest mismatch",
            InvalidResponseShape::ConfiguredInputDigestMismatch,
            Some("validation read response"),
        ),
        (
            "forged read passed flag",
            InvalidResponseShape::ForgedReadPassedFlag,
            None,
        ),
        (
            "forged event passed flag",
            InvalidResponseShape::ForgedEventPassedFlag,
            None,
        ),
    ] {
        let mut response = validation_read_response(&context, &input);

        match shape {
            InvalidResponseShape::ConfiguredInputDigestMismatch => {
                response.configured_input_digest = contract_profile_digest_ref(0x99);
            }
            InvalidResponseShape::ForgedReadPassedFlag => {
                response.read_results = vec![ValidationReadResult {
                    function: "isConfigured".to_owned(),
                    args: Vec::new(),
                    expected: expected_value(serde_json::json!(true)),
                    actual: expected_value(serde_json::json!(false)),
                    passed: true,
                }];
            }
            InvalidResponseShape::ForgedEventPassedFlag => {
                response.event_results = vec![ValidationEventResult {
                    event: "Configured".to_owned(),
                    min_count: 2,
                    observed_count: 1,
                    passed: true,
                }];
            }
        }

        let error = state
            .report_from_response(&input, response, &context)
            .expect_err(name);
        if let Some(expected_fragment) = expected_fragment {
            assert!(error.to_string().contains(expected_fragment), "{name}");
        }
    }
}

#[test]
fn import_deployed_admits_verified_source_run_evidence() {
    let context = certified_contract_context();
    let source_value = deployed_instance(&context);
    let value_digest = digest_for_config(&source_value).expect("value digest");
    let import: ImportDeployedSpec = serde_json::from_value(source_run_import_json::<
        DeployedContractInstance,
    >(
        &context, "deployed", &value_digest
    ))
    .expect("source-run import");

    let admitted =
        ImportDeployedContractState::admit_verified_mfm_run_import(&import, source_value, &context)
            .expect("admitted");

    assert_eq!(admitted.context_ref(), context.context_ref());
    assert_eq!(admitted.deploy_evidence.len(), 4);
    assert!(matches!(
        admitted.deploy_provenance,
        DeployProvenance::ImportedMfmRun { .. }
    ));
}

#[test]
fn import_configured_admits_verified_source_run_evidence() {
    let context = certified_contract_context();
    let source_value = configured_instance(&context);
    let value_digest = digest_for_config(&source_value).expect("value digest");
    let import: ImportConfiguredSpec =
        serde_json::from_value(source_run_import_json::<ConfiguredContractInstance>(
            &context,
            "configured",
            &value_digest,
        ))
        .expect("source-run import");

    let admitted = ImportConfiguredContractState::admit_verified_mfm_run_import(
        &import,
        source_value,
        &context,
    )
    .expect("admitted");

    assert_eq!(admitted.context_ref(), context.context_ref());
    assert_eq!(admitted.configure_or_import_evidence.len(), 4);
    assert!(matches!(
        admitted.configuration_claim,
        ConfigurationClaim::ImportedMfmConfigured { .. }
    ));
}

#[test]
fn import_admission_rejects_source_value_digest_mismatch() {
    let context = certified_contract_context();
    let source_value = deployed_instance(&context);
    let value_digest = digest_for_config(&source_value).expect("value digest");
    let import: ImportDeployedSpec = serde_json::from_value(source_run_import_json::<
        DeployedContractInstance,
    >(
        &context, "deployed", &value_digest
    ))
    .expect("source-run import");
    let mut mismatched = source_value;
    mismatched.deployed_block_number = Some(99);

    let error =
        ImportDeployedContractState::admit_verified_mfm_run_import(&import, mismatched, &context)
            .expect_err("digest mismatch rejected");

    assert!(error.to_string().contains("source-run import"));
}

#[test]
fn import_configured_external_claim_requires_certified_policy() {
    let context = certified_contract_context();
    let rejected_import = import_configured_spec();
    let rejected_policy = match &rejected_import {
        ImportConfiguredSpec::AdoptExternalAddress { adoption } => &adoption.evidence_policy,
        _ => panic!("external adoption import expected"),
    };
    let rejected = ImportConfiguredContractState::admit_verified_external_adoption(
        &rejected_import,
        &context,
        external_adoption_evidence(
            &context,
            rejected_policy,
            ContractLifecycleStage::Configured,
        ),
        None,
        None,
    );
    assert!(rejected.is_err());

    let admitted_import = import_configured_claimed_spec();
    let admitted_policy = match &admitted_import {
        ImportConfiguredSpec::AdoptExternalAddress { adoption } => &adoption.evidence_policy,
        _ => panic!("external adoption import expected"),
    };
    let admitted = ImportConfiguredContractState::admit_verified_external_adoption(
        &admitted_import,
        &context,
        external_adoption_evidence(
            &context,
            admitted_policy,
            ContractLifecycleStage::Configured,
        ),
        None,
        None,
    )
    .expect("claimed configured policy admits");

    assert!(matches!(
        admitted.configuration_claim,
        ConfigurationClaim::ExternalClaimedConfigured { .. }
    ));
    assert_eq!(admitted.context_ref(), context.context_ref());
}

#[test]
fn external_adoption_rejects_forged_assertion_passed_flags() {
    let context = certified_contract_context();
    let expected_true = expected_value(serde_json::json!(true));
    let actual_false = expected_value(serde_json::json!(false));
    let read_policy = external_adoption_policy(
        vec![read_assertion("isConfigured", expected_true.clone())],
        Vec::new(),
    );
    let read_import = import_configured_with_policy(&read_policy);
    let mut read_evidence =
        external_adoption_evidence(&context, &read_policy, ContractLifecycleStage::Configured);
    read_evidence.read_assertion_evidence = vec![external_read_assertion_evidence(
        &context,
        "isConfigured",
        expected_true,
        actual_false,
        true,
    )];
    let read_snapshot = snapshot_from_external_evidence(&read_evidence);

    assert!(
        ImportConfiguredContractState::admit_verified_external_adoption(
            &read_import,
            &context,
            read_evidence,
            Some(read_snapshot),
            None,
        )
        .is_err()
    );

    let event_policy =
        external_adoption_policy(Vec::new(), vec![event_assertion("Configured", 2, None)]);
    let event_import = import_configured_with_policy(&event_policy);
    let mut event_evidence =
        external_adoption_evidence(&context, &event_policy, ContractLifecycleStage::Configured);
    event_evidence.event_assertion_evidence = vec![external_event_assertion_evidence(
        &context,
        "Configured",
        2,
        1,
        true,
    )];
    let event_snapshot = snapshot_from_external_evidence(&event_evidence);

    assert!(
        ImportConfiguredContractState::admit_verified_external_adoption(
            &event_import,
            &context,
            event_evidence,
            Some(event_snapshot),
            None,
        )
        .is_err()
    );
}

#[test]
fn external_adoption_rejects_mismatched_assertion_content() {
    enum AssertionMismatch {
        Read,
        Event,
    }

    let context = certified_contract_context();

    for (name, mismatch) in [
        ("read assertion", AssertionMismatch::Read),
        ("event assertion", AssertionMismatch::Event),
    ] {
        let (import, evidence, snapshot) = match mismatch {
            AssertionMismatch::Read => {
                let expected_true = expected_value(serde_json::json!(true));
                let policy = external_adoption_policy(
                    vec![read_assertion("isConfigured", expected_true.clone())],
                    Vec::new(),
                );
                let import = import_configured_with_policy(&policy);
                let mut evidence = external_adoption_evidence(
                    &context,
                    &policy,
                    ContractLifecycleStage::Configured,
                );
                evidence.read_assertion_evidence = vec![external_read_assertion_evidence(
                    &context,
                    "isAdmin",
                    expected_true.clone(),
                    expected_true,
                    true,
                )];
                let snapshot = snapshot_from_external_evidence(&evidence);
                (import, evidence, snapshot)
            }
            AssertionMismatch::Event => {
                let policy = external_adoption_policy(
                    Vec::new(),
                    vec![event_assertion("Configured", 1, None)],
                );
                let import = import_configured_with_policy(&policy);
                let mut evidence = external_adoption_evidence(
                    &context,
                    &policy,
                    ContractLifecycleStage::Configured,
                );
                evidence.event_assertion_evidence = vec![external_event_assertion_evidence(
                    &context, "Upgraded", 1, 1, true,
                )];
                let snapshot = snapshot_from_external_evidence(&evidence);
                (import, evidence, snapshot)
            }
        };

        let error = ImportConfiguredContractState::admit_verified_external_adoption(
            &import,
            &context,
            evidence,
            Some(snapshot),
            None,
        )
        .expect_err(name);

        assert!(error.to_string().contains("external adoption"), "{name}");
    }
}

#[test]
fn external_adoption_event_assertion_with_bounds_fails_closed() {
    let context = certified_contract_context();
    let policy = external_adoption_policy(
        Vec::new(),
        vec![event_assertion(
            "Configured",
            1,
            Some(BlockSelector::Number { number: 1 }),
        )],
    );
    let import = import_configured_with_policy(&policy);
    let mut evidence =
        external_adoption_evidence(&context, &policy, ContractLifecycleStage::Configured);
    evidence.event_assertion_evidence = vec![external_event_assertion_evidence(
        &context,
        "Configured",
        1,
        1,
        true,
    )];
    let snapshot = snapshot_from_external_evidence(&evidence);

    let error = ImportConfiguredContractState::admit_verified_external_adoption(
        &import,
        &context,
        evidence,
        Some(snapshot),
        None,
    )
    .expect_err("bounded event assertion rejected");

    assert!(error.to_string().contains("external adoption"));
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
