use mfm_evm_contract_model::*;
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, ContextRef, DigestAlgorithm, DigestBytes, NodeId, RunId,
    SchemaId, SemanticTypeId, SpecHash,
};
use mfm_program::StateContext;
use mfm_spec::v1::{CertifiedContextSpec, StateContextDescriptorSpec};
use mfm_values::{ContextBoundOutput, ContextRefValue, MfmValue};

const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn digest() -> DigestBytes {
    DigestBytes::from_hex(DIGEST_HEX).expect("digest")
}

fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn context_ref(byte: u8) -> ContextRef {
    ContextRef::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn cell_id(byte: u8) -> CellId {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn spec_hash(byte: u8) -> SpecHash {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn profile_digest(byte: u8) -> ContractProfileDigestRef {
    ContractProfileDigestRef::from(content_digest(byte))
}

fn contract_profile() -> ContractProfile {
    ContractProfile {
        profile_id: ContractProfileId::new("example-profile").expect("profile"),
        artifact_digest: Some(profile_digest(0x20)),
        interface_digest: Some(profile_digest(0x21)),
        creation_bytecode_digest: Some(profile_digest(0x22)),
        deployed_code_hash: Some(EvmCodeHash::new(format!("0x{}", "11".repeat(32))).expect("hash")),
        selector_event_policy_digest: Some(profile_digest(0x23)),
    }
}

fn contract_context(chain_id: u64) -> EvmContractContext {
    EvmContractContext {
        lifecycle_key: LifecycleKey::new("example-lifecycle").expect("lifecycle"),
        network: EvmNetworkContext::new(
            EvmNetworkId::new("ethereum-mainnet").expect("network"),
            chain_id,
        )
        .expect("network context"),
        contract_profile: contract_profile(),
    }
}

fn derived_context_ref(context: &EvmContractContext) -> ContextRef {
    let StateContextDescriptorSpec::Required(requirement) =
        <EvmContractContext as StateContext>::descriptor().expect("descriptor")
    else {
        panic!("contract context must require context");
    };
    let json = serde_json::to_string(context).expect("json");
    let canonical =
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical");
    CertifiedContextSpec::derive_context_ref(
        &requirement.context_descriptor_id,
        &requirement.schema_id,
        &requirement.semantic_type_id,
        &requirement.canonicalizer_identity,
        &canonical,
    )
    .expect("context ref")
}

fn configured_claim() -> ConfigurationClaim {
    ConfigurationClaim::MfmConfigured {
        configure_node: LifecycleNodeIdRef::from(node_id(0x30)),
        configure_action_digest: profile_digest(0x31),
        call_evidence_refs: Vec::new(),
        confirmation_evidence_refs: Vec::new(),
    }
}

#[test]
fn expected_matches_normalizes_hex_wrappers() {
    let actual = ExpectedValue::from_json_value(&serde_json::json!("0xAA")).expect("actual");
    let expected = ExpectedValue::from_json_value(&serde_json::json!("0xaa")).expect("expected");

    assert!(expected_matches(&actual, &expected));
}

#[test]
fn json_wrappers_reject_non_canonical_float_values() {
    assert!(AbiArgumentValue::from_json_value(&serde_json::json!(1.5)).is_err());
    assert!(ExpectedValue::from_json_value(&serde_json::json!(1.5)).is_err());
}

#[test]
fn typed_json_wrappers_have_no_unchecked_value_conversion() {
    let source = include_str!("../src/lib.rs");

    assert!(!source.contains(&["pub ", "json_text"].concat()));
    for forbidden in [
        ["impl ", "TryFrom", "<Value> for ", "Abi", "Json"].concat(),
        ["impl ", "TryFrom", "<Value> for ", "Bytecode", "Json"].concat(),
        [
            "impl ",
            "TryFrom",
            "<Value> for ",
            "Abi",
            "Argument",
            "Value",
        ]
        .concat(),
        ["impl ", "TryFrom", "<Value> for ", "Expected", "Value"].concat(),
        ["impl ", "From", "<Value> for ", "Abi", "Json"].concat(),
        ["impl ", "From", "<Value> for ", "Bytecode", "Json"].concat(),
        ["impl ", "From", "<Value> for ", "Abi", "Argument", "Value"].concat(),
        ["impl ", "From", "<Value> for ", "Expected", "Value"].concat(),
    ] {
        assert!(!source.contains(&forbidden), "found {forbidden}");
    }
}

#[test]
fn json_wrappers_require_typed_deserialize_shape() {
    assert!(serde_json::from_value::<AbiJson>(serde_json::json!([
        {"type": "function", "name": "owner"}
    ]))
    .is_err());

    let typed = AbiJson::from_json_value(&serde_json::json!([
        {"type": "function", "name": "owner"}
    ]))
    .expect("typed abi");
    let serialized = serde_json::to_value(&typed).expect("serialized");
    let decoded: AbiJson = serde_json::from_value(serialized).expect("decoded");
    assert_eq!(decoded, typed);

    let abi = serde_json::from_value::<AbiJson>(serde_json::json!({
        "json_text": r#"[{"type":"function","name":"owner"}]"#
    }))
    .expect("typed wrapper");

    assert_eq!(abi.json_text(), r#"[{"name":"owner","type":"function"}]"#);
}

#[test]
fn prepare_validate_assertions_preserves_typed_expected_values() {
    let abi_json = AbiJson::from_json_value(&serde_json::json!([
        {
            "type": "function",
            "name": "owner",
            "inputs": [],
            "outputs": [{ "name": "", "type": "address" }],
            "stateMutability": "view"
        }
    ]))
    .expect("abi json");
    let abi = parse_abi(&abi_json).expect("abi");
    let expected = ExpectedValue::from_json_value(&serde_json::json!(
        "0x0000000000000000000000000000000000000000"
    ))
    .expect("expected");

    let (reads, events) = prepare_validate_assertions(
        &abi,
        &[ReadAssertionConfig {
            function: FunctionName::new("owner").expect("function name"),
            args: vec![],
            expected: expected.clone(),
        }],
        &[],
    )
    .expect("prepare");

    assert_eq!(events.len(), 0);
    assert_eq!(reads[0].expected, expected);
}

#[test]
fn contract_context_refs_are_stable_content_addresses() {
    let first = contract_context(1);
    let second = contract_context(1);
    let different = contract_context(31337);

    assert_eq!(derived_context_ref(&first), derived_context_ref(&second));
    assert_ne!(derived_context_ref(&first), derived_context_ref(&different));
}

#[test]
fn contract_context_rejects_float_chain_ids() {
    let context = serde_json::json!({
        "lifecycle_key": "example-lifecycle",
        "network": {
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1.5,
            "chain_fingerprint": null,
            "finality_or_observation_policy": null
        },
        "contract_profile": {
            "profile_id": "example-profile",
            "artifact_digest": null,
            "interface_digest": null,
            "creation_bytecode_digest": null,
            "deployed_code_hash": null,
            "selector_event_policy_digest": null
        }
    });

    assert!(serde_json::from_value::<EvmContractContext>(context).is_err());
}

#[test]
fn context_bound_instances_expose_typed_context_metadata() {
    let context = context_ref(0x40);
    let context_ref_value = ContextRefValue::new(context.clone());
    let address =
        ContractAddress::new("0X1111111111111111111111111111111111111111").expect("address");
    let deployed = DeployedContractInstance {
        lifecycle_version: 1,
        context_ref: context_ref_value.clone(),
        address: address.clone(),
        deploy_provenance: DeployProvenance::MfmDeploy {
            deploy_tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
        },
        deploy_evidence: Vec::new(),
        deployed_block_number: Some(10),
    };
    let configured = ConfiguredContractInstance {
        lifecycle_version: 1,
        context_ref: context_ref_value.clone(),
        address: address.clone(),
        configured_from: ConfiguredFrom {
            deployed_context_ref: context_ref_value.clone(),
            deployed_address: address.clone(),
        },
        configuration_claim: configured_claim(),
        configure_or_import_evidence: Vec::new(),
        configured_block_number: Some(11),
        asserted_configuration_snapshot: None,
    };
    let report = ContextBoundValidationReport {
        report_version: 1,
        context_ref: context_ref_value,
        configured_instance: ConfiguredContractInstanceRef::from_configured(&configured),
        configuration_read_results: Vec::new(),
        configuration_event_results: Vec::new(),
        read_results: Vec::new(),
        event_results: Vec::new(),
        evidence_refs: Vec::new(),
        valid: true,
    };

    assert_eq!(deployed.context_ref(), &context);
    assert_eq!(
        deployed.context_resource_kind(),
        contract_instance_resource_kind()
    );
    assert_eq!(deployed.context_stage(), deployed_contract_stage());
    assert_eq!(configured.context_ref(), &context);
    assert_eq!(configured.context_stage(), configured_contract_stage());
    assert_eq!(report.context_ref(), &context);
    assert_eq!(
        report.context_resource_kind(),
        validation_report_resource_kind()
    );
    assert_eq!(report.context_stage(), validation_report_stage());
}

#[test]
fn configuration_claims_distinguish_mfm_provenance_from_external_adoption() {
    let imported = ConfigurationClaim::ImportedMfmConfigured {
        source_run_id: LifecycleRunIdRef::from(run_id(0x40)),
        source_spec_hash: LifecycleSpecHashRef::from(spec_hash(0x41)),
        source_cell_or_output_id: SourceCellOrOutputRef::Cell {
            cell_id: LifecycleCellIdRef::from(cell_id(0x42)),
        },
        source_value_digest: profile_digest(0x43),
        source_context_ref: ContextRefValue::new(context_ref(0x44)),
    };
    let observed = ConfigurationClaim::ExternalObservedConfigured {
        provenance_label: ProvenanceLabel::new("audited-external").expect("label"),
        evidence_policy_digest: profile_digest(0x45),
        assertion_evidence_refs: Vec::new(),
    };
    let claimed = ConfigurationClaim::ExternalClaimedConfigured {
        provenance_label: ProvenanceLabel::new("claimed-external").expect("label"),
        evidence_policy_digest: profile_digest(0x46),
    };

    assert!(configured_claim().proves_mfm_configuration());
    assert!(imported.proves_mfm_configuration());
    assert!(!observed.proves_mfm_configuration());
    assert!(!claimed.proves_mfm_configuration());
}

#[test]
fn context_bound_addresses_normalize_and_reject_malformed_values() {
    let normalized =
        ContractAddress::new("0XAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").expect("address");

    assert_eq!(
        normalized.as_str(),
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert!(ContractAddress::new("0xaaaa").is_err());
}

#[test]
fn json_wrappers_canonicalize_authored_values() {
    let arg = AbiArgumentValue::from_json_value(&serde_json::json!({
        "b": 2,
        "a": 1
    }))
    .expect("arg");

    assert_eq!(arg.json_text(), r#"{"a":1,"b":2}"#);
    assert_eq!(
        arg.to_json_value().expect("json value"),
        serde_json::json!({"a": 1, "b": 2})
    );
}

#[test]
fn block_selector_uses_typed_shape_only() {
    let typed_number: BlockSelector =
        serde_json::from_value(serde_json::json!({"kind": "number", "number": 12}))
            .expect("selector");
    let typed_tag: BlockSelector =
        serde_json::from_value(serde_json::json!({"kind": "tag", "tag": "latest"}))
            .expect("selector");

    assert_eq!(typed_number, BlockSelector::Number { number: 12 });
    assert_eq!(
        typed_tag,
        BlockSelector::Tag {
            tag: BlockTag::Latest
        }
    );
    assert!(serde_json::from_value::<BlockSelector>(serde_json::json!(12)).is_err());
}

#[test]
fn lifecycle_values_have_contract_and_abi_schema_ids() {
    let schema_ids = [
        AbiJson::schema_id().expect("schema").to_string(),
        AbiArgumentValue::schema_id().expect("schema").to_string(),
        ContractArtifactConfig::schema_id()
            .expect("schema")
            .to_string(),
        EvmNetworkContext::schema_id().expect("schema").to_string(),
        EvmContractContext::schema_id().expect("schema").to_string(),
        ContractProfile::schema_id().expect("schema").to_string(),
        ImportFromMfmRun::schema_id().expect("schema").to_string(),
        ImportFromMfmRunEvidence::schema_id()
            .expect("schema")
            .to_string(),
        AdoptExternalAddress::schema_id()
            .expect("schema")
            .to_string(),
        ExternalAdoptionEvidencePolicy::schema_id()
            .expect("schema")
            .to_string(),
        ExternalAdoptionEvidence::schema_id()
            .expect("schema")
            .to_string(),
        ConfigurationClaim::schema_id().expect("schema").to_string(),
        DeployedContractInstance::schema_id()
            .expect("schema")
            .to_string(),
        ConfiguredContractInstance::schema_id()
            .expect("schema")
            .to_string(),
        ContextBoundValidationReport::schema_id()
            .expect("schema")
            .to_string(),
        DeployedContract::schema_id().expect("schema").to_string(),
        ConfiguredContract::schema_id().expect("schema").to_string(),
        ConfiguredContractRef::schema_id()
            .expect("schema")
            .to_string(),
        ExistingConfiguredContractRef::schema_id()
            .expect("schema")
            .to_string(),
        ValidationReport::schema_id().expect("schema").to_string(),
    ];

    assert!(schema_ids.iter().all(
        |schema_id| schema_id.contains("mfm.evm.contract") || schema_id.contains("mfm.evm.abi")
    ));
}

#[test]
fn lifecycle_artifact_evidence_refs_round_trip_typed_ids() {
    let evidence = LifecycleArtifactEvidenceRef::new(
        ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
        128,
        Some(
            SchemaId::new(
                "mfm.evm.contract.value.test",
                "v1",
                DigestAlgorithm::Sha256JcsV1,
                digest(),
            )
            .expect("schema"),
        ),
        Some(
            SemanticTypeId::new(
                "mfm.evm.contract",
                "test",
                "v1",
                DigestAlgorithm::Sha256JcsV1,
                digest(),
            )
            .expect("semantic"),
        ),
    );

    let json = serde_json::to_value(&evidence).expect("json");
    let decoded: LifecycleArtifactEvidenceRef =
        serde_json::from_value(json).expect("decoded evidence");

    assert_eq!(decoded, evidence);
}

#[test]
fn lifecycle_artifact_evidence_refs_reject_invalid_identity_fields() {
    let invalid_artifact_id = serde_json::json!({
        "artifact_id": "not-an-artifact-id",
        "content_digest": ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest()
        ).to_string(),
        "byte_len": 128
    });

    assert!(serde_json::from_value::<LifecycleArtifactEvidenceRef>(invalid_artifact_id).is_err());
}

#[test]
fn artifact_port_is_a_checked_string_authority() {
    let port = ArtifactPort::new("artifact").expect("artifact port");

    assert_eq!(port.as_str(), "artifact");
}
