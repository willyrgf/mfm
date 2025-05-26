use crate::keystore::KeystoreConfig;
use std::time::Duration;

use super::error::KeystoreError;
use super::Keystore;
use alloy_primitives::{Signature as AlloySignature, B256, U256}; // Removed Address
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
// use k256::ecdsa::signature::hazmat::PrehashSigner; // Not directly used in tests
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

    // Create a keystore with a custom config that has minimal rate limiting for testing
    let config = KeystoreConfig {
        // KDF parameters
        m_cost: 4096,   // Lower memory cost for faster tests
        t_cost: 1,      // Lower time cost for faster tests
        p_cost: 1,      // Default parallelism
        output_len: 32, // Minimum required output length
        // Session management
        auto_lock_timeout: Duration::from_secs(300), // 5 minutes
        // Rate limiting with minimal delays for testing
        unlock_min_delay: Duration::from_millis(1),
        unlock_max_attempts: 10,
        unlock_backoff_factor: 1.0,
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
fn test_persisted_rate_limiting_across_instances() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let fast_kdf_config = KeystoreConfig {
        m_cost: 4096, // Low for speed
        t_cost: 1,    // Low for speed
        p_cost: 1,
        output_len: 32,
        unlock_min_delay: Duration::from_millis(100), // Slightly faster for test
        unlock_max_attempts: 3,                       // Lower attempts to trigger faster
        unlock_backoff_factor: 1.5,
        ..Default::default()
    };

    // --- KS1: Trigger rate limiting ---
    {
        let mut ks1 =
            Keystore::new_with_config(Some(keystore_path.clone()), fast_kdf_config.clone())
                .unwrap();
        ks1.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
        ks1.unlock(TEST_PASSWORD).unwrap(); // Initial successful unlock
        ks1.lock();

        for i in 0..fast_kdf_config.unlock_max_attempts {
            let res = ks1.unlock(WRONG_PASSWORD);
            assert!(
                matches!(res, Err(KeystoreError::InvalidPassword)),
                "Attempt {} should be InvalidPassword",
                i + 1
            );
        }
        // ks1 is now rate limited
        let res_limit_ks1 = ks1.unlock(WRONG_PASSWORD);
        assert!(
            matches!(res_limit_ks1, Err(KeystoreError::FsError(msg)) if msg.contains("Too many unlock attempts")),
            "ks1 should be rate limited"
        );
    } // ks1 is dropped, its persisted rate limit state should remain

    // --- KS2: Should be rate limited by ks1's state ---
    {
        let mut ks2 =
            Keystore::new_with_config(Some(keystore_path.clone()), fast_kdf_config.clone())
                .unwrap();
        // Must load existing state for rate limiting to be checked based on persisted files
        ks2.initialize_or_load(None)
            .expect("Loading existing keystore for ks2 failed");

        // Attempt with wrong password - should hit persisted rate limit
        let res_limit_ks2_wrong_pass = ks2.unlock(WRONG_PASSWORD);
        assert!(
            matches!(res_limit_ks2_wrong_pass, Err(KeystoreError::FsError(msg)) if msg.contains("Too many unlock attempts")),
            "ks2 unlock with WRONG password should be immediately rate limited due to ks1's state"
        );

        // Attempt with correct password - should also hit persisted rate limit
        let res_limit_ks2_correct_pass = ks2.unlock(TEST_PASSWORD);
        assert!(
            matches!(res_limit_ks2_correct_pass, Err(KeystoreError::FsError(msg)) if msg.contains("Too many unlock attempts")),
            "ks2 unlock with CORRECT password should also be rate limited"
        );

        // Wait for backoff duration
        // Max attempts is 3. Backoff delay will be unlock_min_delay * factor^2 for the 3rd failed attempt.
        // Then for the 4th attempt (which res_limit_ks1 was), it's max backoff (30s by default, but here it's based on calculate_backoff_delay logic)
        // The `calculate_backoff_delay` uses `unlock_max_attempts` for its ceiling.
        // If attempts >= unlock_max_attempts, it's 30s.
        // Here, unlock_max_attempts = 3. The 3rd failed attempt on ks1 sets the timestamp.
        // The check on ks2 happens. Attempts count is 3. So, delay is 30s (default max).
        // Let's use a more controlled delay based on our config.
        // After 3 failed attempts, the 4th attempt (which is what ks2 experiences) will check against the 3rd attempt's state.
        let required_delay = fast_kdf_config.unlock_min_delay.mul_f32(
            fast_kdf_config
                .unlock_backoff_factor
                .powi(fast_kdf_config.unlock_max_attempts as i32 - 1),
        );
        // If unlock_max_attempts is 3, this is factor^2.
        // Or, more generally, use the `calculate_backoff_delay` logic for the current attempt count
        let (attempts, _last_ts) = ks2.read_persisted_unlock_state().unwrap();
        let actual_delay_needed = ks2.calculate_backoff_delay(attempts);
        std::thread::sleep(actual_delay_needed + Duration::from_millis(50)); // Add a small buffer

        // Attempt with correct password - should now succeed
        ks2.unlock(TEST_PASSWORD)
            .expect("ks2 unlock with correct password after wait should succeed");
        ks2.lock();

        // --- KS2: Fail once more to set a new rate limit state (1 attempt) ---
        let res_fail_once_ks2 = ks2.unlock(WRONG_PASSWORD);
        assert!(
            matches!(res_fail_once_ks2, Err(KeystoreError::InvalidPassword)),
            "ks2 single fail should be InvalidPassword"
        );
    } // ks2 is dropped. State: 1 failed attempt.

    // --- KS3: Should NOT be rate limited by ks2's single failed attempt if ks2 previously had a successful unlock ---
    // The successful unlock in ks2 should have reset attempts to 0.
    // The subsequent single failure in ks2 then sets attempts to 1.
    // So ks3 should not be rate-limited.
    {
        let mut ks3 =
            Keystore::new_with_config(Some(keystore_path.clone()), fast_kdf_config.clone())
                .unwrap();
        ks3.initialize_or_load(None)
            .expect("Loading existing keystore for ks3 failed");

        ks3.unlock(TEST_PASSWORD).expect(
            "ks3 unlock with correct password should succeed as ks2 reset the main lockout",
        );
    }
}

#[test]
fn test_argon2_param_minimums_new_with_config() {
    let (_temp_dir1, temp_path_1) = create_temp_keystore_path();
    let (_temp_dir2, temp_path_2) = create_temp_keystore_path();
    let (_temp_dir3, temp_path_3) = create_temp_keystore_path();

    let mut config = KeystoreConfig::default();

    // Test too low m_cost
    config.m_cost = 1000; // Way too low
    let res_m_cost = Keystore::new_with_config(Some(temp_path_1), config.clone());
    assert!(
        matches!(res_m_cost, Err(KeystoreError::FsError(msg)) if msg.contains("KDF m_cost must be at least 131072")),
        "Low m_cost check failed. Msg: {:?}",
        res_m_cost.err()
    );

    // Test too low t_cost
    config.m_cost = KeystoreConfig::default().m_cost; // Reset m_cost to valid
    config.t_cost = 1; // Too low
    let res_t_cost = Keystore::new_with_config(Some(temp_path_2), config.clone());
    assert!(
        matches!(res_t_cost, Err(KeystoreError::FsError(msg)) if msg.contains("KDF t_cost must be at least 4")),
        "Low t_cost check failed. Msg: {:?}",
        res_t_cost.err()
    );

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
    ks.lock(); // Lock before trying to get signer

    assert!(matches!(ks.get_signer(id), Err(KeystoreError::Locked)));
}

#[test]
fn test_get_signer_with_wrong_password_unlock() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();
    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();
    ks.lock();

    // Unlock with wrong password should now fail with InvalidPassword due to verification
    let unlock_result = ks.unlock(WRONG_PASSWORD);
    assert!(
        matches!(unlock_result, Err(KeystoreError::InvalidPassword)),
        "Unlock with wrong password should fail with InvalidPassword"
    );

    // Keystore should remain locked
    assert!(!ks.is_unlocked);

    // get_signer should fail because the keystore is still locked
    assert!(matches!(ks.get_signer(id), Err(KeystoreError::Locked)));
}

// --- Tests for verify_signature ---
#[test]
fn test_verify_signature_success_and_failure() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();
    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();

    let zeroizing_signing_key = ks.get_signer(id.clone()).unwrap();
    // Need to clone the inner SigningKey to pass by value to PrivateKeySigner::from
    let signing_key = zeroizing_signing_key.as_ref().clone();
    let wallet = PrivateKeySigner::from(signing_key);

    let message_hash = B256::from_slice(&[42u8; 32]);
    let alloy_signature = wallet
        .sign_hash_sync(&message_hash)
        .expect("Signing failed");

    assert!(
        ks.verify_signature(id, message_hash, alloy_signature.clone())
            .unwrap(),
        "Signature verification should succeed"
    );

    let wrong_alloy_sig = AlloySignature::new(
        // Use alias or full path
        U256::from(12345),
        alloy_signature.s(),
        alloy_signature.v(),
    );
    assert!(
        !ks.verify_signature(id, message_hash, wrong_alloy_sig)
            .unwrap(),
        "Verification with wrong R should fail"
    );

    let wrong_hash = B256::from_slice(&[11u8; 32]);
    assert!(
        !ks.verify_signature(id, wrong_hash, alloy_signature)
            .unwrap(),
        "Verification with wrong hash should fail"
    );
}

#[test]
fn test_verify_signature_key_not_found() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();

    let non_existent_id = uuid::Uuid::new_v4();
    let message_hash = B256::ZERO;
    let dummy_sig = AlloySignature::test_signature();

    assert!(matches!(
        ks.verify_signature(non_existent_id, message_hash, dummy_sig),
        Err(KeystoreError::KeyNotFound(_))
    ));
}

#[test]
fn test_verify_signature_locked() {
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
