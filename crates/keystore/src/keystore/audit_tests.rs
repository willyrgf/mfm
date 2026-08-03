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
fn read_attestation_key_access_never_attempts_an_audit_write() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("observational_key_access.keystore");
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("read-attestation".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
    let persisted_before = std::fs::read(&keystore_path).unwrap();
    let audit_before = keystore.audit_log.len();
    let access = crate::signer::ReadAttestationKeyAccess::for_test();

    keystore.fail_next_write_for_test();
    let key = keystore
        .private_key_for_read_attestation(key_id, &access)
        .expect("observational key access");
    drop(key);

    assert_eq!(keystore.audit_log.len(), audit_before);
    assert!(
        std::fs::read(&keystore_path).unwrap() == persisted_before,
        "Read attestation changed persisted keystore bytes"
    );
    assert!(matches!(
        keystore.delete_key(key_id),
        Err(KeystoreError::FileError(_))
    ));
    assert_eq!(keystore.audit_log.len(), audit_before);
    assert!(keystore.entries.iter().any(|entry| entry.id == key_id));
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
}

#[test]
fn test_audit_append_at_boundary_compacts_oldest_entries() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.audit_log = (0..MAX_AUDIT_LOG_ENTRIES)
        .map(|_| audit_test_entry(AuditEvent::Unlock))
        .collect();

    let first_event_id = Uuid::new_v4();
    keystore.append_audit_event(AuditEvent::ImportPrivateKey { id: first_event_id }, true);

    assert_eq!(keystore.audit_log().len(), MAX_AUDIT_LOG_ENTRIES);
    assert!(matches!(
        keystore.audit_log().first().unwrap().event,
        AuditEvent::AuditLogCompacted { dropped_entries: 2 }
    ));
    assert!(matches!(
        keystore.audit_log().last().unwrap().event,
        AuditEvent::ImportPrivateKey { id } if id == first_event_id
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
fn historical_v1_key_access_audit_record_remains_decodable_and_mac_covered() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("historical_v1_audit.keystore");
    let (keystore, key_id) = unlocked_keystore_with_one_key(&keystore_path, "historical");
    rewrite_keystore_json_with_valid_mac(&keystore, &keystore_path, |json| {
        json["audit_log"].as_array_mut().expect("audit log").push(
            serde_json::to_value(audit_test_entry(AuditEvent::GetPrivateKey { id: key_id }))
                .expect("historical audit record"),
        );
    });
    drop(keystore);

    let mut reopened =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    reopened.unlock("strong_password_123").unwrap();
    assert!(reopened
        .audit_log()
        .iter()
        .any(|entry| matches!(entry.event, AuditEvent::GetPrivateKey { id } if id == key_id)));
    drop(reopened);

    mutate_keystore_json_retaining_mac(&keystore_path, |json| {
        let last = json["audit_log"]
            .as_array_mut()
            .and_then(|entries| entries.last_mut())
            .expect("historical audit record");
        last["event"] = serde_json::to_value(AuditEvent::GetPrivateKey { id: Uuid::new_v4() })
            .expect("substituted historical record");
    });
    let mut tampered =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    assert!(matches!(
        tampered.unlock("strong_password_123"),
        Err(KeystoreError::InvalidInput(_))
    ));
}

#[test]
fn test_audit_log_entries_created_for_operations() {
    let (_temp_dir, mut keystore) = test_keystore();

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

    // delete_key logs.
    keystore.delete_key(pk_id).unwrap();
    assert!(keystore
        .audit_log()
        .iter()
        .any(|e| matches!(e.event, AuditEvent::DeleteKey { id } if id == pk_id) && e.success));
}
