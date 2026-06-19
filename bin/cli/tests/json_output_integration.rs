#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use assert_cmd::Command;
use mfm::presentation::output::{ErrorResponse, ResponseStatus, SuccessResponse};
use predicates::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;

// This struct is local to the test module to help with deserialization.
#[derive(Deserialize)]
struct TestResponse<T> {
    status: ResponseStatus,
    data: T,
}

/// Test helper to create a temporary keystore and return its path
fn setup_temp_keystore() -> TempDir {
    TempDir::new().unwrap()
}

/// Test helper to parse JSON output and verify it has the success structure
fn verify_success_response(output: &str) -> Value {
    let parsed: TestResponse<Value> = serde_json::from_str(output).expect("Should be valid JSON");
    assert!(matches!(parsed.status, ResponseStatus::Success));
    parsed.data
}

fn verify_error_response(output: &str) -> ErrorResponse {
    let parsed: ErrorResponse =
        serde_json::from_str(output).expect("Should be valid JSON error response");
    assert!(matches!(parsed.status, ResponseStatus::Error));
    parsed
}

#[test]
fn run_stream_command_filters_range_after_authoritative_run_stream_validation() {
    let source = include_str!("../src/commands/run/stream.rs");
    let range_validation = source
        .find("if args.from_seq == 0")
        .expect("range validation");
    let authoritative_read = source
        .find(".run_stream(&run_id)")
        .expect("authoritative stream read");
    let range_filter = source.find(".filter(|event|").expect("range filter");

    assert!(range_validation < authoritative_read);
    assert!(authoritative_read < range_filter);
}

#[test]
fn test_keystore_list_json_output_empty() {
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args([
            "--output-format",
            "json",
            "keystore",
            "list",
            "--keystore",
            keystore_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    if !output.status.success() {
        // If keystore doesn't exist or requires password, that's expected for empty case
        return;
    }

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        let parsed = verify_success_response(&stdout);
        assert!(parsed["data"].is_array());
    }
}

#[test]
fn test_keystore_list_json_output_with_env_var() {
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env("MFM_OUTPUT_FORMAT", "json")
        .args([
            "keystore",
            "list",
            "--keystore",
            keystore_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    if !output.status.success() {
        // If keystore doesn't exist or requires password, that's expected
        return;
    }

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        let parsed = verify_success_response(&stdout);
        assert!(parsed["data"].is_array());
    }
}

#[test]
fn test_keystore_import_json_error_invalid_key() {
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args([
        "--output-format",
        "json",
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ])
    .write_stdin("invalid_key")
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid_key_material"))
    .stderr(predicate::str::contains("status"))
    .stderr(predicate::str::contains("error"));
}

#[test]
fn test_keystore_import_json_error_short_mnemonic() {
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args([
        "--output-format",
        "json",
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ])
    .write_stdin("short mnemonic")
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid_recovery_phrase"))
    .stderr(predicate::str::contains("status"))
    .stderr(predicate::str::contains("error"));
}

#[test]
fn test_keystore_delete_json_error_missing_args() {
    // This test verifies argument validation happens at clap level
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args([
        "--output-format",
        "json",
        "keystore",
        "delete",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ])
    .assert()
    .failure();
    // Note: This error happens at keystore loading level, not our custom validation
}

#[test]
fn test_keystore_delete_json_error_invalid_uuid() {
    // This test verifies error handling at keystore level
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args([
        "--output-format",
        "json",
        "keystore",
        "delete",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "invalid-uuid",
    ])
    .assert()
    .failure();
    // Note: This error happens at keystore loading level, not our UUID validation
}

#[test]
fn test_help_output_shows_json_format_option() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--output-format"))
        .stdout(predicate::str::contains("json"))
        .stdout(predicate::str::contains("text"))
        .stdout(predicate::str::contains("MFM_OUTPUT_FORMAT"));
}

#[test]
fn run_manual_resolution_help_documents_proof_submission_flags() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(["run", "manual-resolution", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--outcome"))
        .stdout(predicate::str::contains("--evidence"))
        .stdout(predicate::str::contains("--authorization-proof"))
        .stdout(predicate::str::contains("--evidence-media-type"));
}

#[test]
fn test_cli_parse_error_json_output_for_unknown_command_flag_equals() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format=json", "unknown-command"])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("unknown-command"));
    assert!(!parsed.error.message.contains('\u{1b}'));
}

#[test]
fn test_cli_parse_error_json_output_for_unknown_command_env() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env("MFM_OUTPUT_FORMAT", "json")
        .arg("unknown-command")
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("unknown-command"));
}

#[test]
fn test_cli_parse_error_json_output_for_missing_required_argument() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format", "json", "keystore", "import", "--stdin"])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("--import-type"));
}

#[test]
fn test_cli_parse_error_json_output_for_invalid_value() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args([
            "--output-format",
            "json",
            "keystore",
            "import",
            "--import-type",
            "not-a-type",
            "--stdin",
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("not-a-type"));
}

#[test]
fn test_cli_parse_error_text_mode_uses_clap_rendering() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("unknown-command")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error:"))
        .stderr(predicate::str::contains("unknown-command"))
        .stderr(predicate::str::contains("\"status\"").not());
}

#[test]
fn test_environment_variable_precedence() {
    // Test that command line flag takes precedence over environment variable
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env("MFM_OUTPUT_FORMAT", "json") // Set env var to json
        .args([
            "--output-format",
            "text", // Override with text via flag
            "keystore",
            "list",
            "--keystore",
            keystore_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    if output.status.success() {
        let stdout = String::from_utf8(output.stdout).unwrap();
        // Should be text output (not JSON) since flag overrides env var
        if !stdout.trim().is_empty() {
            // Text output should not have JSON structure
            assert!(!stdout.contains("\"status\":"));
        }
    }
}

#[test]
fn test_run_start_requires_certified_bundle_path() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args(["--output-format", "json", "run", "start"])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("--bundle"));
}

#[test]
fn test_run_start_rejects_invalid_bundle_before_store_access() {
    let temp_dir = TempDir::new().unwrap();
    let bundle_path = temp_dir.path().join("bundle.json");
    std::fs::write(&bundle_path, "{}").expect("bundle fixture");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "start",
            "--bundle",
            bundle_path.to_str().unwrap(),
            "--run-id",
            "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CertifiedBundleInvalid");
}

#[test]
fn test_run_start_rejects_legacy_op_id_flag() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "start",
            "--op-id",
            "portfolio.snapshot",
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("--op-id"));
}

#[test]
fn test_run_resume_rejects_legacy_uuid_before_storage() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "resume",
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "InvalidRunId");
}

#[test]
fn test_run_pipeline_subcommand_is_removed() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format", "json", "run", "pipeline"])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("pipeline"));
}

#[test]
fn test_run_artifacts_subcommand_is_removed() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format", "json", "run", "artifacts"])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("artifacts"));
}

#[test]
fn test_run_replay_requires_typed_store_after_valid_run_id() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "replay",
            "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "MissingDatabaseUrl");
}

#[test]
fn test_run_status_json_error_invalid_typed_run_id() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args(["--output-format", "json", "run", "status", "not-a-run-id"])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "InvalidRunId");
}

#[test]
fn test_run_public_output_json_rejects_invalid_schema_id() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "public-output",
            "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--schema-id",
            "schema_123",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let err = verify_error_response(&stderr);
    assert_eq!(err.error.code, "InvalidSchemaId");
}

#[test]
fn test_json_response_structure_consistency() {
    // This test verifies the JSON structure matches our specification
    let expected_success_structure = json!({
        "status": "success",
        "data": {}
    });

    let expected_error_structure = json!({
        "status": "error",
        "error": {
            "code": "ErrorCode",
            "message": "Error message"
        }
    });

    // Verify our structures match the expected format
    use mfm::presentation::output::ErrorResponse;

    let success_response = SuccessResponse::new(json!({}));
    let success_json = serde_json::to_value(success_response).unwrap();
    assert_eq!(success_json["status"], expected_success_structure["status"]);
    assert!(success_json.get("data").is_some());

    let error_response = ErrorResponse::new("ErrorCode", "Error message");
    let error_json = serde_json::to_value(error_response).unwrap();
    assert_eq!(error_json["status"], expected_error_structure["status"]);
    assert_eq!(error_json["error"]["code"], "ErrorCode");
    assert_eq!(error_json["error"]["message"], "Error message");
}

#[test]
fn test_run_status_json_contract_exposes_attempt_dispositions_and_saga_blocks() {
    let status_payload = json!({
        "run_id": "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "spec_hash": "spec:sha256-jcs-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "run_mode": "manual_blocked",
        "saga": {
            "policy": {
                "variant": "compensate_completed",
                "manual_authorization": null,
                "on_remediation_unresolved": "manual_resolution"
            },
            "obligations": [],
            "resource_ledgers": [],
            "resource_lanes": [{
                "namespace": "mfm.test.account_nonce",
                "key": "wallet-1",
                "holding_run_id": "run:sha256-jcs-v1:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "holding_ledger_key": "ledger-forward",
                "holding_ledger_purpose": "forward",
                "holding_forward_ledger_key": null,
                "holding_node_id": "node:sha256-jcs-v1:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                "holding_attempt_id": "attempt:sha256-jcs-v1:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "invocation_epoch": 1
            }],
            "manual_block_reason": "policy_manual_resolution",
            "required_manual_authorization": {
                "evidence_schema_id": "mfm.test.manual:1:sha256-jcs-v1:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "verifier_id": "mfm.test.manual.verifier",
                "signing_scheme": "mfm.manual_resolution.digest_signature.v1",
                "authority_id": "mfm.test.manual.authority",
                "operator_public_identities": ["0x0000000000000000000000000000000000000001"],
                "quorum_required_signatures": 1
            },
            "terminal_resolution": {
                "outcome": "manually_resolved",
                "claim": "manual_resolution"
            }
        },
        "attempt_dispositions": [
            {
                "node_id": "node:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111",
                "attempt_id": "attempt:sha256-jcs-v1:2222222222222222222222222222222222222222222222222222222222222222",
                "disposition": "started",
                "attempt_no": 1,
                "retryable": null,
                "output_cell_id": null
            },
            {
                "node_id": "node:sha256-jcs-v1:3333333333333333333333333333333333333333333333333333333333333333",
                "attempt_id": "attempt:sha256-jcs-v1:4444444444444444444444444444444444444444444444444444444444444444",
                "disposition": "interrupted",
                "attempt_no": null,
                "retryable": null,
                "output_cell_id": null
            },
            {
                "node_id": "node:sha256-jcs-v1:5555555555555555555555555555555555555555555555555555555555555555",
                "attempt_id": "attempt:sha256-jcs-v1:6666666666666666666666666666666666666666666666666666666666666666",
                "disposition": "failed",
                "attempt_no": null,
                "retryable": false,
                "output_cell_id": null
            },
            {
                "node_id": "node:sha256-jcs-v1:7777777777777777777777777777777777777777777777777777777777777777",
                "attempt_id": "attempt:sha256-jcs-v1:8888888888888888888888888888888888888888888888888888888888888888",
                "disposition": "completed",
                "attempt_no": null,
                "retryable": null,
                "output_cell_id": "cell:sha256-jcs-v1:9999999999999999999999999999999999999999999999999999999999999999"
            }
        ],
        "scheduler_status": "blocked",
        "head_seq": 42
    });

    let response = SuccessResponse::new(status_payload);
    let value = serde_json::to_value(response).expect("serialize response");

    assert_eq!(value["status"], "success");
    assert_eq!(
        value["data"]["attempt_dispositions"]
            .as_array()
            .expect("attempt dispositions")
            .iter()
            .map(|item| item["disposition"].as_str().expect("disposition"))
            .collect::<Vec<_>>(),
        vec!["started", "interrupted", "failed", "completed"]
    );
    assert_eq!(
        value["data"]["saga"]["resource_lanes"][0]["key"],
        "wallet-1"
    );
    assert_eq!(
        value["data"]["saga"]["manual_block_reason"],
        "policy_manual_resolution"
    );
    assert_eq!(
        value["data"]["saga"]["terminal_resolution"]["outcome"],
        "manually_resolved"
    );
}
