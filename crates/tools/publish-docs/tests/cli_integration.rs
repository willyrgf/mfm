use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;

fn verify_error_response(output: &str) -> Value {
    let parsed: Value = serde_json::from_str(output).expect("valid json");
    assert_eq!(parsed["status"], "error");
    parsed
}

#[test]
fn help_output_shows_output_format_and_env_var() {
    let mut cmd = Command::cargo_bin("mfm-publish-docs").expect("binary");
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--output-format"))
        .stdout(predicate::str::contains("MFM_OUTPUT_FORMAT"))
        .stdout(predicate::str::contains("LOG_LEVEL"))
        .stdout(predicate::str::contains("plan"))
        .stdout(predicate::str::contains("apply"))
        .stdout(predicate::str::contains("cargo publish"))
        .stdout(predicate::str::contains(
            "omitting the subcommand defaults to apply",
        ))
        .stdout(predicate::str::contains("resume"))
        .stdout(predicate::str::contains("sync-umbrella"))
        .stdout(predicate::str::contains("yank"));
}

#[test]
fn architecture_docs_do_not_describe_publish_docs_as_plan_only() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/architecture.md");
    let docs = std::fs::read_to_string(docs_path).expect("architecture docs");

    assert!(
        !docs.contains(
            "publish-docs` intentionally stops at canonical config plus typed plan artifacts"
        ),
        "architecture docs must not describe publish-docs as plan-only"
    );
    assert!(docs.contains("`publish-docs` is separate release tooling"));
    assert!(docs.contains("`apply` and the default command can run `cargo publish`"));
}

#[test]
fn unknown_package_renders_json_error_envelope() {
    let mut cmd = Command::cargo_bin("mfm-publish-docs").expect("binary");
    let output = cmd
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "--output-format",
            "json",
            "plan",
            "--allow-dirty",
            "--only",
            "not-a-package",
        ])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    let parsed = verify_error_response(&stdout);
    assert_eq!(parsed["error"]["code"], "UnknownPackage");
    assert!(!stderr.contains("\"status\""));
}

#[test]
fn output_flag_overrides_json_env_var() {
    let mut cmd = Command::cargo_bin("mfm-publish-docs").expect("binary");
    let output = cmd
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("MFM_OUTPUT_FORMAT", "json")
        .args([
            "--output-format",
            "text",
            "plan",
            "--allow-dirty",
            "--only",
            "not-a-package",
        ])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert!(stderr.contains("Error:"));
    assert!(!stderr.contains("\"status\""));
}

#[test]
fn resume_missing_run_renders_json_error_envelope() {
    let mut cmd = Command::cargo_bin("mfm-publish-docs").expect("binary");
    let output = cmd
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["resume", "missing-run", "--json"])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    let parsed = verify_error_response(&stdout);
    assert_eq!(parsed["error"]["code"], "IoError");
    assert!(!stderr.contains("\"status\""));
}

#[test]
fn yank_disallowed_by_catalog_renders_json_error_envelope() {
    let mut cmd = Command::cargo_bin("mfm-publish-docs").expect("binary");
    let output = cmd
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["yank", "mfm-machine", "--json"])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    let parsed = verify_error_response(&stdout);
    assert_eq!(parsed["error"]["code"], "CommandFailed");
    assert_eq!(
        parsed["error"]["message"],
        "catalog disallows yanking package=mfm-machine"
    );
    assert!(!stderr.contains("\"status\""));
}

#[test]
fn json_output_stays_on_stdout_when_logs_are_enabled() {
    let mut cmd = Command::cargo_bin("mfm-publish-docs").expect("binary");
    let output = cmd
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("LOG_LEVEL", "info")
        .args([
            "--output-format",
            "json",
            "plan",
            "--allow-dirty",
            "--only",
            "not-a-package",
        ])
        .output()
        .expect("command output");

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    let parsed = verify_error_response(&stdout);
    assert_eq!(parsed["error"]["code"], "UnknownPackage");
    assert!(stderr.contains("publish-docs command starting"));
    assert!(stderr.contains("preparing publish-docs run"));
    assert!(stderr.contains("loaded publish wave"));
    assert!(!stderr.contains("\"status\""));
}
