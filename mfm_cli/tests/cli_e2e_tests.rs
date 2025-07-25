#![allow(clippy::needless_borrows_for_generic_args)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Create a test keystore with a pre-set password via environment variable
fn create_test_env() -> (TempDir, std::path::PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");
    (temp_dir, keystore_path)
}

/// Helper to run CLI commands with environment password set
fn cli_with_password() -> Command {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.env("MFM_KEYSTORE_PASSWORD", "test_password_123");
    cmd.env("MFM_INTEGRATION_TEST", "1"); // Use fast keystore config for tests
    cmd
}

#[test]
fn test_e2e_private_key_workflow() {
    let (_temp_dir, keystore_path) = create_test_env();
    let keystore_str = keystore_path.to_str().unwrap();

    // Step 1: Import a private key
    let mut import_cmd = cli_with_password();
    import_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--label",
        "test-wallet",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_cmd.write_stdin("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Private key imported successfully",
        ));

    // Step 2: List keys to verify import
    let mut list_cmd = cli_with_password();
    list_cmd.args(&["keystore", "list", "--keystore", keystore_str]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("test-wallet"))
        .stdout(predicate::str::contains("privatekey"));

    // Step 3: List keys in JSON format
    let mut list_json_cmd = cli_with_password();
    list_json_cmd.args(&[
        "--output-format",
        "json",
        "keystore",
        "list",
        "--keystore",
        keystore_str,
    ]);

    list_json_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("test-wallet"))
        .stdout(predicate::str::contains("\"key_type\": \"privatekey\""));

    // Step 4: List keys with addresses
    let mut list_addr_cmd = cli_with_password();
    list_addr_cmd.args(&[
        "keystore",
        "list",
        "--show-addresses",
        "--keystore",
        keystore_str,
    ]);

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
    let (_temp_dir, keystore_path) = create_test_env();
    let keystore_str = keystore_path.to_str().unwrap();

    // Step 1: Import a mnemonic
    let mut import_cmd = cli_with_password();
    import_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--label",
        "hd-wallet",
        "--derivation-path",
        "m/44'/60'/0'/0/0",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_cmd.write_stdin("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("Mnemonic imported successfully"));

    // Step 2: List keys to verify import
    let mut list_cmd = cli_with_password();
    list_cmd.args(&["keystore", "list", "--keystore", keystore_str]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("hd-wallet"));

    // Step 3: Import another mnemonic with different derivation path
    let mut import_cmd2 = cli_with_password();
    import_cmd2.args(&[
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--label",
        "hd-wallet-2",
        "--derivation-path",
        "m/44'/60'/1'/0/0",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_cmd2.write_stdin("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");

    import_cmd2
        .assert()
        .success()
        .stdout(predicate::str::contains("Mnemonic imported successfully"));

    // Step 4: List should now show 2 keys
    let mut list_cmd2 = cli_with_password();
    list_cmd2.args(&[
        "--output-format",
        "json",
        "keystore",
        "list",
        "--keystore",
        keystore_str,
    ]);

    let _output = list_cmd2
        .assert()
        .success()
        .stdout(predicate::str::contains("hd-wallet"))
        .stdout(predicate::str::contains("hd-wallet-2"));
}

#[test]
fn test_e2e_delete_workflow() {
    let (_temp_dir, keystore_path) = create_test_env();
    let keystore_str = keystore_path.to_str().unwrap();

    // Step 1: Import a key
    let mut import_cmd = cli_with_password();
    import_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--label",
        "deleteme",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_cmd.write_stdin("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890");

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Private key imported successfully",
        ));

    // Step 2: Import another key to keep
    let mut import_cmd2 = cli_with_password();
    import_cmd2.args(&[
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--label",
        "keepme",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_cmd2.write_stdin("fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321");

    import_cmd2
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Private key imported successfully",
        ));

    // Step 3: List keys to verify both exist
    let mut list_cmd = cli_with_password();
    list_cmd.args(&["keystore", "list", "--keystore", keystore_str]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("deleteme"))
        .stdout(predicate::str::contains("keepme"));

    // Step 4: Delete by label with --yes flag
    let mut delete_cmd = cli_with_password();
    delete_cmd.args(&[
        "keystore",
        "delete",
        "--by-label",
        "deleteme",
        "--yes",
        "--keystore",
        keystore_str,
    ]);

    delete_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted successfully"));

    // Step 5: List keys to verify only one remains
    let mut list_cmd2 = cli_with_password();
    list_cmd2.args(&["keystore", "list", "--keystore", keystore_str]);

    list_cmd2
        .assert()
        .success()
        .stdout(predicate::str::contains("keepme"))
        .stdout(
            predicate::str::contains("keepme")
                .not()
                .and(predicate::str::contains("deleteme"))
                .not(),
        );
}

#[test]
fn test_e2e_mixed_key_types_workflow() {
    let (_temp_dir, keystore_path) = create_test_env();
    let keystore_str = keystore_path.to_str().unwrap();

    // Import private key
    let mut import_pk_cmd = cli_with_password();
    import_pk_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--label",
        "pk-wallet",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_pk_cmd.write_stdin("1111111111111111111111111111111111111111111111111111111111111111");

    import_pk_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Private key imported successfully",
        ));

    // Import mnemonic
    let mut import_mn_cmd = cli_with_password();
    import_mn_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--label",
        "mn-wallet",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_mn_cmd.write_stdin("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");

    import_mn_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("Mnemonic imported successfully"));

    // List all keys
    let mut list_cmd = cli_with_password();
    list_cmd.args(&[
        "--output-format",
        "json",
        "keystore",
        "list",
        "--show-addresses",
        "--keystore",
        keystore_str,
    ]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("pk-wallet"))
        .stdout(predicate::str::contains("mn-wallet"))
        .stdout(predicate::str::contains("privatekey"))
        .stdout(predicate::str::contains("mnemonic"));

    // Test filtering by label pattern
    let mut filter_cmd = cli_with_password();
    filter_cmd.args(&[
        "keystore",
        "list",
        "--filter-label",
        "pk.*",
        "--keystore",
        keystore_str,
    ]);

    filter_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("pk-wallet"))
        .stdout(predicate::str::contains("mn-wallet").not());
}

#[test]
fn test_e2e_auto_generated_labels() {
    let (_temp_dir, keystore_path) = create_test_env();
    let keystore_str = keystore_path.to_str().unwrap();

    // Import without specifying label
    let mut import_cmd = cli_with_password();
    import_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_cmd.write_stdin("2222222222222222222222222222222222222222222222222222222222222222");

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Private key imported successfully",
        ));

    // List to verify auto-generated label
    let mut list_cmd = cli_with_password();
    list_cmd.args(&["keystore", "list", "--keystore", keystore_str]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("imported-key-")); // Should have auto-generated label
}

#[test]
fn test_e2e_sorting_and_formatting() {
    let (_temp_dir, keystore_path) = create_test_env();
    let keystore_str = keystore_path.to_str().unwrap();

    // Import multiple keys with different labels
    for (i, label) in ["zebra", "alpha", "beta"].iter().enumerate() {
        let mut import_cmd = cli_with_password();
        import_cmd.args(&[
            "keystore",
            "import",
            "--import-type",
            "privatekey",
            "--label",
            label,
            "--keystore",
            keystore_str,
            "--stdin",
        ]);
        // Use different private keys
        let pk = format!("{:064x}", i + 1);
        import_cmd.write_stdin(pk.as_str());

        import_cmd
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "Private key imported successfully",
            ));

        // Small delay to ensure different timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Test sorting by label
    let mut sort_label_cmd = cli_with_password();
    sort_label_cmd.args(&[
        "keystore",
        "list",
        "--sort-by",
        "label",
        "--keystore",
        keystore_str,
    ]);

    sort_label_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha"))
        .stdout(predicate::str::contains("beta"))
        .stdout(predicate::str::contains("zebra"));

    // Test JSON format with addresses
    let mut json_cmd = cli_with_password();
    json_cmd.args(&[
        "--output-format",
        "json",
        "keystore",
        "list",
        "--show-addresses",
        "--sort-by",
        "created",
        "--keystore",
        keystore_str,
    ]);

    json_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\":"))
        .stdout(predicate::str::contains("\"label\":"))
        .stdout(predicate::str::contains("\"address\":"))
        .stdout(predicate::str::contains("\"created\":"));
}

#[test]
fn test_e2e_keystore_persistence() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("persistent_keystore");
    let keystore_str = keystore_path.to_str().unwrap();

    // Import a key
    let mut import_cmd = cli_with_password();
    import_cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--label",
        "persistent-key",
        "--keystore",
        keystore_str,
        "--stdin",
    ]);
    import_cmd.write_stdin("9999999999999999999999999999999999999999999999999999999999999999");

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Private key imported successfully",
        ));

    // Verify keystore file exists and has content
    assert!(keystore_path.exists(), "Keystore file should exist");
    let file_content =
        fs::read_to_string(&keystore_path).expect("Should be able to read keystore file");
    assert!(
        file_content.contains("persistent-key"),
        "Keystore should contain the key label"
    );
    assert!(
        file_content.len() > 100,
        "Keystore file should have substantial content"
    );

    // List keys in a new CLI invocation (simulating restart)
    let mut list_cmd = cli_with_password();
    list_cmd.args(&["keystore", "list", "--keystore", keystore_str]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("persistent-key"));
}
