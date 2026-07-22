#![allow(clippy::needless_borrows_for_generic_args)]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn create_test_keystore() -> (TempDir, std::path::PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");
    (temp_dir, keystore_path)
}

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("--help");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("MFM - On-chain operations tool"))
        .stdout(predicate::str::contains("facts"))
        .stdout(predicate::str::contains("keystore"))
        .stdout(predicate::str::contains("ops"));
}

#[test]
fn test_ops_help_and_list() {
    let mut help = Command::cargo_bin("mfm_cli").unwrap();
    help.args(["ops", "--help"]);
    help.assert()
        .success()
        .stdout(predicate::str::contains(
            "Public entry-point operation discovery",
        ))
        .stdout(predicate::str::contains("list"));

    let mut list = Command::cargo_bin("mfm_cli").unwrap();
    let output = list
        .args(["ops", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rendered = String::from_utf8(output).expect("ops output is UTF-8");
    let entry_points = rendered
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    assert_eq!(entry_points.len(), 1, "public operation output: {rendered}");
    assert!(
        entry_points[0] == "mfm.portfolio/snapshot@1",
        "unexpected public operation: {rendered}"
    );
}

#[test]
fn test_facts_help() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&["facts", "--help"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "Public fact discovery and query operations",
        ))
        .stdout(predicate::str::contains("kinds"))
        .stdout(predicate::str::contains("describe"))
        .stdout(predicate::str::contains("explain"))
        .stdout(predicate::str::contains("query"))
        .stdout(predicate::str::contains("latest"))
        .stdout(predicate::str::contains("history"))
        .stdout(predicate::str::contains("top"))
        .stdout(predicate::str::contains("show"));
}

#[test]
fn test_facts_query_help_has_public_query_shape() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&["facts", "query", "--help"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("--kind"))
        .stdout(predicate::str::contains("--shape"))
        .stdout(predicate::str::contains("--order"))
        .stdout(predicate::str::contains("--subject"))
        .stdout(predicate::str::contains("--result"))
        .stdout(predicate::str::contains("--where"))
        .stdout(predicate::str::contains("--field"))
        .stdout(predicate::str::contains("--limit"))
        .stdout(predicate::str::contains("--audience").not())
        .stdout(predicate::str::contains("--scope").not())
        .stdout(predicate::str::contains("control").not())
        .stdout(predicate::str::contains("RunPrivate").not());
}

#[test]
fn facts_commands_use_public_evidence_only_services() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source =
        std::fs::read_to_string(manifest_dir.join("src/commands/facts.rs")).expect("facts source");

    assert!(
        source.contains("connect_application"),
        "facts commands must use the opaque application facade"
    );

    for forbidden in [
        "FactAudience::Control",
        "RunPrivate",
        "audience:",
        "scope:",
        "runtime_config",
        "start_entry_point_run",
        "resume_run",
        "record_manual_resolution",
    ] {
        assert!(
            !source.contains(forbidden),
            "facts command source must not expose {forbidden}"
        );
    }
}

#[test]
fn test_keystore_help_surfaces() {
    for (args, expected) in [
        (
            vec!["keystore", "--help"],
            vec!["Keystore management operations", "import", "delete", "list"],
        ),
        (
            vec!["keystore", "import", "--help"],
            vec![
                "Import a private key or mnemonic",
                "--import-type",
                "--passphrase-file",
                "--passphrase-prompt",
                "privatekey",
                "mnemonic",
            ],
        ),
        (
            vec!["keystore", "list", "--help"],
            vec!["List keys in the keystore", "--show-addresses"],
        ),
        (
            vec!["keystore", "delete", "--help"],
            vec!["Delete a key from the keystore", "--by-label", "--yes"],
        ),
        (
            vec!["keystore", "tx-sign", "--help"],
            vec!["--out", "--overwrite"],
        ),
    ] {
        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args(args);

        let output = cmd.assert().success().get_output().stdout.clone();
        let rendered = String::from_utf8(output).expect("help output is UTF-8");
        for expected in expected {
            assert!(
                rendered.contains(expected),
                "help output missing {expected:?}: {rendered}"
            );
        }
    }
}

#[test]
fn test_list_missing_keystore_error_cases() {
    for args in [
        vec!["keystore", "list"],
        vec!["--output-format", "json", "keystore", "list"],
    ] {
        let (_temp_dir, keystore_path) = create_test_keystore();

        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args(args)
            .args(&["--keystore", keystore_path.to_str().unwrap()]);

        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("Keystore not found"));
    }
}

#[test]
fn test_import_invalid_private_key_format() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ]);
    cmd.write_stdin("invalid_key");

    // This will fail during keystore creation since we need interactive password input
    // In a real scenario, we'd need to mock the password input
    cmd.assert().failure();
}

#[test]
fn test_import_missing_type() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "import",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_delete_error_cases() {
    for (args, expected) in [
        (Vec::new(), "Must specify either key ID or --by-label"),
        (vec!["invalid-uuid"], "Invalid UUID format"),
    ] {
        let (_temp_dir, keystore_path) = create_test_keystore();

        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args(&["keystore", "delete"])
            .args(args)
            .args(&["--keystore", keystore_path.to_str().unwrap()]);

        cmd.assert()
            .failure()
            .stderr(predicate::str::contains(expected));
    }
}

#[test]
fn test_list_invalid_format() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "--output-format",
        "invalid",
        "keystore",
        "list",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'invalid'"));
}

#[test]
fn test_version_flag() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("--version");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("mfm"));
}

#[test]
fn test_invalid_subcommands() {
    for args in [vec!["invalid"], vec!["keystore", "invalid"]] {
        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args(args);

        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("unrecognized subcommand"));
    }
}
