use mfm_evm_contract_model::*;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, DigestBytes, SchemaId, SemanticTypeId};
use mfm_values::MfmValue;

const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn digest() -> DigestBytes {
    DigestBytes::from_hex(DIGEST_HEX).expect("digest")
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
fn configured_ref_is_derived_from_configured_lifecycle_value() {
    let evidence = LifecycleArtifactEvidenceRef::new(
        ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
        128,
        None,
        None,
    );
    let deployed = DeployedContract {
        lifecycle_version: 1,
        network_id: "ethereum-mainnet".to_string(),
        expected_chain_id: 1,
        contract_address: "0x1111111111111111111111111111111111111111".to_string(),
        deploy_tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_string(),
        deploy_receipt_evidence: None,
        deployed_block_number: Some(10),
    };
    let configured = ConfiguredContract {
        lifecycle_version: 1,
        deployed,
        configure_calls: Vec::new(),
        confirmation_read_assertions: Vec::new(),
        confirmation_event_assertions: Vec::new(),
        configure_tx_hashes: vec![
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        ],
        configure_receipt_evidence: vec![evidence.clone()],
        configured_block_number: Some(11),
    };

    let configured_ref = ConfiguredContractRef::from_configured(&configured);

    assert_eq!(configured_ref.network_id, configured.deployed.network_id);
    assert_eq!(
        configured_ref.contract_address,
        configured.deployed.contract_address
    );
    assert_eq!(configured_ref.evidence, Some(evidence));
}

#[test]
fn validation_report_targets_configured_ref_not_raw_lifecycle_stage() {
    let report = ValidationReport {
        report_version: 1,
        configured_contract: ConfiguredContractRef {
            ref_version: 1,
            network_id: "ethereum-mainnet".to_string(),
            expected_chain_id: 1,
            contract_address: "0x1111111111111111111111111111111111111111".to_string(),
            evidence: None,
        },
        expected_chain_id: 1,
        observed_chain_id: 1,
        client_version: "reth-test".to_string(),
        configuration_read_results: Vec::new(),
        configuration_event_results: Vec::new(),
        read_results: Vec::new(),
        event_results: Vec::new(),
        valid: true,
    };

    assert!(report.valid);
    assert_eq!(report.configured_contract.expected_chain_id, 1);
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
