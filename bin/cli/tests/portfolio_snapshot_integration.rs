use assert_cmd::Command;
use mfm::presentation::output::{ErrorResponse, ResponseStatus};

fn parse_error_response(stderr: &[u8]) -> ErrorResponse {
    let s = String::from_utf8(stderr.to_vec()).expect("stderr must be utf-8");
    let parsed: ErrorResponse =
        serde_json::from_str(&s).expect("stderr must be valid JSON ErrorResponse");
    assert!(matches!(parsed.status, ResponseStatus::Error));
    parsed
}

#[test]
fn portfolio_snapshot_help_works() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(["portfolio", "snapshot", "--help"])
        .assert()
        .success();
}

#[test]
fn portfolio_snapshot_invalid_tokens_json_is_stable_error() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .env_remove("MFM_EVM_RPC_URL")
        .args([
            "--output-format",
            "json",
            "portfolio",
            "snapshot",
            "0x000000000000000000000000000000000000dead",
            "--tokens-json",
            "not-json",
        ])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let parsed = parse_error_response(&output.stderr);
    assert_eq!(parsed.error.code, "InvalidJson");
}

#[test]
fn portfolio_snapshot_tokens_json_must_be_array_is_stable_error() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .env_remove("MFM_EVM_RPC_URL")
        .args([
            "--output-format",
            "json",
            "portfolio",
            "snapshot",
            "0x000000000000000000000000000000000000dead",
            "--tokens-json",
            r#"{"not":"an array"}"#,
        ])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let parsed = parse_error_response(&output.stderr);
    assert_eq!(parsed.error.code, "InvalidJson");
}

