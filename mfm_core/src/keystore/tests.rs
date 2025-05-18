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
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path.clone())).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();

    ks.unlock(WRONG_PASSWORD)
        .expect("Unlock with wrong password should derive a (wrong) key without erroring here");

    let import_res =
        ks.import_private_key_hex(Some("key_with_wrong_pass".to_string()), DUMMY_PK_HEX);
    assert!(
        import_res.is_ok(),
        "Import with wrong key should appear to succeed as encryption uses this wrong key."
    );
    let (key_id_wrong_pass, _) = import_res.unwrap();

    ks.lock();
    ks.unlock(TEST_PASSWORD).unwrap();

    let get_signer_res = ks.get_signer(key_id_wrong_pass);
    assert!(
        matches!(get_signer_res, Err(KeystoreError::AesGcm(_))),
        "get_signer should fail with AesGcm error due to wrong master key used for encryption"
    );
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

    // Unlock with wrong password
    ks.unlock(WRONG_PASSWORD)
        .expect("Unlock with wrong password should derive a key");

    // get_signer should fail because the master key is wrong for decryption
    assert!(matches!(ks.get_signer(id), Err(KeystoreError::AesGcm(_))));
}

// --- Tests for verify_signature ---
#[test]
fn test_verify_signature_success_and_failure() {
    let (_temp_dir, keystore_path) = create_temp_keystore_path();
    let mut ks = Keystore::new(Some(keystore_path)).unwrap();
    ks.initialize_or_load(Some(TEST_PASSWORD)).unwrap();
    ks.unlock(TEST_PASSWORD).unwrap();
    let (id, _) = ks.import_private_key_hex(None, DUMMY_PK_HEX).unwrap();

    let signing_key_k256 = ks.get_signer(id.clone()).unwrap();
    // Now that get_signer returns SigningKey directly, we can use it without dereferencing
    let wallet = PrivateKeySigner::from(signing_key_k256);

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
