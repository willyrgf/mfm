# Security Audit Report – `mfm_core::keystore` (v0.1.29)

**Date:** 2025‑05‑30
**Auditor:** OpenAI o3 (cryptography & secure‑coding specialist)

---

## 1. Executive Summary

The `mfm_core::keystore` module is a self‑contained, file‑backed key‑management component for blockchain wallets.
The overall cryptographic design is **sound and modern** (Argon2id + per‑entry AES‑256‑GCM, HKDF‑SHA‑256, zeroisation, rate‑limiting, auto‑lock).
Nevertheless, several **critical and high‑severity issues** were identified that must be addressed before the library can be considered production‑ready.  In addition, a number of medium/low‑severity hardening recommendations are provided.

## 2. Positive Security Aspects

* **Modern primitives:** Argon2id for password‑based KDF, AES‑256‑GCM for authenticated encryption, HKDF‑SHA‑256 for sub‑keys.
* **Per‑entry domain separation:** fresh 32‑byte salt and unique 96‑bit nonce per key.
* **Complete AAD:** UUID + address bind ciphertexts to metadata.
* **Zeroisation policy:** the codebase consciously uses the `zeroize` crate and wraps key material.
* **Defense layers:** verification tag to prevent wrong‑password oracle; exponential back‑off & auto‑lock; JSON versioning & KDF sanity checks; atomic file writes + `fs2` locking.
* **Extensive test‑suite:** >45 unit tests covering happy‑path and error cases.

## 3. Findings & Recommendations

| ID          | Severity     | Area                       | Description                                                                                                                                                       | Recommendation                                                                                                                                                                                              |   |                                                                                     |
| ----------- | ------------ | -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | - | ----------------------------------------------------------------------------------- |
| **KM‑C‑01** | **Critical** | `derive_master_key`        | Returns **plain `Vec<u8>`**, cloning key material outside `Zeroizing`, leaving a non‑zeroised copy on the heap.                                                   | Return `Zeroizing<Vec<u8>>`, or wrap in new‑type that implements `Deref<Target=[u8]> + ZeroizeOnDrop`. Refactor all call‑sites accordingly.                                                                 |   |                                                                                     |
| **KM‑C‑02** | **Critical** | Secret lifetime            | `ZeroizingSigningKey::zeroize` swaps in a dummy key but **clones the original scalar** when `from_slice` parses `[1;32]`, momentarily duplicating sensitive data. | Replace the wrapper with `pub struct SigningKey(Zeroizing<k256::ecdsa::SigningKey>);` and rely on `k256`’s own `ZeroizeOnDrop`. Avoid custom dummy‑swap logic.                                              |   |                                                                                     |
| **KM‑H‑01** | High         | Rate‑limit persistence     | Attempts & timestamps are stored in **world‑writable temp files without integrity**; an attacker can delete or roll‑back to circumvent lock‑out.                  | Addressed: Rate limiting data is now stored inside the encrypted keystore JSON file with proper MAC protection. Legacy files are still used for backward compatibility but now have `0o600` permissions. |   |                                                                                     |
| **KM‑H‑02** | High         | Concurrency / File locking | The lock‑file is manually unlocked. `panic!` or `abort` ⇒ stale lock → application dead‑lock.                                                                     | Use an RAII guard (`FileLockGuard`) so the lock is **automatically released** even on unwinding.                                                                                                            |   |                                                                                     |
| **KM‑H‑03** | High         | Error channels             | Distinct errors: *"InvalidPassword"* vs *"MissingVerificationTag"* vs *"KDF params not initialised"* leak state to adversaries.                                   | Return a **uniform authentication failure** on all password issues; log specifics internally.                                                                                                               |   |                                                                                     |
| **KM‑H‑04** | High         | AAD scope                  | HKDF `info` is constant (`"mfm-keystore-entry-key-v1"`). If the random salt ever repeats (birthday bound), two distinct entries share the same key.               | Include \*\*\`UUID                                                                                                                                                                                          |   | address`** in HKDF `info\` to guarantee domain separation even with salt collision. |
| **KM‑M‑01** | Medium       | RNG assumptions            | `OsRng.try_fill_bytes()` may fail; errors are propagated but **not retried** – causes DOS on transient RNG failure.                                               | Retry with exponential back‑off or abort early with custom error explaining entropy exhaustion.                                                                                                             |   |                                                                                     |
| **KM‑M‑02** | Medium       | Argon2 parameters          | Hard‑coded `m_cost = 128 MiB`, `t_cost = 4`. Excellent for servers but may brick low‑resource mobiles.                                                            | Provide **configurable profiles** (e.g. *interactive*, *moderate*, *sensitive*) and runtime checks.                                                                                                         |   |                                                                                     |
| **KM‑M‑03** | Medium       | Nonce management           | Random 96‑bit nonces satisfy GCM uniqueness with negligible risk, but nothing **prevents accidental reuse**.                                                      | Persist a `u64` counter per‑entry or combine `HKDF(key, "nonce")` + counter to generate deterministic, single‑use nonces.                                                                                   |   |                                                                                     |
| **KM‑M‑04** | Medium       | Directory permissions      | `dirs_next::data_dir()` can resolve to a directory with `0o755`. Secrets may be **readable by other users**.                                                      | Explicitly `create_dir_all` with `0o700` and **verify** resulting permissions.                                                                                                                              |   |                                                                                     |
| **KM‑L‑01** | Low          | Logging / `println!`       | A few unit‑tests `println!` entire JSON blobs – risk of leaking ciphertexts in CI logs.                                                                           | Remove or gate behind `#[cfg(test)]` + `trace` log‑level.                                                                                                                                                   |   |                                                                                     |
| **KM‑L‑02** | Low          | Unwraps in library code    | Several `unwrap()` remain (e.g. `dirs_next::data_dir().unwrap()`) – fine in tests, dangerous in lib.                                                              | Replace with `?` and convert to `KeystoreError::FsError`.                                                                                                                                                   |   |                                                                                     |
| **KM‑I‑01** | Info         | Documentation              | No explicit guidance on **backup / key recovery**.                                                                                                                | Ship CLI examples & doc outlining secure backup, rotation & revocation procedures.                                                                                                                          |   |                                                                                     |

### Detailed Discussion of Critical Findings

#### KM‑C‑01 – Plain `Vec<u8>` Leakage

```rust
fn derive_master_key(...) -> Result<Vec<u8>, KeystoreError> { ... }
```

The function internally zeroises `output_key_material`, but cloning it with `.to_vec()` re‑allocates the secret on the heap **without zeroisation guarantees**.

*Impact:* An attacker with memory read capabilities can recover the master key after it was supposedly cleared.

*Fix:* Return `Zeroizing<Vec<u8>>` directly and update call‑sites:

```rust
pub fn derive_master_key(...) -> Result<zeroize::Zeroizing<Vec<u8>>, KeystoreError>
```

This eliminates the extra copy and leverages `ZeroizeOnDrop`.

#### KM‑C‑02 – Unreliable Zeroisation of `SigningKey`

`ZeroizingSigningKey::zeroize()` replaces the inner scalar with a dummy key, but constructing that dummy **copies the 32‑byte array**, briefly duplicating the secret.  Relying on k256’s built‑in `ZeroizeOnDrop` is safer and simpler.

*Fix:* Wrap the key in `Zeroizing` instead of custom type:

```rust
pub type EphemeralSigningKey = zeroize::Zeroizing<k256::ecdsa::SigningKey>;
```

### Other Noteworthy Points

* **Password verification tag:** good design; consider encrypting random 16‑byte challenge (not constant `"ok"`) to eliminate potential known‑plaintext fingerprints.
* **HKDF salt length:** 32 bytes is strong; no change needed.
* **JSON format:** name → `keystore_v1.json`.  Plan a migration strategy before v2 to avoid ecosystem fragmentation.

## 4. Defense‑in‑Depth Improvements

1. **Integrity seal for rate‑limit & config files** – HMAC(Key=master\_key) over config struct to detect tampering.
2. **Side‑channel‑resistant timing** – pad unlock routine to a constant minimum time to deter online brute‑force.
3. **Secure deletion** – use `AtomicFile::persist_noclobber()` + `sync_all()` and call `fsync` on parent dir to ensure on‑disk durability.
4. **Process isolation** – expose optional integration with Linux `secretmem`/`mprotect(PROT_NONE)` or Windows DPAPI for in‑memory key shielding.

## 5. Testing & CI

* Add **fuzz tests** (cargo‑fuzz) for JSON parsing and the state‑machine (lock/unlock/change‑password).
* Instrument with `miri` + `loom` to detect data races when two threads access the same keystore path.
* Continuous benchmarking of Argon2 params on supported platforms to adjust defaults.

## 6. Documentation & UX

* Provide a **hardening guide** in README: choosing parameters, backup strategy, threat‑model worksheet.
* Offer a migration CLI (`mfm‑keystore migrate ←→ dump`) for future formats.

## 7. Conclusion

The library demonstrates a **solid foundation** and thoughtful security posture.  Addressing the critical/high issues highlighted above, and adopting the recommended hardening measures, will significantly raise its resilience and make it suitable for production deployment in cryptocurrency wallet software.

---

*End of report*
