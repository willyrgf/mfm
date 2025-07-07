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
        .stdout(predicate::str::contains("keystore"));
}

#[test]
fn test_keystore_help() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&["keystore", "--help"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Keystore management operations"))
        .stdout(predicate::str::contains("import"))
        .stdout(predicate::str::contains("delete"))
        .stdout(predicate::str::contains("list"));
}

#[test]
fn test_import_help() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&["keystore", "import", "--help"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Import a private key or mnemonic"))
        .stdout(predicate::str::contains("--import-type"))
        .stdout(predicate::str::contains("privatekey"))
        .stdout(predicate::str::contains("mnemonic"));
}

#[test]
fn test_list_help() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&["keystore", "list", "--help"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("List keys in the keystore"))
        .stdout(predicate::str::contains("--format"))
        .stdout(predicate::str::contains("--show-addresses"));
}

#[test]
fn test_delete_help() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&["keystore", "delete", "--help"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Delete a key from the keystore"))
        .stdout(predicate::str::contains("--by-label"))
        .stdout(predicate::str::contains("--yes"));
}

#[test]
fn test_list_empty_keystore() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "list",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Keystore not found"));
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
fn test_delete_missing_arguments() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "delete",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Keystore not found"));
}

#[test]
fn test_delete_invalid_uuid() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "delete",
        "invalid-uuid",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Keystore not found"));
}

#[test]
fn test_list_json_format_option() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "list",
        "--format",
        "json",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Keystore not found"));
}

#[test]
fn test_list_invalid_format() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "list",
        "--format",
        "invalid",
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

// Test invalid commands
#[test]
fn test_invalid_subcommand() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("invalid");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn test_keystore_invalid_subcommand() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&["keystore", "invalid"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}
