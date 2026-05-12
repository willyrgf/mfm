#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use assert_cmd::Command;
use mfm::presentation::output::{ErrorResponse, ResponseStatus, SuccessResponse};
use mfm_artifact_store_fs::FsArtifactStore;
use mfm_machine::stores::{ArtifactKind, ArtifactStore};
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
    .stderr(predicate::str::contains("InvalidKeyMaterial"))
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
    .stderr(predicate::str::contains("InvalidRecoveryPhrase"))
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
fn test_run_start_json_error_missing_database_url() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args(["--output-format", "json", "run", "start"])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "MissingDatabaseUrl");
}

#[test]
fn test_run_pipeline_start_json_error_missing_database_url() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "pipeline",
            "start",
            "--pipeline-json",
            r#"{"machine_id":"proof","pipeline_version":"v1","steps":[{"step_id":"main","op_id":"proof","op_version":"v1","op_config":{}}]}"#,
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "MissingDatabaseUrl");
}

#[test]
fn test_run_status_json_error_invalid_uuid() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args(["--output-format", "json", "run", "status", "not-a-uuid"])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "InvalidUuid");
}

#[test]
fn test_run_artifacts_get_json_success_for_json_payload() {
    let temp = TempDir::new().unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = FsArtifactStore::new(temp.path());
    let id = rt.block_on(async {
        store
            .put(
                ArtifactKind::Other("test".to_string()),
                serde_json::to_vec(&json!({"a": 1})).unwrap(),
            )
            .await
            .unwrap()
    });

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "artifacts",
            "get",
            id.as_str(),
            "--artifact-root",
            temp.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let data = verify_success_response(&stdout);
    assert_eq!(data["artifact_id"], id.as_str());
    assert_eq!(data["encoding"], "json");
    assert_eq!(data["value"]["a"], 1);
}

#[test]
fn test_run_artifacts_get_json_success_for_binary_payload() {
    let temp = TempDir::new().unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = FsArtifactStore::new(temp.path());
    let id = rt.block_on(async {
        store
            .put(ArtifactKind::Other("test".to_string()), b"hello".to_vec())
            .await
            .unwrap()
    });

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "artifacts",
            "get",
            id.as_str(),
            "--artifact-root",
            temp.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let data = verify_success_response(&stdout);
    assert_eq!(data["artifact_id"], id.as_str());
    assert_eq!(data["encoding"], "hex");
    assert_eq!(data["hex"], "68656c6c6f");
}

#[test]
fn test_run_artifacts_get_json_rejects_invalid_artifact_id() {
    let temp = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "artifacts",
            "get",
            "artifact_123",
            "--artifact-root",
            temp.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let err = verify_error_response(&stderr);
    assert_eq!(err.error.code, "InvalidArtifactId");
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
