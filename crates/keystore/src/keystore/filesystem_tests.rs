use super::*;

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

#[test]
fn test_password_policy_accepts_boundary_and_rejects_weak_values() {
    #[derive(Clone, Copy)]
    enum Case {
        Short,
        StrongBoundary,
        Common,
        Whitespace,
    }

    fn assert_policy_rejects(result: Result<(), KeystoreError>, expected_message: Option<&str>) {
        match result {
            Err(KeystoreError::InvalidInput(msg)) => {
                if let Some(expected_message) = expected_message {
                    assert!(
                        msg.contains(expected_message),
                        "expected password rejection to mention {expected_message:?}, got: {msg}"
                    );
                }
            }
            other => panic!("expected InvalidInput for weak password, got: {other:?}"),
        }
    }

    for (label, case) in [
        ("short-create", Case::Short),
        ("strong-boundary-create", Case::StrongBoundary),
        ("common-create", Case::Common),
        ("whitespace-create", Case::Whitespace),
    ] {
        let temp_dir = tempdir().unwrap();
        let keystore_path = temp_dir.path().join(format!("{label}.keystore"));
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

        match case {
            Case::Short => assert_policy_rejects(keystore.unlock("short"), None),
            Case::StrongBoundary => {
                keystore.unlock("A1b2C3d4E5f6").unwrap();
                assert_eq!(keystore.list_keys().unwrap().len(), 0);
            }
            Case::Common => {
                assert_policy_rejects(keystore.unlock("QWERTY123456"), Some("too weak"));
            }
            Case::Whitespace => {
                assert_policy_rejects(keystore.unlock("            "), Some("too weak"));
            }
        }
    }
}

#[test]
fn test_tampered_file_metadata_not_exposed_before_unlock() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("metadata_gate.keystore");

    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("strong_password_123").unwrap();
        keystore
            .import_private_key(
                Some("original-alias".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap();
    }

    let original_content = std::fs::read_to_string(&keystore_path).unwrap();
    let tampered_content = original_content.replace("original-alias", "tampered-alias");
    std::fs::write(&keystore_path, tampered_content).unwrap();

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    assert!(matches!(keystore.list_keys(), Err(KeystoreError::Locked)));

    let unlock_err = keystore.unlock("strong_password_123").unwrap_err();
    assert!(matches!(unlock_err, KeystoreError::InvalidInput(_)));
}

#[test]
fn test_duplicate_entry_ids_rejected() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("duplicate_ids.keystore");

    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("strong_password_123").unwrap();
        keystore
            .import_private_key(
                Some("dup".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap();
    }

    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&keystore_path).unwrap()).unwrap();
    let first_entry = json["entries"][0].clone();
    json["entries"].as_array_mut().unwrap().push(first_entry);
    std::fs::write(&keystore_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

    let mut loaded = Keystore::new_with_config(&keystore_path, KeystoreConfig::development())
        .expect("unauthenticated load must not hydrate tampered entries");
    assert!(matches!(
        loaded.unlock("strong_password_123"),
        Err(KeystoreError::InvalidInput(_))
    ));
}

#[test]
fn test_oversized_audit_log_rejected() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("large_audit.keystore");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    rewrite_keystore_json_with_valid_mac(&keystore, &keystore_path, |json| {
        json["audit_log"] = serde_json::Value::Array(
            (0..=MAX_AUDIT_LOG_ENTRIES)
                .map(|_| serde_json::to_value(audit_test_entry(AuditEvent::Unlock)).unwrap())
                .collect(),
        );
    });
    drop(keystore);

    let mut loaded = Keystore::new_with_config(&keystore_path, KeystoreConfig::development())
        .expect("unauthenticated load must not hydrate tampered audit records");
    assert!(matches!(
        loaded.unlock("strong_password_123"),
        Err(KeystoreError::InvalidInput(_))
    ));
}

#[test]
fn test_non_regular_keystore_path_rejected() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("not_a_file");
    std::fs::create_dir_all(&keystore_path).unwrap();

    let result = Keystore::new_with_config(&keystore_path, KeystoreConfig::development());
    assert!(matches!(result, Err(KeystoreError::InvalidInput(_))));
}

#[cfg(unix)]
#[test]
fn test_unsafe_parent_directory_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = tempdir().unwrap();
    let unsafe_parent = temp_dir.path().join("unsafe_parent");
    std::fs::create_dir_all(&unsafe_parent).unwrap();
    std::fs::set_permissions(&unsafe_parent, std::fs::Permissions::from_mode(0o777)).unwrap();

    let keystore_path = unsafe_parent.join("unsafe.keystore");
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let result = keystore.unlock("strong_password_123");
    assert!(matches!(result, Err(KeystoreError::InvalidInput(_))));

    std::fs::set_permissions(&unsafe_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn test_concurrent_write_conflict_detected() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("conflict.keystore");

    let mut keystore1 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore1.unlock("strong_password_123").unwrap();

    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore2.unlock("strong_password_123").unwrap();

    keystore1
        .import_private_key(
            Some("writer1".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    let result = keystore2.import_private_key(
        Some("writer2".to_string()),
        "0000000000000000000000000000000000000000000000000000000000000002",
    );
    assert!(matches!(result, Err(KeystoreError::InvalidInput(_))));
}

#[test]
fn test_auto_lock_timeout_expires_session() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("autolock".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    keystore.set_auto_lock_timeout(Some(std::time::Duration::from_millis(1)));
    std::thread::sleep(std::time::Duration::from_millis(10));

    assert!(matches!(keystore.list_keys(), Err(KeystoreError::Locked)));
    assert!(matches!(
        keystore.private_key_for_test(key_id),
        Err(KeystoreError::Locked)
    ));
}

#[test]
fn test_auto_lock_timeout_can_be_disabled() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    keystore.set_auto_lock_timeout(None);

    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(keystore.list_keys().is_ok());
}

#[test]
fn test_observational_test_key_access_does_not_refresh_auto_lock() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("refresh".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    keystore.set_auto_lock_timeout(Some(std::time::Duration::from_millis(400)));

    std::thread::sleep(std::time::Duration::from_millis(250));
    assert!(keystore.private_key_for_test(key_id).is_ok());

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(matches!(
        keystore.private_key_for_test(key_id),
        Err(KeystoreError::Locked)
    ));
}

#[test]
fn test_audit_log_limit_boundary_is_accepted() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("audit_boundary.keystore");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    rewrite_keystore_json_with_valid_mac(&keystore, &keystore_path, |json| {
        json["audit_log"] = serde_json::Value::Array(
            (0..MAX_AUDIT_LOG_ENTRIES)
                .map(|_| serde_json::to_value(audit_test_entry(AuditEvent::Unlock)).unwrap())
                .collect(),
        );
    });
    drop(keystore);

    let mut loaded =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    loaded.unlock("strong_password_123").unwrap();
    assert_eq!(loaded.audit_log().len(), MAX_AUDIT_LOG_ENTRIES);
}

#[cfg(unix)]
#[test]
fn test_symlink_parent_directory_rejected() {
    use std::os::unix::fs::symlink;

    let temp_dir = tempdir().unwrap();
    let real_parent = temp_dir.path().join("real_parent");
    let linked_parent = temp_dir.path().join("linked_parent");
    std::fs::create_dir_all(&real_parent).unwrap();
    symlink(&real_parent, &linked_parent).unwrap();

    let keystore_path = linked_parent.join("symlink_parent.keystore");
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let err = keystore.unlock("strong_password_123").unwrap_err();
    match err {
        KeystoreError::InvalidInput(msg) => assert!(msg.contains("symlinked parent directory")),
        other => panic!("expected InvalidInput for symlink parent, got: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn test_symlink_keystore_path_rejected() {
    use std::os::unix::fs::symlink;

    let temp_dir = tempdir().unwrap();
    let real_path = temp_dir.path().join("real.keystore");
    {
        let mut keystore =
            Keystore::new_with_config(&real_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("strong_password_123").unwrap();
    }

    let symlink_path = temp_dir.path().join("symlink.keystore");
    symlink(&real_path, &symlink_path).unwrap();
    let result = Keystore::new_with_config(&symlink_path, KeystoreConfig::development());
    assert!(matches!(result, Err(KeystoreError::InvalidInput(_))));
}

#[test]
fn test_write_fails_if_backing_file_deleted_during_session() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("deleted_file_conflict.keystore");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    keystore
        .import_private_key(
            Some("first".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    std::fs::remove_file(&keystore_path).unwrap();
    let result = keystore.import_private_key(
        Some("second".to_string()),
        "0000000000000000000000000000000000000000000000000000000000000002",
    );
    assert!(matches!(result, Err(KeystoreError::InvalidInput(_))));
}
