// use crate::keystore::KeystoreConfig; // Removed as unused
// use std::time::Duration; // No longer used

use super::error::KeystoreError;
use super::Keystore;
use alloy_primitives::{Signature as AlloySignature, B256, U256}; // Added U256 back
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
fn test_initialize_and_load_empty_keystore() {
    // Renamed
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    // initialize_or_load no longer takes a password.
    // It's expected not to create a file if one doesn't exist.
    ks.initialize_or_load().expect("Initialization/load failed");
    assert!(
        !keystore_path.exists(),
        "Keystore file should NOT be created if it doesn't exist on load"
    );

    // No global lock state, so import should succeed if a password is provided for the key.
    let import_res = ks.import_private_key_hex(None, DUMMY_PK_HEX, TEST_PASSWORD);
    assert!(
        import_res.is_ok(),
        "Import should succeed with a password for the key"
    );
    // After import, the file should exist.
    assert!(
        keystore_path.exists(),
        "Keystore file should be created after first import"
    );
}

#[test]
fn test_load_non_existent_keystore() {
    // Renamed
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();

    // initialize_or_load no longer takes a password.
    ks.initialize_or_load()
        .expect("Loading non-existent keystore failed");

    assert!(
        !keystore_path.exists(),
        "Keystore file should not be created when loading a non-existent keystore"
    );

    // No global KDF params or unlock state to check.
}

// OBSOLETE: test_unlock_lock_cycle removed due to removal of global lock/unlock.

// OBSOLETE: test_unlock_with_wrong_password_on_existing_keystore removed.
// Per-entry password failures will be tested differently.

#[test]
fn test_load_existing_keystore() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let original_key_alias = "key_to_load".to_string();
    let original_key_id;
    {
        let mut ks_orig = Keystore::new(Some(keystore_path.clone())).unwrap();
        // initialize_or_load no longer takes a password.
        // For a new keystore, it won't create a file.
        ks_orig.initialize_or_load().unwrap();
        // Import requires a password.
        let (id, _) = ks_orig
            .import_private_key_hex(
                Some(original_key_alias.clone()),
                DUMMY_PK_HEX,
                TEST_PASSWORD,
            )
            .unwrap();
        original_key_id = id;
    } // ks_orig is dropped, data saved to disk.

    let mut ks_loaded = Keystore::new(Some(keystore_path)).unwrap();
    // initialize_or_load no longer takes a password, it just loads.
    ks_loaded
        .initialize_or_load()
        .expect("Loading existing keystore failed");

    // Attempting to import a new key requires a password for that new key.
    // This does not test if the keystore is "locked" in the old sense.
    let import_res_new_key = ks_loaded.import_private_key_hex(None, DUMMY_PK_HEX_2, "new_password");
    assert!(
        import_res_new_key.is_ok(),
        "Importing a new key should succeed with its own password"
    );

    // Listing keys does not require a password.
    let keys = ks_loaded.list_keys().unwrap();
    assert_eq!(
        keys.len(),
        2,
        "Should have two keys after loading and importing another"
    );
    assert!(keys
        .iter()
        .any(|k| k.id == original_key_id && k.alias == Some(original_key_alias.clone())));

    // To get the signer for the original key, its password must be provided.
    let signer_res = ks_loaded.get_signer(original_key_id, TEST_PASSWORD);
    assert!(
        signer_res.is_ok(),
        "Getting signer for original key should succeed with correct password"
    );

    let signer_res_wrong_pass = ks_loaded.get_signer(original_key_id, WRONG_PASSWORD);
    assert!(
        matches!(
            signer_res_wrong_pass,
            Err(KeystoreError::Argon2Error(_)) | Err(KeystoreError::InvalidPassword)
        ),
        "Getting signer with wrong password should fail."
    );
}

// OBSOLETE: test_unlock_uninitialized_keystore removed.
// Global initialization and unlock concepts have changed.

// --- Tests for import_private_key_hex ---
#[test]
fn test_import_private_key_hex_success() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock

    let (id1, addr1) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX, TEST_PASSWORD) // Add password
        .expect("Failed to import PK hex");
    assert!(!id1.is_nil());

    let (id2, addr2) = ks
        .import_private_key_hex(Some("alias2".to_string()), DUMMY_PK_HEX_2, TEST_PASSWORD) // Add password
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
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock

    ks.import_private_key_hex(Some("mykey".to_string()), DUMMY_PK_HEX, TEST_PASSWORD) // Add password
        .unwrap();
    let result =
        ks.import_private_key_hex(Some("mykey".to_string()), DUMMY_PK_HEX_2, TEST_PASSWORD); // Add password
    assert!(matches!(result, Err(KeystoreError::AliasExists(alias)) if alias == "mykey"));
}

#[test]
fn test_import_private_key_hex_invalid_key_format() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock

    let invalid_hex = "not-a-hex-string";
    assert!(matches!(
        ks.import_private_key_hex(None, invalid_hex, TEST_PASSWORD), // Add password
        Err(KeystoreError::Hex(_))
    ));

    let short_hex = "010203";
    assert!(matches!(
        ks.import_private_key_hex(None, short_hex, TEST_PASSWORD), // Add password
        Err(KeystoreError::InvalidPrivateKey)
    ));

    let empty_hex = "";
    assert!(matches!(
        ks.import_private_key_hex(None, empty_hex, TEST_PASSWORD), // Add password
        Err(KeystoreError::InvalidPrivateKey)
    ));
}

// OBSOLETE: test_import_private_key_hex_keystore_locked removed.
// No global lock to test against for import.

// --- Tests for import_mnemonic ---
const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const TEST_DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";
const TEST_MNEMONIC_2_VALID: &str = "test test test test test test test test test test test junk"; // Known valid mnemonic

#[test]
fn test_import_mnemonic_success() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock

    let (id1, addr1) = ks
        .import_mnemonic(
            None,
            TEST_MNEMONIC,
            None,
            TEST_DERIVATION_PATH,
            TEST_PASSWORD,
        ) // Add password
        .expect("Failed to import mnemonic");
    assert!(!id1.is_nil());

    let (id2, addr2) = ks
        .import_mnemonic(
            Some("mnemonic_key".to_string()),
            TEST_MNEMONIC_2_VALID,
            Some("secret_passphrase"), // This is BIP39 passphrase, not entry password
            TEST_DERIVATION_PATH,
            TEST_PASSWORD, // Add entry password
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
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock

    ks.import_mnemonic(
        Some("my_mnemonic".to_string()),
        TEST_MNEMONIC,
        None,
        TEST_DERIVATION_PATH,
        TEST_PASSWORD, // Add password
    )
    .unwrap();
    let result = ks.import_mnemonic(
        Some("my_mnemonic".to_string()),
        TEST_MNEMONIC_2_VALID,
        None,
        TEST_DERIVATION_PATH,
        TEST_PASSWORD, // Add password
    );
    assert!(matches!(result, Err(KeystoreError::AliasExists(alias)) if alias == "my_mnemonic"));
}

#[test]
fn test_import_mnemonic_invalid_phrase() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock

    let invalid_phrase = "this is not a valid mnemonic phrase definitely";
    assert!(matches!(
        ks.import_mnemonic(
            None,
            invalid_phrase,
            None,
            TEST_DERIVATION_PATH,
            TEST_PASSWORD
        ), // Add password
        Err(KeystoreError::Bip39(_))
    ));
}

#[test]
fn test_import_mnemonic_invalid_path() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock

    let invalid_path = "m/44'/60a/0'/0/0"; // Invalid character 'a'
    assert!(matches!(
        ks.import_mnemonic(None, TEST_MNEMONIC, None, invalid_path, TEST_PASSWORD), // Add password
        Err(KeystoreError::InvalidPath(_))
    ));
}

// OBSOLETE: test_import_mnemonic_keystore_locked removed.
// No global lock to test against for import.

// --- Tests for list_keys and delete_key ---
#[test]
fn test_list_and_delete_keys() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock

    assert!(
        ks.list_keys().unwrap().is_empty(),
        "Keystore should be empty initially"
    );

    let (id1, _) = ks
        .import_private_key_hex(Some("key1".to_string()), DUMMY_PK_HEX, TEST_PASSWORD) // Add password
        .unwrap();
    let (id2, _) = ks
        .import_mnemonic(
            Some("key2".to_string()),
            TEST_MNEMONIC,
            None, // BIP39 passphrase
            TEST_DERIVATION_PATH,
            TEST_PASSWORD, // Entry password
        )
        .unwrap();
    let (id3, _) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX_2, TEST_PASSWORD)
        .unwrap(); // Add password

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

    // ks.lock(); // Removed
    // assert!(matches!(ks.delete_key(id2), Err(KeystoreError::Locked))); // Removed
    // ks.unlock(TEST_PASSWORD).unwrap(); // Global unlock removed

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
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock
    let (id, _) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX, TEST_PASSWORD)
        .unwrap(); // Pass password for import

    let signer = ks.get_signer(id, TEST_PASSWORD); // Pass password for get_signer
    assert!(signer.is_ok());
}

#[test]
fn test_get_signer_key_not_found() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock
    let non_existent_id = uuid::Uuid::new_v4();

    assert!(matches!(
        ks.get_signer(non_existent_id, TEST_PASSWORD), // Pass password for get_signer
        Err(KeystoreError::KeyNotFound(_))
    ));
}

// OBSOLETE: test_get_signer_locked removed.
// No global lock.

// OBSOLETE: test_get_signer_with_wrong_password_unlock removed.
// Global unlock concept is gone.

// --- Tests for verify_signature ---
#[test]
fn test_verify_signature_success_and_failure() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // No password for init
                                      // No global unlock
    let (id, _) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX, TEST_PASSWORD)
        .unwrap(); // Pass password for import

    let zeroizing_signing_key = ks.get_signer(id.clone(), TEST_PASSWORD).unwrap(); // Pass password for get_signer
                                                                                   // Need to clone the inner SigningKey to pass by value to PrivateKeySigner::from
    let signing_key = zeroizing_signing_key.as_ref().clone();
    let wallet = PrivateKeySigner::from(signing_key);

    let message_hash = B256::from_slice(&[42u8; 32]);
    let alloy_signature = wallet
        .sign_hash_sync(&message_hash)
        .expect("Signing failed");

    assert!(
        ks.verify_signature(id, TEST_PASSWORD, message_hash, alloy_signature.clone()) // Pass password for verify
            .unwrap(),
        "Signature verification should succeed"
    );

    let wrong_alloy_sig = AlloySignature::new(
        U256::from(12345), // Changed R value
        alloy_signature.s(),
        alloy_signature.v(),
    );
    assert!(
        !ks.verify_signature(id, TEST_PASSWORD, message_hash, wrong_alloy_sig) // Pass password
            .unwrap(),
        "Verification with wrong R should fail"
    );

    let wrong_hash = B256::from_slice(&[11u8; 32]);
    assert!(
        !ks.verify_signature(id, TEST_PASSWORD, wrong_hash, alloy_signature) // Pass password
            .unwrap(),
        "Verification with wrong hash should fail"
    );
}

#[test]
fn test_verify_signature_key_not_found() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap(); // initialize_or_load takes no args
                                      // ks.unlock(TEST_PASSWORD).unwrap(); // unlock removed

    let non_existent_id = uuid::Uuid::new_v4();
    let message_hash = B256::ZERO;
    let dummy_sig = AlloySignature::test_signature();

    assert!(matches!(
        ks.verify_signature(non_existent_id, TEST_PASSWORD, message_hash, dummy_sig), // Added password
        Err(KeystoreError::KeyNotFound(_))
    ));
}

#[test]
fn test_verify_signature_wrong_password_for_key() {
    // Renamed and logic corrected
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap();
    let (id, _) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX, TEST_PASSWORD)
        .unwrap();

    let message_hash = B256::ZERO;
    let dummy_sig = AlloySignature::test_signature();

    // Attempt to verify signature using the WRONG password for the key
    let verify_res = ks.verify_signature(id, WRONG_PASSWORD, message_hash, dummy_sig);
    assert!(
        matches!(verify_res, Err(KeystoreError::Argon2Error(_)) | Err(KeystoreError::InvalidPassword)),
        "Expected Argon2Error or InvalidPassword when verifying signature with wrong key password, got {:?}", verify_res,
    );
}

// OBSOLETE: test_verify_signature_locked removed.
// No global lock.

// --- New Tests for Per-Entry Password Error Handling and Change Password ---

#[test]
fn test_get_signer_wrong_password() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap();
    let (id, _) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX, TEST_PASSWORD)
        .unwrap();

    let signer_res = ks.get_signer(id, WRONG_PASSWORD);
    assert!(
        matches!(
            signer_res,
            Err(KeystoreError::Argon2Error(_)) | Err(KeystoreError::InvalidPassword)
        ),
        "Expected Argon2Error or InvalidPassword when getting signer with wrong password, got {:?}",
        signer_res
    );
}

#[test]
fn test_change_password_success() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap();
    let (id, _) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX, TEST_PASSWORD)
        .unwrap();

    let new_password = "new_super_secret_password";
    let change_res = ks.change_password(id, TEST_PASSWORD, new_password);
    assert!(
        change_res.is_ok(),
        "Changing password should succeed with correct old password"
    );

    // Try with old password - should fail
    let signer_old_pass = ks.get_signer(id, TEST_PASSWORD);
    assert!(
        matches!(
            signer_old_pass,
            Err(KeystoreError::Argon2Error(_)) | Err(KeystoreError::InvalidPassword)
        ),
        "Getting signer with old password should fail after change, got {:?}",
        signer_old_pass
    );

    // Try with new password - should succeed
    let signer_new_pass = ks.get_signer(id, new_password);
    assert!(
        signer_new_pass.is_ok(),
        "Getting signer with new password should succeed, got {:?}",
        signer_new_pass
    );
}

#[test]
fn test_change_password_wrong_old_password() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap();
    let (id, _) = ks
        .import_private_key_hex(None, DUMMY_PK_HEX, TEST_PASSWORD)
        .unwrap();

    let new_password = "another_new_password";
    let change_res = ks.change_password(id, WRONG_PASSWORD, new_password);
    assert!(
        matches!(
            change_res,
            Err(KeystoreError::Argon2Error(_)) | Err(KeystoreError::InvalidPassword)
        ),
        "Changing password with wrong old password should fail, got {:?}",
        change_res
    );

    // Key should still be accessible with the original password
    let signer_original_pass = ks.get_signer(id, TEST_PASSWORD);
    assert!(
        signer_original_pass.is_ok(),
        "Getting signer with original password should still succeed, got {:?}",
        signer_original_pass
    );
}

#[test]
fn test_entry_password_isolation() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load().unwrap();

    let password_1 = "password_for_key_1";
    let password_2 = "password_for_key_2";

    let (id1, _) = ks
        .import_private_key_hex(Some("key1".to_string()), DUMMY_PK_HEX, password_1)
        .unwrap();
    let (id2, _) = ks
        .import_private_key_hex(Some("key2".to_string()), DUMMY_PK_HEX_2, password_2)
        .unwrap();

    // Attempt to get signer for key1 using password_2
    let signer1_wrong_pass = ks.get_signer(id1, password_2);
    assert!(
        matches!(
            signer1_wrong_pass,
            Err(KeystoreError::Argon2Error(_)) | Err(KeystoreError::InvalidPassword)
        ),
        "Signer for key1 with key2's password should fail, got {:?}",
        signer1_wrong_pass
    );

    // Attempt to get signer for key2 using password_1
    let signer2_wrong_pass = ks.get_signer(id2, password_1);
    assert!(
        matches!(
            signer2_wrong_pass,
            Err(KeystoreError::Argon2Error(_)) | Err(KeystoreError::InvalidPassword)
        ),
        "Signer for key2 with key1's password should fail, got {:?}",
        signer2_wrong_pass
    );

    // Successfully get signer for key1 with password_1
    let signer1_correct_pass = ks.get_signer(id1, password_1);
    assert!(
        signer1_correct_pass.is_ok(),
        "Signer for key1 with correct password should succeed, got {:?}",
        signer1_correct_pass
    );

    // Successfully get signer for key2 with password_2
    let signer2_correct_pass = ks.get_signer(id2, password_2);
    assert!(
        signer2_correct_pass.is_ok(),
        "Signer for key2 with correct password should succeed, got {:?}",
        signer2_correct_pass
    );
}
