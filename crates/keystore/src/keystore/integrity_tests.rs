use super::*;

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
        KeystoreError::InvalidInput(_)
    ));
}

#[test]
fn test_v1_writer_and_unsupported_version_fail_closed_without_rewrite() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("version_test.keystore");
    let (keystore, _) = unlocked_keystore_with_one_key(&keystore_path, "version-test");
    assert_eq!(
        read_keystore_json(&keystore_path)["version"],
        serde_json::json!(1)
    );

    rewrite_keystore_json_with_valid_mac(&keystore, &keystore_path, |json| {
        json["version"] = serde_json::json!(u8::MAX);
        json["kdf_params"]["memory_kb"] = serde_json::json!(u32::MAX);
    });
    let unsupported_bytes = std::fs::read(&keystore_path).unwrap();
    let error = Keystore::new_with_config(&keystore_path, KeystoreConfig::development())
        .expect_err("unsupported version must fail before KDF validation");

    match error {
        KeystoreError::InvalidInput(message) => {
            assert!(message.contains("Unsupported keystore version"));
            assert!(!message.contains("KDF memory cost"));
        }
        other => panic!("expected unsupported version rejection, got: {other:?}"),
    }
    assert_eq!(std::fs::read(&keystore_path).unwrap(), unsupported_bytes);
}

#[test]
fn test_strict_loading_rejects_unknown_header_fields() {
    enum Case {
        TopLevel,
        Kdf,
    }

    for (case, file_name, expected_message) in [
        (
            Case::TopLevel,
            "unknown_top_level.keystore",
            "Unknown keystore file field",
        ),
        (
            Case::Kdf,
            "unknown_kdf.keystore",
            "Malformed keystore header",
        ),
    ] {
        let temp_dir = tempdir().unwrap();
        let keystore_path = temp_dir.path().join(file_name);

        {
            let mut keystore =
                Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
            keystore.unlock("strong_password_123").unwrap();
        }

        let mut json = read_keystore_json(&keystore_path);
        match case {
            Case::TopLevel => json["unexpected"] = serde_json::json!(true),
            Case::Kdf => json["kdf_params"]["unexpected"] = serde_json::json!(true),
        }
        write_keystore_json(&keystore_path, &json);

        let err =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap_err();
        match err {
            KeystoreError::InvalidInput(msg) => assert!(
                msg.contains(expected_message),
                "{file_name}: expected {expected_message:?}, got {msg:?}"
            ),
            other => panic!("{file_name}: expected unknown field rejection, got: {other:?}"),
        }
    }
}

#[test]
fn test_strict_loading_rejects_unknown_entry_field_after_authentication() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("unknown_entry.keystore");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    keystore
        .import_private_key(
            Some("entry".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    rewrite_keystore_json_with_valid_mac(&keystore, &keystore_path, |json| {
        json["entries"][0]["unexpected"] = serde_json::json!(true);
    });

    let mut loaded =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let err = loaded.unlock("strong_password_123").unwrap_err();
    match err {
        KeystoreError::InvalidInput(msg) => {
            assert!(msg.contains("Malformed authenticated keystore payload"))
        }
        other => panic!("expected strict authenticated entry rejection, got: {other:?}"),
    }
}

#[test]
fn test_kdf_memory_above_configured_max_rejected_before_unlock() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("kdf_too_large.keystore");

    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("strong_password_123").unwrap();
    }

    let mut json = read_keystore_json(&keystore_path);
    json["kdf_params"]["memory_kb"] =
        serde_json::json!(KeystoreConfig::development().argon2_memory_kb + 1);
    write_keystore_json(&keystore_path, &json);

    let err = Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap_err();
    match err {
        KeystoreError::InvalidInput(msg) => assert!(msg.contains("KDF memory cost out of bounds")),
        other => panic!("expected KDF bound rejection, got: {other:?}"),
    }
}

#[test]
fn test_tampered_kdf_param_with_unchanged_mac_fails_integrity() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("tampered_kdf.keystore");

    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("strong_password_123").unwrap();
    }

    let mut json = read_keystore_json(&keystore_path);
    json["kdf_params"]["iterations"] = serde_json::json!(1);
    write_keystore_json(&keystore_path, &json);

    let mut loaded =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let err = loaded.unlock("strong_password_123").unwrap_err();
    match err {
        KeystoreError::InvalidInput(msg) => assert!(msg.contains("integrity verification failed")),
        other => panic!("expected KDF tamper integrity failure, got: {other:?}"),
    }
}

#[test]
fn test_tampered_audit_log_and_mac_fail_integrity() {
    let temp_dir = tempdir().unwrap();
    let audit_path = temp_dir.path().join("tampered_audit.keystore");
    let mac_path = temp_dir.path().join("tampered_mac.keystore");

    for path in [&audit_path, &mac_path] {
        let mut keystore = Keystore::new_with_config(path, KeystoreConfig::development()).unwrap();
        keystore.unlock("strong_password_123").unwrap();
        keystore
            .import_private_key(
                Some("persist-audit".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap();
    }

    let mut audit_json = read_keystore_json(&audit_path);
    audit_json["audit_log"][0]["success"] = serde_json::json!(false);
    write_keystore_json(&audit_path, &audit_json);
    let mut loaded_audit =
        Keystore::new_with_config(&audit_path, KeystoreConfig::development()).unwrap();
    assert!(matches!(
        loaded_audit.unlock("strong_password_123"),
        Err(KeystoreError::InvalidInput(_))
    ));

    let mut mac_json = read_keystore_json(&mac_path);
    let mac_byte = mac_json["file_integrity_mac"][0]
        .as_u64()
        .expect("file integrity MAC byte");
    mac_json["file_integrity_mac"][0] = serde_json::json!((mac_byte + 1) % 256);
    write_keystore_json(&mac_path, &mac_json);
    let mut loaded_mac =
        Keystore::new_with_config(&mac_path, KeystoreConfig::development()).unwrap();
    assert!(matches!(
        loaded_mac.unlock("strong_password_123"),
        Err(KeystoreError::InvalidInput(_))
    ));
}

#[test]
fn test_save_time_mac_helper_recomputes_current_body() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("save_time_mac_helper.keystore");
    let (keystore, _) = unlocked_keystore_with_one_key(&keystore_path, "authenticated");

    let master_key = keystore.master_key.as_ref().unwrap();
    keystore
        .verify_current_file_matches_memory_mac(master_key)
        .unwrap();

    mutate_keystore_json_retaining_mac(&keystore_path, |value| {
        value["entries"][0]["alias"] = serde_json::json!("tampered");
    });

    assert_concurrent_write_rejected(keystore.verify_current_file_matches_memory_mac(master_key));
}

#[test]
fn test_import_after_retained_mac_body_tamper_fails_without_rewrite() {
    assert_import_after_tamper_fails_without_rewrite(|value| {
        value["entries"][0]["alias"] = serde_json::json!("old-mac-body-tamper");
    });
}

#[test]
fn test_operations_after_external_tamper_fail_without_rewrite() {
    #[derive(Clone, Copy)]
    enum Case {
        Get,
        Delete,
    }

    for case in [Case::Get, Case::Delete] {
        let temp_dir = tempdir().unwrap();
        let (file_name, alias) = match case {
            Case::Get => ("get_after_tamper.keystore", "read-target"),
            Case::Delete => ("delete_after_tamper.keystore", "delete-target"),
        };
        let keystore_path = temp_dir.path().join(file_name);
        let (mut keystore, key_id) = unlocked_keystore_with_one_key(&keystore_path, alias);

        mutate_keystore_json_retaining_mac(&keystore_path, |value| match case {
            Case::Get => value["audit_log"][0]["success"] = serde_json::json!(false),
            Case::Delete => {
                value["entries"][0]["alias"] = serde_json::json!("tampered-delete-target")
            }
        });
        let tampered_bytes = std::fs::read(&keystore_path).unwrap();

        match case {
            Case::Get => {
                assert_concurrent_write_rejected(keystore.get_private_key(key_id).map(|_| ()))
            }
            Case::Delete => assert_concurrent_write_rejected(keystore.delete_key(key_id)),
        }
        assert_eq!(std::fs::read(&keystore_path).unwrap(), tampered_bytes);
    }
}

#[test]
fn removed_password_rotation_audit_event_is_rejected() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("removed-password-rotation.keystore");
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    rewrite_keystore_json_with_valid_mac(&keystore, &keystore_path, |value| {
        value["audit_log"]
            .as_array_mut()
            .expect("audit log")
            .push(serde_json::json!({
                "timestamp": chrono::Utc::now(),
                "event": "change_password",
                "success": true,
            }));
    });
    drop(keystore);

    let mut reopened =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    assert!(matches!(
        reopened.unlock("strong_password_123"),
        Err(KeystoreError::InvalidInput(_))
    ));
}

#[test]
fn test_save_time_validation_rejects_authenticated_regions_and_unknown_fields() {
    assert_import_after_tamper_fails_without_rewrite(|value| {
        value["entries"][0]["alias"] = serde_json::json!("tampered-entry");
    });
    assert_import_after_tamper_fails_without_rewrite(|value| {
        value["audit_log"][0]["success"] = serde_json::json!(false);
    });
    assert_import_after_tamper_fails_without_rewrite(|value| {
        let salt = value["kdf_params"]["salt"].as_array_mut().unwrap();
        let first = salt[0].as_u64().unwrap();
        salt[0] = serde_json::json!((first + 1) % 256);
    });
    assert_import_after_tamper_fails_without_rewrite(|value| {
        value["unexpected"] = serde_json::json!(true);
    });
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
        assert_ne!(
            key1.ethereum_address().unwrap(),
            key2.ethereum_address().unwrap()
        );

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
