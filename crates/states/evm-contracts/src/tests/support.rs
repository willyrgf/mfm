use super::*;
use mfm_evm_contract_model::ImportFromMfmRun;
pub(super) fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

pub(super) fn content_digest_str(byte: u8) -> String {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn artifact_id_str(byte: u8) -> String {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn lifecycle_artifact_ref(byte: u8) -> LifecycleArtifactEvidenceRef {
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

pub(super) fn contract_profile_digest_ref(byte: u8) -> ContractProfileDigestRef {
    ContractProfileDigestRef::from(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(byte),
    ))
}

pub(super) fn cell_id_str(byte: u8) -> String {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn descriptor_id_str(byte: u8) -> String {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn event_id_str(byte: u8) -> String {
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn run_id_str(byte: u8) -> String {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn spec_hash_str(byte: u8) -> String {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

pub(super) fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

pub(super) fn validated_config<T: mfm_values::MfmConfig>(
    config: T,
) -> mfm_program::ValidatedConfig<T> {
    mfm_program::ValidatedConfig::new(config).expect("valid config")
}

pub(super) fn signer_json() -> serde_json::Value {
    serde_json::json!({
        "signer_ref": "deployer",
        "expected_signer_address": "0x000000000000000000000000000000000000dead",
    })
}

pub(super) fn context_json() -> serde_json::Value {
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

pub(super) fn deploy_action() -> DeployAction {
    serde_json::from_value(serde_json::json!({
        "signer": signer_json(),
    }))
    .expect("deploy action")
}

pub(super) fn configure_action() -> ConfigureAction {
    serde_json::from_value(serde_json::json!({
        "signer": signer_json(),
        "calls": [{
            "function": "configure",
            "args": [],
        }],
    }))
    .expect("configure action")
}

pub(super) fn validate_action() -> ValidateAction {
    serde_json::from_value(serde_json::json!({})).expect("validate action")
}

pub(super) fn import_configured_spec() -> ImportConfiguredSpec {
    serde_json::from_value(serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000beef",
            "provenance_label": "audited-external",
        },
    }))
    .expect("import configured")
}

pub(super) fn import_configured_claimed_spec() -> ImportConfiguredSpec {
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

pub(super) fn certified_contract_context() -> mfm_program::CertifiedContext<EvmContractContext> {
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

pub(super) fn test_evm_network_context_ref(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> String {
    digest_for_config(&context.value().network)
        .expect("network digest")
        .to_string()
}

pub(super) fn deployed_instance(
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

pub(super) fn configured_instance(
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

pub(super) fn external_adoption_evidence(
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

pub(super) fn expected_value(value: serde_json::Value) -> ExpectedValue {
    ExpectedValue::from_json_value(&value).expect("expected value")
}

pub(super) fn read_assertion(function: &str, expected: ExpectedValue) -> ReadAssertionConfig {
    ReadAssertionConfig {
        function: FunctionName::new(function).expect("function"),
        args: Vec::new(),
        expected,
    }
}

pub(super) fn event_assertion(
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

pub(super) fn external_adoption_policy(
    reads: Vec<ReadAssertionConfig>,
    events: Vec<EventAssertionConfig>,
) -> ExternalAdoptionEvidencePolicy {
    ExternalAdoptionEvidencePolicy {
        initial_read_assertions: reads,
        initial_event_assertions: events,
        ..Default::default()
    }
}

pub(super) fn import_configured_with_policy(
    policy: &ExternalAdoptionEvidencePolicy,
) -> ImportConfiguredSpec {
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

pub(super) fn external_source_evidence(
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

pub(super) fn external_read_assertion_evidence(
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

pub(super) fn external_event_assertion_evidence(
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

pub(super) fn snapshot_from_external_evidence(
    evidence: &ExternalAdoptionEvidence,
) -> ConfigurationSnapshot {
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

pub(super) fn validation_read_response(
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

pub(super) fn artifact_ref_json(
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

pub(super) fn source_run_import_spec_json(
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

pub(super) fn source_run_import_evidence_json<T>(
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

pub(super) fn source_run_import_json<T>(
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
