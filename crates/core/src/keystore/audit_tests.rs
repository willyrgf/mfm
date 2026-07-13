use super::*;

#[test]
fn test_import_write_failure_rolls_back_memory_and_audit() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("import_write_failure.keystore");
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();

    let previous_entries = keystore.entries.len();
    let previous_audit = keystore.audit_log.len();
    let previous_mac = keystore.file_integrity_mac;
    keystore.fail_next_write_for_test();

    let err = keystore
        .import_private_key(
            Some("failed-import".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap_err();

    assert!(matches!(err, KeystoreError::FileError(_)));
    assert_eq!(keystore.entries.len(), previous_entries);
    assert_eq!(keystore.audit_log.len(), previous_audit);
    assert_eq!(keystore.file_integrity_mac, previous_mac);
    assert!(!std::fs::read_to_string(&keystore_path)
        .unwrap()
        .contains("failed-import"));
}

#[test]
fn test_delete_write_failure_rolls_back_memory_and_disk() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("delete_write_failure.keystore");
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("delete-rollback".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    let previous_entries = keystore.entries.clone();
    let previous_audit = keystore.audit_log.len();
    keystore.fail_next_write_for_test();
    let err = keystore.delete_key(key_id).unwrap_err();

    assert!(matches!(err, KeystoreError::FileError(_)));
    assert_eq!(keystore.entries.len(), previous_entries.len());
    assert!(keystore.entries.iter().any(|entry| entry.id == key_id));
    assert_eq!(keystore.audit_log.len(), previous_audit);
    assert!(std::fs::read_to_string(&keystore_path)
        .unwrap()
        .contains("delete-rollback"));
}

#[test]
fn test_get_private_key_write_failure_returns_error_without_audit() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("get_key_write_failure.keystore");
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("signing-key".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    let previous_audit = keystore.audit_log.len();
    keystore.fail_next_write_for_test();
    let err = match keystore.get_private_key(key_id) {
        Ok(_) => panic!("get_private_key must fail closed when audit persistence fails"),
        Err(err) => err,
    };

    assert!(matches!(err, KeystoreError::FileError(_)));
    assert_eq!(keystore.audit_log.len(), previous_audit);
    let persisted = persisted_audit_log(&keystore_path);
    assert!(!persisted
        .iter()
        .any(|entry| matches!(entry.event, AuditEvent::GetPrivateKey { id } if id == key_id)));
}

#[test]
fn test_successful_mutations_persist_exactly_one_audit_record() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("atomic_success_audit.keystore");
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();

    let key_id = keystore
        .import_private_key(
            Some("audited".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
    assert_eq!(
        persisted_audit_log(&keystore_path)
            .iter()
            .filter(
                |entry| matches!(entry.event, AuditEvent::ImportPrivateKey { id } if id == key_id)
                    && entry.success
            )
            .count(),
        1
    );

    keystore.delete_key(key_id).unwrap();
    assert_eq!(
        persisted_audit_log(&keystore_path)
            .iter()
            .filter(
                |entry| matches!(entry.event, AuditEvent::DeleteKey { id } if id == key_id)
                    && entry.success
            )
            .count(),
        1
    );

    keystore
        .change_password("strong_password_123", "new_password_123")
        .unwrap();
    assert_eq!(
        persisted_audit_log(&keystore_path)
            .iter()
            .filter(|entry| matches!(entry.event, AuditEvent::ChangePassword) && entry.success)
            .count(),
        1
    );

    let mut reopened =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    reopened.unlock("new_password_123").unwrap();
}

#[test]
fn test_audit_append_at_boundary_compacts_oldest_entries() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.audit_log = (0..MAX_AUDIT_LOG_ENTRIES)
        .map(|_| audit_test_entry(AuditEvent::Unlock))
        .collect();

    let first_event_id = Uuid::new_v4();
    keystore.append_audit_event(AuditEvent::GetPrivateKey { id: first_event_id }, true);

    assert_eq!(keystore.audit_log().len(), MAX_AUDIT_LOG_ENTRIES);
    assert!(matches!(
        keystore.audit_log().first().unwrap().event,
        AuditEvent::AuditLogCompacted { dropped_entries: 2 }
    ));
    assert!(matches!(
        keystore.audit_log().last().unwrap().event,
        AuditEvent::GetPrivateKey { id } if id == first_event_id
    ));

    let second_event_id = Uuid::new_v4();
    keystore.append_audit_event(
        AuditEvent::DeleteKey {
            id: second_event_id,
        },
        false,
    );

    assert_eq!(keystore.audit_log().len(), MAX_AUDIT_LOG_ENTRIES);
    assert!(matches!(
        keystore.audit_log().first().unwrap().event,
        AuditEvent::AuditLogCompacted { dropped_entries: 3 }
    ));
    assert!(matches!(
        keystore.audit_log().last().unwrap().event,
        AuditEvent::DeleteKey { id } if id == second_event_id
    ));
}

#[test]
fn test_get_private_key_past_audit_limit_compacts_and_persists() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("audit_compaction.keystore");
    let (keystore, key_id) = unlocked_keystore_with_one_key(&keystore_path, "audit-target");

    rewrite_keystore_json_with_valid_mac(&keystore, &keystore_path, |json| {
        json["audit_log"] = serde_json::Value::Array(
            (0..MAX_AUDIT_LOG_ENTRIES)
                .map(|_| serde_json::to_value(audit_test_entry(AuditEvent::Unlock)).unwrap())
                .collect(),
        );
    });
    drop(keystore);

    let mut reopened =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    reopened.unlock("strong_password_123").unwrap();
    for _ in 0..4 {
        reopened.get_private_key(key_id).unwrap();
        assert!(reopened.audit_log().len() <= MAX_AUDIT_LOG_ENTRIES);
    }

    let persisted = persisted_audit_log(&keystore_path);
    assert_eq!(persisted.len(), MAX_AUDIT_LOG_ENTRIES);
    assert!(matches!(
        persisted.first().unwrap().event,
        AuditEvent::AuditLogCompacted { dropped_entries } if dropped_entries >= 6
    ));
    assert!(matches!(
        persisted.last().unwrap().event,
        AuditEvent::GetPrivateKey { id } if id == key_id
    ));
}

#[cfg(feature = "dangerous-secret-export")]
#[test]
fn test_audit_log_entries_created_for_operations() {
    let (_temp_dir, mut keystore) = test_keystore_with_exports();

    // Unlock logs.
    keystore.unlock("test_password").unwrap();
    assert!(matches!(
        keystore.audit_log().last().unwrap().event,
        AuditEvent::Unlock
    ));

    let pk_id = keystore
        .import_private_key(
            Some("pk".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000003",
        )
        .unwrap();
    assert!(keystore
        .audit_log()
        .iter()
        .any(|e| matches!(e.event, AuditEvent::ImportPrivateKey { .. }) && e.success));

    // get_private_key logs.
    keystore.get_private_key(pk_id).unwrap();
    assert!(keystore
        .audit_log()
        .iter()
        .any(|e| matches!(e.event, AuditEvent::GetPrivateKey { id } if id == pk_id) && e.success));

    // export_private_key logs.
    keystore.export_private_key(pk_id).unwrap();
    assert!(keystore.audit_log().iter().any(
        |e| matches!(e.event, AuditEvent::ExportPrivateKey { id } if id == pk_id) && e.success
    ));

    // delete_key logs.
    keystore.delete_key(pk_id).unwrap();
    assert!(keystore
        .audit_log()
        .iter()
        .any(|e| matches!(e.event, AuditEvent::DeleteKey { id } if id == pk_id) && e.success));

    // lock logs.
    keystore.lock();
    assert!(matches!(
        keystore.audit_log().last().unwrap().event,
        AuditEvent::Lock
    ));

    // Locked failures cannot be durably authenticated in the single-file format.
    let previous_audit_len = keystore.audit_log().len();
    assert!(keystore
        .change_password("old_password", "new_password")
        .is_err());
    assert_eq!(keystore.audit_log().len(), previous_audit_len);
}

#[cfg(feature = "dangerous-secret-export")]
#[test]
fn test_audit_log_persisted_for_read_only_access_operations() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("audit_persist.keystore");

    let (pk_id, mnemonic_id) = {
        let mut config = KeystoreConfig::development();
        config.allow_secret_exports = true;
        let mut keystore = Keystore::new_with_config(&keystore_path, config).unwrap();
        keystore.unlock("test_password").unwrap();

        let pk_id = keystore
            .import_private_key(
                Some("pk".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000004",
            )
            .unwrap();

        let mnemonic_id = keystore
            .import_mnemonic(
                Some("mn".to_string()),
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
                "m/44'/60'/0'/0/0",
                None,
            )
            .unwrap();

        // These operations should persist their audit entries to disk.
        keystore.get_private_key(pk_id).unwrap();
        keystore.export_private_key(pk_id).unwrap();
        keystore.export_private_key(mnemonic_id).unwrap();

        (pk_id, mnemonic_id)
    };

    // Re-open keystore and verify audit entries survived the process boundary.
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore2.unlock("test_password").unwrap();

    assert!(keystore2.audit_log().iter().any(|e| {
        matches!(e.event, AuditEvent::GetPrivateKey { id } if id == pk_id) && e.success
    }));
    assert!(keystore2.audit_log().iter().any(|e| {
        matches!(e.event, AuditEvent::ExportPrivateKey { id } if id == pk_id) && e.success
    }));
    assert!(keystore2.audit_log().iter().any(|e| {
        matches!(e.event, AuditEvent::ExportPrivateKey { id } if id == mnemonic_id) && e.success
    }));
}
