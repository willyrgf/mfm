use super::*;
use mfm_collectors_evm::parse_u64_hex_value;
use mfm_sdk::op::{LeafOpSpec, OpInterface, PlannedOp, PlannedOpKind};
use mfm_state_common::test_support as op_test_support;

fn planned_interface(planned: PlannedOp) -> OpInterface {
    planned.interface
}

fn into_leaf(planned: PlannedOp) -> LeafOpSpec {
    match planned.kind {
        PlannedOpKind::Leaf(spec) => spec,
        PlannedOpKind::Composite(_) => panic!("expected leaf planned op"),
    }
}

fn sample_artifact() -> serde_json::Value {
    serde_json::json!({
        "abi": [
            {
                "type": "constructor",
                "inputs": [{"name": "x", "type": "uint256"}]
            },
            {
                "type": "function",
                "name": "setValue",
                "inputs": [{"name": "x", "type": "uint256"}],
                "outputs": []
            },
            {
                "type": "function",
                "name": "getValue",
                "inputs": [],
                "outputs": [{"name": "", "type": "uint256"}]
            },
            {
                "type": "event",
                "name": "ValueSet",
                "inputs": [{"name": "x", "type": "uint256", "indexed": false}],
                "anonymous": false
            }
        ],
        "bytecode": {
            "object": "0x60006000"
        }
    })
}

#[test]
fn encode_params_uint_address() {
    let types = vec!["uint256".to_string(), "address".to_string()];
    let args = vec![
        serde_json::json!(7),
        serde_json::json!("0x1111111111111111111111111111111111111111"),
    ];

    let out = encode_params(&types, &args).expect("encode");
    assert_eq!(out.len(), 64);
    assert_eq!(out[31], 7u8);
    assert_eq!(
        &out[44..64],
        &hex_to_bytes("0x1111111111111111111111111111111111111111").unwrap()
    );
}

#[test]
fn encode_dynamic_string() {
    let types = vec!["string".to_string()];
    let args = vec![serde_json::json!("hello")];

    let out = encode_params(&types, &args).expect("encode");
    assert_eq!(out.len(), 96);
    // offset = 0x20
    assert_eq!(out[31], 32u8);
    // len = 5
    assert_eq!(out[63], 5u8);
}

#[test]
fn resolve_function_call_works() {
    let artifact: ContractArtifactConfig =
        serde_json::from_value(sample_artifact()).expect("artifact config");
    let (abi, _bytecode) = parse_artifact(&artifact).expect("parse artifact");

    let (calldata, _outputs) =
        resolve_function_call(&abi, "setValue", &[serde_json::json!(42)]).expect("call");

    assert!(calldata.len() >= 4 + 32);
}

#[test]
fn deploy_io_exports_contract_address() {
    let op = EvmDeployOp;
    let io = planned_interface(
        op.expand(
            OpPath("m.main".to_string()),
            &serde_json::json!({
            "artifact": sample_artifact(),
            "network_id": "ethereum-mainnet",
            "from": "0x1111111111111111111111111111111111111111",
            "signing_key_env": "MFM_DEPLOYER_KEY"
            }),
            &op_test_support::run_config_live(),
        )
        .expect("expand"),
    );
    assert!(io
        .exports
        .iter()
        .any(|k| k.0.as_str() == KEY_CONTRACT_ADDRESS));
}

#[test]
fn deploy_io_imports_artifact_port_when_artifact_is_unset() {
    let op = EvmDeployOp;
    let io = planned_interface(
        op.expand(
            OpPath("m.main".to_string()),
            &serde_json::json!({
            "artifact_port": "contract_artifact",
            "network_id": "ethereum-mainnet",
            "from": "0x1111111111111111111111111111111111111111",
            "signing_key_env": "MFM_DEPLOYER_KEY"
            }),
            &op_test_support::run_config_live(),
        )
        .expect("expand"),
    );
    assert!(io
        .imports
        .iter()
        .any(|k| k.0.as_str() == KEY_CONTRACT_ARTIFACT));
}

#[test]
fn configure_io_imports_contract_address_when_unset() {
    let op = EvmConfigureOp;
    let io = planned_interface(
        op.expand(
            OpPath("m.main".to_string()),
            &serde_json::json!({
            "artifact": sample_artifact(),
            "network_id": "ethereum-mainnet",
            "from": "0x1111111111111111111111111111111111111111",
            "signing_key_env": "MFM_DEPLOYER_KEY",
            "calls": [{"function":"setValue","args":[1]}]
            }),
            &op_test_support::run_config_live(),
        )
        .expect("expand"),
    );

    assert!(io
        .imports
        .iter()
        .any(|k| k.0.as_str() == KEY_CONTRACT_ADDRESS));
}

#[test]
fn configure_io_exports_custom_keys_when_overridden() {
    let op = EvmConfigureOp;
    let io = planned_interface(
        op.expand(
            OpPath("m.main".to_string()),
            &serde_json::json!({
                "artifact": sample_artifact(),
                "network_id": "ethereum-mainnet",
                "from": "0x1111111111111111111111111111111111111111",
                "signing_key_env": "MFM_DEPLOYER_KEY",
                "calls": [{"function":"setValue","args":[1]}],
                "tx_hashes_export_key": "approve_tx_hashes",
                "receipts_export_key": "approve_receipts"
            }),
            &op_test_support::run_config_live(),
        )
        .expect("expand"),
    );

    assert!(io
        .exports
        .iter()
        .any(|k| k.0.as_str() == "approve_tx_hashes"));
    assert!(io
        .exports
        .iter()
        .any(|k| k.0.as_str() == "approve_receipts"));
}

#[test]
fn deploy_rejects_node_managed_unsigned_config() {
    let op = EvmDeployOp;
    let err = match op.expand(
        OpPath("m.main".to_string()),
        &serde_json::json!({
            "artifact": sample_artifact(),
            "network_id": "ethereum-mainnet",
            "from": "0x1111111111111111111111111111111111111111"
        }),
        &op_test_support::run_config_live(),
    ) {
        Ok(_) => panic!("unsigned deploy config must be rejected"),
        Err(err) => err,
    };

    assert_eq!(err.info.code.0, "invalid_op_config");
    assert!(err.info.message.contains("signing_key_env is required"));
}

#[test]
fn configure_rejects_node_managed_unsigned_config() {
    let op = EvmConfigureOp;
    let err = match op.expand(
        OpPath("m.main".to_string()),
        &serde_json::json!({
            "artifact": sample_artifact(),
            "network_id": "ethereum-mainnet",
            "from": "0x1111111111111111111111111111111111111111",
            "calls": [{"function":"setValue","args":[1]}]
        }),
        &op_test_support::run_config_live(),
    ) {
        Ok(_) => panic!("unsigned configure config must be rejected"),
        Err(err) => err,
    };

    assert_eq!(err.info.code.0, "invalid_op_config");
    assert!(err.info.message.contains("signing_key_env is required"));
}

#[test]
fn contract_from_nix_io_imports_result_and_exports_artifact() {
    let op = EvmContractFromNixOp;
    let io = planned_interface(
        op.expand(
            OpPath("m.main".to_string()),
            &serde_json::json!({
            "result_pointer": "/artifact"
            }),
            &op_test_support::run_config_live(),
        )
        .expect("expand"),
    );

    assert!(io.imports.iter().any(|k| k.0.as_str() == KEY_NIX_RESULT));
    assert!(io
        .exports
        .iter()
        .any(|k| k.0.as_str() == KEY_CONTRACT_ARTIFACT));
}

#[test]
fn deploy_contract_set_io_imports_contract_set_and_exports_manifest() {
    let op = EvmDeployContractSetOp;
    let io = planned_interface(
        op.expand(
            OpPath("m.main".to_string()),
            &serde_json::json!({
                "contract_set_port": "compile_origin_result",
                "network_id": "ethereum-mainnet",
                "signing_key_env": "MFM_DEPLOYER_KEY"
            }),
            &op_test_support::run_config_live(),
        )
        .expect("expand"),
    );

    assert!(io
        .imports
        .iter()
        .any(|k| k.0.as_str() == "compile_origin_result"));
    assert!(io.exports.iter().any(|k| k.0.as_str() == "deploy_manifest"));
}

#[test]
fn validate_expand_builds_read_and_event_assertions() {
    let op = EvmValidateOp;
    let plan = into_leaf(
        op.expand(
            OpPath("m.validate".to_string()),
            &serde_json::json!({
                "artifact": sample_artifact(),
                "network_id": "ethereum-mainnet",
                "contract_address": "0x1111111111111111111111111111111111111111",
                "expected_chain_id": 1337,
                "read_assertions": [
                    {"function":"getValue","args":[],"expected": 1}
                ],
                "event_assertions": [
                    {"event":"ValueSet","min_count":1}
                ]
            }),
            &RunConfig {
                nix_flake_allowlist: Vec::new(),
                io_mode: mfm_machine::config::IoMode::Live,
                retry_policy: mfm_machine::config::RetryPolicy {
                    max_attempts: 1,
                    backoff: mfm_machine::config::BackoffPolicy::Fixed {
                        delay: Duration::from_millis(0),
                    },
                },
                event_profile: mfm_machine::config::EventProfile::Normal,
                execution_mode: mfm_machine::config::ExecutionMode::Sequential,
                context_checkpointing: mfm_machine::config::ContextCheckpointing::AfterEveryState,
                replay_missing_fact_retryable: false,
                skip_tags: Vec::new(),
            },
        )
        .expect("expand"),
    );

    assert_eq!(plan.states.len(), 1);
    assert!(plan.edges.is_empty());
}

#[test]
fn parse_u64_hex_value_still_works_for_receipt_numbers() {
    let v = serde_json::json!("0x7b");
    assert_eq!(parse_u64_hex_value(&v).unwrap(), 123);
}

#[test]
fn signing_key_from_env_parses_valid_key() {
    let env_name = "MFM_TEST_SIGNING_KEY_VALID";
    std::env::set_var(
        env_name,
        "0x0000000000000000000000000000000000000000000000000000000000000001",
    );

    let key = signing_key_from_env(env_name).expect("signing key should parse");
    assert_eq!(
        signer_address_hex(&key),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );

    std::env::remove_var(env_name);
}

#[test]
fn signing_key_from_env_rejects_short_key() {
    let env_name = "MFM_TEST_SIGNING_KEY_INVALID";
    std::env::set_var(env_name, "0x1234");

    let err = signing_key_from_env(env_name).expect_err("short key should fail");
    assert!(err.info.message.contains("exactly 32 bytes"));

    std::env::remove_var(env_name);
}

#[test]
fn signing_key_from_env_rejects_missing_env() {
    let env_name = "MFM_TEST_SIGNING_KEY_MISSING";
    std::env::remove_var(env_name);

    let err = signing_key_from_env(env_name).expect_err("missing env should fail");
    assert!(err
        .info
        .message
        .contains("did not exist in process environment"));
}

#[test]
fn signing_key_from_env_rejects_invalid_hex() {
    let env_name = "MFM_TEST_SIGNING_KEY_BAD_HEX";
    std::env::set_var(
        env_name,
        "0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    );

    let err = signing_key_from_env(env_name).expect_err("invalid hex should fail");
    assert!(err.info.message.contains("hex was invalid"));

    std::env::remove_var(env_name);
}

#[test]
fn signing_key_from_env_rejects_invalid_curve_key() {
    let env_name = "MFM_TEST_SIGNING_KEY_ZERO";
    std::env::set_var(
        env_name,
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    );

    let err = signing_key_from_env(env_name).expect_err("zero key should fail");
    assert!(err.info.message.contains("valid secp256k1 key"));

    std::env::remove_var(env_name);
}
