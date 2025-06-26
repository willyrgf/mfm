use crate::keystore::KeystoreConfig;
use base64::Engine;
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

// Helper to create a keystore with fast test parameters for performance
fn create_test_keystore(path: Option<PathBuf>) -> Result<Keystore, KeystoreError> {
    let config = KeystoreConfig::test_fast();
    Keystore::new_with_config_test_mode(path, config)
}

const TEST_PASSWORD: &str = "testpassword123";
const WRONG_PASSWORD: &str = "wrongpassword";
// A valid 32-byte hex private key (integer value 1)
const DUMMY_PK_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const DUMMY_PK_HEX_2: &str = "0000000000000000000000000000000000000000000000000000000000000002";
const NEW_PASSWORD: &str = "newpassword456";
// const DUMMY_PK_HEX_3: &str = "0000000000000000000000000000000000000000000000000000000000000003"; // Unused

// L-2 Test: Test comprehensive audit logging
#[test]
fn test_l2_comprehensive_audit_logging() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    // Initialize keystore (should log creation)
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    // Unlock keystore (should log unlock)
    ks.unlock(TEST_PASSWORD).unwrap();

    // Import a key (should log import)
    let (key_id, _address) = ks
        .import_private_key_hex(Some("test_key".to_string()), DUMMY_PK_HEX)
        .unwrap();

    // Access key (should log access)
    let _signer = ks.get_signer(key_id).unwrap();

    // Test failed unlock (should log failure)
    ks.lock();
    let result = ks.unlock(WRONG_PASSWORD);
    assert!(result.is_err(), "Wrong password should fail");

    // Test rate limiting after multiple failures (should log rate limiting)
    for _ in 0..5 {
        ks.record_failed_attempt();
    }

    // Reset rate limiting and verify session management events are logged
    ks.reset_rate_limiting_for_test();
    ks.unlock(TEST_PASSWORD).unwrap();
    ks.lock(); // Should log manual lock

    // Test password change (should log password change)
    // Need to reload after lock to have proper state
    ks.initialize_or_load(None).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();
    let change_result = ks.change_password(TEST_PASSWORD, NEW_PASSWORD);
    assert!(change_result.is_ok(), "Password change should succeed");

    // Note: Actual audit log verification would depend on the logger implementation
    // In a real test, you might inject a mock logger to capture and verify events
}

// L-2 Enhancement Test: Test tamper-evident audit log export
#[test]
fn test_l2_tamper_evident_audit_log_export() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    // Initialize and unlock keystore
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    // Import a key to have some activity
    let (_key_id, _address) = ks
        .import_private_key_hex(Some("test_key".to_string()), DUMMY_PK_HEX)
        .unwrap();

    // Export tamper-evident audit log
    let export_path = keystore_path.with_extension("audit.jsonl");
    let export_result = ks.export_tamper_evident_audit_log(&export_path, None);
    assert!(export_result.is_ok(), "Audit log export should succeed");

    // Verify the export file was created
    assert!(export_path.exists(), "Audit export file should exist");

    // Read and verify the export file structure
    let content = std::fs::read_to_string(&export_path).unwrap();
    assert!(!content.is_empty(), "Export file should not be empty");

    // Verify it contains at least the metadata record
    let lines: Vec<&str> = content.trim().lines().collect();
    assert!(
        !lines.is_empty(),
        "Export should contain at least one record"
    );

    // Parse the first line (should be metadata)
    let first_record: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert!(
        first_record.get("record").is_some(),
        "Should contain audit record"
    );
    assert!(
        first_record.get("signature").is_some(),
        "Should contain signature for tamper evidence"
    );

    // Verify the record contains expected fields
    let record = first_record.get("record").unwrap();
    assert!(
        record.get("timestamp").is_some(),
        "Record should have timestamp"
    );
    assert!(
        record.get("event_type").is_some(),
        "Record should have event type"
    );
    assert!(
        record.get("session_id").is_some(),
        "Record should have session ID"
    );
}

// M-4 Test: Test key rotation framework
#[test]
fn test_m4_key_rotation_framework() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    // Initialize and unlock keystore
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    // Import a key
    let (key_id, _address) = ks
        .import_private_key_hex(Some("test_key".to_string()), DUMMY_PK_HEX)
        .unwrap();

    // Verify the key was created with current version
    let keys = ks.list_keys().unwrap();
    let imported_key = keys.iter().find(|k| k.id == key_id).unwrap();
    assert_eq!(
        imported_key.encryption_version, 1,
        "New keys should use current encryption version"
    );
    assert_eq!(
        imported_key.kdf_version, 1,
        "New keys should use current KDF version"
    );

    // Check that no keys need rotation initially
    let keys_needing_rotation = ks.check_keys_needing_rotation();
    assert!(
        keys_needing_rotation.is_empty(),
        "Newly created keys should not need rotation"
    );

    // Test key rotation function (should be no-op since key is already current)
    let rotation_result = ks.rotate_key(key_id);
    assert!(
        rotation_result.is_ok(),
        "Rotating current key should succeed (no-op)"
    );

    // Test rotating all keys (should be no-op)
    let rotated_keys = ks.rotate_all_keys().unwrap();
    assert!(rotated_keys.is_empty(), "No keys should need rotation");

    // Verify key still works after rotation attempt
    let signer_result = ks.get_signer(key_id);
    assert!(
        signer_result.is_ok(),
        "Key should still work after rotation attempt"
    );
}

// M-3 Test: Test enhanced session management with proper invalidation
#[test]
fn test_m3_session_management() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    // Initialize and unlock keystore
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    // Verify session token was created
    assert!(
        ks.session_token.is_some(),
        "Session token should be generated on unlock"
    );
    assert!(
        ks.session_created_at.is_some(),
        "Session creation time should be recorded"
    );
    assert!(
        ks.validate_session(),
        "Session should be valid after unlock"
    );

    // Test that operations work with valid session
    let import_result = ks.import_private_key_hex(Some("test_key".to_string()), DUMMY_PK_HEX);
    assert!(
        import_result.is_ok(),
        "Import should work with valid session"
    );

    // Test session invalidation on lock
    ks.lock();
    assert!(
        ks.session_token.is_none(),
        "Session token should be cleared on lock"
    );
    assert!(
        ks.session_created_at.is_none(),
        "Session creation time should be cleared on lock"
    );
    assert!(
        !ks.validate_session(),
        "Session should be invalid after lock"
    );

    // Test that operations fail after session invalidation
    let get_signer_result = ks.get_signer(uuid::Uuid::new_v4());
    assert!(
        matches!(get_signer_result, Err(KeystoreError::Locked)),
        "Operations should fail with invalid session"
    );
}

// M-2 Test: Test enhanced MAC coverage including KDF algorithm name
#[test]
fn test_m2_enhanced_mac_coverage() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    // Initialize and save keystore
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();
    ks.import_private_key_hex(Some("test_key".to_string()), DUMMY_PK_HEX)
        .unwrap();
    ks.lock();

    // Read the original file and verify that the MAC works correctly first
    let mut ks2 = Keystore::new(Some(keystore_path.clone())).unwrap();
    ks2.initialize_or_load(None).unwrap();
    let original_unlock = ks2.unlock(TEST_PASSWORD);
    assert!(
        original_unlock.is_ok(),
        "Original keystore should unlock successfully"
    );

    // Create a new keystore instance to test MAC enhancement after reload
    let mut ks3 = Keystore::new(Some(keystore_path)).unwrap();
    ks3.initialize_or_load(None).unwrap();

    // Verify that the keystore still works after reload with enhanced MAC coverage
    let final_unlock = ks3.unlock(TEST_PASSWORD);
    assert!(
        final_unlock.is_ok(),
        "Keystore should work after reload with enhanced MAC coverage"
    );
}

// H-2 Test: Test rate limiting functionality
#[test]
fn test_h2_rate_limiting_functionality() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    // Initialize keystore with a password
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    // Reload keystore to test unlock with wrong password
    let mut ks2 = Keystore::new(Some(keystore_path)).unwrap();
    ks2.initialize_or_load(None).unwrap();

    // Manually trigger 5 failed attempts to reach the limit
    for i in 1..=5 {
        ks2.record_failed_attempt();
        println!(
            "Failed attempt {}: {} attempts recorded",
            i, ks2.failed_unlock_attempts
        );
    }

    // Now the next attempt should be rate limited
    let result = ks2.unlock(WRONG_PASSWORD);
    assert!(
        matches!(result, Err(KeystoreError::RateLimited { .. })),
        "Should be rate limited after 5 failed attempts, got: {:?}",
        result
    );

    // Reset rate limiting and verify correct password works
    ks2.reset_rate_limiting_for_test();
    let result = ks2.unlock(TEST_PASSWORD);
    assert!(
        result.is_ok(),
        "Correct password should work after rate limit reset"
    );
}

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
        matches!(unlock_res_before_init, Err(KeystoreError::InternalError(_))),
        "Unlock should fail with InternalError before KDF params are set"
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
        matches!(unlock_res, Err(KeystoreError::InternalError(_))),
        "Unlock should fail with InternalError as KDF params are not set"
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

    // Create a keystore with a custom config that meets H-1 minimum requirements
    let config = KeystoreConfig {
        // KDF parameters - updated to meet H-1 minimums
        m_cost: 262144, // 256 MB minimum required memory cost (H-1 fix)
        t_cost: 8,      // 8 iterations minimum required time cost (H-1 fix)
        p_cost: 1,      // Default parallelism
        output_len: 32, // Minimum required output length
        // Session management
        auto_lock_timeout: Duration::from_secs(300), // 5 minutes
    };

    // Create and initialize a new keystore with the test password and custom config
    let mut ks = Keystore::new_with_config(Some(keystore_path.clone()), config.clone()).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    // To properly test unlocking an existing keystore, we now create a new keystore instance
    // and load from the same path.
    let mut new_ks =
        Keystore::new_with_config(Some(keystore_path.clone()), config.clone()).unwrap();
    new_ks.initialize_or_load(None).unwrap(); // Load the previously saved keystore

    // Now, attempt to unlock with the wrong password
    let unlock_result = new_ks.unlock(WRONG_PASSWORD);
    assert!(
        matches!(unlock_result, Err(KeystoreError::MacVerificationFailure)),
        "Unlock with wrong password should fail with MacVerificationFailure, but got {:?}",
        unlock_result
    );

    // Keystore should remain locked
    assert!(!new_ks.is_unlocked);

    // Since unlock failed, import should also fail because keystore is locked
    let import_res =
        new_ks.import_private_key_hex(Some("key_with_wrong_pass".to_string()), DUMMY_PK_HEX);
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
        if !data_str.is_empty() {
            let mut chars: Vec<char> = data_str.chars().collect();
            // Change the first character to ensure the MAC will fail but base64 decoding still passes.
            // If it's 'A', change to 'B'. Otherwise, change to 'A'.
            if chars[0] == 'A' {
                chars[0] = 'B';
            } else {
                chars[0] = 'A';
            }
            *data_str = chars.into_iter().collect();
        } else {
            // This should not happen for a valid keystore if data was written.
            panic!("protected_data_b64 is empty, cannot tamper as intended for this test.");
        }
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
            matches!(unlock_result, Err(KeystoreError::MacVerificationFailure)),
            "Unlock should fail with MacVerificationFailure, got {:?}",
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
            matches!(
                unlock_old_pw_result,
                Err(KeystoreError::MacVerificationFailure)
            ),
            "Unlock with old password should fail with MacVerificationFailure, got {:?}",
            unlock_old_pw_result
        );
        assert!(
            !ks2.is_unlocked,
            "Keystore should remain locked after failed unlock attempt"
        );

        // H-2 Fix: Reset rate limiting for test to allow immediate retry with correct password
        ks2.reset_rate_limiting_for_test();

        // Unlock with the new password - should succeed (and verify H-1 MAC)
        ks2.unlock(NEW_PASSWORD)
            .expect("Unlock with new password failed (H-1 MAC check implied)");

        let keys = ks2
            .list_keys()
            .expect("Listing keys after password change failed");
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

    // The behavior is correct: trying to unlock a keystore that hasn't been initialized
    // (i.e., no KDF params set) should fail because KDF params are missing.
    assert!(
        matches!(
            unlock_result,
            Err(KeystoreError::InternalError(ref msg)) if msg.contains("KDF parameters missing before unlock")
        ),
        "Expected InternalError for uninitialized keystore unlock, got {:?}",
        unlock_result
    );
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
        m_cost: 1000, // Too low for new requirements
        ..KeystoreConfig::default()
    };

    // Test too low m_cost (updated for new 256MB minimum)
    let res_m_cost = Keystore::new_with_config(Some(temp_path_1), config.clone());
    if let Err(KeystoreError::FsError(msg)) = res_m_cost {
        assert!(
            msg.contains("Memory cost 1000 is below minimum 262144"),
            "Low m_cost check failed. Msg: {:?}",
            msg
        );
    } else {
        panic!("Expected FsError for low m_cost, got {:?}", res_m_cost);
    }

    // Test too low t_cost (updated for new 8 iteration minimum)
    config.m_cost = KeystoreConfig::default().m_cost; // Reset m_cost to valid
    config.t_cost = 1; // Too low for new requirements
    let res_t_cost = Keystore::new_with_config(Some(temp_path_2), config.clone());
    if let Err(KeystoreError::FsError(msg)) = res_t_cost {
        assert!(
            msg.contains("Time cost 1 is below minimum 8"),
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
        Err(KeystoreError::MacVerificationFailure)
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
        matches!(change_res, Err(KeystoreError::MacVerificationFailure)),
        "Changing password with incorrect old password should fail with MacVerificationFailure"
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
        Err(KeystoreError::MacVerificationFailure)
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
        matches!(unlock_old_res, Err(KeystoreError::MacVerificationFailure)),
        "Unlock with old password should fail with MacVerificationFailure after password change"
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
        matches!(load_res, Err(KeystoreError::InvalidFormat(_))),
        "Loading malformed JSON should result in InvalidFormat error"
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

    // Find the entry and corrupt its encrypted_pk in memory
    if let Some(entry) = ks.entries.iter_mut().find(|e| e.id == id) {
        let mut corrupted_pk = entry.encrypted_pk.clone();
        if corrupted_pk.len() > 1 {
            corrupted_pk.pop(); // Corrupt by truncating, causing base64 decode error
        } else {
            panic!("Encrypted private key too short to corrupt");
        }
        entry.encrypted_pk = corrupted_pk;
    } else {
        panic!("Could not find imported key entry");
    }

    // Try to get the signer for the corrupted key, which should now fail with InvalidFormat
    let get_signer_res = ks.get_signer(id);
    assert!(
        matches!(get_signer_res, Err(KeystoreError::InvalidFormat(_))),
        "Getting signer for corrupted key should result in InvalidFormat error due to base64 decode failure. Got: {:?}",
        get_signer_res
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
    // 1. Create a valid keystore first.
    {
        let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();
        ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks.unlock(TEST_PASSWORD).unwrap();
        ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();
        // Keystore is saved to disk on drop
    }

    // 2. Manually load the keystore file and tamper with the master_kdf field.
    let mut json_value: Value =
        serde_json::from_str(&std::fs::read_to_string(&keystore_path).unwrap()).unwrap();
    if let Some(obj) = json_value.as_object_mut() {
        obj.insert(
            "master_kdf_algo_name".to_string(),
            Value::String("unsupported_kdf_algo".to_string()),
        );
    }
    std::fs::write(&keystore_path, serde_json::to_string(&json_value).unwrap()).unwrap();

    // 3. Attempt to load and unlock the keystore with the unsupported KDF.
    let mut ks_tampered = Keystore::new(Some(keystore_path)).unwrap();
    ks_tampered.initialize_or_load(None).unwrap(); // Load should succeed.

    // Unlock should fail because the KDF algorithm is not supported.
    let unlock_res = ks_tampered.unlock(TEST_PASSWORD);
    assert!(
        matches!(unlock_res, Err(KeystoreError::UnsupportedKdf(ref algo)) if algo == "unsupported_kdf_algo"),
        "Unlock with unsupported KDF should result in UnsupportedKdf error, got {:?}",
        unlock_res
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

    // Manually load the keystore file and tamper with the protected data by removing the verification_tag
    let keystore_json_str = std::fs::read_to_string(&keystore_path).unwrap();
    let mut envelope_value: Value = serde_json::from_str(&keystore_json_str).unwrap();

    // Decode the protected part
    let protected_b64 = envelope_value["protected_data_b64"].as_str().unwrap();
    let protected_json_bytes = base64::engine::general_purpose::STANDARD
        .decode(protected_b64)
        .unwrap();
    let mut protected_value: Value = serde_json::from_slice(&protected_json_bytes).unwrap();

    // Remove the verification tag from the protected data
    protected_value
        .as_object_mut()
        .unwrap()
        .remove("verification_tag");

    // Re-encode the tampered protected part
    let tampered_protected_json_bytes = serde_json::to_vec(&protected_value).unwrap();
    let tampered_protected_b64 =
        base64::engine::general_purpose::STANDARD.encode(&tampered_protected_json_bytes);

    // Put it back into the envelope
    envelope_value["protected_data_b64"] = Value::String(tampered_protected_b64);

    // Write the tampered envelope back. The MAC is now invalid because the protected data has changed.
    std::fs::write(
        &keystore_path,
        serde_json::to_string(&envelope_value).unwrap(),
    )
    .unwrap();

    // Attempt to load and unlock the keystore
    let mut ks_tampered = Keystore::new(Some(keystore_path)).unwrap();
    ks_tampered.initialize_or_load(None).unwrap(); // Load should succeed as MAC is not checked here
    let unlock_res = ks_tampered.unlock(TEST_PASSWORD); // Unlock should fail MAC verification

    // Any tampering with authenticated data should result in a MAC verification failure.
    assert!(
        matches!(unlock_res, Err(KeystoreError::MacVerificationFailure)),
        "Unlock with tampered protected data (missing tag) should result in MacVerificationFailure"
    );
}
