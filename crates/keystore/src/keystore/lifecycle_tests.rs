use super::*;

#[test]
fn test_keystore_persistence() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("persistent.keystore");

    let key_id = {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();

        let test_key = "0000000000000000000000000000000000000000000000000000000000000002";
        keystore
            .import_private_key(Some("persistent_key".to_string()), test_key)
            .unwrap()
    };

    // Load keystore again
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore2.unlock("test_password").unwrap();

    // Verify key persisted
    let keys = keystore2.list_keys().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].id, key_id);
    assert_eq!(keys[0].alias, Some("persistent_key".to_string()));

    // Verify we can still retrieve the key
    let secure_key = keystore2.private_key_for_test(key_id).unwrap();
    let test_hash = [1u8; 32];
    let _signature = secure_key.sign_hash(&test_hash).unwrap();
}

#[test]
fn test_wrong_password() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("password_test.keystore");

    // Create keystore with password
    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("correct_password").unwrap();
    }

    // Try to unlock with wrong password
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let result = keystore2.unlock("wrong_password");

    assert!(result.is_err());
    match result.unwrap_err() {
        KeystoreError::InvalidInput(msg) => {
            assert!(msg.contains("integrity verification failed"));
            assert!(!msg.contains("correct_password"));
            assert!(!msg.contains("wrong_password"));
        }
        other => panic!("expected safe authentication failure, got: {other:?}"),
    }
}

#[test]
fn test_locked_operations() {
    let (_temp_dir, mut keystore) = test_keystore();

    // Try operations on locked keystore
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000001"
        ),
        Err(KeystoreError::Locked)
    ));

    assert!(matches!(
        keystore.private_key_for_test(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));

    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));

    assert!(matches!(keystore.list_keys(), Err(KeystoreError::Locked)));
}

#[test]
fn test_key_deletion() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_key = "0000000000000000000000000000000000000000000000000000000000000003";
    let key_id = keystore
        .import_private_key(Some("deletable_key".to_string()), test_key)
        .unwrap();

    // Verify key exists
    assert_eq!(keystore.list_keys().unwrap().len(), 1);

    // Delete key
    keystore.delete_key(key_id).unwrap();

    // Verify key is gone
    assert_eq!(keystore.list_keys().unwrap().len(), 0);
}

// ===== COMPREHENSIVE API TESTS =====

#[test]
fn test_keystore_new_variants() {
    let temp_dir = tempdir().unwrap();

    // Test new() with default config
    let keystore_path1 = temp_dir.path().join("test1.keystore");
    let keystore1 = Keystore::new(&keystore_path1).unwrap();
    assert_eq!(keystore1.config.argon2_memory_kb, 1_048_576); // 1GB default

    // Test new_with_config() with development config
    let keystore_path2 = temp_dir.path().join("test2.keystore");
    let keystore2 =
        Keystore::new_with_config(&keystore_path2, KeystoreConfig::development()).unwrap();
    assert_eq!(keystore2.config.argon2_memory_kb, 8192); // 8MB development

    // Test with production config
    let keystore_path3 = temp_dir.path().join("test3.keystore");
    let keystore3 =
        Keystore::new_with_config(&keystore_path3, KeystoreConfig::production()).unwrap();
    assert_eq!(keystore3.config.argon2_memory_kb, 1_048_576); // 1GB production
}

#[test]
fn test_unlock_edge_cases() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("unlock_test.keystore");

    // Test initial unlock (creates new keystore)
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("initial_password").unwrap();

    // Verify keystore file was created
    assert!(keystore_path.exists());

    // Test unlock on a freshly reopened existing keystore.
    drop(keystore);
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("initial_password").unwrap();

    // Test multiple unlock calls (should be idempotent)
    keystore.unlock("initial_password").unwrap();
    keystore.unlock("initial_password").unwrap();

    // Test an empty password against a locked, reopened keystore.
    drop(keystore);
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let result = keystore.unlock("");
    assert!(result.is_err());
}

#[test]
fn test_reopened_keystore_requires_unlock() {
    let (temp_dir, mut keystore) = test_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");
    keystore.unlock("test_password").unwrap();

    // Import a key while unlocked
    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let _key_id = keystore
        .import_private_key(Some("test".to_string()), test_key)
        .unwrap();

    drop(keystore);
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

    // Metadata access is locked behind an unlocked session.
    assert!(matches!(keystore.list_keys(), Err(KeystoreError::Locked)));

    // Verify operations requiring master key fail
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000002"
        ),
        Err(KeystoreError::Locked)
    ));
}

#[test]
fn test_import_private_key_accepts_valid_formats() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Test various valid private key formats
    let valid_keys = [
        "0000000000000000000000000000000000000000000000000000000000000001",
        "0x0000000000000000000000000000000000000000000000000000000000000002",
        "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364140", // Large but valid secp256k1 key (curve order - 1)
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    ];

    for (i, key) in valid_keys.iter().enumerate() {
        assert!(
            keystore
                .import_private_key(Some(format!("key_{i}")), key)
                .is_ok(),
            "valid private key import rejected at case {i}"
        );
    }
}

#[test]
fn test_import_mnemonic_edge_cases() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Test valid mnemonic with different derivation paths
    let valid_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    let valid_paths = [
        "m/44'/60'/0'/0/0", // Standard Ethereum path
        "m/44'/60'/0'/0/1", // Different account
        "m/44'/60'/1'/0/0", // Different account index
        "m/0'/0/0",         // Simplified path
    ];

    for (i, path) in valid_paths.iter().enumerate() {
        let result =
            keystore.import_mnemonic(Some(format!("mnemonic_{i}")), valid_mnemonic, path, None);
        assert!(result.is_ok(), "Failed to import with valid path: {path}");
    }

    // Test invalid mnemonics
    let invalid_mnemonics = [
        "invalid mnemonic phrase",
        "",
        "abandon", // Too short
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon invalid", // Invalid word
    ];

    for (i, mnemonic) in invalid_mnemonics.iter().enumerate() {
        let result = keystore.import_mnemonic(None, mnemonic, "m/44'/60'/0'/0/0", None);
        assert!(result.is_err(), "invalid mnemonic accepted at case {i}");
    }
}

#[test]
fn test_test_only_private_key_decryption_covers_imported_key_types() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Import test keys
    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id = keystore
        .import_private_key(Some("test_key".to_string()), test_key)
        .unwrap();

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let mnemonic_id = keystore
        .import_mnemonic(
            Some("test_mnemonic".to_string()),
            test_mnemonic,
            "m/44'/60'/0'/0/0",
            None,
        )
        .unwrap();

    // Test retrieval of both key types
    let private_key = keystore.private_key_for_test(key_id).unwrap();
    let mnemonic_key = keystore.private_key_for_test(mnemonic_id).unwrap();

    // Verify they produce different addresses
    assert_ne!(
        private_key.ethereum_address().unwrap(),
        mnemonic_key.ethereum_address().unwrap()
    );
}

#[test]
fn decrypt_ownership_transfer_is_witnessed_on_success_and_cleanup() {
    use super::super::secure_key::KeyMaterialWitness;

    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id = keystore
        .import_private_key(Some("ownership".to_string()), test_key)
        .unwrap();

    let witness = KeyMaterialWitness::new();
    let secure = keystore
        .private_key_for_test_with_ownership_witness(key_id, witness.clone())
        .expect("decrypt");
    assert!(witness.observed_transfer());
    assert!(!witness.observed_cleanup());
    let address = secure.ethereum_address().expect("address");
    assert_eq!(
        format!("{address:?}"),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
    secure
        .sign_hash_recoverable(&[0_u8; 32])
        .expect("sign with the transferred production allocation");
    assert!(witness.observed_same_allocation());
    drop(secure);
    assert!(witness.observed_cleanup());
}

#[test]
fn decrypt_authentication_failure_zeroizes_the_production_allocation() {
    use super::super::secure_key::KeyMaterialWitness;

    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("tampered".to_owned()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
    keystore
        .entries
        .iter_mut()
        .find(|entry| entry.id == key_id)
        .expect("tampered entry")
        .encrypted_data[0] ^= 0x01;

    let witness = KeyMaterialWitness::new();
    assert!(matches!(
        keystore.private_key_for_test_with_ownership_witness(key_id, witness.clone()),
        Err(KeystoreError::InvalidPrivateKey)
    ));
    assert!(witness.observed_cleanup());
    assert!(!witness.observed_transfer());
}

#[test]
fn decrypt_wrong_length_rejects_before_allocating_plaintext() {
    use super::super::secure_key::KeyMaterialWitness;

    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("truncated".to_owned()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
    keystore
        .entries
        .iter_mut()
        .find(|entry| entry.id == key_id)
        .expect("truncated entry")
        .encrypted_data
        .truncate(31);

    let witness = KeyMaterialWitness::new();
    assert!(matches!(
        keystore.private_key_for_test_with_ownership_witness(key_id, witness.clone()),
        Err(KeystoreError::InvalidPrivateKey)
    ));
    assert!(!witness.observed_cleanup());
    assert!(!witness.observed_transfer());
}

#[test]
fn decrypt_unwind_zeroizes_the_production_allocation() {
    use super::super::secure_key::KeyMaterialWitness;

    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("unwind".to_owned()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
    let entry = keystore
        .entries
        .iter()
        .find(|entry| entry.id == key_id)
        .expect("unwind entry");
    let master_key = keystore.master_key.as_ref().expect("unlocked master key");
    let witness = KeyMaterialWitness::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = keystore.decrypt_data_with_witness_and_unwind(
            master_key,
            &entry.nonce,
            &entry.encrypted_data,
            entry.id.as_bytes(),
            &witness,
        );
    }));
    assert!(result.is_err());
    assert!(witness.observed_cleanup());
    assert!(!witness.observed_transfer());
}

#[test]
fn decrypt_missing_entry_fails_closed_without_secret_output() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    let missing = Uuid::new_v4();
    match keystore.private_key_for_test(missing) {
        Err(err @ KeystoreError::KeyNotFound(_)) => {
            let rendered = format!("{err:?}{err}");
            assert!(!rendered
                .contains("0000000000000000000000000000000000000000000000000000000000000001"));
        }
        Ok(_) => panic!("missing key must fail closed"),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn decrypt_locked_keystore_fails_closed() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("locked_decrypt.keystore");
    let key_id = {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();
        let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
        keystore.import_private_key(None, test_key).unwrap()
    };
    let locked = Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    match locked.private_key_for_test(key_id) {
        Err(KeystoreError::Locked) => {}
        Ok(_) => panic!("locked keystore must fail closed"),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn test_list_keys_comprehensive() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Initially empty
    assert_eq!(keystore.list_keys().unwrap().len(), 0);

    // Add some keys
    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id1 = keystore
        .import_private_key(Some("key1".to_string()), test_key)
        .unwrap();

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let key_id2 = keystore
        .import_mnemonic(
            Some("mnemonic1".to_string()),
            test_mnemonic,
            "m/44'/60'/0'/0/0",
            None,
        )
        .unwrap();

    // Test listing
    let keys = keystore.list_keys().unwrap();
    assert_eq!(keys.len(), 2);

    // Verify key info
    let key1_info = keys.iter().find(|k| k.id == key_id1).unwrap();
    assert_eq!(key1_info.alias, Some("key1".to_string()));
    assert!(matches!(key1_info.key_type, KeyType::PrivateKey));

    let key2_info = keys.iter().find(|k| k.id == key_id2).unwrap();
    assert_eq!(key2_info.alias, Some("mnemonic1".to_string()));
    assert!(matches!(key2_info.key_type, KeyType::HdDerived { .. }));
}

#[test]
fn test_secure_key_methods() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id = keystore
        .import_private_key(Some("test_key".to_string()), test_key)
        .unwrap();

    let secure_key = keystore.private_key_for_test(key_id).unwrap();

    // Test sign_hash
    let test_hash1 = [1u8; 32];
    let test_hash2 = [2u8; 32];

    let sig1 = secure_key.sign_hash(&test_hash1).unwrap();
    let sig2 = secure_key.sign_hash(&test_hash2).unwrap();

    // Signatures should be different for different hashes
    assert_ne!(sig1.to_bytes(), sig2.to_bytes());

    // Signatures should be deterministic for same hash
    let sig1_again = secure_key.sign_hash(&test_hash1).unwrap();
    assert_eq!(sig1.to_bytes(), sig1_again.to_bytes());

    let recoverable = secure_key.sign_hash_recoverable(&test_hash1).unwrap();
    let recoverable_again = secure_key.sign_hash_recoverable(&test_hash1).unwrap();
    assert_eq!(recoverable, recoverable_again);

    // Test ethereum_address
    let address1 = secure_key.ethereum_address().unwrap();
    let address2 = secure_key.ethereum_address().unwrap();
    assert_eq!(address1, address2); // Should be consistent
    assert_ne!(address1, Address::ZERO); // Should not be zero

    // Test public_key
    let pubkey1 = secure_key.public_key().unwrap();
    let pubkey2 = secure_key.public_key().unwrap();
    assert_eq!(
        pubkey1.to_encoded_point(false),
        pubkey2.to_encoded_point(false)
    ); // Should be consistent
}

#[test]
fn test_error_conditions() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("error_test.keystore");

    // Test various error conditions systematically
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

    // Unlock and test other errors
    keystore.unlock("test_password").unwrap();

    // Key not found
    assert!(matches!(
        keystore.private_key_for_test(Uuid::new_v4()),
        Err(KeystoreError::KeyNotFound(_))
    ));
    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::KeyNotFound(_))
    ));

    // Invalid private key
    assert!(matches!(
        keystore.import_private_key(None, "invalid"),
        Err(KeystoreError::InvalidPrivateKey)
    ));
    let invalid_private_key_hex = "g".repeat(64);
    assert!(matches!(
        keystore.import_private_key(None, &invalid_private_key_hex),
        Err(KeystoreError::InvalidPrivateKey)
    ));
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        Err(KeystoreError::InvalidPrivateKey)
    )); // Zero key
    assert!(keystore.list_keys().unwrap().is_empty());

    // Invalid mnemonic
    assert!(matches!(
        keystore.import_mnemonic(None, "invalid mnemonic", "m/44'/60'/0'/0/0", None),
        Err(KeystoreError::InvalidMnemonic(_))
    ));

    // Invalid derivation path
    let valid_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    assert!(matches!(
        keystore.import_mnemonic(None, valid_mnemonic, "invalid/path", None),
        Err(KeystoreError::InvalidDerivationPath(_))
    ));
}
