# Keystore Test Coverage TODO List

This document outlines areas of the `mfm_core/src/keystore/mod.rs` implementation that are not currently covered or fully covered by `mfm_core/src/keystore/tests.rs`.

## High Priority

### 1. `Keystore::change_password` Functionality

The `change_password` method is critical for security and is currently untested.
It involves:
- Verifying the old password.
- Generating new KDF parameters.
- Deriving a new master key.
- Decrypting all existing private keys with the old master key.
- Re-encrypting all private keys with the new master key.
- Updating the master KDF parameters and verification tag in the keystore file.

**TODO:**
- Add a test case for successful password change.
- Add a test case for changing password with an incorrect old password.
- Add a test case to verify that keys are still accessible after a password change.
- Add a test case to ensure the new KDF parameters and verification tag are correctly persisted and used.

### 2. `KeystoreConfig::auto_lock_timeout` and Session Management

The automatic locking mechanism (`check_auto_lock` and `update_activity_timestamp`) is not explicitly tested. This feature is important for security by ensuring the keystore locks itself after a period of inactivity.

**TODO:**
- Add a test case to verify that the keystore automatically locks after `auto_lock_timeout` has passed since the last activity.
- Add a test case to ensure that activity (e.g., `import_private_key_hex`, `get_signer`, `delete_key`) resets the `last_activity_at` timestamp, preventing auto-lock.
- Test that `list_keys` also updates the activity timestamp.

## Medium Priority

### 3. Robustness of File I/O (`save_to_disk`, `load_from_disk`)

While basic loading and saving are covered, explicit tests for error conditions related to file system operations are limited.

**TODO:**
- Test `save_to_disk` failure scenarios (e.g., permissions denied, disk full - though this might be hard to simulate in a unit test).
- Test `load_from_disk` with a corrupted keystore file (e.g., malformed JSON, invalid version string not handled by migration logic).
- Test concurrent access to the keystore file (though `fs2` locking helps, explicit tests would confirm behavior).

### 4. Comprehensive `KeystoreError` Coverage

Not all error variants defined in `mfm_core/src/keystore/error.rs` are explicitly triggered and asserted in the existing tests.

**TODO:**
- Identify and create test cases that specifically trigger and assert each `KeystoreError` variant (e.g., `AesGcm` errors, `Bip32` errors, `DerivationFailed` in other contexts, `MissingVerificationTag`, `UnsupportedKdf` for future versions, `InvalidFormat` for various decoding/parsing failures).

## Low Priority

### 5. Explicit Testing for Internal Key Derivation and Encryption/Decryption Helpers

Functions like `encrypt_pk`, `decrypt_pk`, `derive_entry_key`, `derive_master_key`, and `create_aad` are implicitly tested through higher-level operations (`import`, `get_signer`). However, explicit unit tests for these functions could provide more granular coverage and ensure their correctness in isolation.

**TODO:**
- Consider adding isolated unit tests for `encrypt_pk` and `decrypt_pk` with known inputs/outputs (if feasible without exposing sensitive data).
- Consider adding isolated unit tests for `derive_entry_key` and `derive_master_key` to verify key derivation logic.
- Add a test for `create_aad` to ensure it generates the correct AAD.
