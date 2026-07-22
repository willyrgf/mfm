#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use assert_cmd::Command;
use mfm::presentation::output::{ErrorResponse, ResponseStatus, SuccessResponse};
use predicates::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;

const OUTPUT_FORMAT_ENV: &str = "MFM_OUTPUT_FORMAT";

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
fn test_ops_list_json_output() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format", "json", "ops", "list"])
        .output()
        .expect("run ops list");

    assert!(output.status.success());
    let data = verify_success_response(&String::from_utf8(output.stdout).expect("UTF-8 output"));
    let entry_points = data["entry_points"].as_array().expect("entry-points array");
    assert_eq!(entry_points.len(), 1);
    assert_eq!(entry_points[0], "mfm.portfolio/snapshot@2");
}

fn json_cli_error_without_database(args: &[&str]) -> (Option<i32>, ErrorResponse) {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args(args)
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    (output.status.code(), verify_error_response(&stderr))
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

    assert!(
        !source.contains(".load_run_stream("),
        "run stream command must delegate to app-level full stream validation"
    );
    assert!(range_validation < authoritative_read);
    assert!(authoritative_read < range_filter);
}

#[test]
fn test_keystore_list_json_output_sources() {
    enum JsonSource {
        Flag,
        Env,
    }

    for source in [JsonSource::Flag, JsonSource::Env] {
        let temp_dir = setup_temp_keystore();
        let keystore_path = temp_dir.path().join("test.keystore");
        let keystore = keystore_path.to_str().unwrap();

        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        let output = match source {
            JsonSource::Flag => cmd
                .args([
                    "--output-format",
                    "json",
                    "keystore",
                    "list",
                    "--keystore",
                    keystore,
                ])
                .output(),
            JsonSource::Env => cmd
                .env(OUTPUT_FORMAT_ENV, "json")
                .args(["keystore", "list", "--keystore", keystore])
                .output(),
        }
        .expect("Failed to execute command");

        if !output.status.success() {
            // If keystore doesn't exist or requires password, that's expected for empty case.
            continue;
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            let parsed = verify_success_response(&stdout);
            assert!(parsed["data"].is_array());
        }
    }
}

#[test]
fn test_keystore_import_json_errors_for_invalid_input() {
    for (import_type, stdin, error_code) in [
        (
            "privatekey",
            "raw-secret-must-not-escape",
            "invalid_key_material",
        ),
        (
            "mnemonic",
            "hidden phrase must never escape",
            "invalid_recovery_phrase",
        ),
    ] {
        let temp_dir = setup_temp_keystore();
        let keystore_path = temp_dir.path().join("test.keystore");

        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args([
            "--output-format",
            "json",
            "keystore",
            "import",
            "--import-type",
            import_type,
            "--keystore",
            keystore_path.to_str().unwrap(),
            "--stdin",
        ])
        .write_stdin(stdin)
        .assert()
        .failure()
        .stderr(predicate::str::contains(error_code))
        .stderr(predicate::str::contains("status"))
        .stderr(predicate::str::contains("error"))
        .stdout(predicate::str::contains(stdin).not())
        .stderr(predicate::str::contains(stdin).not());
    }
}

#[test]
fn test_keystore_delete_json_error_cases() {
    for args in [Vec::new(), vec!["invalid-uuid"]] {
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
        .args(args)
        .assert()
        .failure();
    }
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
        .stdout(predicate::str::contains(OUTPUT_FORMAT_ENV));
}

#[test]
fn test_cli_parse_error_json_output_cases() {
    struct Case {
        name: &'static str,
        env_json: bool,
        args: &'static [&'static str],
        message_fragment: &'static str,
        assert_no_ansi: bool,
    }

    for case in [
        Case {
            name: "unknown command via equals flag",
            env_json: false,
            args: &["--output-format=json", "unknown-command"],
            message_fragment: "unknown-command",
            assert_no_ansi: true,
        },
        Case {
            name: "unknown command via env",
            env_json: true,
            args: &["unknown-command"],
            message_fragment: "unknown-command",
            assert_no_ansi: false,
        },
        Case {
            name: "missing required argument",
            env_json: false,
            args: &["--output-format", "json", "keystore", "import", "--stdin"],
            message_fragment: "--import-type",
            assert_no_ansi: false,
        },
        Case {
            name: "invalid value",
            env_json: false,
            args: &[
                "--output-format",
                "json",
                "keystore",
                "import",
                "--import-type",
                "not-a-type",
                "--stdin",
            ],
            message_fragment: "not-a-type",
            assert_no_ansi: false,
        },
    ] {
        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        if case.env_json {
            cmd.env(OUTPUT_FORMAT_ENV, "json");
        }
        let output = cmd
            .args(case.args)
            .output()
            .expect("Failed to execute command");

        assert_eq!(output.status.code(), Some(2), "{}", case.name);
        assert!(output.stdout.is_empty(), "{}", case.name);
        let stderr = String::from_utf8(output.stderr).unwrap();
        let parsed = verify_error_response(&stderr);
        assert_eq!(parsed.error.code, "CliParseError", "{}", case.name);
        assert!(
            parsed.error.message.contains(case.message_fragment),
            "{}: {:?}",
            case.name,
            parsed.error.message
        );
        if case.assert_no_ansi {
            assert!(!parsed.error.message.contains('\u{1b}'), "{}", case.name);
        }
    }
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
        .env(OUTPUT_FORMAT_ENV, "json")
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
fn test_run_start_requires_entry_point_and_target() {
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
    assert!(parsed.error.message.contains("<ENTRY_POINT>"));
    assert!(parsed.error.message.contains("<TARGET>"));
}

#[test]
fn test_run_start_run_id_flag_is_not_a_start_option() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "start",
            "mfm.unknown/missing@1",
            "acme/primary",
            "--run-id",
            "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("--run-id"));
}

#[test]
fn test_run_start_rejects_legacy_config_flags() {
    for flag in [
        "--entry-point",
        "--request",
        "--op",
        "--op-version",
        "--config",
        "--config-format",
    ] {
        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        let output = cmd
            .env_remove("DATABASE_URL")
            .args([
                "--output-format",
                "json",
                "run",
                "start",
                "mfm.unknown/missing@1",
                "acme/primary",
                flag,
                "legacy-value",
            ])
            .output()
            .expect("run CLI");

        assert!(!output.status.success(), "legacy flag {flag} was accepted");
        assert!(output.stdout.is_empty());
        let parsed = verify_error_response(&String::from_utf8(output.stderr).unwrap());
        assert_eq!(parsed.error.code, "CliParseError");
        assert!(
            parsed.error.message.contains(flag),
            "legacy flag {flag} missing from parse error: {}",
            parsed.error.message
        );
    }
}

#[test]
fn test_json_commands_reach_store_connection_after_local_validation() {
    struct Case<'a> {
        name: &'static str,
        args: Vec<&'a str>,
    }

    for case in [
        Case {
            name: "run start decodes target ingress before store connection",
            args: vec![
                "--output-format",
                "json",
                "run",
                "start",
                "mfm.unknown/missing@1",
                "acme/primary",
            ],
        },
        Case {
            name: "facts kinds reaches evidence-only store connection",
            args: vec!["--output-format", "json", "facts", "kinds"],
        },
        Case {
            name: "facts query parses public flags before store connection",
            args: vec![
                "--output-format",
                "json",
                "facts",
                "query",
                "--kind",
                "wallet.balance",
                "--shape",
                "mfm.wallet.balance.v1",
                "--order",
                "result.amount_sat.desc",
                "--subject",
                "asset_ref=btc",
                "--result",
                "amount_sat.gt=1000",
                "--where",
                "metadata.recorded_at.lte=timestamp:2026-07-02T00:00:00Z",
                "--field",
                "subject.asset_ref",
                "--field",
                "result.amount_sat",
                "--limit",
                "20",
            ],
        },
        Case {
            name: "run replay validates run id before store connection",
            args: vec![
                "--output-format",
                "json",
                "run",
                "replay",
                "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ],
        },
    ] {
        let (_status, parsed) = json_cli_error_without_database(&case.args);
        assert_eq!(parsed.error.code, "MissingDatabaseUrl", "{}", case.name);
    }
}

#[test]
fn test_json_commands_report_local_validation_errors_before_store_connection() {
    struct Case {
        name: &'static str,
        args: &'static [&'static str],
        expected_status: Option<i32>,
        expected_code: &'static str,
        message_fragment: Option<&'static str>,
    }

    for case in [
        Case {
            name: "facts show rejects invalid public ref",
            args: &[
                "--output-format",
                "json",
                "facts",
                "show",
                "not-a-public-ref",
            ],
            expected_status: None,
            expected_code: "PublicFactRefInvalid",
            message_fragment: None,
        },
        Case {
            name: "facts query requires return fields",
            args: &[
                "--output-format",
                "json",
                "facts",
                "query",
                "--kind",
                "wallet.balance",
                "--order",
                "result.amount_sat.desc",
            ],
            expected_status: Some(2),
            expected_code: "CliParseError",
            message_fragment: Some("--field"),
        },
        Case {
            name: "run status rejects invalid run id",
            args: &["--output-format", "json", "run", "status", "not-a-run-id"],
            expected_status: None,
            expected_code: "InvalidRunId",
            message_fragment: None,
        },
        Case {
            name: "run public output rejects invalid schema id",
            args: &[
                "--output-format",
                "json",
                "run",
                "public-output",
                "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "--schema-id",
                "schema_123",
            ],
            expected_status: None,
            expected_code: "InvalidSchemaId",
            message_fragment: None,
        },
    ] {
        let (status, parsed) = json_cli_error_without_database(case.args);
        if let Some(expected) = case.expected_status {
            assert_eq!(status, Some(expected), "{}", case.name);
        }
        assert_eq!(parsed.error.code, case.expected_code, "{}", case.name);
        if let Some(fragment) = case.message_fragment {
            assert!(
                parsed.error.message.contains(fragment),
                "{}: {:?}",
                case.name,
                parsed.error.message
            );
        }
    }
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

    let error_response = ErrorResponse::new(mfm_app::PublicError::bad_request(
        "ErrorCode",
        "Error message",
    ));
    let error_json = serde_json::to_value(error_response).unwrap();
    assert_eq!(error_json["status"], expected_error_structure["status"]);
    assert_eq!(error_json["error"]["code"], "ErrorCode");
    assert_eq!(error_json["error"]["message"], "Error message");
}
