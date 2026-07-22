#![allow(clippy::disallowed_methods)]
#![allow(clippy::needless_borrows_for_generic_args)]

use assert_cmd::Command;
use mfm_app::{initialize_insecure_keystore_for_test, SecretInput};
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

const TEST_PASSWORD: &str = "test_password_123";

struct TestEnv {
    temp_dir: TempDir,
    keystore_path: std::path::PathBuf,
    runtime_config_path: std::path::PathBuf,
}

/// Create a fast test keystore and runtime config profile.
fn create_test_env() -> TestEnv {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");
    let password_file = temp_dir.path().join("password.txt");
    let runtime_config_path = temp_dir.path().join("runtime.toml");
    fs::write(&password_file, format!("{TEST_PASSWORD}\n")).expect("write password file");

    initialize_insecure_keystore_for_test(
        keystore_path.clone(),
        SecretInput::new(TEST_PASSWORD.to_owned()),
        Vec::new(),
    )
    .expect("create test keystore");

    let runtime_config = format!(
        r#"
[keystores.default]
keystore_path = {keystore_path}
unlock_file = {password_file}
"#,
        keystore_path = toml_string(&keystore_path.display().to_string()),
        password_file = toml_string(&password_file.display().to_string()),
    );
    fs::write(&runtime_config_path, runtime_config).expect("write runtime config");

    TestEnv {
        temp_dir,
        keystore_path,
        runtime_config_path,
    }
}

/// Helper to run CLI commands with runtime config set.
fn cli_with_runtime_config(env: &TestEnv) -> Command {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.env(
        "MFM_RUNTIME_CONFIG_FILE",
        env.runtime_config_path.to_str().unwrap(),
    );
    cmd
}

fn import_private_key(test_env: &TestEnv, label: Option<&str>, private_key: &str) {
    let mut import_cmd = cli_with_runtime_config(test_env);
    import_cmd.args(&["keystore", "import", "--import-type", "privatekey"]);
    if let Some(label) = label {
        import_cmd.args(&["--label", label]);
    }
    import_cmd.arg("--stdin");
    import_cmd.write_stdin(private_key);

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "private key imported successfully",
        ))
        .stdout(predicate::str::contains(private_key).not())
        .stderr(predicate::str::contains(private_key).not());
}

fn import_mnemonic(test_env: &TestEnv, label: &str, derivation_path: Option<&str>, mnemonic: &str) {
    let mut import_cmd = cli_with_runtime_config(test_env);
    import_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--label",
        label,
    ]);
    if let Some(derivation_path) = derivation_path {
        import_cmd.args(&["--derivation-path", derivation_path]);
    }
    import_cmd.arg("--stdin");
    import_cmd.write_stdin(mnemonic);

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("hd_derived imported successfully"))
        .stdout(predicate::str::contains(mnemonic).not())
        .stderr(predicate::str::contains(mnemonic).not());
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("toml string")
}

#[test]
fn test_e2e_private_key_workflow() {
    let test_env = create_test_env();
    let keystore_path = test_env.keystore_path.clone();

    // Step 1: Import a private key
    import_private_key(
        &test_env,
        Some("test-wallet"),
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    );
    import_private_key(
        &test_env,
        None,
        "2222222222222222222222222222222222222222222222222222222222222222",
    );

    // Step 2: List keys to verify explicit and generated labels.
    let mut list_cmd = cli_with_runtime_config(&test_env);
    list_cmd.args(&["keystore", "list"]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("test-wallet"))
        .stdout(predicate::str::contains("imported-key-"))
        .stdout(predicate::str::contains("privatekey"));

    // Step 3: List keys in JSON format
    let mut list_json_cmd = cli_with_runtime_config(&test_env);
    list_json_cmd.args(&["--output-format", "json", "keystore", "list"]);

    list_json_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("test-wallet"))
        .stdout(predicate::str::contains("\"key_type\": \"privatekey\""));

    // Step 4: List keys with addresses
    let mut list_addr_cmd = cli_with_runtime_config(&test_env);
    list_addr_cmd.args(&["keystore", "list", "--show-addresses"]);

    list_addr_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("test-wallet"))
        .stdout(predicate::str::contains("0x")); // Should show Ethereum address

    // Verify keystore file was actually created
    assert!(keystore_path.exists(), "Keystore file should exist");
}

#[test]
fn test_e2e_mnemonic_workflow() {
    let test_env = create_test_env();

    // Step 1: Import a mnemonic
    import_mnemonic(
        &test_env,
        "hd-wallet",
        Some("m/44'/60'/0'/0/0"),
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    );

    // Step 2: List keys to verify import
    let mut list_cmd = cli_with_runtime_config(&test_env);
    list_cmd.args(&["keystore", "list"]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("hd-wallet"));

    // Step 3: Import another mnemonic with different derivation path
    import_mnemonic(
        &test_env,
        "hd-wallet-2",
        Some("m/44'/60'/1'/0/0"),
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    );

    // Step 4: List should now show 2 keys
    let mut list_cmd2 = cli_with_runtime_config(&test_env);
    list_cmd2.args(&["--output-format", "json", "keystore", "list"]);

    let _output = list_cmd2
        .assert()
        .success()
        .stdout(predicate::str::contains("hd-wallet"))
        .stdout(predicate::str::contains("hd-wallet-2"));
}

#[test]
fn test_e2e_mnemonic_passphrase_file_workflow() {
    let test_env = create_test_env();
    let extra_path = test_env.temp_dir.path().join("bip39-extra");
    fs::write(&extra_path, "test extra input\n").expect("write BIP-39 extra input file");
    let extra_str = extra_path.to_str().unwrap();
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    let mut import_with_extra = cli_with_runtime_config(&test_env);
    import_with_extra.args(&[
        "--output-format",
        "json",
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--label",
        "hd-wallet-extra",
        "--passphrase-file",
        extra_str,
        "--stdin",
    ]);
    import_with_extra.write_stdin(mnemonic);
    let with_extra_output = import_with_extra
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let with_extra_json: serde_json::Value =
        serde_json::from_slice(&with_extra_output).expect("import output should be json");
    let with_extra_addr = with_extra_json["data"]["address"]
        .as_str()
        .expect("import output should include address")
        .to_string();

    let mut import_without_extra = cli_with_runtime_config(&test_env);
    import_without_extra.args(&[
        "--output-format",
        "json",
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--label",
        "hd-wallet-no-extra",
        "--stdin",
    ]);
    import_without_extra.write_stdin(mnemonic);
    let without_extra_output = import_without_extra
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let without_extra_json: serde_json::Value =
        serde_json::from_slice(&without_extra_output).expect("import output should be json");
    let without_extra_addr = without_extra_json["data"]["address"]
        .as_str()
        .expect("import output should include address");

    assert_ne!(
        with_extra_addr, without_extra_addr,
        "file-sourced BIP-39 extra input should affect derived key"
    );
}

#[test]
fn test_e2e_mnemonic_stdin_rejects_two_field_protocol() {
    let test_env = create_test_env();

    let mut import_cmd = cli_with_runtime_config(&test_env);
    import_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--label",
        "hd-wallet",
        "--stdin",
    ]);
    import_cmd.write_stdin(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about\nsecond-field\n",
    );

    import_cmd
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "stdin secret material must contain exactly one line",
        ))
        .stderr(predicate::str::contains("second-field").not());
}

#[test]
fn test_e2e_delete_workflow() {
    let test_env = create_test_env();

    // Step 1: Import a key
    import_private_key(
        &test_env,
        Some("deleteme"),
        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    );

    // Step 2: Import another key to keep
    import_private_key(
        &test_env,
        Some("keepme"),
        "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
    );

    // Step 3: List keys to verify both exist
    let mut list_cmd = cli_with_runtime_config(&test_env);
    list_cmd.args(&["keystore", "list"]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("deleteme"))
        .stdout(predicate::str::contains("keepme"));

    // Step 4: Delete by label with --yes flag
    let mut delete_cmd = cli_with_runtime_config(&test_env);
    delete_cmd.args(&["keystore", "delete", "--by-label", "deleteme", "--yes"]);

    delete_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted successfully"));

    // Step 5: List keys to verify only one remains
    let mut list_cmd2 = cli_with_runtime_config(&test_env);
    list_cmd2.args(&["keystore", "list"]);

    list_cmd2
        .assert()
        .success()
        .stdout(predicate::str::contains("keepme"))
        .stdout(predicate::str::contains("deleteme").not());
}

#[test]
fn test_e2e_mixed_key_types_workflow() {
    let test_env = create_test_env();

    // Import private key
    import_private_key(
        &test_env,
        Some("pk-wallet"),
        "1111111111111111111111111111111111111111111111111111111111111111",
    );

    // Import mnemonic
    import_mnemonic(
        &test_env,
        "mn-wallet",
        None,
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    );

    // List all keys
    let mut list_cmd = cli_with_runtime_config(&test_env);
    list_cmd.args(&[
        "--output-format",
        "json",
        "keystore",
        "list",
        "--show-addresses",
    ]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("pk-wallet"))
        .stdout(predicate::str::contains("mn-wallet"))
        .stdout(predicate::str::contains("privatekey"))
        .stdout(predicate::str::contains("hd_derived"));

    // Test filtering by label pattern
    let mut filter_cmd = cli_with_runtime_config(&test_env);
    filter_cmd.args(&["keystore", "list", "--filter-label", "pk.*"]);

    filter_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("pk-wallet"))
        .stdout(predicate::str::contains("mn-wallet").not());
}

#[test]
fn test_e2e_sorting_and_formatting() {
    let test_env = create_test_env();

    // Import multiple keys with different labels
    for (i, label) in ["zebra", "alpha", "beta"].iter().enumerate() {
        let pk = format!("{:064x}", i + 1);
        import_private_key(&test_env, Some(label), pk.as_str());

        // Small delay to ensure different timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Test sorting by label
    let mut sort_label_cmd = cli_with_runtime_config(&test_env);
    sort_label_cmd.args(&["keystore", "list", "--sort-by", "label"]);

    sort_label_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha"))
        .stdout(predicate::str::contains("beta"))
        .stdout(predicate::str::contains("zebra"));

    // Test JSON format with addresses
    let mut json_cmd = cli_with_runtime_config(&test_env);
    json_cmd.args(&[
        "--output-format",
        "json",
        "keystore",
        "list",
        "--show-addresses",
        "--sort-by",
        "created",
    ]);

    json_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\":"))
        .stdout(predicate::str::contains("\"label\":"))
        .stdout(predicate::str::contains("\"address\":"))
        .stdout(predicate::str::contains("\"created\":"));
}
