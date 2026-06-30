use mfm_evm_contract_config::*;
use mfm_values::MfmConfig;

fn network_json(chain_id: u64) -> serde_json::Value {
    serde_json::json!({
        "network_id": "ethereum-mainnet",
        "expected_chain_id": chain_id,
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
    ];

    assert!(schema_ids
        .iter()
        .all(|schema_id| schema_id.contains("mfm.evm.contract.config")));
}
