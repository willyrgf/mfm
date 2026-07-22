use super::*;

#[test]
fn test_new_keystore_creation() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("new.keystore");

    // Create new keystore
    let keystore = Keystore::new(&keystore_path).unwrap();

    // Verify initial state
    assert!(!keystore_path.exists()); // File not created until first unlock
    assert!(matches!(keystore.list_keys(), Err(KeystoreError::Locked)));
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
    let address1 = secure_key.ethereum_address().unwrap();
    let address2 = secure_key.ethereum_address().unwrap();
    assert_eq!(address1, address2);
    assert_eq!(
        format!("{address1:?}").to_lowercase(),
        "0x9858effd232b4033e47d90003d41ec34ecaeda94"
    );
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
        key_no_pass.ethereum_address().unwrap(),
        key_with_pass.ethereum_address().unwrap()
    );
}

#[test]
fn test_mnemonic_passphrase_not_persisted_in_plaintext() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("mnemonic_passphrase_secure.keystore");
    let mnemonic =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("test_password").unwrap();
    keystore
        .import_mnemonic(
            Some("mnemonic-secure".to_string()),
            mnemonic,
            "m/44'/60'/0'/0/0",
            Some("super-secret-passphrase"),
        )
        .unwrap();

    let file_content = std::fs::read_to_string(&keystore_path).unwrap();
    assert!(!file_content.contains(mnemonic));
    assert!(!file_content.contains("super-secret-passphrase"));
    assert!(!file_content.contains("\"mnemonic\""));
    assert!(!file_content.contains("\"passphrase\""));
}

#[test]
fn test_unsupported_mnemonic_entry_fails_closed_without_secret_error() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("unsupported_mnemonic_entry.keystore");
    let mnemonic =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let passphrase = "unsupported-secret-passphrase";

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    keystore
        .import_private_key(
            Some("unsupported".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    rewrite_keystore_json_with_valid_mac(&keystore, &keystore_path, |json| {
        json["entries"][0]["key_type"] = serde_json::json!({
            "Mnemonic": {
                "derivation_path": "m/44'/60'/0'/0/0"
            }
        });
    });
    drop(keystore);

    let mut loaded =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let err = loaded.unlock("strong_password_123").unwrap_err();
    let rendered = err.to_string();
    assert!(matches!(err, KeystoreError::InvalidInput(_)));
    assert!(!rendered.contains(mnemonic));
    assert!(!rendered.contains(passphrase));
}
