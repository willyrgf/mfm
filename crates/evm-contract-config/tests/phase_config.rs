use mfm_evm_contract_config::*;
use mfm_evm_contract_model::{
    AcceptedContextPolicy, ConfiguredContractInstance, ContractLifecycleStage,
    DeployedContractInstance, SourceCellOrOutputRef,
};
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, ContextDescriptorId, ContextRef, DescriptorId,
    DigestAlgorithm, DigestBytes, EventId, RunId, SpecHash,
};
use mfm_values::MfmConfig;

fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest_str(byte: u8) -> String {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn context_ref_str(byte: u8) -> String {
    ContextRef::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn run_id_str(byte: u8) -> String {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn spec_hash_str(byte: u8) -> String {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn cell_id_str(byte: u8) -> String {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn artifact_id_str(byte: u8) -> String {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn descriptor_id_str(byte: u8) -> String {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn context_descriptor_id_str(byte: u8) -> String {
    ContextDescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn event_id_str(byte: u8) -> String {
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte)).to_string()
}

fn network_json(chain_id: u64) -> serde_json::Value {
    serde_json::json!({
        "network_id": "ethereum-mainnet",
        "expected_chain_id": chain_id,
    })
}

fn context_json(chain_id: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "lifecycle_key": "example-lifecycle",
        "network": {
            "network_id": "ethereum-mainnet",
            "expected_chain_id": chain_id,
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

fn signer_json() -> serde_json::Value {
    serde_json::json!({
        "signer_ref": "deployer",
        "expected_signer_address": "0x000000000000000000000000000000000000dead",
    })
}

fn deploy_json() -> serde_json::Value {
    serde_json::json!({
        "signer": signer_json(),
    })
}

fn configure_json() -> serde_json::Value {
    serde_json::json!({
        "signer": signer_json(),
        "calls": [],
    })
}

fn validate_json() -> serde_json::Value {
    serde_json::json!({})
}

fn deploy_entry_json(context: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "context": context,
        "deploy": {
            "signer": signer_json(),
        },
    })
}

fn import_from_mfm_run_json(required_stage: &str) -> serde_json::Value {
    serde_json::json!({
        "source_run_id": run_id_str(0x30),
        "source_spec_hash": spec_hash_str(0x31),
        "source_cell_or_output_id": {
            "kind": "cell",
            "cell_id": cell_id_str(0x32),
        },
        "source_value_digest": content_digest_str(0x33),
        "source_context_ref": context_ref_str(0x34),
        "required_stage": required_stage,
    })
}

fn artifact_ref_json(
    byte: u8,
    digest_byte: u8,
    schema_id: Option<String>,
    semantic_type_id: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": artifact_id_str(byte),
        "content_digest": content_digest_str(digest_byte),
        "byte_len": 128,
        "schema_id": schema_id,
        "semantic_type_id": semantic_type_id,
    })
}

fn import_from_mfm_run_evidence_json(required_stage: &str) -> serde_json::Value {
    let (schema_id, semantic_type_id) = match required_stage {
        "deployed" => (
            <DeployedContractInstance as mfm_values::MfmValue>::schema_id()
                .expect("deployed schema")
                .to_string(),
            <DeployedContractInstance as mfm_values::MfmValue>::semantic_id()
                .expect("deployed semantic")
                .to_string(),
        ),
        "configured" => (
            <ConfiguredContractInstance as mfm_values::MfmValue>::schema_id()
                .expect("configured schema")
                .to_string(),
            <ConfiguredContractInstance as mfm_values::MfmValue>::semantic_id()
                .expect("configured semantic")
                .to_string(),
        ),
        _ => panic!("unsupported stage"),
    };
    serde_json::json!({
        "source_spec_hash": spec_hash_str(0x31),
        "source_spec_artifact_ref": artifact_ref_json(0x4e, 0x4f, None, None),
        "source_spec_certificate_ref": artifact_ref_json(0x50, 0x51, None, None),
        "source_run_stream_ref": artifact_ref_json(0x52, 0x53, None, None),
        "source_cell_or_output_id": {
            "kind": "cell",
            "cell_id": cell_id_str(0x32),
        },
        "source_cell_schema_id": schema_id,
        "source_cell_semantic_type_id": semantic_type_id,
        "source_producer_descriptor_id": descriptor_id_str(0x54),
        "source_stage": required_stage,
        "source_context_ref": context_ref_str(0x34),
        "source_context_descriptor_id": context_descriptor_id_str(0x55),
        "source_value_digest": content_digest_str(0x33),
        "source_value_artifact_ref_or_inline_canonical_value": artifact_ref_json(
            0x56,
            0x33,
            Some(schema_id),
            Some(semantic_type_id),
        ),
        "source_terminal_cell_or_output_event_ref": event_id_str(0x57),
        "import_policy_digest": content_digest_str(0x58),
    })
}

#[test]
fn eip1559_transaction_policy_is_default_for_deploy_action() {
    let deploy: DeployAction = serde_json::from_value(deploy_json()).expect("deploy");

    assert_eq!(deploy.transaction().style(), EvmTransactionStyle::Eip1559);
    assert_eq!(deploy.receipt().poll_interval_ms(), 500);
    assert_eq!(deploy.signer().signer_ref_str(), "deployer");
}

#[test]
fn receipt_policy_rejects_unbounded_waits() {
    assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS + 1, 1).is_err());
    assert!(ReceiptRetryPolicy::new(1, MAX_RECEIPT_POLLS + 1).is_err());
    assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS, 62).is_err());
    assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS, 61).is_ok());
    assert!(serde_json::from_value::<DeployAction>(serde_json::json!({
        "signer": signer_json(),
        "receipt": {
            "poll_interval_ms": MAX_RECEIPT_POLL_INTERVAL_MS + 1,
            "max_receipt_polls": 1
        }
    }))
    .is_err());
}

#[test]
fn transaction_policy_accepts_legacy_and_rejects_mixed_fee_fields() {
    let deploy: DeployAction = serde_json::from_value(serde_json::json!({
        "signer": signer_json(),
        "transaction": {
            "style": "legacy",
            "gas_price": "1000000000",
        },
    }))
    .expect("legacy deploy config");

    assert_eq!(deploy.transaction().style(), EvmTransactionStyle::Legacy);
    assert_eq!(deploy.transaction().gas_price(), Some("1000000000"));

    assert!(serde_json::from_value::<DeployAction>(serde_json::json!({
        "signer": signer_json(),
        "transaction": {
            "style": "eip1559",
            "gas_price": "1000000000",
        },
    }))
    .is_err());
}

#[test]
fn configs_deny_provider_and_runtime_fields() {
    let mut deploy = deploy_json();
    let provider_key = ["keystore", "_path_env"].concat();
    deploy
        .get_mut("signer")
        .and_then(serde_json::Value::as_object_mut)
        .expect("signer object")
        .insert(provider_key, serde_json::json!("MFM_SIGNER_FILE"));
    assert!(serde_json::from_value::<DeployAction>(deploy).is_err());

    let mut validate = validate_json();
    let routing_key = ["rpc", "_url"].concat();
    validate
        .as_object_mut()
        .expect("validate object")
        .insert(routing_key, serde_json::json!("http://127.0.0.1:8545"));
    assert!(serde_json::from_value::<ValidateAction>(validate).is_err());
}

#[test]
fn configure_and_validate_actions_parse() {
    let configure: ConfigureAction = serde_json::from_value(configure_json()).expect("configure");
    let validate: ValidateAction = serde_json::from_value(validate_json()).expect("validate");

    assert_eq!(configure.calls().len(), 0);
    assert_eq!(validate.read_assertions().len(), 0);
}

#[test]
fn context_actions_reject_embedded_network_fields() {
    assert!(serde_json::from_value::<DeployAction>(serde_json::json!({
        "network": network_json(1),
        "signer": signer_json(),
    }))
    .is_err());
    assert!(
        serde_json::from_value::<ConfigureAction>(serde_json::json!({
            "network": network_json(1),
            "signer": signer_json(),
            "calls": [],
        }))
        .is_err()
    );
    assert!(serde_json::from_value::<ValidateAction>(serde_json::json!({
        "network": network_json(1),
    }))
    .is_err());
}

#[test]
fn context_entry_configs_parse_new_shapes() {
    let deploy: EvmContractDeployEntryConfig =
        serde_json::from_value(deploy_entry_json(context_json(serde_json::json!(1))))
            .expect("deploy entry");
    assert_eq!(deploy.context().network.expected_chain_id(), 1);
    assert_eq!(deploy.deploy().signer().signer_ref_str(), "deployer");

    let configure: EvmContractConfigureEntryConfig = serde_json::from_value(serde_json::json!({
        "context": context_json(serde_json::json!(1)),
        "import_deployed": {
            "kind": "from_mfm_run",
            "source": import_from_mfm_run_json("deployed"),
            "evidence": import_from_mfm_run_evidence_json("deployed"),
        },
        "configure": {
            "signer": signer_json(),
            "calls": [],
        },
    }))
    .expect("configure entry");
    assert_eq!(configure.configure().calls().len(), 0);

    let validate: EvmContractValidateEntryConfig = serde_json::from_value(serde_json::json!({
        "context": context_json(serde_json::json!(1)),
        "import_configured": {
            "kind": "from_mfm_run",
            "source": import_from_mfm_run_json("configured"),
            "evidence": import_from_mfm_run_evidence_json("configured"),
        },
        "validate": {},
    }))
    .expect("validate entry");
    assert_eq!(validate.validate().read_assertions().len(), 0);

    let lifecycle: EvmContractLifecycleEntryConfig = serde_json::from_value(serde_json::json!({
        "context": context_json(serde_json::json!(1)),
        "deploy": {
            "signer": signer_json(),
        },
        "configure": {
            "signer": signer_json(),
            "calls": [],
        },
        "validate": {},
    }))
    .expect("lifecycle entry");
    assert_eq!(lifecycle.context().network.expected_chain_id(), 1);
}

#[test]
fn context_entry_configs_reject_malformed_or_loose_contexts() {
    assert!(
        serde_json::from_value::<EvmContractDeployEntryConfig>(deploy_entry_json(context_json(
            serde_json::json!(1.5)
        )))
        .is_err()
    );

    let mut missing_chain = context_json(serde_json::json!(1));
    missing_chain
        .get_mut("network")
        .and_then(serde_json::Value::as_object_mut)
        .expect("network object")
        .remove("expected_chain_id");
    assert!(
        serde_json::from_value::<EvmContractDeployEntryConfig>(deploy_entry_json(missing_chain))
            .is_err()
    );

    let mut unknown_profile_field = context_json(serde_json::json!(1));
    unknown_profile_field
        .get_mut("contract_profile")
        .and_then(serde_json::Value::as_object_mut)
        .expect("contract profile object")
        .insert("unexpected".to_owned(), serde_json::json!(true));
    assert!(
        serde_json::from_value::<EvmContractDeployEntryConfig>(deploy_entry_json(
            unknown_profile_field
        ))
        .is_err()
    );
}

#[test]
fn old_import_entry_shapes_are_not_accepted_as_context_entries() {
    assert!(
        serde_json::from_value::<EvmContractConfigureEntryConfig>(serde_json::json!({
            "config": configure_json(),
            "deployed": {},
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<EvmContractValidateEntryConfig>(serde_json::json!({
            "config": validate_json(),
            "configured": {},
        }))
        .is_err()
    );
}

#[test]
fn imports_default_to_exact_context_and_accept_explicit_cross_context_policy() {
    let deployed: ImportDeployedSpec = serde_json::from_value(serde_json::json!({
        "kind": "from_mfm_run",
        "source": import_from_mfm_run_json("deployed"),
        "evidence": import_from_mfm_run_evidence_json("deployed"),
    }))
    .expect("deployed import");
    let ImportDeployedSpec::FromMfmRun { source, .. } = deployed else {
        panic!("expected MFM-run import");
    };
    assert_eq!(
        source.accepted_context_policy,
        AcceptedContextPolicy::ExactContext {}
    );
    assert_eq!(source.required_stage, ContractLifecycleStage::Deployed);
    assert!(matches!(
        source.source_cell_or_output_id,
        SourceCellOrOutputRef::Cell { .. }
    ));

    let accepted_context = context_ref_str(0x44);
    let mut configured_source = import_from_mfm_run_json("configured");
    configured_source
        .as_object_mut()
        .expect("import source object")
        .insert(
            "accepted_context_policy".to_owned(),
            serde_json::json!({
                "kind": "accepted_context_refs",
                "context_refs": [accepted_context],
            }),
        );

    let configured: ImportConfiguredSpec = serde_json::from_value(serde_json::json!({
        "kind": "from_mfm_run",
        "source": configured_source,
        "evidence": import_from_mfm_run_evidence_json("configured"),
    }))
    .expect("configured import");
    let ImportConfiguredSpec::FromMfmRun { source, .. } = configured else {
        panic!("expected MFM-run import");
    };
    let AcceptedContextPolicy::AcceptedContextRefs { context_refs } =
        source.accepted_context_policy
    else {
        panic!("expected explicit accepted context refs");
    };
    assert_eq!(context_refs[0].as_str(), context_ref_str(0x44));
    assert_eq!(source.required_stage, ContractLifecycleStage::Configured);
}

#[test]
fn source_run_imports_reject_non_authoritative_public_shapes() {
    let identifier_only = serde_json::json!({
        "kind": "from_mfm_run",
        "source_run_id": run_id_str(0x30),
        "source_cell_or_output_id": {
            "kind": "cell",
            "cell_id": cell_id_str(0x32),
        },
    });
    let public_json = serde_json::json!({
        "kind": "from_mfm_run",
        "deployed": {
            "address": "0x1111111111111111111111111111111111111111",
            "context_ref": context_ref_str(0x34),
        },
    });
    let projection_row = serde_json::json!({
        "kind": "from_mfm_run",
        "projection_row": {
            "cell_id": cell_id_str(0x32),
            "content_digest": content_digest_str(0x33),
            "public_field_path": "deployed",
        },
    });
    let raw_payload = serde_json::json!({
        "kind": "from_mfm_run",
        "source": import_from_mfm_run_json("deployed"),
        "raw_payload": {
            "address": "0x1111111111111111111111111111111111111111",
        },
    });

    for shape in [identifier_only, public_json, projection_row, raw_payload] {
        assert!(serde_json::from_value::<ImportDeployedSpec>(shape.clone()).is_err());
        assert!(serde_json::from_value::<ImportConfiguredSpec>(shape).is_err());
    }
}

#[test]
fn external_adoption_defaults_require_code_and_reject_claimed_configured() {
    let configured: ImportConfiguredSpec = serde_json::from_value(serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x1111111111111111111111111111111111111111",
            "provenance_label": "audited-external",
        },
    }))
    .expect("external adoption");

    let ImportConfiguredSpec::AdoptExternalAddress { adoption } = configured else {
        panic!("expected external adoption");
    };
    assert!(adoption.evidence_policy.require_code);
    assert!(!adoption.evidence_policy.allow_external_claimed_configured);
    assert!(adoption.evidence_policy.expected_code_hash.is_none());
}

#[test]
fn schema_ids_use_contract_config_namespace() {
    let schema_ids = [
        EvmSignerIntent::schema_id().expect("schema").to_string(),
        EvmTransactionPolicy::schema_id()
            .expect("schema")
            .to_string(),
        DeployAction::schema_id().expect("schema").to_string(),
        ConfigureAction::schema_id().expect("schema").to_string(),
        ValidateAction::schema_id().expect("schema").to_string(),
        ImportDeployedSpec::schema_id().expect("schema").to_string(),
        ImportConfiguredSpec::schema_id()
            .expect("schema")
            .to_string(),
        EvmContractDeployEntryConfig::schema_id()
            .expect("schema")
            .to_string(),
        EvmContractConfigureEntryConfig::schema_id()
            .expect("schema")
            .to_string(),
        EvmContractValidateEntryConfig::schema_id()
            .expect("schema")
            .to_string(),
        EvmContractLifecycleEntryConfig::schema_id()
            .expect("schema")
            .to_string(),
    ];

    assert!(schema_ids
        .iter()
        .all(|schema_id| schema_id.contains("mfm.evm.contract.config")));
}
