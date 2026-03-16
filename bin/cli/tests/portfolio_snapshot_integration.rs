use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use assert_cmd::Command;
use mfm::presentation::output::{ErrorResponse, ResponseStatus};

fn parse_error_response(stderr: &[u8]) -> ErrorResponse {
    let s = String::from_utf8(stderr.to_vec()).expect("stderr must be utf-8");
    let parsed: ErrorResponse =
        serde_json::from_str(&s).expect("stderr must be valid JSON ErrorResponse");
    assert!(matches!(parsed.status, ResponseStatus::Error));
    parsed
}

fn write_temp_request_file(contents: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("mfm-portfolio-request-{unique}.json"));
    fs::write(&path, contents).expect("write request file");
    path
}

#[test]
fn portfolio_snapshot_help_works() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(["portfolio", "snapshot", "--help"])
        .assert()
        .success();
}

#[test]
fn portfolio_snapshot_requires_request_json_or_file() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .env_remove("MFM_EVM_RPC_SOURCES_JSON")
        .args(["--output-format", "json", "portfolio", "snapshot"])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let parsed = parse_error_response(&output.stderr);
    assert_eq!(parsed.error.code, "MissingArgument");
}

#[test]
fn portfolio_snapshot_invalid_request_json_is_stable_error() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .env_remove("MFM_EVM_RPC_SOURCES_JSON")
        .args([
            "--output-format",
            "json",
            "portfolio",
            "snapshot",
            "--request-json",
            "not-json",
        ])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let parsed = parse_error_response(&output.stderr);
    assert_eq!(parsed.error.code, "InvalidJson");
}

#[test]
fn portfolio_snapshot_rejects_both_request_json_and_file() {
    let request_file = write_temp_request_file("{}");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .env_remove("MFM_EVM_RPC_SOURCES_JSON")
        .args([
            "--output-format",
            "json",
            "portfolio",
            "snapshot",
            "--request-json",
            "{}",
            "--request-file",
            request_file.to_str().expect("path utf8"),
        ])
        .output()
        .expect("command output");

    let _ = fs::remove_file(request_file);

    assert!(!output.status.success());
    let parsed = parse_error_response(&output.stderr);
    assert_eq!(parsed.error.code, "InvalidArguments");
}
