#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use assert_cmd::Command;
use mfm::presentation::output::{ErrorResponse, ResponseStatus};
use predicates::prelude::*;
use tempfile::TempDir;

const RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn standalone_ops_json_fails_the_same_production_bootstrap() {
    let error = json_error(vec!["ops", "list"]);
    assert_eq!(error.error.code(), "AuthoritativeWriterFenceUnavailable");
    assert_eq!(
        error.error.message(),
        "A deployment-owned authoritative-writer fence is required"
    );
}

#[test]
fn every_run_command_requires_the_global_credential_file() {
    let cases = [
        vec![
            "run",
            "admit",
            "mfm.portfolio/snapshot@1",
            "--invocation-identity",
            "00000000-0000-4000-8000-000000000000",
            "--target",
            "sample",
        ],
        vec!["run", "drive", RUN_ID],
        vec!["run", "show", RUN_ID],
        vec!["run", "replay", RUN_ID, "--mode", "verify"],
        vec!["run", "trace", RUN_ID],
        vec!["run", "audit", RUN_ID],
        vec![
            "run",
            "export",
            RUN_ID,
            "--kind",
            "semantic",
            "--output",
            "unused.stream",
            "--ref-output",
            "unused.stream.ref",
        ],
    ];

    for args in cases {
        let error = json_error(args);
        assert_eq!(error.error.code(), "AuthenticationRequired");
    }
}

#[test]
fn standalone_run_composition_fails_closed_without_a_writer_fence() {
    let directory = TempDir::new().expect("temporary credential directory");
    let credential = directory.path().join("access-token");
    std::fs::write(&credential, b"opaque\n").expect("credential file");

    let run_error = json_error(vec![
        "--access-token-file",
        credential.to_str().expect("credential path"),
        "run",
        "drive",
        RUN_ID,
    ]);
    assert_eq!(
        run_error.error.code(),
        "AuthoritativeWriterFenceUnavailable"
    );
}

#[test]
fn run_ids_and_page_limits_are_validated_before_process_composition() {
    let directory = TempDir::new().expect("temporary credential directory");
    let credential = directory.path().join("access-token");
    std::fs::write(&credential, b"opaque\n").expect("credential file");
    let credential = credential.to_str().expect("credential path");
    let output = directory.path().join("unused.stream");
    let output = output.to_str().expect("output path");
    let ref_output = directory.path().join("unused.stream.ref");
    let ref_output = ref_output.to_str().expect("ref output path");

    for args in [
        vec!["--access-token-file", credential, "run", "drive", "invalid"],
        vec!["--access-token-file", credential, "run", "show", "invalid"],
        vec![
            "--access-token-file",
            credential,
            "run",
            "replay",
            "invalid",
            "--mode",
            "verify",
        ],
        vec!["--access-token-file", credential, "run", "trace", "invalid"],
        vec!["--access-token-file", credential, "run", "audit", "invalid"],
        vec![
            "--access-token-file",
            credential,
            "run",
            "export",
            "invalid",
            "--kind",
            "semantic",
            "--output",
            output,
            "--ref-output",
            ref_output,
        ],
    ] {
        assert_eq!(json_error(args).error.code(), "InvalidRunId");
    }

    for command in ["trace", "audit"] {
        for limit in ["0", "501"] {
            let error = json_error(vec![
                "--access-token-file",
                credential,
                "run",
                command,
                RUN_ID,
                "--limit",
                limit,
            ]);
            assert_eq!(error.error.code(), "PageLimitInvalid");
        }
        let error = json_error(vec![
            "--access-token-file",
            credential,
            "run",
            command,
            RUN_ID,
            "--limit",
            "500",
        ]);
        assert_eq!(error.error.code(), "AuthoritativeWriterFenceUnavailable");
    }
}

#[test]
fn removed_commands_and_raw_authority_arguments_are_absent() {
    for args in [
        vec!["facts", "kinds"],
        vec!["run", "start"],
        vec!["run", "resume", RUN_ID],
        vec!["run", "status", RUN_ID],
        vec!["run", "stream", RUN_ID],
        vec!["run", "manual-resolution", RUN_ID],
        vec!["run", "public-output", RUN_ID],
        vec!["run", "list"],
        vec!["run", "watch"],
        vec!["--access-token", "opaque", "run", "show", RUN_ID],
        vec!["--tenant", "sample", "run", "show", RUN_ID],
    ] {
        Command::cargo_bin("mfm_cli")
            .expect("CLI")
            .args(args)
            .assert()
            .code(2);
    }
}

#[test]
fn compare_current_cli_syntax_is_rejected_by_current_parser() {
    Command::cargo_bin("mfm_cli")
        .expect("CLI")
        .args(["run", "replay", RUN_ID, "--mode", "compare_current"])
        .assert()
        .code(2);
    Command::cargo_bin("mfm_cli")
        .expect("CLI")
        .args(["run", "replay", RUN_ID, "--mode", "compare-current"])
        .assert()
        .code(2);
}

#[test]
fn replay_portable_export_flags_are_required_only_for_non_verify_modes() {
    let directory = TempDir::new().expect("temporary replay directory");
    let credential = directory.path().join("access-token");
    std::fs::write(&credential, b"opaque\n").expect("credential file");
    let credential = credential.to_str().expect("credential path");

    let required = json_error(vec![
        "--access-token-file",
        credential,
        "run",
        "replay",
        RUN_ID,
        "--mode",
        "reproduce",
    ]);
    assert_eq!(required.error.code(), "ReplayArtifactInvalid");
    assert_eq!(required.error.message(), "The replay artifact is invalid.");

    let forbidden = json_error(vec![
        "--access-token-file",
        credential,
        "run",
        "replay",
        RUN_ID,
        "--mode",
        "verify",
        "--portable-export",
        "unused.stream",
        "--portable-export-ref-file",
        "unused.stream.ref",
    ]);
    assert_eq!(forbidden.error.code(), "ReplayArtifactInvalid");
    assert_eq!(forbidden.error.message(), "The replay artifact is invalid.");
}

fn json_error(args: Vec<&str>) -> ErrorResponse {
    let output = Command::cargo_bin("mfm_cli")
        .expect("CLI")
        .env_remove("DATABASE_URL")
        .args(["--output-format", "json"])
        .args(args)
        .output()
        .expect("CLI invocation");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error: ErrorResponse = serde_json::from_slice(&output.stderr).expect("JSON error");
    assert!(matches!(error.status, ResponseStatus::Error));
    error
}

#[test]
fn json_parse_errors_remain_redaction_safe() {
    Command::cargo_bin("mfm_cli")
        .expect("CLI")
        .args(["--output-format=json", "unknown-command"])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("\"status\": \"error\""))
        .stderr(predicate::str::contains("CliParseError"));
}
