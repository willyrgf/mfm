## 1. Cryptographic Primitives & Parameters

* **Argon2id KDF**

  * Uses `m_cost = 65 536 KiB`, `t_cost = 3`, `p_cost = 1`, `output_len = 32` → strong by modern desktop standards, but consider making parameters configurable for low-end devices or future tuning.
  * **Pepper**: introduce a small secret (embedded at build-time or provided at runtime) so that an attacker needs both the keystore file and the pepper to mount an offline brute-force.

* **AES-256-GCM Encryption**

  * Correctly uses a 32-byte key and fresh 96-bit nonce via `OsRng`.
  * **AAD Binding**: include per-entry metadata (e.g. UUID ∥ address bytes) as Additional Authenticated Data to prevent ciphertext-swapping or silent metadata tampering under the same master key.
  * **Password-Change Flow**: if you add a `change_password`, re-encrypt all entries with brand-new nonces.

---

## 2. In-Memory Secrets & Zeroization

* **Master Key**: wrapped in `Zeroizing<Vec<u8>>` and zeroized on `.lock()`.
* **Missing Zeroization**:

  * **Raw hex-decoded private keys** in `import_private_key_hex`: should be stored immediately in `Zeroizing<[u8;32]>` or `Zeroizing<Vec<u8>>`.
  * **Mnemonic seed** (`[u8;64]` from `to_seed()`): copy into a zeroizing buffer and immediately clear the original array.
  * **k256 SecretKey/SigningKey**: wrap or extend these so the private scalar is zeroized on drop (they don’t zeroize by default).

---

## 3. File & Concurrency Hardening

* **File Permissions**: after writing `keystore_v1.json`, explicitly set to `0o600` (owner-read/write) on Unix, or Windows ACLs denying non-owners.
* **Atomic Writes + Locking**: `AtomicFile` prevents partial writes but does *not* serialize concurrent access. Wrap `load_from_disk` + `save_to_disk` in an advisory file lock (e.g. `flock`) to avoid TOCTOU races when multiple processes or threads touch the keystore.

---

## 4. Session Management & Abuse Mitigation

* **Auto-Lock on Inactivity**: use `last_activity_at` to trigger an automatic `.lock()` after a configurable idle timeout (e.g. 5 minutes), either via a background timer or on each public API call.
* **Unlock Rate-Limiting**: throttle repeated `unlock(password)` attempts by enforcing a minimum delay or simple retry counter with exponential backoff—thwarts rapid online brute-force.

---

## 5. Metadata & Integrity

* **Authenticated Metadata**: right now, only the private-key ciphertext is authenticated by GCM. Swapping two entries (ciphertexts and nonces) leaves the JSON valid but mismatches alias∕address. AAD binding or a per-entry HMAC over ID+address+nonce+ciphertext would detect these swaps before decryption.
* **Versioning & Migration**: you lock `version == "1.0.0"`. Plan for future upgrades by including a migration path—e.g., if `version == "1.1.0"`, re-derive new KDF parameters or tighten algorithms.

---

## 6. Key Lifecycle Enhancements

* **Change-Password API**: allow users to rotate the master key by `unlock` → re-derive new master with new password → re-encrypt all entries → overwrite file.
* **Per-Key Encryption Keys (Optional)**: for maximal compartmentalization, derive per-entry keys from the master key (e.g. HKDF(master, entry-id)) so that a breach of one entry’s ciphertext/nonces doesn’t endanger others.

---

### Summary of Critical Next Steps

1. **Zeroize all transient buffers** (seed arrays, hex-decoded keys, k256 secret scalars).
2. **Lock down file access** via strict permissions and file locking.
3. **Bind metadata into encryption** with AAD or HMAC to detect entry swapping.
4. **Harden session management** by auto-locking on idle and rate-limiting unlocks.
5. **Introduce a pepper** to complement the per-keystore salt and foil offline attacks.

Applying these refinements will elevate the library from “secure by design” to “hardened for production.” Let me know if you’d like implementation sketches or code snippets for any of these enhancements!
