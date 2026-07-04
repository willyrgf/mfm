use mfm_evm_contract_config::*;
use mfm_evm_contract_model::{
    AcceptedContextPolicy, ContractLifecycleStage, SourceCellOrOutputRef,
};
use mfm_ids::{CellId, ContentDigest, ContextRef, DigestAlgorithm, DigestBytes, RunId, SpecHash};
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
        "network": network_json(1),
        "signer": signer_json(),
    })
}

fn configure_json() -> serde_json::Value {
    serde_json::json!({
        "network": network_json(1),
        "signer": signer_json(),
        "calls": [],
    })
}

fn validate_json() -> serde_json::Value {
    serde_json::json!({
        "network": network_json(1),
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

#[test]
fn phase_configs_require_expected_chain_id() {
    let network_without_chain = serde_json::json!({"network_id": "ethereum-mainnet"});

    assert!(
        serde_json::from_value::<DeployPhaseConfig>(serde_json::json!({
            "network": network_without_chain,
            "signer": signer_json(),
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ConfigurePhaseConfig>(serde_json::json!({
            "network": serde_json::json!({"network_id": "ethereum-mainnet"}),
            "signer": signer_json(),
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ValidatePhaseConfig>(serde_json::json!({
            "network": serde_json::json!({"network_id": "ethereum-mainnet"}),
        }))
        .is_err()
    );
}

#[test]
fn eip1559_transaction_policy_is_default() {
    let deploy: DeployPhaseConfig = serde_json::from_value(deploy_json()).expect("deploy");

    assert_eq!(deploy.transaction().style(), EvmTransactionStyle::Eip1559);
    assert_eq!(deploy.receipt().poll_interval_ms(), 500);
    assert_eq!(deploy.network().expected_chain_id(), 1);
    assert_eq!(deploy.signer().signer_ref_str(), "deployer");
}

#[test]
fn receipt_policy_rejects_unbounded_waits() {
    assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS + 1, 1).is_err());
    assert!(ReceiptRetryPolicy::new(1, MAX_RECEIPT_POLLS + 1).is_err());
    assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS, 62).is_err());
    assert!(ReceiptRetryPolicy::new(MAX_RECEIPT_POLL_INTERVAL_MS, 61).is_ok());
    assert!(
        serde_json::from_value::<DeployPhaseConfig>(serde_json::json!({
            "network": network_json(1),
            "signer": signer_json(),
            "receipt": {
                "poll_interval_ms": MAX_RECEIPT_POLL_INTERVAL_MS + 1,
                "max_receipt_polls": 1
            }
        }))
        .is_err()
    );
}

#[test]
fn legacy_transaction_style_remains_accepted() {
    let deploy: DeployPhaseConfig = serde_json::from_value(serde_json::json!({
        "network": network_json(1),
        "signer": signer_json(),
        "transaction": {
            "style": "legacy",
            "gas_price": "1000000000",
        },
    }))
    .expect("legacy deploy config");

    assert_eq!(deploy.transaction().style(), EvmTransactionStyle::Legacy);
    assert_eq!(deploy.transaction().gas_price(), Some("1000000000"));
}

#[test]
fn eip1559_transaction_policy_rejects_legacy_fee_field() {
    assert!(
        serde_json::from_value::<DeployPhaseConfig>(serde_json::json!({
            "network": network_json(1),
            "signer": signer_json(),
            "transaction": {
                "style": "eip1559",
                "gas_price": "1000000000",
            },
        }))
        .is_err()
    );
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
    assert!(serde_json::from_value::<DeployPhaseConfig>(deploy).is_err());

    let mut validate = validate_json();
    let routing_key = ["rpc", "_url"].concat();
    validate
        .as_object_mut()
        .expect("validate object")
        .insert(routing_key, serde_json::json!("http://127.0.0.1:8545"));
    assert!(serde_json::from_value::<ValidatePhaseConfig>(validate).is_err());
}

#[test]
fn configure_and_validate_phase_configs_parse() {
    let configure: ConfigurePhaseConfig =
        serde_json::from_value(configure_json()).expect("configure");
    let validate: ValidatePhaseConfig = serde_json::from_value(validate_json()).expect("validate");

    assert_eq!(configure.calls().len(), 0);
    assert_eq!(validate.validation().read_assertions().len(), 0);
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
    let deploy: EvmContractDeployEntryConfig = serde_json::from_value(serde_json::json!({
        "context": context_json(serde_json::json!(1)),
        "deploy": {
            "signer": signer_json(),
        },
    }))
    .expect("deploy entry");
    assert_eq!(deploy.context().network.expected_chain_id(), 1);
    assert_eq!(deploy.deploy().signer().signer_ref_str(), "deployer");

    let configure: EvmContractConfigureEntryConfig = serde_json::from_value(serde_json::json!({
        "context": context_json(serde_json::json!(1)),
        "import_deployed": {
            "kind": "from_mfm_run",
            "source": import_from_mfm_run_json("deployed"),
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
        serde_json::from_value::<EvmContractDeployEntryConfig>(serde_json::json!({
            "context": context_json(serde_json::json!(1.5)),
            "deploy": {
                "signer": signer_json(),
            },
        }))
        .is_err()
    );

    let mut missing_chain = context_json(serde_json::json!(1));
    missing_chain
        .get_mut("network")
        .and_then(serde_json::Value::as_object_mut)
        .expect("network object")
        .remove("expected_chain_id");
    assert!(
        serde_json::from_value::<EvmContractDeployEntryConfig>(serde_json::json!({
            "context": missing_chain,
            "deploy": {
                "signer": signer_json(),
            },
        }))
        .is_err()
    );

    let mut unknown_profile_field = context_json(serde_json::json!(1));
    unknown_profile_field
        .get_mut("contract_profile")
        .and_then(serde_json::Value::as_object_mut)
        .expect("contract profile object")
        .insert("unexpected".to_owned(), serde_json::json!(true));
    assert!(
        serde_json::from_value::<EvmContractDeployEntryConfig>(serde_json::json!({
            "context": unknown_profile_field,
            "deploy": {
                "signer": signer_json(),
            },
        }))
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
    }))
    .expect("deployed import");
    let ImportDeployedSpec::FromMfmRun { source } = deployed else {
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
    }))
    .expect("configured import");
    let ImportConfiguredSpec::FromMfmRun { source } = configured else {
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
        DeployPhaseConfig::schema_id().expect("schema").to_string(),
        ConfigurePhaseConfig::schema_id()
            .expect("schema")
            .to_string(),
        ValidatePhaseConfig::schema_id()
            .expect("schema")
            .to_string(),
        EvmNetworkIntent::schema_id().expect("schema").to_string(),
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
