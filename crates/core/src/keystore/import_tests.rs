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
    let address = secure_key.ethereum_address().unwrap();
    assert_ne!(address, Address::ZERO);

    // Test public key derivation
    let public_key = secure_key.public_key().unwrap();
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

#[cfg(feature = "dangerous-secret-export")]
#[test]
fn test_export_private_key_covers_raw_and_hd_derived_entries() {
    enum Case {
        RawPrivateKey,
        HdDerivedMnemonic,
    }

    for case in [Case::RawPrivateKey, Case::HdDerivedMnemonic] {
        let (_temp_dir, mut keystore) = test_keystore_with_exports();
        keystore.unlock("test_password").unwrap();

        match case {
            Case::RawPrivateKey => {
                let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
                let id = keystore
                    .import_private_key(Some("export_pk".to_string()), test_key)
                    .unwrap();

                let exported = keystore.export_private_key(id).unwrap();
                assert_eq!(exported.as_str(), format!("0x{test_key}"));
            }
            Case::HdDerivedMnemonic => {
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

                let exported_pk = keystore.export_private_key(mnemonic_id).unwrap();
                let derived_id = keystore
                    .import_private_key(Some("derived".to_string()), exported_pk.as_str())
                    .unwrap();

                let key_from_mnemonic = keystore.get_private_key(mnemonic_id).unwrap();
                let key_from_exported = keystore.get_private_key(derived_id).unwrap();
                assert_eq!(
                    key_from_mnemonic.ethereum_address().unwrap(),
                    key_from_exported.ethereum_address().unwrap()
                );
            }
        }
    }
}

#[cfg(feature = "dangerous-secret-export")]
#[test]
fn test_secret_exports_disabled_by_default() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let id = keystore
        .import_private_key(
            Some("no_export".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000005",
        )
        .unwrap();

    assert!(matches!(
        keystore.export_private_key(id),
        Err(KeystoreError::OperationNotPermitted(_))
    ));
}

#[cfg(not(feature = "dangerous-secret-export"))]
#[test]
fn test_secret_export_feature_is_disabled_by_default() {
    const { assert!(!cfg!(feature = "dangerous-secret-export")) };
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
        Err(KeystoreError::InvalidInput(_))
    ));

    // New password should succeed and keys should still be accessible.
    keystore2.unlock("new_password").unwrap();
    assert!(keystore2.get_private_key(pk_id).is_ok());
    assert!(keystore2.get_private_key(mnemonic_id).is_ok());
}
