#![allow(clippy::needless_borrows_for_generic_args)]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_environment_variable_configuration() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");

    // Set password via environment variable
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.env("MFM_KEYSTORE_PASSWORD", "env_password_123");
    cmd.env("MFM_INTEGRATION_TEST", "1");
    cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "private-key",
        "--label",
        "env-test",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ]);
    cmd.write_stdin("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");

    cmd.assert().success().stdout(predicate::str::contains(
        "Private key imported successfully",
    ));

    // Verify we can list with same environment variable
    let mut list_cmd = Command::cargo_bin("mfm_cli").unwrap();
    list_cmd.env("MFM_KEYSTORE_PASSWORD", "env_password_123");
    list_cmd.env("MFM_INTEGRATION_TEST", "1");
    list_cmd.args(&[
        "keystore",
        "list",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("env-test"));
}

#[test]
fn test_wrong_password_environment_variable() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");

    // Create keystore with one password
    let mut create_cmd = Command::cargo_bin("mfm_cli").unwrap();
    create_cmd.env("MFM_KEYSTORE_PASSWORD", "correct_password");
    create_cmd.env("MFM_INTEGRATION_TEST", "1");
    create_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "private-key",
        "--label",
        "test-key",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ]);
    create_cmd.write_stdin("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");

    create_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Private key imported successfully",
        ));

    // Try to access with wrong password
    let mut wrong_cmd = Command::cargo_bin("mfm_cli").unwrap();
    wrong_cmd.env("MFM_KEYSTORE_PASSWORD", "wrong_password");
    wrong_cmd.env("MFM_INTEGRATION_TEST", "1");
    wrong_cmd.args(&[
        "keystore",
        "list",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    wrong_cmd.assert().failure().stderr(
        predicate::str::contains("Invalid password")
            .or(predicate::str::contains("InvalidPassword")),
    );
}

#[test]
fn test_keystore_path_environment_variable() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("env_keystore");

    // Use environment variable for keystore path
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.env("MFM_KEYSTORE_PASSWORD", "test_password");
    cmd.env("MFM_KEYSTORE_PATH", keystore_path.to_str().unwrap());
    cmd.env("MFM_INTEGRATION_TEST", "1");
    cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "private-key",
        "--label",
        "env-path-test",
        "--stdin",
    ]);
    cmd.write_stdin("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");

    cmd.assert().success().stdout(predicate::str::contains(
        "Private key imported successfully",
    ));

    // Verify keystore was created at environment path
    assert!(
        keystore_path.exists(),
        "Keystore should exist at environment path"
    );

    // List using environment path
    let mut list_cmd = Command::cargo_bin("mfm_cli").unwrap();
    list_cmd.env("MFM_KEYSTORE_PASSWORD", "test_password");
    list_cmd.env("MFM_KEYSTORE_PATH", keystore_path.to_str().unwrap());
    list_cmd.env("MFM_INTEGRATION_TEST", "1");
    list_cmd.args(&["keystore", "list"]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("env-path-test"));
}

#[test]
fn test_output_mode_environment_variable() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");

    // Create a key first
    let mut create_cmd = Command::cargo_bin("mfm_cli").unwrap();
    create_cmd.env("MFM_KEYSTORE_PASSWORD", "test_password");
    create_cmd.env("MFM_INTEGRATION_TEST", "1");
    create_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "private-key",
        "--label",
        "output-test",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ]);
    create_cmd.write_stdin("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");

    create_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Private key imported successfully",
        ));

    // Test machine output mode via environment
    let mut list_cmd = Command::cargo_bin("mfm_cli").unwrap();
    list_cmd.env("MFM_KEYSTORE_PASSWORD", "test_password");
    list_cmd.env("MFM_OUTPUT_MODE", "machine");
    list_cmd.env("MFM_INTEGRATION_TEST", "1");
    list_cmd.args(&[
        "keystore",
        "list",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    // When output mode environment variable is implemented, this should produce JSON
    // For now, this tests that the environment variable doesn't break the command
    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("output-test"));
}
