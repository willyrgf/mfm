###  Executive Summary

Your keystore is thoughtfully written—it uses modern primitives (Argon2 id, AES-256-GCM, HKDF-SHA-256, k256, Zeroize), stores only encrypted private keys on disk, and includes unit-tests that cover many abuse cases.
However, our audit uncovered several **critical design flaws** that can lead to silent wallet corruption, unlimited offline-brute-force, and private-key leakage in memory.  We also found a number of high/medium-severity issues and opportunities to harden the implementation.

---

## 1 – Findings by Severity

| #        | Severity     | Area                            | Finding                                                                                                                                                                                                                                   |
| -------- | ------------ | ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **F-1**  | **Critical** | `unlock()` / authentication     | Unlocking **never verifies** the password; any string yields a “master key”. Keys imported after a wrong unlock are irreversibly encrypted with a bogus key (DoS / silent data loss).                                                     |
| **F-2**  | **Critical** | Rate-limiting                   | `increment_unlock_attempts()` is only called for *empty* passwords. Wrong passwords therefore do **not** back-off, enabling unlimited offline brute-force.                                                                                |
| **F-3**  | **Critical** | In-memory secrecy               | `ZeroizingSigningKey::zeroize()` zeroises an unused temp buffer, **not the secret scalar**. The raw `SigningKey` returned by `get_signer()` is never zeroised and can linger in memory.                                                   |
| **F-4**  | **High**     | Cryptographic domain separation | `derive_entry_key()` uses HKDF with **empty salt**; this gives no domain separation between different master keys and removes HKDF’s collision-resistance properties.                                                                     |
| **F-5**  | **High**     | Pepper misuse                   | `DEFAULT_PEPPER` is hard-coded and public. Appending a known string to the password adds no entropy but doubles the password’s memory footprint.                                                                                          |
| **F-6**  | **High**     | Configurable KDF output         | `output_len` is user-configurable; values < 32 bytes produce AES-256 keys with only *n* × 8 bits of entropy.                                                                                                                              |
| **F-7**  | **High**     | File-system locking             | You lock the **old** file handle, then atomically replace the path (`atomicwrites`). The lock no longer protects the new file—another process can open it concurrently, causing torn reads or time-of-check/time-of-use (TOCTOU) attacks. |
| **F-8**  | **Medium**   | Nonce-misuse resistance         | A 96-bit random nonce is safe, but a defensive design would *derive* the nonce from `(masterKey, id, counter)` to make reuse cryptographically impossible.                                                                                |
| **F-9**  | **Medium**   | Thread safety                   | `Keystore` is not `Send + Sync`; concurrent calls can produce inconsistent state (e.g., `entries` race with `auto_lock`).                                                                                                                 |
| **F-10** | **Medium**   | Memory locking                  | Secrets can be swapped to disk; consider `mlock` / `memmap2` or OS-specific “locked page” APIs.                                                                                                                                           |
| **F-11** | **Low**      | Cross-platform perms            | `chmod 600` only runs on Unix; Windows files inherit directory ACLs and may be world-readable.                                                                                                                                            |
| **F-12** | **Low**      | Error wording                   | Detailed `AesGcm` errors leak whether decryption failed vs. AAD mismatch—this can be an oracle.                                                                                                                                           |
| **F-13** | **Info**     | Code hygiene                    | Many intermediate `Vec<u8>` copies of secrets are not wrapped in `Zeroizing`, increasing the window for memory scraping.                                                                                                                  |

---

## 2 – Detailed Discussion & Recommendations

### F-1 / F-2 — Password Verification & Brute-Force

| Problem | Any password derives a key, so `unlock()` always “succeeds”; rate-limiting never trips.                                                                                                                                                                                                                                               |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Fix** | 1. Add a *password-verification tag* to the keystore header: e.g. `tag = AES-GCM(MK, nonce=0, AAD="verify", plaintext="ok")`.  <br>2. On `unlock()`, decrypt the tag; failure → wrong password, increment attempts, refuse to mark keystore `is_unlocked`. <br>3. Call `increment_unlock_attempts()` for **any** failed verification. |

### F-3 — Zeroization Bugs

* Implement `Zeroize` correctly:

```rust
impl Zeroize for ZeroizingSigningKey {
    fn zeroize(&mut self) {
        use k256::elliptic_curve::SecretKey;
        // Convert to mutable bytes, zero them, then force re-randomisation of scalar
        let mut bytes = self.0.to_bytes();
        bytes.zeroize();
    }
}
```

* Expose only opaque signer objects; never hand out raw `SigningKey`.
  Provide a `Signer` trait impl that signs internally and zeroises on drop.

### F-4 — HKDF with Empty Salt

Deriving keys with `salt=""` loses one of HKDF’s two inputs and weakens collision-resistance.

*Use*: `Salt = H(master_key || "entry-key-salt")` or simply reuse the per-keystore `salt`.
Alternatively, prepend a fixed domain string in `info` to separate contexts.

### F-5 — Pepper

A global, published pepper is equivalent to none. Either:

* Remove it and clearly document that **only the user’s password** protects the keystore, *or*
* Generate a **per-installation random pepper** on first run and store it in the OS key-chain/environment.

### F-6 — KDF Output Length

Force `output_len >= 32` (or 64 if you may switch to XChaCha20-Poly1305 later).

### F-7 — File Lock vs. Atomic Rename

Lock a separate `.lock` file **in the target directory**, or switch to `flock`/`fcntl` on the final path *after* the rename.  Document that only one process may write concurrently.

### F-8 — Nonce Reuse

Nonce collisions are catastrophic for GCM.  You already store the nonce; to be extra-safe:

```text
nonce = HKDF(master_key, "nonce" || id || counter)[..12]
```

where `counter` is a per-entry u64 incremented on each re-encryption.

### F-9 — Thread Safety

Wrap mutable state in `RwLock<KeystoreInner>` or require the caller to take an `&mut` reference *and* mark `KeystoreInner: !Send`.

### F-10 — Memory Locking

Consider [`secrecy::Secret`](https://docs.rs/secrecy) + [`mlock`](https://docs.rs/mlock) on Unix and `VirtualLock` on Windows.

### F-11 — Windows File ACLs

Call `winapi::um::aclapi::SetNamedSecurityInfoW` to grant only `FILE_GENERIC_READ | FILE_GENERIC_WRITE` to the current SID.

### F-12 — Error Messages

Return a generic `KeystoreError::InvalidPassword` for decryption / AAD failures.

### F-13 — Additional Hardening

* **MAC then encrypt** metadata (id, address) to detect bit-flips early.
* **Constant-time comparisons** for nonce, UUID matches.
* Use `rand::rngs::EntropyRng` + `getrandom` for embedded/SGX builds.
* CI: `cargo-audit`, `cargo-deny` and compile with `RUSTFLAGS="-C force-frame-pointers=yes"` for better post-mortem analysis.

