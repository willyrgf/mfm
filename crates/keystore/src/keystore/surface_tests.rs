use super::*;

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
    let keystore_path = keystore.path.to_string_lossy().into_owned();
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
        keystore_path.as_str(),
    ] {
        assert!(
            !debug.contains(forbidden),
            "keystore debug output leaked `{forbidden}`: {debug}"
        );
    }
}
