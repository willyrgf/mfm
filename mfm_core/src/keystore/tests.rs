use crate::keystore::KeystoreConfig;
use std::time::Duration;

use super::error::KeystoreError;
use super::Keystore;
use alloy_primitives::{Signature as AlloySignature, B256}; // Removed Address
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use serde_json::Value; // Needed for manipulating JSON for tests
use std::path::PathBuf;
use tempfile::tempdir;

// Helper to create a keystore in a temporary directory
fn create_temp_keystore_path() -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_keystore.json");
    (dir, path)
}

const TEST_PASSWORD: &str = "testpassword123";
const WRONG_PASSWORD: &str = "wrongpassword";
// A valid 32-byte hex private key (integer value 1)
const DUMMY_PK_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const DUMMY_PK_HEX_2: &str = "0000000000000000000000000000000000000000000000000000000000000002";
const NEW_PASSWORD: &str = "newpassword456";
// const DUMMY_PK_HEX_3: &str = "0000000000000000000000000000000000000000000000000000000000000003"; // Unused

#[test]
fn test_new_keystore_creation() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let ks = Keystore::new(Some(keystore_path));
    assert!(ks.is_ok(), "Keystore::new should succeed");
}

#[test]
fn test_initialize_new_keystore_with_password() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    let unlock_res_before_init = ks.unlock(TEST_PASSWORD);
    assert!(
        matches!(unlock_res_before_init, Err(KeystoreError::FsError(_))),
        "Unlock should fail before KDF params are set by initialize_or_load"
    );

    ks.initialize_or_load(Some(TEST_PASSWORD))
        .expect("Initialization with password failed");
    assert!(keystore_path.exists(), "Keystore file should be created");

    let import_res = ks.import_private_key_hex(None, DUMMY_PK_HEX);
    assert!(
        matches!(import_res, Err(KeystoreError::Locked)),
        "Import should fail, keystore should be locked after init"
    );

    ks.unlock(TEST_PASSWORD)
        .expect("Unlock should succeed after init with password");

    ks.import_private_key_hex(Some("test_init_key".to_string()), DUMMY_PK_HEX)
        .expect("Import should succeed on initialized and unlocked keystore");
}

#[test]
fn test_initialize_new_keystore_no_password() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    ks.initialize_or_load(None)
        .expect("Initialization without password failed");

    assert!(
        !keystore_path.exists(),
        "Keystore file should not be created if no password for new keystore"
    );

    let unlock_res = ks.unlock(TEST_PASSWORD);
    assert!(
        matches!(unlock_res, Err(KeystoreError::FsError(msg)) if msg.contains("Keystore is not initialized")),
        "Unlock should fail as KDF params are not set"
    );
}

#[test]
fn test_unlock_lock_cycle() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    ks.unlock(TEST_PASSWORD).expect("Unlock failed");
    let import_res_unlocked = ks.import_private_key_hex(Some("key1".to_string()), DUMMY_PK_HEX);
    assert!(
        import_res_unlocked.is_ok(),
        "Import should succeed when unlocked"
    );

    ks.lock();
    let import_res_locked = ks.import_private_key_hex(Some("key2".to_string()), DUMMY_PK_HEX);
    assert!(
        matches!(import_res_locked, Err(KeystoreError::Locked)),
        "Import should fail when locked"
    );
}

#[test]
fn test_unlock_with_wrong_password_on_existing_keystore() {
    // Create a completely new temporary directory for this test to avoid rate-limiting state persistence
    let (_temp_dir, keystore_path) = create_temp_keystore_path();

    // Create a keystore with a custom config
    let config = KeystoreConfig {
        // KDF parameters
        m_cost: 131072, // Minimum required memory cost
        t_cost: 4,      // Minimum required time cost
        p_cost: 1,      // Default parallelism
        output_len: 32, // Minimum required output length
        // Session management
        auto_lock_timeout: Duration::from_secs(300), // 5 minutes
    };

    // Create and initialize a new keystore with the test password and custom config
    let mut ks = Keystore::new_with_config(Some(keystore_path.clone()), config.clone()).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    // First, unlock with the correct password to ensure everything is set up properly
    ks.unlock(TEST_PASSWORD).unwrap();
    ks.lock();

    // With the new security model, unlock with wrong password should fail
    let unlock_result = ks.unlock(WRONG_PASSWORD);
    assert!(
        matches!(unlock_result, Err(KeystoreError::InvalidPassword)),
        "Unlock with wrong password should fail with InvalidPassword"
    );

    // Keystore should remain locked
    assert!(!ks.is_unlocked);

    // Since unlock failed, import should also fail because keystore is locked
    let import_res =
        ks.import_private_key_hex(Some("key_with_wrong_pass".to_string()), DUMMY_PK_HEX);
    assert!(
        matches!(import_res, Err(KeystoreError::Locked)),
        "Import should fail because keystore is locked after failed unlock"
    );

    // Create a completely new keystore in a different path with minimal rate limiting
    let (_new_temp_dir, new_keystore_path) = create_temp_keystore_path();
    let mut new_ks =
        Keystore::new_with_config(Some(new_keystore_path.clone()), config.clone()).unwrap();

    // Initialize the new keystore
    new_ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    // Now unlock with correct password (should work as this is a fresh keystore)
    new_ks.unlock(TEST_PASSWORD).unwrap();

    // Import a key with correct password
    let (id, _) = new_ks
        .import_private_key_hex(Some("key_with_correct_pass".to_string()), DUMMY_PK_HEX)
        .unwrap();

    // Should be able to get signer for this key
    assert!(new_ks.get_signer(id).is_ok());
}

#[test]
fn test_load_existing_keystore() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let original_key_alias = "key_to_load".to_string();
    {
        let mut ks_orig = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks_orig.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks_orig.unlock(TEST_PASSWORD).unwrap();
        ks_orig
            .import_private_key_hex(Some(original_key_alias.clone()), DUMMY_PK_HEX)
            .unwrap();
    }

    let mut ks_loaded = Keystore::new(Some(keystore_path)).unwrap();
    ks_loaded
        .initialize_or_load(None)
        .expect("Loading existing keystore (no password) failed");

    let import_res_locked = ks_loaded.import_private_key_hex(None, DUMMY_PK_HEX_2);
    assert!(matches!(import_res_locked, Err(KeystoreError::Locked)));

    ks_loaded
        .unlock(TEST_PASSWORD)
        .expect("Unlocking loaded keystore failed");

    let keys = ks_loaded.list_keys().unwrap();
    assert_eq!(keys.len(), 1, "Should have one key after loading");
    assert_eq!(keys[0].alias, Some(original_key_alias));
}


#[test]
fn test_h1_authenticated_keystore_round_trip() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let key_alias = "test_h1_key_alias";

    // Phase 1: Create, initialize, import key, and save (implicitly by initialize and import)
    let imported_key_id = {
        let mut ks1 = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks1.initialize_or_load(Some(TEST_PASSWORD))
            .expect("Phase 1: Keystore initialization failed");
        ks1.unlock(TEST_PASSWORD)
            .expect("Phase 1: Keystore unlock failed");
        let (key_id, _) = ks1
            .import_private_key_hex(Some(key_alias.to_string()), DUMMY_PK_HEX)
            .expect("Phase 1: Key import failed");
        // Keystore is saved on import and on drop if changes were made.
        key_id
    }; // ks1 goes out of scope, file lock released

    // Phase 2: Load the keystore, unlock, and verify key
    {
        let mut ks2 = Keystore::new(Some(keystore_path)).unwrap();
        ks2.initialize_or_load(None) // No password here, just load
            .expect("Phase 2: Keystore loading failed");

        // At this point, ks2 has pending_protected_data_b64 and pending_mac_b64

        ks2.unlock(TEST_PASSWORD) // This will trigger H-1 MAC verification
            .expect("Phase 2: Keystore unlock with MAC verification failed");

        let keys = ks2.list_keys().expect("Phase 2: Listing keys failed");
        assert_eq!(keys.len(), 1, "Expected one key after loading");
        let loaded_key_info = &keys[0];
        assert_eq!(loaded_key_info.id, imported_key_id);
        assert_eq!(loaded_key_info.alias.as_deref(), Some(key_alias));
    }
}


#[test]
fn test_h1_mac_verification_failure_tampered_data() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();

    // Phase 1: Create and save a valid keystore
    {
        let mut ks1 = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks1.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks1.unlock(TEST_PASSWORD).unwrap();
        ks1.import_private_key_hex(Some("key".to_string()), DUMMY_PK_HEX)
            .unwrap();
    } // ks1 goes out of scope, file saved and lock released

    // Phase 2: Tamper with the protected_data_b64 in the keystore file
    let mut content_value: Value = serde_json::from_str(
        &std::fs::read_to_string(keystore_path.as_path()).expect("Failed to read keystore file"),
    )
    .expect("Failed to parse keystore JSON");

    if let Some(Value::String(data_str)) = content_value.get_mut("protected_data_b64") {
        data_str.push_str("tamper"); // Append some chars to invalidate Base64 or content
    } else {
        panic!("protected_data_b64 field not found or not a string in keystore JSON");
    }
    std::fs::write(
        keystore_path.as_path(),
        serde_json::to_string_pretty(&content_value).unwrap(),
    )
    .expect("Failed to write tampered keystore file");

    // Phase 3: Attempt to load and unlock the tampered keystore
    {
        let mut ks2 = Keystore::new(Some(keystore_path)).unwrap();
        ks2.initialize_or_load(None)
            .expect("Loading tampered keystore data should succeed (parsing envelope)");
        
        let unlock_result = ks2.unlock(TEST_PASSWORD);
        assert!(
            matches!(unlock_result, Err(KeystoreError::MacVerificationFailure)),
            "Unlock should fail with MacVerificationFailure, got {:?}",
            unlock_result
        );
    }
}


#[test]
fn test_h1_mac_verification_failure_tampered_mac() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();

    // Phase 1: Create and save a valid keystore
    {
        let mut ks1 = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks1.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks1.unlock(TEST_PASSWORD).unwrap();
        ks1.import_private_key_hex(Some("key".to_string()), DUMMY_PK_HEX)
            .unwrap();
    } // ks1 goes out of scope, file saved and lock released

    // Phase 2: Tamper with the mac_b64 in the keystore file
    let mut content_value: Value = serde_json::from_str(
        &std::fs::read_to_string(keystore_path.as_path()).expect("Failed to read keystore file"),
    )
    .expect("Failed to parse keystore JSON");

    if let Some(Value::String(mac_str)) = content_value.get_mut("mac_b64") {
        // Change a character in the Base64 MAC. A valid HMAC-SHA256 Base64 is 44 chars.
        if !mac_str.is_empty() {
            let first_char = mac_str.chars().next().unwrap();
            let replacement = if first_char == 'A' { 'B' } else { 'A' };
            mac_str.replace_range(..1, &replacement.to_string());
        }
    } else {
        panic!("mac_b64 field not found or not a string in keystore JSON");
    }
    std::fs::write(
        keystore_path.as_path(),
        serde_json::to_string_pretty(&content_value).unwrap(),
    )
    .expect("Failed to write tampered keystore file");

    // Phase 3: Attempt to load and unlock the tampered keystore
    {
        let mut ks2 = Keystore::new(Some(keystore_path)).unwrap();
        ks2.initialize_or_load(None)
            .expect("Loading tampered keystore data should succeed (parsing envelope)");
        
        let unlock_result = ks2.unlock(TEST_PASSWORD);
        assert!(
            matches!(unlock_result, Err(KeystoreError::MacVerificationFailure)),
            "Unlock should fail with MacVerificationFailure due to tampered MAC, got {:?}",
            unlock_result
        );
    }
}


#[test]
fn test_h1_correct_mac_wrong_password() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();

    // Phase 1: Create and save a valid keystore
    {
        let mut ks1 = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks1.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks1.unlock(TEST_PASSWORD).unwrap();
        ks1.import_private_key_hex(Some("key".to_string()), DUMMY_PK_HEX)
            .unwrap();
    } // ks1 goes out of scope, file saved and lock released

    // Phase 2: Attempt to load and unlock with the wrong password
    {
        let mut ks2 = Keystore::new(Some(keystore_path)).unwrap();
        ks2.initialize_or_load(None) // Load the envelope
            .expect("Loading keystore data should succeed (parsing envelope)");
        
        let unlock_result = ks2.unlock(WRONG_PASSWORD); // Use wrong password
        assert!(
            matches!(unlock_result, Err(KeystoreError::InvalidPassword)),
            "Unlock should fail with InvalidPassword, got {:?}",
            unlock_result
        );
    }
}


#[test]
fn test_h1_invalid_envelope_format() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();

    // Phase 1: Create a malformed keystore file
    let malformed_json = r#"{"error": "this is not a valid keystore envelope"}"#;
    std::fs::write(keystore_path.as_path(), malformed_json)
        .expect("Failed to write malformed keystore file");

    // Phase 2: Attempt to load the malformed keystore
    {
        let mut ks = Keystore::new(Some(keystore_path)).unwrap();
        let load_result = ks.initialize_or_load(None);
        
        assert!(
            matches!(load_result, Err(KeystoreError::InvalidFormat(_))),
            "Loading malformed keystore should fail with InvalidFormat, got {:?}",
            load_result
        );
    }
}


#[test]
fn test_h1_change_password_interaction() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let key_alias = "test_h1_change_pass_key";

    // Phase 1: Create, initialize, import key
    let imported_key_id = {
        let mut ks1 = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks1.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks1.unlock(TEST_PASSWORD).unwrap();
        let (key_id, _) = ks1
            .import_private_key_hex(Some(key_alias.to_string()), DUMMY_PK_HEX)
            .unwrap();
        
        // Phase 2: Change password
        ks1.change_password(TEST_PASSWORD, NEW_PASSWORD)
            .expect("Password change failed");
        // Keystore is saved by change_password and on drop
        key_id
    }; // ks1 goes out of scope, file lock released

    // Phase 3: Load with new keystore instance, try old and new passwords
    {
        let mut ks2 = Keystore::new(Some(keystore_path)).unwrap();
        ks2.initialize_or_load(None) // Load the envelope
            .expect("Loading keystore after password change failed");

        // Try unlocking with the old password - should fail
        let unlock_old_pw_result = ks2.unlock(TEST_PASSWORD);
        assert!(
            matches!(unlock_old_pw_result, Err(KeystoreError::InvalidPassword)),
            "Unlock with old password should fail, got {:?}",
            unlock_old_pw_result
        );
        assert!(!ks2.is_unlocked, "Keystore should remain locked after failed unlock attempt");

        // Unlock with the new password - should succeed (and verify H-1 MAC)
        ks2.unlock(NEW_PASSWORD)
            .expect("Unlock with new password failed (H-1 MAC check implied)");

        let keys = ks2.list_keys().expect("Listing keys after password change failed");
        assert_eq!(keys.len(), 1, "Expected one key after password change");
        let loaded_key_info = &keys[0];
        assert_eq!(loaded_key_info.id, imported_key_id);
        assert_eq!(loaded_key_info.alias.as_deref(), Some(key_alias));
    }
}

#[test]
fn test_unlock_uninitialized_keystore() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    let unlock_result = ks.unlock("anypassword");
    assert!(unlock_result.is_err());
    if let Err(KeystoreError::FsError(msg)) = unlock_result {
        assert!(msg.contains("Keystore is not initialized with KDF parameters."));
    } else {
        panic!(
            "Expected FsError for uninitialized keystore unlock, got {:?}",
            unlock_result
        );
    }
}

// --- Tests for import_private_key_hex ---
#[test]
fn test_import_private_key_hex_success() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let (id1, addr1) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX)
        .expect("Failed to import PK hex");
    assert!(!id1.is_nil());

    let (id2, addr2) = ks
        .import_private_key_hex(Some("alias2".to_string()), DUMMY_PK_HEX_2)
        .expect("Failed to import PK hex with alias");
    assert!(!id2.is_nil());
    assert_ne!(addr1, addr2);

    let keys = ks.list_keys().unwrap();
    assert_eq!(keys.len(), 2);
    assert!(keys.iter().any(|k| k.id == id1 && k.alias.is_none()));
    assert!(keys
        .iter()
        .any(|k| k.id == id2 && k.alias == Some("alias2".to_string())));
}

#[test]
fn test_import_private_key_hex_duplicate_alias() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    ks.import_private_key_hex(Some("mykey".to_string()), DUMMY_PK_HEX)
        .unwrap();
    let result = ks.import_private_key_hex(Some("mykey".to_string()), DUMMY_PK_HEX_2);
    assert!(matches!(result, Err(KeystoreError::AliasExists(alias)) if alias == "mykey"));
}

#[test]
fn test_import_private_key_hex_invalid_key_format() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let invalid_hex = "not-a-hex-string";
    assert!(matches!(
        ks.import_private_key_hex(None, invalid_hex),
        Err(KeystoreError::Hex(_))
    ));

    let short_hex = "010203";
    assert!(matches!(
        ks.import_private_key_hex(None, short_hex),
        Err(KeystoreError::InvalidPrivateKey)
    ));

    let empty_hex = "";
    assert!(matches!(
        ks.import_private_key_hex(None, empty_hex),
        Err(KeystoreError::InvalidPrivateKey)
    ));
}

#[test]
fn test_argon2_param_minimums_new_with_config() {
    let (_temp_dir1, temp_path_1) = create_temp_keystore_path();
    let (_temp_dir2, temp_path_2) = create_temp_keystore_path();
    let (_temp_dir3, temp_path_3) = create_temp_keystore_path();

    let mut config = KeystoreConfig {
        m_cost: 1000,
        ..KeystoreConfig::default()
    };

    // Test too low m_cost
    let res_m_cost = Keystore::new_with_config(Some(temp_path_1), config.clone());
    if let Err(KeystoreError::FsError(msg)) = res_m_cost {
        assert!(
            msg.contains("KDF m_cost must be at least 131072"),
            "Low m_cost check failed. Msg: {:?}",
            msg
        );
    } else {
        panic!("Expected FsError for low m_cost, got {:?}", res_m_cost);
    }

    // Test too low t_cost
    config.m_cost = KeystoreConfig::default().m_cost; // Reset m_cost to valid
    config.t_cost = 1; // Too low
    let res_t_cost = Keystore::new_with_config(Some(temp_path_2), config.clone());
    if let Err(KeystoreError::FsError(msg)) = res_t_cost {
        assert!(
            msg.contains("KDF t_cost must be at least 4"),
            "Low t_cost check failed. Msg: {:?}",
            msg
        );
    } else {
        panic!("Expected FsError for low t_cost, got {:?}", res_t_cost);
    }

    // Test valid params succeed
    config.t_cost = KeystoreConfig::default().t_cost; // Reset t_cost to valid
    let res_valid = Keystore::new_with_config(Some(temp_path_3), config.clone());
    assert!(
        res_valid.is_ok(),
        "Valid params should succeed. Err: {:?}",
        res_valid.err()
    );
}

#[test]
fn test_import_private_key_hex_keystore_locked() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    let result = ks.import_private_key_hex(None, DUMMY_PK_HEX);
    assert!(matches!(result, Err(KeystoreError::Locked)));
}

// --- Tests for import_mnemonic ---
const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const TEST_DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";
const TEST_MNEMONIC_2_VALID: &str = "test test test test test test test test test test test junk"; // Known valid mnemonic

#[test]
fn test_import_mnemonic_success() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let (id1, addr1) = ks
        .import_mnemonic(None, TEST_MNEMONIC, None, TEST_DERIVATION_PATH)
        .expect("Failed to import mnemonic");
    assert!(!id1.is_nil());

    let (id2, addr2) = ks
        .import_mnemonic(
            Some("mnemonic_key".to_string()),
            TEST_MNEMONIC_2_VALID,
            Some("secret_passphrase"),
            TEST_DERIVATION_PATH,
        )
        .expect("Failed to import mnemonic with alias and passphrase");
    assert!(!id2.is_nil());
    assert_ne!(addr1, addr2);

    let keys = ks.list_keys().unwrap();
    assert_eq!(keys.len(), 2);
    assert!(keys.iter().any(|k| k.id == id1 && k.alias.is_none()));
    assert!(keys
        .iter()
        .any(|k| k.id == id2 && k.alias == Some("mnemonic_key".to_string())));
}

#[test]
fn test_import_mnemonic_duplicate_alias() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    ks.import_mnemonic(
        Some("my_mnemonic".to_string()),
        TEST_MNEMONIC,
        None,
        TEST_DERIVATION_PATH,
    )
    .unwrap();
    let result = ks.import_mnemonic(
        Some("my_mnemonic".to_string()),
        TEST_MNEMONIC_2_VALID,
        None,
        TEST_DERIVATION_PATH,
    );
    assert!(matches!(result, Err(KeystoreError::AliasExists(alias)) if alias == "my_mnemonic"));
}

#[test]
fn test_import_mnemonic_invalid_phrase() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let invalid_phrase = "this is not a valid mnemonic phrase definitely";
    assert!(matches!(
        ks.import_mnemonic(None, invalid_phrase, None, TEST_DERIVATION_PATH),
        Err(KeystoreError::Bip39(_))
    ));
}

#[test]
fn test_import_mnemonic_invalid_path() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let invalid_path = "m/44'/60a/0'/0/0"; // Invalid character 'a'
    assert!(matches!(
        ks.import_mnemonic(None, TEST_MNEMONIC, None, invalid_path),
        Err(KeystoreError::InvalidPath(_))
    ));
}

#[test]
fn test_import_mnemonic_keystore_locked() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    let result = ks.import_mnemonic(None, TEST_MNEMONIC, None, TEST_DERIVATION_PATH);
    assert!(matches!(result, Err(KeystoreError::Locked)));
}

// --- Tests for list_keys and delete_key ---
#[test]
fn test_list_and_delete_keys() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    assert!(
        ks.list_keys().unwrap().is_empty(),
        "Keystore should be empty initially"
    );

    let (id1, _) = ks
        .import_private_key_hex(Some("key1".to_string()), DUMMY_PK_HEX)
        .unwrap();
    let (id2, _) = ks
        .import_mnemonic(
            Some("key2".to_string()),
            TEST_MNEMONIC,
            None,
            TEST_DERIVATION_PATH,
        )
        .unwrap();
    let (id3, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX_2).unwrap();

    let keys = ks.list_keys().unwrap();
    assert_eq!(keys.len(), 3, "Should have 3 keys");
    assert!(keys
        .iter()
        .any(|k| k.id == id1 && k.alias == Some("key1".to_string())));
    assert!(keys
        .iter()
        .any(|k| k.id == id2 && k.alias == Some("key2".to_string())));
    assert!(keys.iter().any(|k| k.id == id3 && k.alias.is_none()));

    ks.delete_key(id1).expect("Failed to delete key1");
    let keys_after_delete1 = ks.list_keys().unwrap();
    assert_eq!(
        keys_after_delete1.len(),
        2,
        "Should have 2 keys after deleting one"
    );
    assert!(!keys_after_delete1.iter().any(|k| k.id == id1));

    let non_existent_id = uuid::Uuid::new_v4();
    assert!(matches!(
        ks.delete_key(non_existent_id),
        Err(KeystoreError::KeyNotFound(_))
    ));

    ks.lock();
    assert!(matches!(ks.delete_key(id2), Err(KeystoreError::Locked)));
    ks.unlock(TEST_PASSWORD).unwrap();

    ks.delete_key(id2).unwrap();
    ks.delete_key(id3).unwrap();
    assert!(
        ks.list_keys().unwrap().is_empty(),
        "Keystore should be empty after deleting all keys"
    );
}

// --- Tests for get_signer ---
#[test]
fn test_get_signer_success() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();
    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();

    let signer = ks.get_signer(id);
    assert!(signer.is_ok());
}

#[test]
fn test_get_signer_key_not_found() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();
    let non_existent_id = uuid::Uuid::new_v4();

    assert!(matches!(
        ks.get_signer(non_existent_id),
        Err(KeystoreError::KeyNotFound(_))
    ));
}

#[test]
fn test_get_signer_locked() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();
    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();
    ks.lock();

    let message_hash = B256::ZERO;
    let dummy_sig = AlloySignature::test_signature();
    assert!(matches!(
        ks.verify_signature(id, message_hash, dummy_sig),
        Err(KeystoreError::Locked)
    ));
}

#[test]
fn test_change_password_success() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();

    ks.change_password(TEST_PASSWORD, NEW_PASSWORD)
        .expect("Failed to change password");
    // The change_password function now locks the keystore and resets rate limiting.
    // Add a small delay to ensure file system sync for rate limiting state.
    std::thread::sleep(Duration::from_secs(1));

    // Keystore should be locked after password change
    assert!(matches!(ks.get_signer(id), Err(KeystoreError::Locked)));

    // Unlock with old password should fail
    assert!(matches!(
        ks.unlock(TEST_PASSWORD),
        Err(KeystoreError::InvalidPassword)
    ));

    std::thread::sleep(Duration::from_secs(1));

    // Unlock with new password should succeed
    ks.unlock(NEW_PASSWORD)
        .expect("Unlock with new password failed");

    // Verify key is still accessible and can sign
    let signer = ks
        .get_signer(id)
        .expect("Key should be accessible after password change");
    let signing_key = signer.as_ref().clone(); // Clone the inner SigningKey
    let wallet = PrivateKeySigner::from(signing_key); // Create PrivateKeySigner
    let message_hash = B256::from_slice(&[1u8; 32]);
    let _signature = wallet
        .sign_hash_sync(&message_hash)
        .expect("Signing with re-encrypted key failed");
}

#[test]
fn test_change_password_incorrect_old_password() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();

    // Add a small delay to ensure any previous rate limiting state has expired.
    std::thread::sleep(Duration::from_millis(100));
    let change_res = ks.change_password(WRONG_PASSWORD, NEW_PASSWORD);
    assert!(
        matches!(change_res, Err(KeystoreError::InvalidPassword)),
        "Changing password with incorrect old password should fail with InvalidPassword"
    );

    // Keystore should remain unlocked and functional with the old password
    assert!(ks.is_unlocked);
    ks.get_signer(id)
        .expect("Key should still be accessible with old password");

    // Lock the keystore to force an actual unlock attempt with NEW_PASSWORD
    ks.lock();

    // Attempt to unlock with new password should fail
    assert!(matches!(
        ks.unlock(NEW_PASSWORD),
        Err(KeystoreError::InvalidPassword)
    ));
}

#[test]
fn test_change_password_persisted_kdf_params() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let original_key_alias = "persisted_key".to_string();

    {
        let mut ks_orig = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks_orig.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks_orig.unlock(TEST_PASSWORD).unwrap();
        ks_orig
            .import_private_key_hex(Some(original_key_alias.clone()), DUMMY_PK_HEX)
            .unwrap();
        ks_orig
            .change_password(TEST_PASSWORD, NEW_PASSWORD)
            .expect("Failed to change password in original instance");
    } // ks_orig is dropped, changes should be persisted
    std::thread::sleep(Duration::from_secs(1));

    // Load a new keystore instance from the same path
    let mut ks_loaded = Keystore::new(Some(keystore_path)).unwrap();
    ks_loaded
        .initialize_or_load(None) // Load without password, it should be locked
        .expect("Loading existing keystore failed");

    // Attempt to unlock with the old password should fail
    let unlock_old_res = ks_loaded.unlock(TEST_PASSWORD);
    assert!(
        matches!(unlock_old_res, Err(KeystoreError::InvalidPassword)),
        "Unlock with old password should fail after password change"
    );
    std::thread::sleep(Duration::from_secs(1));

    // Unlock with the new password should succeed
    ks_loaded
        .unlock(NEW_PASSWORD)
        .expect("Unlock with new password failed after loading");
    // Add a small delay to ensure file system sync for rate limiting state.
    std::thread::sleep(Duration::from_secs(1));

    // Verify the key is still present and accessible
    let keys = ks_loaded.list_keys().unwrap();
    assert_eq!(
        keys.len(),
        1,
        "Should have one key after loading and unlocking"
    );
    let imported_key_id = keys[0].id;
    assert_eq!(keys[0].alias, Some(original_key_alias));

    let signer = ks_loaded
        .get_signer(imported_key_id)
        .expect("Key should be accessible after loading and unlocking with new password");
    let signing_key = signer.as_ref().clone(); // Clone the inner SigningKey
    let wallet = PrivateKeySigner::from(signing_key); // Create PrivateKeySigner
    let message_hash = B256::from_slice(&[2u8; 32]);
    let _signature = wallet
        .sign_hash_sync(&message_hash)
        .expect("Signing with re-encrypted key after load failed");
}

#[test]
fn test_auto_lock_timeout() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let config = KeystoreConfig {
        auto_lock_timeout: Duration::from_secs(1), // Short timeout for testing
        ..Default::default()
    };
    let mut ks = Keystore::new_with_config(Some(keystore_path), config).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();

    // Wait for timeout to pass
    std::thread::sleep(Duration::from_secs(2)); // 1 second timeout + buffer

    // Keystore should now be locked
    assert!(matches!(ks.get_signer(id), Err(KeystoreError::Locked)));

    // Unlock should succeed
    ks.unlock(TEST_PASSWORD)
        .expect("Unlock after auto-lock failed");
    assert!(
        ks.get_signer(id).is_ok(),
        "Key should be accessible after re-unlock"
    );
}

#[test]
fn test_activity_resets_auto_lock_timestamp() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let config = KeystoreConfig {
        auto_lock_timeout: Duration::from_secs(2), // 2 second timeout
        ..Default::default()
    };
    let mut ks = Keystore::new_with_config(Some(keystore_path), config).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let (id1, _) = ks
        .import_private_key_hex(Some("key1".to_string()), DUMMY_PK_HEX)
        .unwrap();

    // Wait for half the timeout
    std::thread::sleep(Duration::from_millis(1000)); // 1 second

    // Perform an activity: import another key
    let (id2, _) = ks
        .import_private_key_hex(Some("key2".to_string()), DUMMY_PK_HEX_2)
        .unwrap();
    // This should reset the timestamp

    // Wait for another half the timeout (total time passed is now 2 seconds since initial import)
    std::thread::sleep(Duration::from_millis(1000)); // 1 second

    // Keystore should NOT be locked because activity reset the timer
    assert!(
        ks.get_signer(id1).is_ok(),
        "Key1 should still be accessible after activity"
    );
    assert!(
        ks.get_signer(id2).is_ok(),
        "Key2 should be accessible after activity"
    );

    // Test with get_signer activity
    std::thread::sleep(Duration::from_millis(1000)); // 1 second
    ks.get_signer(id1).unwrap(); // Activity
    std::thread::sleep(Duration::from_millis(1000)); // 1 second
    assert!(
        ks.get_signer(id1).is_ok(),
        "Key1 should still be accessible after get_signer activity"
    );

    // Test with delete_key activity
    std::thread::sleep(Duration::from_millis(1000)); // 1 second
    ks.delete_key(id2).unwrap(); // Activity
    std::thread::sleep(Duration::from_millis(1000)); // 1 second
    assert!(
        ks.get_signer(id1).is_ok(),
        "Key1 should still be accessible after delete_key activity"
    );

    // Test with list_keys activity
    std::thread::sleep(Duration::from_millis(1000)); // 1 second
    ks.list_keys().unwrap(); // Activity
    std::thread::sleep(Duration::from_millis(1000)); // 1 second
    assert!(
        ks.get_signer(id1).is_ok(),
        "Key1 should still be accessible after list_keys activity"
    );

    // Finally, let it auto-lock
    std::thread::sleep(Duration::from_secs(3)); // 2 second timeout + buffer
    assert!(matches!(ks.get_signer(id1), Err(KeystoreError::Locked)));
}

#[test]
fn test_load_from_disk_malformed_json() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    // Write malformed JSON to the file
    std::fs::write(&keystore_path, "{ \"invalid_json\": ").unwrap();

    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    let load_res = ks.initialize_or_load(None); // Attempt to load
    assert!(
        matches!(load_res, Err(KeystoreError::SerdeJson(_))),
        "Loading malformed JSON should result in SerdeJson error"
    );
}

#[test]
fn test_load_from_disk_unsupported_version() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    // Create a dummy keystore structure with an unsupported version
    let unsupported_keystore_json = serde_json::json!({
        "version": "99.0.0", // Future, unsupported version
        "master_kdf": "unsupported_kdf_algo", // Set master_kdf at the root level
        "master_kdf_params": { // Renamed from kdf_params to master_kdf_params
            "salt": "00000000000000000000000000000000", // Dummy salt, hex encoded
            "m_cost": 131072,
            "t_cost": 4,
            "p_cost": 1,
            "output_len": 32,
            "kdf_version": 0 // Add kdf_version as it's part of MasterKdfParams
        },
        "verification_tag": "dummy_tag",
        "verification_nonce": "dummy_nonce", // Add verification_nonce
        "entries": [] // Renamed from keys to entries
    });
    std::fs::write(&keystore_path, unsupported_keystore_json.to_string()).unwrap();

    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    let load_res = ks.initialize_or_load(None);
    assert!(
        matches!(load_res, Err(KeystoreError::InvalidFormat(_))), // Changed to InvalidFormat
        "Loading unsupported version should result in InvalidFormat error, got {:?}",
        load_res
    );
}

#[test]
fn test_aes_gcm_error_trigger() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();
    ks.lock(); // Lock to ensure data is encrypted on disk

    // Manually load the keystore file, tamper with the encrypted private key
    let mut json_value: Value =
        serde_json::from_str(&std::fs::read_to_string(&keystore_path).unwrap()).unwrap();
    if let Some(entries_array) = json_value["entries"].as_array_mut() {
        if let Some(key_entry) = entries_array.get_mut(0) {
            if let Some(encrypted_pk_val) = key_entry["encrypted_pk"].as_str() {
                // Corrupt the encrypted private key by changing a byte
                let mut corrupted_pk = encrypted_pk_val.to_string();
                // Corrupt the encrypted private key by truncating it, causing an invalid length for base64 decoding
                if corrupted_pk.len() > 1 {
                    // Ensure there's at least one character to remove
                    corrupted_pk.pop(); // Remove the last character
                } else {
                    panic!("Encrypted private key too short to corrupt");
                }
                key_entry["encrypted_pk"] = Value::String(corrupted_pk);
            }
        }
    }
    std::fs::write(&keystore_path, serde_json::to_string(&json_value).unwrap()).unwrap();

    // Load the corrupted keystore
    let mut ks_corrupted = Keystore::new(Some(keystore_path)).unwrap();
    ks_corrupted.initialize_or_load(None).unwrap(); // Should load successfully, but decryption will fail
    ks_corrupted.unlock(TEST_PASSWORD).unwrap(); // Unlock to attempt decryption

    // Try to get the signer for the corrupted key, which should now fail with InvalidFormat
    let get_signer_res = ks_corrupted.get_signer(id);
    assert!(
        matches!(get_signer_res, Err(KeystoreError::InvalidFormat(_))),
        "Getting signer for corrupted key should result in InvalidFormat error due to base64 decode failure"
    );
}

#[test]
fn test_kdf_params_mismatch_error() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    {
        let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks.unlock(TEST_PASSWORD).unwrap();
        ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();
    } // Keystore saved to disk

    // Manually load the keystore file and tamper with KDF parameters
    let mut json_value: Value =
        serde_json::from_str(&std::fs::read_to_string(&keystore_path).unwrap()).unwrap();
    println!("json_value: {:?}", json_value);
    if let Some(kdf_params) = json_value["master_kdf_params"].as_object_mut() {
        kdf_params.insert("m_cost".to_string(), Value::from(1024)); // Change m_cost
    }
    std::fs::write(&keystore_path, serde_json::to_string(&json_value).unwrap()).unwrap();

    let json_value: Value =
        serde_json::from_str(&std::fs::read_to_string(&keystore_path).unwrap()).unwrap();

    println!("json_value: {:?}", json_value);
    // Attempt to load and unlock the keystore with mismatched KDF params
    let mut ks_mismatched = Keystore::new(Some(keystore_path)).unwrap();
    let load_res = ks_mismatched.initialize_or_load(None); // Load should now fail
    assert!(
        matches!(load_res, Err(KeystoreError::Argon2Error(_))),
        "Loading with tampered KDF params should result in Argon2Error; got: {:?}",
        load_res
    );
}

#[test]
fn test_unsupported_kdf_error() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    // Create a dummy keystore structure with an unsupported KDF
    let unsupported_kdf_json = serde_json::json!({
        "version": "1.0.0",
        "master_kdf": "unsupported_kdf_algo", // Set master_kdf at the root level
        "master_kdf_params": { // Renamed from kdf_params to master_kdf_params
            "salt": "00000000000000000000000000000000", // Dummy salt, hex encoded
            "m_cost": 131072,
            "t_cost": 4,
            "p_cost": 1,
            "output_len": 32,
            "kdf_version": 0 // Add kdf_version as it's part of MasterKdfParams
        },
        "verification_tag": "dummy_tag",
        "verification_nonce": "dummy_nonce", // Add verification_nonce
        "entries": [] // Renamed from keys to entries
    });
    std::fs::write(&keystore_path, unsupported_kdf_json.to_string()).unwrap();

    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    let load_res = ks.initialize_or_load(None);
    assert!(
        matches!(load_res, Err(KeystoreError::UnsupportedKdf(algo)) if algo == "unsupported_kdf_algo"),
        "Loading with unsupported KDF should result in UnsupportedKdf error"
    );
}

#[test]
fn test_missing_verification_tag_error() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    {
        let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks.unlock(TEST_PASSWORD).unwrap();
        ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();
    } // Keystore saved to disk

    // Manually load the keystore file and remove the verification_tag
    let mut json_value: Value =
        serde_json::from_str(&std::fs::read_to_string(&keystore_path).unwrap()).unwrap();
    json_value
        .as_object_mut()
        .unwrap()
        .remove("verification_tag");
    std::fs::write(&keystore_path, serde_json::to_string(&json_value).unwrap()).unwrap();

    // Attempt to load and unlock the keystore
    let mut ks_missing_tag = Keystore::new(Some(keystore_path)).unwrap();
    ks_missing_tag.initialize_or_load(None).unwrap(); // Load should succeed
    let unlock_res = ks_missing_tag.unlock(TEST_PASSWORD); // Unlock should fail due to missing tag
    assert!(
        matches!(unlock_res, Err(KeystoreError::MissingVerificationTag)),
        "Unlock with missing verification tag should result in MissingVerificationTag error"
    );
}
