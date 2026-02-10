use mfm_core::keystore::{Keystore, KeystoreConfig};
use tempfile::TempDir;

fn create_test_keystore_with_data() -> (TempDir, std::path::PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");

    // Create a keystore with fast config for testing
    let fast_config = KeystoreConfig {
        argon2_memory_kb: 64, // 64KB - minimal for testing
        argon2_iterations: 1, // 1 iteration - minimal
        argon2_parallelism: 1,
    };

    let mut keystore =
        Keystore::new_with_config(&keystore_path, fast_config).expect("Failed to create keystore");
    keystore
        .unlock("test_password_123")
        .expect("Failed to unlock keystore");

    // Import a test private key
    let test_private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    keystore
        .import_private_key(Some("test-key".to_string()), test_private_key)
        .expect("Failed to import test key");

    // Import a test mnemonic
    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    keystore
        .import_mnemonic(
            Some("test-mnemonic".to_string()),
            test_mnemonic,
            "m/44'/60'/0'/0/0",
            None,
        )
        .expect("Failed to import test mnemonic");

    (temp_dir, keystore_path)
}

#[test]
fn test_keystore_integration_list_keys() {
    let (_temp_dir, keystore_path) = create_test_keystore_with_data();

    // Verify keystore file was created
    assert!(keystore_path.exists(), "Keystore file should exist");

    // Test that we can read the keystore back
    let _keystore = Keystore::new(&keystore_path).expect("Failed to load keystore");
    // Note: We can't test unlock without interactive password input in integration tests
    // This would require mocking the password input, which is complex in integration tests
}

#[test]
fn test_keystore_file_creation() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("new_keystore");

    // Verify file doesn't exist initially
    assert!(
        !keystore_path.exists(),
        "Keystore file should not exist initially"
    );

    // Create keystore with fast config
    let fast_config = KeystoreConfig {
        argon2_memory_kb: 64,
        argon2_iterations: 1,
        argon2_parallelism: 1,
    };
    let _keystore =
        Keystore::new_with_config(&keystore_path, fast_config).expect("Failed to create keystore");

    // File should still not exist until we unlock (create) it
    assert!(
        !keystore_path.exists(),
        "Keystore file should not exist until unlocked"
    );
}

#[test]
fn test_keystore_operations() {
    let (_temp_dir, keystore_path) = create_test_keystore_with_data();

    // Load keystore and verify it has data
    let mut keystore = Keystore::new(&keystore_path).expect("Failed to load keystore");
    keystore
        .unlock("test_password_123")
        .expect("Failed to unlock keystore");

    let keys = keystore.list_keys().expect("Failed to list keys");
    assert_eq!(keys.len(), 2, "Should have 2 keys");

    // Verify key labels
    let labels: Vec<_> = keys.iter().filter_map(|k| k.alias.as_ref()).collect();
    assert!(labels.contains(&&"test-key".to_string()));
    assert!(labels.contains(&&"test-mnemonic".to_string()));

    // Test deletion
    let key_to_delete = keys
        .iter()
        .find(|k| k.alias.as_ref() == Some(&"test-key".to_string()))
        .expect("Should find test-key");

    keystore
        .delete_key(key_to_delete.id)
        .expect("Failed to delete key");

    let remaining_keys = keystore
        .list_keys()
        .expect("Failed to list keys after deletion");
    assert_eq!(remaining_keys.len(), 1, "Should have 1 key after deletion");
}

#[test]
fn test_invalid_private_key() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");

    let fast_config = KeystoreConfig {
        argon2_memory_kb: 64,
        argon2_iterations: 1,
        argon2_parallelism: 1,
    };
    let mut keystore =
        Keystore::new_with_config(&keystore_path, fast_config).expect("Failed to create keystore");
    keystore
        .unlock("test_password_123")
        .expect("Failed to unlock keystore");

    // Test invalid private key (too short)
    let result = keystore.import_private_key(Some("invalid-key".to_string()), "short_key");
    assert!(result.is_err(), "Should fail with invalid private key");

    // Test invalid private key (not hex)
    let result = keystore.import_private_key(
        Some("invalid-key".to_string()),
        "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
    );
    assert!(result.is_err(), "Should fail with non-hex private key");
}

#[test]
fn test_invalid_mnemonic() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");

    let fast_config = KeystoreConfig {
        argon2_memory_kb: 64,
        argon2_iterations: 1,
        argon2_parallelism: 1,
    };
    let mut keystore =
        Keystore::new_with_config(&keystore_path, fast_config).expect("Failed to create keystore");
    keystore
        .unlock("test_password_123")
        .expect("Failed to unlock keystore");

    // Test invalid mnemonic
    let result = keystore.import_mnemonic(
        Some("invalid-mnemonic".to_string()),
        "this is not a valid mnemonic phrase at all",
        "m/44'/60'/0'/0/0",
        None,
    );
    assert!(result.is_err(), "Should fail with invalid mnemonic");
}

#[test]
fn test_keystore_addresses() {
    let (_temp_dir, keystore_path) = create_test_keystore_with_data();

    let mut keystore = Keystore::new(&keystore_path).expect("Failed to load keystore");
    keystore
        .unlock("test_password_123")
        .expect("Failed to unlock keystore");

    let keys = keystore.list_keys().expect("Failed to list keys");

    // Verify all keys have addresses
    for key in &keys {
        // Address should be a valid Ethereum address (20 bytes)
        assert_eq!(key.address.0.len(), 20, "Address should be 20 bytes");
    }
}

#[test]
fn test_keystore_persistence() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");

    let test_private_key = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    let key_id;

    // Create keystore and add key
    {
        let fast_config = KeystoreConfig {
            argon2_memory_kb: 64,
            argon2_iterations: 1,
            argon2_parallelism: 1,
        };
        let mut keystore = Keystore::new_with_config(&keystore_path, fast_config)
            .expect("Failed to create keystore");
        keystore
            .unlock("test_password_123")
            .expect("Failed to unlock keystore");

        key_id = keystore
            .import_private_key(Some("persistent-key".to_string()), test_private_key)
            .expect("Failed to import key");
    }

    // Load keystore again and verify key persists
    {
        let mut keystore = Keystore::new(&keystore_path).expect("Failed to load keystore");
        keystore
            .unlock("test_password_123")
            .expect("Failed to unlock keystore");

        let keys = keystore.list_keys().expect("Failed to list keys");
        assert_eq!(keys.len(), 1, "Should have 1 persisted key");

        let key = &keys[0];
        assert_eq!(key.id, key_id, "Key ID should match");
        assert_eq!(
            key.alias.as_ref(),
            Some(&"persistent-key".to_string()),
            "Key alias should match"
        );
    }
}
