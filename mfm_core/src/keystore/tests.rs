#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    // Helper function to create a temporary keystore path
    fn temp_keystore_path() -> PathBuf {
        let dir = tempdir().unwrap();
        dir.path().join("test_keystore.json")
    }

    #[test]
    fn test_new_keystore() {
        let custom_path = temp_keystore_path();
        let keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        assert_eq!(keystore.file_path, custom_path);
        assert!(!keystore.is_unlocked);
        assert!(keystore.master_key.is_none());
        assert!(keystore.entries.is_empty());
        assert!(keystore.master_kdf_params.is_none());
        assert!(keystore.last_activity_at.is_none());
    }

    #[test]
    fn test_initialize_new_keystore() {
        let custom_path = temp_keystore_path();
        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        let password = "test_password";

        keystore.initialize_or_load(Some(password)).unwrap();

        assert!(keystore.file_path.exists());
        assert!(keystore.is_unlocked);
        assert!(keystore.master_key.is_some());
        assert!(keystore.entries.is_empty());
        assert!(keystore.master_kdf_params.is_some());
        assert!(keystore.last_activity_at.is_some());

        // Verify file content
        let content = std::fs::read_to_string(&custom_path).unwrap();
        let keystore_content: KeystoreFileContent = serde_json::from_str(&content).unwrap();
        assert_eq!(keystore_content.version, "1.0.0");
        assert_eq!(keystore_content.master_kdf, "argon2id");
        assert!(keystore_content.master_kdf_params.salt.len() > 0);
        assert_eq!(keystore_content.entries.len(), 0);
    }

    #[test]
    fn test_load_existing_keystore() {
        let custom_path = temp_keystore_path();
        let password = "test_password";

        // Initialize and save a keystore first
        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();
        keystore.lock(); // Lock it before loading

        // Load the existing keystore
        let mut loaded_keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        loaded_keystore.initialize_or_load(None).unwrap(); // Load without unlocking

        assert!(loaded_keystore.file_path.exists());
        assert!(!loaded_keystore.is_unlocked);
        assert!(loaded_keystore.master_key.is_none());
        assert!(loaded_keystore.entries.is_empty());
        assert!(loaded_keystore.master_kdf_params.is_some());
        assert!(loaded_keystore.last_activity_at.is_none());

        // Try loading and unlocking
        let mut loaded_unlocked_keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        loaded_unlocked_keystore
            .initialize_or_load(Some(password))
            .unwrap();
        assert!(loaded_unlocked_keystore.is_unlocked);
        assert!(loaded_unlocked_keystore.master_key.is_some());
    }

    #[test]
    fn test_unlock_and_lock() {
        let custom_path = temp_keystore_path();
        let password = "test_password";

        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();

        assert!(keystore.is_unlocked);
        assert!(keystore.master_key.is_some());

        keystore.lock();

        assert!(!keystore.is_unlocked);
        assert!(keystore.master_key.is_none());
        assert!(keystore.last_activity_at.is_none());

        // Try unlocking with wrong password
        let mut keystore_to_unlock = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore_to_unlock.initialize_or_load(None).unwrap(); // Load locked
        let unlock_result = keystore_to_unlock.unlock("wrong_password");
        assert!(unlock_result.is_err());
        if let Err(KeystoreError::Argon2(_)) = unlock_result {
            // Expected error type
        } else {
            panic!("Expected Argon2 error, but got {:?}", unlock_result);
        }
        assert!(!keystore_to_unlock.is_unlocked);
    }

    #[test]
    fn test_import_mnemonic() {
        let custom_path = temp_keystore_path();
        let password = "test_password";
        let mnemonic_phrase = "test test test test test test test test test test test junk";
        let derivation_path = "m/44'/60'/0'/0/0";

        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();

        let (uuid, address) = keystore
            .import_mnemonic(
                Some("test_key".to_string()),
                mnemonic_phrase,
                None,
                derivation_path,
            )
            .unwrap();

        assert!(!uuid.is_nil());
        assert!(!address.is_zero());

        // Verify the key is in the entries
        assert_eq!(keystore.entries.len(), 1);
        let entry = &keystore.entries[0];
        assert_eq!(entry.id, uuid);
        assert_eq!(entry.alias, Some("test_key".to_string()));
        assert_eq!(entry.address, address);

        // Verify the keystore file is updated
        let content = std::fs::read_to_string(&custom_path).unwrap();
        let keystore_content: KeystoreFileContent = serde_json::from_str(&content).unwrap();
        assert_eq!(keystore_content.entries.len(), 1);
        assert_eq!(keystore_content.entries[0].id, uuid);
    }

    #[test]
    fn test_import_private_key_hex() {
        let custom_path = temp_keystore_path();
        let password = "test_password";
        let private_key_hex = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"; // Example private key

        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();

        let (uuid, address) = keystore
            .import_private_key_hex(Some("hex_key".to_string()), private_key_hex)
            .unwrap();

        assert!(!uuid.is_nil());
        assert!(!address.is_zero());

        // Verify the key is in the entries
        assert_eq!(keystore.entries.len(), 1);
        let entry = &keystore.entries[0];
        assert_eq!(entry.id, uuid);
        assert_eq!(entry.alias, Some("hex_key".to_string()));
        assert_eq!(entry.address, address);

        // Verify the keystore file is updated
        let content = std::fs::read_to_string(&custom_path).unwrap();
        let keystore_content: KeystoreFileContent = serde_json::from_str(&content).unwrap();
        assert_eq!(keystore_content.entries.len(), 1);
        assert_eq!(keystore_content.entries[0].id, uuid);
    }

    #[test]
    fn test_import_duplicate_alias() {
        let custom_path = temp_keystore_path();
        let password = "test_password";
        let mnemonic_phrase = "test test test test test test test test test test test junk";
        let derivation_path = "m/44'/60'/0'/0/0";

        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();

        keystore
            .import_mnemonic(
                Some("duplicate_alias".to_string()),
                mnemonic_phrase,
                None,
                derivation_path,
            )
            .unwrap();

        // Try importing another key with the same alias
        let private_key_hex = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let import_result =
            keystore.import_private_key_hex(Some("duplicate_alias".to_string()), private_key_hex);

        assert!(import_result.is_err());
        if let Err(KeystoreError::AliasAlreadyExists(alias)) = import_result {
            assert_eq!(alias, "duplicate_alias");
        } else {
            panic!(
                "Expected AliasAlreadyExists error, but got {:?}",
                import_result
            );
        }

        // Verify no new entry was added
        assert_eq!(keystore.entries.len(), 1);
    }

    #[test]
    fn test_import_when_locked() {
        let custom_path = temp_keystore_path();
        let password = "test_password";
        let mnemonic_phrase = "test test test test test test test test test test test junk";
        let derivation_path = "m/44'/60'/0'/0/0";

        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();
        keystore.lock();

        let import_mnemonic_result = keystore.import_mnemonic(
            Some("test_key".to_string()),
            mnemonic_phrase,
            None,
            derivation_path,
        );
        assert!(import_mnemonic_result.is_err());
        if let Err(KeystoreError::Locked) = import_mnemonic_result {
            // Expected error type
        } else {
            panic!(
                "Expected Locked error, but got {:?}",
                import_mnemonic_result
            );
        }

        let private_key_hex = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let import_hex_result =
            keystore.import_private_key_hex(Some("hex_key".to_string()), private_key_hex);
        assert!(import_hex_result.is_err());
        if let Err(KeystoreError::Locked) = import_hex_result {
            // Expected error type
        } else {
            panic!("Expected Locked error, but got {:?}", import_hex_result);
        }
    }

    #[test]
    fn test_list_keys() {
        let custom_path = temp_keystore_path();
        let password = "test_password";
        let mnemonic_phrase = "test test test test test test test test test test test junk";
        let derivation_path = "m/44'/60'/0'/0/0";
        let private_key_hex = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();

        // List keys in an empty keystore
        let initial_keys = keystore.list_keys().unwrap();
        assert!(initial_keys.is_empty());

        // Import some keys
        let (uuid1, address1) = keystore
            .import_mnemonic(
                Some("mnemonic_key".to_string()),
                mnemonic_phrase,
                None,
                derivation_path,
            )
            .unwrap();
        let (uuid2, address2) = keystore
            .import_private_key_hex(Some("hex_key".to_string()), private_key_hex)
            .unwrap();

        // List keys after importing
        let keys = keystore.list_keys().unwrap();
        assert_eq!(keys.len(), 2);

        // Verify the listed keys
        let key1 = keys.iter().find(|k| k.id == uuid1).unwrap();
        assert_eq!(key1.alias, Some("mnemonic_key".to_string()));
        assert_eq!(key1.address, address1);

        let key2 = keys.iter().find(|k| k.id == uuid2).unwrap();
        assert_eq!(key2.alias, Some("hex_key".to_string()));
        assert_eq!(key2.address, address2);
    }

    #[test]
    fn test_get_signer() {
        let custom_path = temp_keystore_path();
        let password = "test_password";
        let private_key_hex = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();

        let (uuid, _) = keystore
            .import_private_key_hex(Some("test_key".to_string()), private_key_hex)
            .unwrap();

        // Get the signer
        let signer = keystore.get_signer(uuid).unwrap();

        // Verify the signer's address
        use alloy_signer::Signer;
        let expected_address: Address = private_key_hex.parse().unwrap(); // Derive address from hex
        assert_eq!(signer.address(), expected_address);

        // Test signing a message
        use alloy_primitives::keccak256;
        let message = b"test message";
        let message_hash = keccak256(message);
        let signature = signer.sign_hash(message_hash).unwrap();

        // Verify the signature (requires a verifier, which we don't have directly here, but we can at least ensure signing doesn't panic)
        println!("Signed message with signature: {:?}", signature);

        // Try getting a signer for a non-existent key
        let non_existent_uuid = Uuid::new_v4();
        let get_signer_result = keystore.get_signer(non_existent_uuid);
        assert!(get_signer_result.is_err());
        if let Err(KeystoreError::KeyNotFound(id)) = get_signer_result {
            assert_eq!(id, non_existent_uuid);
        } else {
            panic!(
                "Expected KeyNotFound error, but got {:?}",
                get_signer_result
            );
        }

        // Try getting a signer when locked
        keystore.lock();
        let get_signer_locked_result = keystore.get_signer(uuid);
        assert!(get_signer_locked_result.is_err());
        if let Err(KeystoreError::Locked) = get_signer_locked_result {
            // Expected error type
        } else {
            panic!(
                "Expected Locked error, but got {:?}",
                get_signer_locked_result
            );
        }
    }

    #[test]
    fn test_delete_key() {
        let custom_path = temp_keystore_path();
        let password = "test_password";
        let private_key_hex1 = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let private_key_hex2 = "0x1111111111111111111111111111111111111111111111111111111111111111";

        let mut keystore = Keystore::new(Some(custom_path.clone())).unwrap();
        keystore.initialize_or_load(Some(password)).unwrap();

        let (uuid1, _) = keystore
            .import_private_key_hex(Some("key1".to_string()), private_key_hex1)
            .unwrap();
        let (uuid2, _) = keystore
            .import_private_key_hex(Some("key2".to_string()), private_key_hex2)
            .unwrap();

        assert_eq!(keystore.entries.len(), 2);

        // Delete the first key
        keystore.delete_key(uuid1).unwrap();
        assert_eq!(keystore.entries.len(), 1);
        assert!(keystore
            .entries
            .iter()
            .find(|entry| entry.id == uuid1)
            .is_none());
        assert!(keystore
            .entries
            .iter()
            .find(|entry| entry.id == uuid2)
            .is_some());

        // Verify the keystore file is updated
        let content = std::fs::read_to_string(&custom_path).unwrap();
        let keystore_content: KeystoreFileContent = serde_json::from_str(&content).unwrap();
        assert_eq!(keystore_content.entries.len(), 1);
        assert!(keystore_content
            .entries
            .iter()
            .find(|entry| entry.id == uuid1)
            .is_none());
        assert!(keystore_content
            .entries
            .iter()
            .find(|entry| entry.id == uuid2)
            .is_some());

        // Delete the second key
        keystore.delete_key(uuid2).unwrap();
        assert_eq!(keystore.entries.len(), 0);
        assert!(keystore
            .entries
            .iter()
            .find(|entry| entry.id == uuid2)
            .is_none());

        // Verify the keystore file is updated
        let content = std::fs::read_to_string(&custom_path).unwrap();
        let keystore_content: KeystoreFileContent = serde_json::from_str(&content).unwrap();
        assert_eq!(keystore_content.entries.len(), 0);

        // Try deleting a non-existent key
        let non_existent_uuid = Uuid::new_v4();
        let delete_result = keystore.delete_key(non_existent_uuid);
        assert!(delete_result.is_err());
        if let Err(KeystoreError::KeyNotFound(id)) = delete_result {
            assert_eq!(id, non_existent_uuid);
        } else {
            panic!("Expected KeyNotFound error, but got {:?}", delete_result);
        }

        // Try deleting when locked
        keystore.lock();
        let delete_locked_result = keystore.delete_key(uuid1); // Use uuid1, it's not in entries but tests locked state
        assert!(delete_locked_result.is_err());
        if let Err(KeystoreError::Locked) = delete_locked_result {
            // Expected error type
        } else {
            panic!("Expected Locked error, but got {:?}", delete_locked_result);
        }
    }
}
