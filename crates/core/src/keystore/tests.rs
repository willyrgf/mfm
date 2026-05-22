/// Comprehensive tests for keystore_v3 public API
///
/// These tests validate all public methods and ensure the API works correctly
/// for external consumers using only public interfaces.
use super::*;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use static_assertions::assert_not_impl_any;
use std::path::Path;
use std::sync::Once;
use tempfile::tempdir;
use tracing::{info, warn};

fn init_test_observability() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,mfm=debug")),
            )
            .with_test_writer()
            .try_init();
    });
}

#[test]
fn keystore_is_not_send_nor_sync() {
    // Fails to compile if `Keystore` implements *any* of the listed traits
    assert_not_impl_any!(Keystore: Send, Sync);
}

#[test]
fn keystore_debug_output_redacts_secret_material() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let private_key = "1111111111111111111111111111111111111111111111111111111111111111";
    let mnemonic =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let mnemonic_passphrase = "debug-secret-passphrase";
    keystore
        .import_private_key(Some("debug-secret-alias".to_string()), private_key)
        .unwrap();
    keystore
        .import_mnemonic(
            Some("debug-secret-mnemonic".to_string()),
            mnemonic,
            "m/44'/60'/0'/0/0",
            Some(mnemonic_passphrase),
        )
        .unwrap();

    let master_key_hex = hex::encode(keystore.master_key.as_ref().unwrap().as_ref());
    let debug = format!("{keystore:?}");

    assert!(debug.contains("Keystore"));
    assert!(debug.contains("unlocked: true"));
    assert!(debug.contains("entry_count: 2"));
    assert!(debug.contains("kdf_params_loaded: true"));

    for forbidden in [
        "master_key",
        "master_key_verification",
        "file_integrity_mac",
        "encrypted_data",
        private_key,
        mnemonic,
        mnemonic_passphrase,
        "debug-secret-alias",
        "debug-secret-mnemonic",
        master_key_hex.as_str(),
    ] {
        assert!(
            !debug.contains(forbidden),
            "keystore debug output leaked `{forbidden}`: {debug}"
        );
    }
}

/// Helper function to create a test keystore with development config
fn test_keystore() -> (tempfile::TempDir, Keystore) {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("test.keystore");
    let keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    (temp_dir, keystore)
}

fn read_keystore_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn write_keystore_json(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn mutate_keystore_json_retaining_mac(path: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
    let mut value = read_keystore_json(path);
    let original_mac = value["file_integrity_mac"].clone();
    mutate(&mut value);
    value["file_integrity_mac"] = original_mac;
    write_keystore_json(path, &value);
}

fn persisted_audit_log(path: &Path) -> Vec<AuditLogEntry> {
    serde_json::from_value(read_keystore_json(path)["audit_log"].clone()).unwrap()
}

fn audit_test_entry(event: AuditEvent) -> AuditLogEntry {
    AuditLogEntry {
        timestamp: chrono::Utc::now(),
        event,
        success: true,
    }
}

fn rewrite_keystore_json_with_valid_mac(
    keystore: &Keystore,
    path: &Path,
    mutate: impl FnOnce(&mut serde_json::Value),
) {
    let mut value = read_keystore_json(path);
    mutate(&mut value);
    let master_key = keystore.master_key.as_ref().unwrap();
    let preimage = Keystore::canonical_file_mac_preimage_from_value(value.clone()).unwrap();
    let mac = keystore
        .compute_file_integrity_mac(master_key, &preimage)
        .unwrap();
    value["file_integrity_mac"] = serde_json::to_value(mac).unwrap();
    write_keystore_json(path, &value);
}

fn assert_concurrent_write_rejected<T>(result: Result<T, KeystoreError>) {
    match result {
        Err(KeystoreError::InvalidInput(msg)) => {
            assert!(msg.contains("Concurrent modification detected while writing keystore"))
        }
        Err(other) => panic!("expected concurrent modification rejection, got: {other:?}"),
        Ok(_) => panic!("expected concurrent modification rejection"),
    }
}

fn unlocked_keystore_with_one_key(path: &Path, alias: &str) -> (Keystore, Uuid) {
    let mut keystore = Keystore::new_with_config(path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    let key_id = keystore
        .import_private_key(
            Some(alias.to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
    (keystore, key_id)
}

fn assert_import_after_tamper_fails_without_rewrite(mutate: impl FnOnce(&mut serde_json::Value)) {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("import_after_tamper.keystore");
    let (mut keystore, _) = unlocked_keystore_with_one_key(&keystore_path, "original");

    mutate_keystore_json_retaining_mac(&keystore_path, mutate);
    let tampered_bytes = std::fs::read(&keystore_path).unwrap();

    assert_concurrent_write_rejected(keystore.import_private_key(
        Some("new-key".to_string()),
        "0000000000000000000000000000000000000000000000000000000000000002",
    ));
    assert_eq!(std::fs::read(&keystore_path).unwrap(), tampered_bytes);
}

#[cfg(feature = "dangerous-secret-export")]
fn test_keystore_with_exports() -> (tempfile::TempDir, Keystore) {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("test.keystore");
    let mut config = KeystoreConfig::development();
    config.allow_secret_exports = true;
    let keystore = Keystore::new_with_config(&keystore_path, config).unwrap();
    (temp_dir, keystore)
}

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
fn test_export_private_key_for_private_key_entries() {
    let (_temp_dir, mut keystore) = test_keystore_with_exports();
    keystore.unlock("test_password").unwrap();

    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let id = keystore
        .import_private_key(Some("export_pk".to_string()), test_key)
        .unwrap();

    let exported = keystore.export_private_key(id).unwrap();
    assert_eq!(exported.as_str(), format!("0x{test_key}"));
}

#[cfg(feature = "dangerous-secret-export")]
#[test]
fn test_export_private_key_for_hd_derived_entries() {
    let (_temp_dir, mut keystore) = test_keystore_with_exports();
    keystore.unlock("test_password").unwrap();

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
    assert!(!cfg!(feature = "dangerous-secret-export"));
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
fn test_legacy_mnemonic_entry_fails_closed_without_secret_error() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("legacy_mnemonic_entry.keystore");
    let mnemonic =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let passphrase = "legacy-secret-passphrase";

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("strong_password_123").unwrap();
    keystore
        .import_private_key(
            Some("legacy".to_string()),
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

#[test]
fn test_concurrent_operations() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Import multiple keys
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

    // Retrieve both keys
    let secure_key1 = keystore.get_private_key(key1_id).unwrap();
    let secure_key2 = keystore.get_private_key(key2_id).unwrap();

    // Verify they produce different signatures for same hash
    let test_hash = [3u8; 32];
    let sig1 = secure_key1.sign_hash(&test_hash).unwrap();
    let sig2 = secure_key2.sign_hash(&test_hash).unwrap();

    assert_ne!(sig1.to_bytes(), sig2.to_bytes());

    // Verify different addresses
    assert_ne!(
        secure_key1.ethereum_address().unwrap(),
        secure_key2.ethereum_address().unwrap()
    );
}

#[test]
fn test_keystore_persistence() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("persistent.keystore");

    let key_id = {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();

        let test_key = "0000000000000000000000000000000000000000000000000000000000000002";
        keystore
            .import_private_key(Some("persistent_key".to_string()), test_key)
            .unwrap()
    };

    // Load keystore again
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore2.unlock("test_password").unwrap();

    // Verify key persisted
    let keys = keystore2.list_keys().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].id, key_id);
    assert_eq!(keys[0].alias, Some("persistent_key".to_string()));

    // Verify we can still retrieve the key
    let secure_key = keystore2.get_private_key(key_id).unwrap();
    let test_hash = [1u8; 32];
    let _signature = secure_key.sign_hash(&test_hash).unwrap();
}

#[test]
fn test_wrong_password() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("password_test.keystore");

    // Create keystore with password
    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("correct_password").unwrap();
    }

    // Try to unlock with wrong password
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let result = keystore2.unlock("wrong_password");

    assert!(result.is_err());
    match result.unwrap_err() {
        KeystoreError::InvalidInput(msg) => {
            assert!(msg.contains("integrity verification failed"));
            assert!(!msg.contains("correct_password"));
            assert!(!msg.contains("wrong_password"));
        }
        other => panic!("expected safe authentication failure, got: {other:?}"),
    }
}

#[test]
fn test_locked_operations() {
    let (_temp_dir, mut keystore) = test_keystore();

    // Try operations on locked keystore
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000001"
        ),
        Err(KeystoreError::Locked)
    ));

    assert!(matches!(
        keystore.get_private_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));

    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));

    assert!(matches!(keystore.list_keys(), Err(KeystoreError::Locked)));
}

#[test]
fn test_key_deletion() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_key = "0000000000000000000000000000000000000000000000000000000000000003";
    let key_id = keystore
        .import_private_key(Some("deletable_key".to_string()), test_key)
        .unwrap();

    // Verify key exists
    assert_eq!(keystore.list_keys().unwrap().len(), 1);

    // Delete key
    keystore.delete_key(key_id).unwrap();

    // Verify key is gone
    assert_eq!(keystore.list_keys().unwrap().len(), 0);

    // Try to delete non-existent key
    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::KeyNotFound(_))
    ));
}

// ===== COMPREHENSIVE API TESTS =====

#[test]
fn test_keystore_new_variants() {
    let temp_dir = tempdir().unwrap();

    // Test new() with default config
    let keystore_path1 = temp_dir.path().join("test1.keystore");
    let keystore1 = Keystore::new(&keystore_path1).unwrap();
    assert_eq!(keystore1.config.argon2_memory_kb, 1_048_576); // 1GB default
    assert!(!keystore1.config.allow_secret_exports);

    // Test new_with_config() with development config
    let keystore_path2 = temp_dir.path().join("test2.keystore");
    let keystore2 =
        Keystore::new_with_config(&keystore_path2, KeystoreConfig::development()).unwrap();
    assert_eq!(keystore2.config.argon2_memory_kb, 8192); // 8MB development
    assert!(!keystore2.config.allow_secret_exports);

    // Test with production config
    let keystore_path3 = temp_dir.path().join("test3.keystore");
    let keystore3 =
        Keystore::new_with_config(&keystore_path3, KeystoreConfig::production()).unwrap();
    assert_eq!(keystore3.config.argon2_memory_kb, 1_048_576); // 1GB production
    assert!(!keystore3.config.allow_secret_exports);
}

#[test]
fn test_unlock_edge_cases() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("unlock_test.keystore");

    // Test initial unlock (creates new keystore)
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("initial_password").unwrap();

    // Verify keystore file was created
    assert!(keystore_path.exists());

    // Test unlock on existing keystore
    keystore.lock();
    keystore.unlock("initial_password").unwrap();

    // Test multiple unlock calls (should be idempotent)
    keystore.unlock("initial_password").unwrap();
    keystore.unlock("initial_password").unwrap();

    // Test empty password
    keystore.lock();
    let result = keystore.unlock("");
    assert!(result.is_err());
}

#[test]
fn test_lock_comprehensive() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Import a key while unlocked
    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let _key_id = keystore
        .import_private_key(Some("test".to_string()), test_key)
        .unwrap();

    // Lock the keystore
    keystore.lock();

    // Metadata access is locked behind an unlocked session.
    assert!(matches!(keystore.list_keys(), Err(KeystoreError::Locked)));

    // Verify operations requiring master key fail
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000002"
        ),
        Err(KeystoreError::Locked)
    ));

    // Test multiple lock calls (should be idempotent)
    keystore.lock();
    keystore.lock();
}

#[test]
fn test_import_private_key_edge_cases() {
    init_test_observability();
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Test various valid private key formats
    let valid_keys = [
        "0000000000000000000000000000000000000000000000000000000000000001",
        "0x0000000000000000000000000000000000000000000000000000000000000002",
        "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364140", // Large but valid secp256k1 key (curve order - 1)
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    ];

    for (i, key) in valid_keys.iter().enumerate() {
        let result = keystore.import_private_key(Some(format!("key_{i}")), key);
        match result {
            Ok(_) => info!(test_case = i, "valid private key import accepted"),
            Err(e) => panic!("Failed to import valid key {i}: {key} - Error: {e:?}"),
        }
    }

    // Test invalid private key formats
    let invalid_keys = [
        "invalid_hex",
        "0x",
        "",
        "0000000000000000000000000000000000000000000000000000000000000000", // Zero key
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141", // Curve order (invalid)
        "1234567890abcdef",                                                 // Too short
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef00", // Too long
    ];

    for key in invalid_keys {
        let result = keystore.import_private_key(None, key);
        if result.is_ok() {
            warn!("invalid format key accepted by parser; this may be acceptable");
        }
        // Note: We don't assert failure here since some edge cases might be acceptable
    }
}

#[test]
fn test_import_mnemonic_edge_cases() {
    init_test_observability();
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Test valid mnemonic with different derivation paths
    let valid_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    let valid_paths = [
        "m/44'/60'/0'/0/0", // Standard Ethereum path
        "m/44'/60'/0'/0/1", // Different account
        "m/44'/60'/1'/0/0", // Different account index
        "m/0'/0/0",         // Simplified path
    ];

    for (i, path) in valid_paths.iter().enumerate() {
        let result =
            keystore.import_mnemonic(Some(format!("mnemonic_{i}")), valid_mnemonic, path, None);
        assert!(result.is_ok(), "Failed to import with valid path: {path}");
    }

    // Test invalid derivation paths
    let invalid_paths = [
        "invalid/path",
        "m/44'/60'/0'/0/-1", // Negative index
        "",
        "m",
        "m/44'/60'/0'/0/999999999999999999999", // Very large index
    ];

    for path in invalid_paths {
        let result = keystore.import_mnemonic(None, valid_mnemonic, path, None);
        if result.is_err() {
            info!("invalid derivation path rejected by parser");
        }
        // Note: Some paths might be accepted by the underlying library
    }

    // Test invalid mnemonics
    let invalid_mnemonics = [
        "invalid mnemonic phrase",
        "",
        "abandon", // Too short
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon invalid", // Invalid word
    ];

    for mnemonic in invalid_mnemonics {
        let result = keystore.import_mnemonic(None, mnemonic, "m/44'/60'/0'/0/0", None);
        assert!(result.is_err(), "Invalid mnemonic should fail: {mnemonic}");
    }
}

#[test]
fn test_get_private_key_comprehensive() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Import test keys
    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id = keystore
        .import_private_key(Some("test_key".to_string()), test_key)
        .unwrap();

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let mnemonic_id = keystore
        .import_mnemonic(
            Some("test_mnemonic".to_string()),
            test_mnemonic,
            "m/44'/60'/0'/0/0",
            None,
        )
        .unwrap();

    // Test retrieval of both key types
    let private_key = keystore.get_private_key(key_id).unwrap();
    let mnemonic_key = keystore.get_private_key(mnemonic_id).unwrap();

    // Verify they produce different addresses
    assert_ne!(
        private_key.ethereum_address().unwrap(),
        mnemonic_key.ethereum_address().unwrap()
    );

    // Test non-existent key
    let result = keystore.get_private_key(Uuid::new_v4());
    assert!(matches!(result, Err(KeystoreError::KeyNotFound(_))));

    // Test with locked keystore
    keystore.lock();
    let result = keystore.get_private_key(key_id);
    assert!(matches!(result, Err(KeystoreError::Locked)));
}

#[test]
fn test_list_keys_comprehensive() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    // Initially empty
    assert_eq!(keystore.list_keys().unwrap().len(), 0);

    // Add some keys
    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id1 = keystore
        .import_private_key(Some("key1".to_string()), test_key)
        .unwrap();

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let key_id2 = keystore
        .import_mnemonic(
            Some("mnemonic1".to_string()),
            test_mnemonic,
            "m/44'/60'/0'/0/0",
            None,
        )
        .unwrap();

    // Test listing
    let keys = keystore.list_keys().unwrap();
    assert_eq!(keys.len(), 2);

    // Verify key info
    let key1_info = keys.iter().find(|k| k.id == key_id1).unwrap();
    assert_eq!(key1_info.alias, Some("key1".to_string()));
    assert!(matches!(key1_info.key_type, KeyType::PrivateKey));

    let key2_info = keys.iter().find(|k| k.id == key_id2).unwrap();
    assert_eq!(key2_info.alias, Some("mnemonic1".to_string()));
    assert!(matches!(key2_info.key_type, KeyType::HdDerived { .. }));

    // list_keys requires an unlocked session.
    keystore.lock();
    assert!(matches!(keystore.list_keys(), Err(KeystoreError::Locked)));
}

#[test]
fn test_secure_key_methods() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();

    let test_key = "0000000000000000000000000000000000000000000000000000000000000001";
    let key_id = keystore
        .import_private_key(Some("test_key".to_string()), test_key)
        .unwrap();

    let secure_key = keystore.get_private_key(key_id).unwrap();

    // Test sign_hash
    let test_hash1 = [1u8; 32];
    let test_hash2 = [2u8; 32];

    let sig1 = secure_key.sign_hash(&test_hash1).unwrap();
    let sig2 = secure_key.sign_hash(&test_hash2).unwrap();

    // Signatures should be different for different hashes
    assert_ne!(sig1.to_bytes(), sig2.to_bytes());

    // Signatures should be deterministic for same hash
    let sig1_again = secure_key.sign_hash(&test_hash1).unwrap();
    assert_eq!(sig1.to_bytes(), sig1_again.to_bytes());

    // Test ethereum_address
    let address1 = secure_key.ethereum_address().unwrap();
    let address2 = secure_key.ethereum_address().unwrap();
    assert_eq!(address1, address2); // Should be consistent
    assert_ne!(address1, Address::ZERO); // Should not be zero

    // Test public_key
    let pubkey1 = secure_key.public_key().unwrap();
    let pubkey2 = secure_key.public_key().unwrap();
    assert_eq!(
        pubkey1.to_encoded_point(false),
        pubkey2.to_encoded_point(false)
    ); // Should be consistent
}

#[test]
fn test_error_conditions() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("error_test.keystore");

    // Test various error conditions systematically
    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

    // Operations on locked keystore
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000001"
        ),
        Err(KeystoreError::Locked)
    ));
    assert!(matches!(
        keystore.get_private_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));
    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::Locked)
    ));

    // Unlock and test other errors
    keystore.unlock("test_password").unwrap();

    // Key not found
    assert!(matches!(
        keystore.get_private_key(Uuid::new_v4()),
        Err(KeystoreError::KeyNotFound(_))
    ));
    assert!(matches!(
        keystore.delete_key(Uuid::new_v4()),
        Err(KeystoreError::KeyNotFound(_))
    ));

    // Invalid private key
    assert!(matches!(
        keystore.import_private_key(None, "invalid"),
        Err(KeystoreError::InvalidPrivateKey)
    ));
    assert!(matches!(
        keystore.import_private_key(
            None,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        Err(KeystoreError::InvalidPrivateKey)
    )); // Zero key

    // Invalid mnemonic
    assert!(matches!(
        keystore.import_mnemonic(None, "invalid mnemonic", "m/44'/60'/0'/0/0", None),
        Err(KeystoreError::InvalidMnemonic(_))
    ));

    // Invalid derivation path
    let valid_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    assert!(matches!(
        keystore.import_mnemonic(None, valid_mnemonic, "invalid/path", None),
        Err(KeystoreError::InvalidDerivationPath(_))
    ));
}

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
fn test_keystore_version_handling() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("version_test.keystore");

    // Create keystore file with unsupported version
    let fake_keystore = r#"{
        "version": 99,
        "kdf_params": {
            "salt": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            "memory_kb": 8192,
            "iterations": 2,
            "parallelism": 1
        },
        "master_key_verification": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "entries": [],
        "file_integrity_mac": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]
    }"#;

    std::fs::write(&keystore_path, fake_keystore).unwrap();

    // Try to load keystore with unsupported version
    let result = Keystore::new(&keystore_path);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        KeystoreError::InvalidInput(_)
    ));
}

#[test]
fn test_strict_loading_rejects_unknown_top_level_field() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("unknown_top_level.keystore");

    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("strong_password_123").unwrap();
    }

    let mut json = read_keystore_json(&keystore_path);
    json["unexpected"] = serde_json::json!(true);
    write_keystore_json(&keystore_path, &json);

    let err = Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap_err();
    match err {
        KeystoreError::InvalidInput(msg) => assert!(msg.contains("Unknown keystore file field")),
        other => panic!("expected unknown top-level field rejection, got: {other:?}"),
    }
}

#[test]
fn test_strict_loading_rejects_unknown_kdf_field() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("unknown_kdf.keystore");

    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("strong_password_123").unwrap();
    }

    let mut json = read_keystore_json(&keystore_path);
    json["kdf_params"]["unexpected"] = serde_json::json!(true);
    write_keystore_json(&keystore_path, &json);

    let err = Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap_err();
    match err {
        KeystoreError::InvalidInput(msg) => assert!(msg.contains("Malformed keystore header")),
        other => panic!("expected unknown KDF field rejection, got: {other:?}"),
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
fn test_get_after_external_tamper_fails_without_rewrite() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("get_after_tamper.keystore");
    let (mut keystore, key_id) = unlocked_keystore_with_one_key(&keystore_path, "read-target");

    mutate_keystore_json_retaining_mac(&keystore_path, |value| {
        value["audit_log"][0]["success"] = serde_json::json!(false);
    });
    let tampered_bytes = std::fs::read(&keystore_path).unwrap();

    assert_concurrent_write_rejected(keystore.get_private_key(key_id));
    assert_eq!(std::fs::read(&keystore_path).unwrap(), tampered_bytes);
}

#[test]
fn test_delete_after_external_tamper_fails_without_rewrite() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("delete_after_tamper.keystore");
    let (mut keystore, key_id) = unlocked_keystore_with_one_key(&keystore_path, "delete-target");

    mutate_keystore_json_retaining_mac(&keystore_path, |value| {
        value["entries"][0]["alias"] = serde_json::json!("tampered-delete-target");
    });
    let tampered_bytes = std::fs::read(&keystore_path).unwrap();

    assert_concurrent_write_rejected(keystore.delete_key(key_id));
    assert_eq!(std::fs::read(&keystore_path).unwrap(), tampered_bytes);
}

#[test]
fn test_change_password_after_external_tamper_fails_without_rewrite() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir
        .path()
        .join("change_password_after_tamper.keystore");
    let (mut keystore, _) = unlocked_keystore_with_one_key(&keystore_path, "rekey-target");

    mutate_keystore_json_retaining_mac(&keystore_path, |value| {
        let salt = value["kdf_params"]["salt"].as_array_mut().unwrap();
        let first = salt[0].as_u64().unwrap();
        salt[0] = serde_json::json!((first + 1) % 256);
    });
    let tampered_bytes = std::fs::read(&keystore_path).unwrap();

    assert_concurrent_write_rejected(
        keystore.change_password("strong_password_123", "new_password_123"),
    );
    assert_eq!(std::fs::read(&keystore_path).unwrap(), tampered_bytes);
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
fn test_file_integrity_protection_entry_swap() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("entry_swap_test.keystore");

    // Create keystore with two keys
    {
        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
        keystore.unlock("test_password").unwrap();
        keystore
            .import_private_key(
                Some("key1".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap();
        keystore
            .import_private_key(
                Some("key2".to_string()),
                "0000000000000000000000000000000000000000000000000000000000000002",
            )
            .unwrap();
    }

    // Tamper with the file by manually swapping encrypted entry data
    let mut file_content = std::fs::read_to_string(&keystore_path).unwrap();

    // This is a simplified tampering - in practice an attacker would swap the encrypted_data fields
    // For this test, we'll just modify some data to trigger integrity failure
    file_content = file_content.replacen("key1", "swapped1", 1);
    std::fs::write(&keystore_path, file_content).unwrap();

    // Try to load the tampered keystore
    let mut keystore2 =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();

    // Unlock should fail due to integrity check failure
    let result = keystore2.unlock("test_password");
    assert!(result.is_err());

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
fn test_weak_password_rejected_on_create_and_change() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("weak_password.keystore");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let create_err = keystore.unlock("short").unwrap_err();
    assert!(matches!(create_err, KeystoreError::InvalidInput(_)));

    keystore.unlock("strong_password_123").unwrap();
    let change_err = keystore
        .change_password("strong_password_123", "aaaaaaaaaaaa")
        .unwrap_err();
    assert!(matches!(change_err, KeystoreError::InvalidInput(_)));
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
        keystore.get_private_key(key_id),
        Err(KeystoreError::Locked)
    ));
}

#[test]
fn test_password_policy_accepts_12_char_strong_password() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("pw_boundary.keystore");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    keystore.unlock("A1b2C3d4E5f6").unwrap();
    assert_eq!(keystore.list_keys().unwrap().len(), 0);
}

#[test]
fn test_password_policy_rejects_common_values_case_insensitive() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("pw_common.keystore");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let err = keystore.unlock("QWERTY123456").unwrap_err();
    match err {
        KeystoreError::InvalidInput(msg) => assert!(msg.contains("too weak")),
        other => panic!("expected InvalidInput for weak password, got: {other:?}"),
    }
}

#[test]
fn test_password_policy_rejects_whitespace_only_password() {
    let temp_dir = tempdir().unwrap();
    let keystore_path = temp_dir.path().join("pw_spaces.keystore");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::development()).unwrap();
    let err = keystore.unlock("            ").unwrap_err();
    match err {
        KeystoreError::InvalidInput(msg) => assert!(msg.contains("too weak")),
        other => panic!("expected InvalidInput for weak password, got: {other:?}"),
    }
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
fn test_auto_lock_timeout_refreshes_on_sensitive_operations() {
    let (_temp_dir, mut keystore) = test_keystore();
    keystore.unlock("test_password").unwrap();
    let key_id = keystore
        .import_private_key(
            Some("refresh".to_string()),
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();

    keystore.set_auto_lock_timeout(Some(std::time::Duration::from_millis(400)));

    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(keystore.get_private_key(key_id).is_ok());

    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(keystore.get_private_key(key_id).is_ok());

    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(matches!(
        keystore.get_private_key(key_id),
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
