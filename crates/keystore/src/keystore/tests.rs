/// Comprehensive tests for the current keystore public API.
///
/// These tests validate all public methods and ensure the API works correctly
/// for external consumers using only public interfaces.
use super::*;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use static_assertions::assert_not_impl_any;
use std::path::Path;
use tempfile::tempdir;

#[path = "audit_tests.rs"]
mod audit_tests;
#[path = "filesystem_tests.rs"]
mod filesystem_tests;
#[path = "import_tests.rs"]
mod import_tests;
#[path = "integrity_tests.rs"]
mod integrity_tests;
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
#[path = "surface_tests.rs"]
mod surface_tests;

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
