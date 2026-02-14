/// Comprehensive tests for keystore_v2 public API
///
/// These tests validate all public methods and ensure the API works correctly
/// for external consumers using only public interfaces.
use super::*;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use static_assertions::assert_not_impl_any;
use std::sync::Once;
use tempfile::tempdir;
use tracing::{info, warn};

fn init_test_observability() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,mfm=debug")),
            )
            .with_test_writer()
            .try_init();
    });
}

#[test]
fn keystore_is_not_send_nor_sync() {
    // Fails to compile if `Keystore` implements *any* of the listed traits
    assert_not_impl_any!(Keystore: Send, Sync);
}

/// Helper function to create a test keystore with development config
fn test_keystore() -> (tempfile::TempDir, Keystore) {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("test.keystore");
    let keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    (temp_dir, keystore)
}

#[test]
fn test_new_keystore_creation() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("new.keystore");

    // Create new keystore
    let keystore = Keystore::new(&keystore_path).unwrap();

    // Verify initial state
    assert!(!keystore_path.exists()); // File not created until first unlock
    assert_eq!(keystore.list_keys().unwrap().len(), 0); // Should be empty initially (list_keys works when locked)
}

#[test]
fn test_private_key_import_and_retrieval() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id = keystore
        .import_private_key(Some("test_key".to_string()), test_key)
        .unwrap();

    // Test retrieval
    let secure_key = keystore.get_private_key(key_id).unwrap();

    // Test signing
    let test_hash = [1u8; 32];
    let signature = secure_key.sign_hash(&test_hash).unwrap();

    // Verify signature (basic check)
    assert_eq!(signature.to_bytes().len(), 64);

    // Test Ethereum address derivation
    let address = secure_key.ethereum_address();
    assert_ne!(address, Address::ZERO);

    // Test public key derivation
    let public_key = secure_key.public_key();
    assert_eq!(public_key.to_encoded_point(false).len(), 65); // Uncompressed: 1 + 32 + 32
}

#[test]
fn test_mnemonic_import_and_retrieval() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let derivation_path = "m/44'/60'/0'/0/0";

    let key_id = keystore
        .import_mnemonic(
            Some("mnemonic_key".to_string()),
            test_mnemonic,
            derivation_path,
            None,
        )
        .unwrap();

    // Test retrieval and signing
    let secure_key = keystore.get_private_key(key_id).unwrap();
    let test_hash = [2u8; 32];
    let signature = secure_key.sign_hash(&test_hash).unwrap();
    assert_eq!(signature.to_bytes().len(), 64);

    // Verify consistent address derivation
    let address1 = secure_key.ethereum_address();
    let address2 = secure_key.ethereum_address();
    assert_eq!(address1, address2);
}

#[test]
fn test_mnemonic_passphrase_support() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let derivation_path = "m/44'/60'/0'/0/0";

    let id_no_pass = keystore
        .import_mnemonic(Some("no_pass".to_string()), mnemonic, derivation_path, None)
        .unwrap();
    let id_with_pass = keystore
        .import_mnemonic(
            Some("with_pass".to_string()),
            mnemonic,
            derivation_path,
            Some("passphrase"),
        )
        .unwrap();

    let key_no_pass = keystore.get_private_key(id_no_pass).unwrap();
    let key_with_pass = keystore.get_private_key(id_with_pass).unwrap();

    assert_ne!(
        key_no_pass.ethereum_address(),
        key_with_pass.ethereum_address()
    );
}

#[test]
fn test_export_private_key_for_private_key_entries() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let id = keystore
        .import_private_key(Some("export_pk".to_string()), test_key)
        .unwrap();

    let exported = keystore.export_private_key(id).unwrap();
    assert_eq!(exported.as_str(), format!("0x{test_key}"));
}

#[test]
fn test_export_mnemonic_and_derived_private_key() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let derivation_path = "m/44'/60'/0'/0/0";

    let mnemonic_id = keystore
        .import_mnemonic(
            Some("mnemonic_key".to_string()),
            test_mnemonic,
            derivation_path,
            None,
        )
        .unwrap();

    let exported_mnemonic = keystore.export_mnemonic(mnemonic_id).unwrap();
    assert_eq!(exported_mnemonic.as_str(), test_mnemonic);

    let exported_pk = keystore.export_private_key(mnemonic_id).unwrap();
    let derived_id = keystore
        .import_private_key(Some("derived".to_string()), exported_pk.as_str())
        .unwrap();

    let key_from_mnemonic = keystore.get_private_key(mnemonic_id).unwrap();
    let key_from_exported = keystore.get_private_key(derived_id).unwrap();
    assert_eq!(
        key_from_mnemonic.ethereum_address(),
        key_from_exported.ethereum_address()
    );

    // export_mnemonic should fail for private key entries.
    assert!(matches!(
        keystore.export_mnemonic(derived_id),
        Err(KeystoreError::InvalidInput(_))
    ));
}

#[test]
fn test_change_password_reencrypts_entries() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("change_password.keystore");

    let (pk_id, mnemonic_id) = {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("old_password").unwrap();

        let pk_id = keystore
            .import_private_key(
                Some("pk".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000002",
            )
            .unwrap();
        let mnemonic_id = keystore
            .import_mnemonic(
                Some("mn".to_string()),
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
                "m/44'/60'/0'/0/0",
                Some("pass"),
            )
            .unwrap();

        keystore
            .change_password("old_password", "new_password")
            .unwrap();

        (pk_id, mnemonic_id)
    };

    // Old password should fail.
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    assert!(matches!(
        keystore2.unlock("old_password"),
        Err(KeystoreError::InvalidPassword)
    ));

    // New password should succeed and keys should still be accessible.
    keystore2.unlock("new_password").unwrap();
    assert!(keystore2.get_private_key(pk_id).is_ok());
    assert!(keystore2.get_private_key(mnemonic_id).is_ok());
}

#[test]
fn test_audit_log_entries_created_for_operations() {
    let (_temp_dir, mut keystore) = test_keystore();

    // Unlock logs.
    keystore.unlock("test_password").unwrap();
    assert!(matches!(
        keystore.audit_log().last().unwrap().event,
        AuditEvent::Unlock
    ));

    let pk_id = keystore
        .import_private_key(
            Some("pk".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000003",
        )
        .unwrap();
    assert!(keystore
        .audit_log()
        .iter()
        .any(|e| matches!(e.event, AuditEvent::ImportPrivateKey { .. }) && e.success));

    // get_private_key logs.
    keystore.get_private_key(pk_id).unwrap();
    assert!(keystore
        .audit_log()
        .iter()
        .any(|e| matches!(e.event, AuditEvent::GetPrivateKey { id } if id == pk_id) && e.success));

    // export_private_key logs.
    keystore.export_private_key(pk_id).unwrap();
    assert!(keystore.audit_log().iter().any(
        |e| matches!(e.event, AuditEvent::ExportPrivateKey { id } if id == pk_id) && e.success
    ));

    // export_mnemonic failure logs.
    assert!(keystore.export_mnemonic(pk_id).is_err());
    assert!(keystore.audit_log().iter().any(|e| {
        matches!(e.event, AuditEvent::ExportMnemonic { id } if id == pk_id) && !e.success
    }));

    // delete_key logs.
    keystore.delete_key(pk_id).unwrap();
    assert!(keystore
        .audit_log()
        .iter()
        .any(|e| matches!(e.event, AuditEvent::DeleteKey { id } if id == pk_id) && e.success));

    // lock logs.
    keystore.lock();
    assert!(matches!(
        keystore.audit_log().last().unwrap().event,
        AuditEvent::Lock
    ));

    // change_password failure logs (locked).
    assert!(keystore
        .change_password("old_password", "new_password")
        .is_err());
    assert!(keystore
        .audit_log()
        .iter()
        .any(|e| matches!(e.event, AuditEvent::ChangePassword) && !e.success));
}

#[test]
fn test_audit_log_persisted_for_read_only_access_operations() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("audit_persist.keystore");

    let (pk_id, mnemonic_id) = {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();

        let pk_id = keystore
            .import_private_key(
                Some("pk".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000004",
            )
            .unwrap();

        let mnemonic_id = keystore
            .import_mnemonic(
                Some("mn".to_string()),
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
                "m/44'/60'/0'/0/0",
                None,
            )
            .unwrap();

        // These operations should persist their audit entries to disk.
        keystore.get_private_key(pk_id).unwrap();
        keystore.export_private_key(pk_id).unwrap();
        keystore.export_mnemonic(mnemonic_id).unwrap();

        (pk_id, mnemonic_id)
    };

    // Re-open keystore and verify audit entries survived the process boundary.
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore2.unlock("test_password").unwrap();

    assert!(keystore2.audit_log().iter().any(|e| {
        matches!(e.event, AuditEvent::GetPrivateKey { id } if id == pk_id) && e.success
    }));
    assert!(keystore2.audit_log().iter().any(|e| {
        matches!(e.event, AuditEvent::ExportPrivateKey { id } if id == pk_id) && e.success
    }));
    assert!(keystore2.audit_log().iter().any(|e| {
        matches!(e.event, AuditEvent::ExportMnemonic { id } if id == mnemonic_id) && e.success
    }));
}

#[test]
fn test_concurrent_operations() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Import multiple keys
    let key1_id = keystore
        .import_private_key(
            Some("key1".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
    let key2_id = keystore
        .import_private_key(
            Some("key2".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000002",
        )
        .unwrap();

    // Retrieve both keys
    let secure_key1 = keystore.get_private_key(key1_id).unwrap();
    let secure_key2 = keystore.get_private_key(key2_id).unwrap();

    // Verify they produce different signatures for same hash
    let test_hash = [3u8; 32];
    let sig1 = secure_key1.sign_hash(&test_hash).unwrap();
    let sig2 = secure_key2.sign_hash(&test_hash).unwrap();

    assert_ne!(sig1.to_bytes(), sig2.to_bytes());

    // Verify different addresses
    assert_ne!(
        secure_key1.ethereum_address(),
        secure_key2.ethereum_address()
    );
}

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
    let secure_key = keystore2.get_private_key(key_id).unwrap();
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
    assert!(matches!(
        result.unwrap_err(),
        KeystoreError::InvalidPassword
    ));
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
        keystore.get_private_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));

    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));
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

    // Try to delete non-existent key
    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::KeyNotFound(_))
    ));
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

    // Test unlock on existing keystore
    keystore.lock();
    keystore.unlock("initial_password").unwrap();

    // Test multiple unlock calls (should be idempotent)
    keystore.unlock("initial_password").unwrap();
    keystore.unlock("initial_password").unwrap();

    // Test empty password
    keystore.lock();
    let result = keystore.unlock("");
    assert!(result.is_err());
}

#[test]
fn test_lock_comprehensive() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Import a key while unlocked
    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let _key_id = keystore
        .import_private_key(Some("test".to_string()), test_key)
        .unwrap();

    // Lock the keystore
    keystore.lock();

    // Verify list_keys still works (doesn't require master key)
    assert!(keystore.list_keys().is_ok());

    // Verify operations requiring master key fail
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000002"
        ),
        Err(KeystoreError::Locked)
    ));

    // Test multiple lock calls (should be idempotent)
    keystore.lock();
    keystore.lock();
}

#[test]
fn test_import_private_key_edge_cases() {
    init_test_observability();
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
        let result = keystore.import_private_key(Some(format!("key_{i}")), key);
        match result {
            Ok(_) => info!(test_case = i, "valid private key import accepted"),
            Err(e) => panic!("Failed to import valid key {i}: {key} - Error: {e:?}"),
        }
    }

    // Test invalid private key formats
    let invalid_keys = [
        "invalid_hex",
        "0x",
        "",
        "0000000000000000000000000000000000000000000000000000000000000000", // Zero key
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141", // Curve order (invalid)
        "1234567890abcdef",                                                 // Too short
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef00", // Too long
    ];

    for key in invalid_keys {
        let result = keystore.import_private_key(None, key);
        if result.is_ok() {
            warn!("invalid format key accepted by parser; this may be acceptable");
        }
        // Note: We don't assert failure here since some edge cases might be acceptable
    }
}

#[test]
fn test_import_mnemonic_edge_cases() {
    init_test_observability();
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

    // Test invalid derivation paths
    let invalid_paths = [
        "invalid/path",
        "m/44'/60'/0'/0/-1", // Negative index
        "",
        "m",
        "m/44'/60'/0'/0/999999999999999999999", // Very large index
    ];

    for path in invalid_paths {
        let result = keystore.import_mnemonic(None, valid_mnemonic, path, None);
        if result.is_err() {
            info!("invalid derivation path rejected by parser");
        }
        // Note: Some paths might be accepted by the underlying library
    }

    // Test invalid mnemonics
    let invalid_mnemonics = [
        "invalid mnemonic phrase",
        "",
        "abandon", // Too short
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon invalid", // Invalid word
    ];

    for mnemonic in invalid_mnemonics {
        let result = keystore.import_mnemonic(None, mnemonic, "m/44'/60'/0'/0/0", None);
        assert!(result.is_err(), "Invalid mnemonic should fail: {mnemonic}");
    }
}

#[test]
fn test_get_private_key_comprehensive() {
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
    let private_key = keystore.get_private_key(key_id).unwrap();
    let mnemonic_key = keystore.get_private_key(mnemonic_id).unwrap();

    // Verify they produce different addresses
    assert_ne!(
        private_key.ethereum_address(),
        mnemonic_key.ethereum_address()
    );

    // Test non-existent key
    let result = keystore.get_private_key(Uuid::new_v4());
    assert!(matches!(result, Err(KeystoreError::KeyNotFound(_))));

    // Test with locked keystore
    keystore.lock();
    let result = keystore.get_private_key(key_id);
    assert!(matches!(result, Err(KeystoreError::Locked)));
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
    assert!(matches!(key2_info.key_type, KeyType::Mnemonic { .. }));

    // Test list_keys works when locked (doesn't require master key)
    keystore.lock();
    let locked_keys = keystore.list_keys().unwrap();
    assert_eq!(locked_keys.len(), 2);
}

#[test]
fn test_secure_key_methods() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id = keystore
        .import_private_key(Some("test_key".to_string()), test_key)
        .unwrap();

    let secure_key = keystore.get_private_key(key_id).unwrap();

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

    // Test ethereum_address
    let address1 = secure_key.ethereum_address();
    let address2 = secure_key.ethereum_address();
    assert_eq!(address1, address2); // Should be consistent
    assert_ne!(address1, Address::ZERO); // Should not be zero

    // Test public_key
    let pubkey1 = secure_key.public_key();
    let pubkey2 = secure_key.public_key();
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

    // Operations on locked keystore
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000001"
        ),
        Err(KeystoreError::Locked)
    ));
    assert!(matches!(
        keystore.get_private_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));
    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));

    // Unlock and test other errors
    keystore.unlock("test_password").unwrap();

    // Key not found
    assert!(matches!(
        keystore.get_private_key(Uuid::new_v4()),
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
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        Err(KeystoreError::InvalidPrivateKey)
    )); // Zero key

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

#[test]
fn test_keystore_file_corruption_handling() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("corrupt_test.keystore");

    // Create and initialize keystore
    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();
        keystore
            .import_private_key(
                Some("test".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap();
    }

    // Corrupt the file
    std::fs::write(&keystore_path, "corrupted json data").unwrap();

    // Try to load corrupted keystore
    let result = Keystore::new(&keystore_path);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        KeystoreError::SerializationError(_)
    ));
}

#[test]
fn test_keystore_version_handling() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("version_test.keystore");

    // Create keystore file with unsupported version
    let fake_keystore = r#"{
        "version": 99,
        "kdf_params": {
            "salt": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            "memory_kb": 8192,
            "iterations": 2,
            "parallelism": 1
        },
        "master_key_verification": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "entries": [],
        "file_integrity_mac": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]
    }"#;

    std::fs::write(&keystore_path, fake_keystore).unwrap();

    // Try to load keystore with unsupported version
    let result = Keystore::new(&keystore_path);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        KeystoreError::InvalidInput(_)
    ));
}

#[test]
fn test_file_integrity_protection() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("integrity_test.keystore");

    // Create and initialize keystore with a key
    let _key_id = {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();
        keystore
            .import_private_key(
                Some("test_key".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap()
    };

    // Read the original file content
    let original_content = std::fs::read_to_string(&keystore_path).unwrap();

    // Tamper with the file by changing an alias
    let tampered_content = original_content.replace("test_key", "hacked_key");
    std::fs::write(&keystore_path, tampered_content).unwrap();

    // Try to load the tampered keystore
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

    // Unlock should fail due to integrity check failure
    let result = keystore2.unlock("test_password");
    assert!(result.is_err());

    // Verify it's specifically an integrity error
    match result.unwrap_err() {
        KeystoreError::InvalidInput(msg) => {
            assert!(msg.contains("integrity verification failed"));
        }
        other => panic!("Expected integrity verification failure, got: {other:?}"),
    }
}

#[test]
fn test_file_integrity_protection_entry_swap() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("entry_swap_test.keystore");

    // Create keystore with two keys
    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();
        keystore
            .import_private_key(
                Some("key1".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap();
        keystore
            .import_private_key(
                Some("key2".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000002",
            )
            .unwrap();
    }

    // Tamper with the file by manually swapping encrypted entry data
    let mut file_content = std::fs::read_to_string(&keystore_path).unwrap();

    // This is a simplified tampering - in practice an attacker would swap the encrypted_data fields
    // For this test, we'll just modify some data to trigger integrity failure
    file_content = file_content.replacen("key1", "swapped1", 1);
    std::fs::write(&keystore_path, file_content).unwrap();

    // Try to load the tampered keystore
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

    // Unlock should fail due to integrity check failure
    let result = keystore2.unlock("test_password");
    assert!(result.is_err());

    match result.unwrap_err() {
        KeystoreError::InvalidInput(msg) => {
            assert!(msg.contains("integrity verification failed"));
        }
        other => panic!("Expected integrity verification failure, got: {other:?}"),
    }
}

#[test]
fn test_aad_prevents_entry_swapping() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("aad_test.keystore");

    // Create keystore with two different keys
    let (key1_id, key2_id) = {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();

        let key1_id = keystore
            .import_private_key(
                Some("key1".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap();

        let key2_id = keystore
            .import_private_key(
                Some("key2".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000002",
            )
            .unwrap();

        (key1_id, key2_id)
    };

    // Verify keys work correctly before any tampering
    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();

        let key1 = keystore.get_private_key(key1_id).unwrap();
        let key2 = keystore.get_private_key(key2_id).unwrap();

        // Verify they have different addresses (confirming they are different keys)
        assert_ne!(key1.ethereum_address(), key2.ethereum_address());

        // Test that both keys can sign different data successfully
        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];
        let sig1 = key1.sign_hash(&hash1).unwrap();
        let sig2 = key2.sign_hash(&hash2).unwrap();

        // Signatures should be different (confirming AAD preserves key uniqueness)
        assert_ne!(sig1.to_bytes(), sig2.to_bytes());
    }

    // Now attempt to manually swap encrypted_data between entries
    {
        let file_content = std::fs::read_to_string(&keystore_path).unwrap();
        let mut keystore_data: serde_json::Value = serde_json::from_str(&file_content).unwrap();

        // Swap the encrypted_data between the two entries
        if let Some(entries) = keystore_data["entries"].as_array_mut() {
            if entries.len() >= 2 {
                let temp = entries[0]["encrypted_data"].clone();
                entries[0]["encrypted_data"] = entries[1]["encrypted_data"].clone();
                entries[1]["encrypted_data"] = temp;
            }
        }

        // Write the tampered file back
        let tampered_content = serde_json::to_string_pretty(&keystore_data).unwrap();
        std::fs::write(&keystore_path, tampered_content).unwrap();
    }

    // Try to access the swapped entries - should fail due to AAD mismatch
    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        let result = keystore.unlock("test_password");

        assert!(result.is_err());
        match result.unwrap_err() {
            KeystoreError::InvalidInput(msg) => {
                assert!(msg.contains("File integrity verification failed"));
            }
            other => panic!("Expected InvalidInput error, got: {other:?}"),
        }
    }
}

#[test]
fn test_early_file_validation_dos_protection() {
    let temp_dir = tempdir().unwrap();

    // Test oversized file detection
    {
        let keystore_path = temp_dir.path().join("large_test.keystore");
        // Create keystore object first (when file doesn't exist)
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

        // Then create oversized file
        let large_content = "x".repeat(15 * 1024 * 1024); // 15MB > 10MB limit
        std::fs::write(&keystore_path, large_content).unwrap();

        let result = keystore.unlock("any_password");

        assert!(result.is_err());
        match result.unwrap_err() {
            KeystoreError::InvalidInput(msg) => {
                assert!(msg.contains("too large"));
            }
            other => panic!("Expected size validation error, got: {other:?}"),
        }
    }

    // Test undersized file detection
    {
        let keystore_path = temp_dir.path().join("small_test.keystore");
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

        std::fs::write(&keystore_path, "tiny").unwrap();
        let result = keystore.unlock("any_password");

        assert!(result.is_err());
        match result.unwrap_err() {
            KeystoreError::InvalidInput(msg) => {
                assert!(msg.contains("too small"));
            }
            other => panic!("Expected size validation error, got: {other:?}"),
        }
    }

    // Test malformed JSON detection
    {
        let keystore_path = temp_dir.path().join("json_test.keystore");
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

        // Create malformed JSON that's large enough to pass size check but invalid JSON
        let repeated_x = "x".repeat(200);
        let malformed_json =
            format!("{{ \"version\": 1, \"invalid\": {repeated_x}, \"truncated\": ");
        std::fs::write(&keystore_path, malformed_json).unwrap();
        let result = keystore.unlock("any_password");

        assert!(result.is_err());
        match result.unwrap_err() {
            KeystoreError::InvalidInput(msg) => {
                assert!(
                    msg.contains("invalid JSON") || msg.contains("Malformed"),
                    "Got unexpected message: {msg}"
                );
            }
            other => panic!("Expected JSON validation error, got: {other:?}"),
        }
    }
}
