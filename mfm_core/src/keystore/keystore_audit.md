# Security Audit Report – `mfm_core::keystore`

*Author: Independent cryptographic‑security consultant*  
*Date: 25 May 2025*

---

## 1. Executive Summary

The audited Rust module implements an **in‑process keystore** that stores blockchain wallet private keys encrypted with per‑entry Argon2‑derived keys and AES‑256‑GCM. Overall the design is **sound at a high level**, but several security issues were identified that could allow an attacker with local‑file access or crash‑dump access to recover secret material or mount effective offline‑password‑guessing attacks. None of the findings appear exploitable for *remote* compromise, yet they could lead to loss of funds in the event of device compromise.

A total of **14 findings** were recorded:

| ID  | Severity | Title |
|-----|----------|-------|
| F‑1 | **High** | Static global “pepper” provides no real entropy and creates migration risk |
| F‑2 | **High** | Private metadata (aliases, addresses, KDF params) stored in clear text |
| F‑3 | **Medium** | `SigningKey` zeroisation is ineffective – scalar stays in memory |
| F‑4 | **Medium** | Derived keys returned as `Vec<u8>` outside a zeroising wrapper |
| F‑5 | **Medium** | No password‑strength policy or rate limiting – offline attack is cheap |
| F‑6 | **Medium** | Windows ACL hardening left as TODO |
| F‑7 | **Medium** | Error variants leak oracle information (Argon2 vs AES failure) |
| F‑8 | **Low** | Missing integrity / authenticity check for the keystore file |
| F‑9 | **Low** | Predictable keystore location aids targeted malware |
| F‑10 | **Low** | Potential truncation risk during atomic write not rolled back |
| F‑11 | **Low** | Memory pages with secrets can be swapped to disk (no `mlock`) |
| F‑12 | **Low** | `password_with_pepper` uses `from_utf8().unwrap()` – latent panic |
| F‑13 | **Info** | Argon2 parameters may be excessive for low‑RAM devices |
| F‑14 | **Info** | Versioning schema lacks migration strategy |

Details and recommendations follow.

---

## 2. Architectural Overview

```
┌─────────────────────────┐      derive_key_for_entry()      ┌───────────────┐
│  KeystoreFile (JSON)    │ ───────────────────────────────▶ │ Argon2id KDF   │
└──────────┬──────────────┘                                  └──────┬────────┘
           │ entries (id, alias, address, encrypted_pk, salt, …)    │ key
           │                                                         ▼
           │                                       ┌───────────────────────────┐
           └──────────────────────────────────────▶│ AES‑256‑GCM (pk encrypt) │
                                                   └───────────────────────────┘
```

Each entry is encrypted independently with a key derived **only** from the per‑entry password + static pepper + 16‑byte random salt. The resulting ciphertext, nonce, salt and parameters are stored verbatim in the same JSON file.

---

## 3. Detailed Findings & Mitigations

### F‑1 (High) – Static build‑time pepper offers no protection

```rust
const DEFAULT_PEPPER: &'static [u8] = b"mfm_keystore_pepper_v1";
…
password_with_pepper.push_str(std::str::from_utf8(self.pepper).unwrap_or(""));
```

*Issue.* Because the pepper is hard‑coded and publicly known, it adds **zero entropy** but complicates future migrations (you cannot change it without invalidating all keys).

*Recommendation.* Remove the pepper entirely **or** replace it with a *per‑installation* random value stored in the OS keychain / TPM / secure enclave. Salt already prevents rainbow‑table attacks.

---

### F‑2 (High) – Metadata stored in clear text

*Issue.* `alias`, `address`, `salt`, `argon2` parameters and the 12‑byte `nonce` are written unencrypted. Possession of the file therefore discloses the wallet addresses and enables unconstrained offline dictionary attacks.

*Recommendation.* Adopt a **two‑layer design**:
1. Derive a *master key* from a **single** unlock password.
2. Encrypt the entire `KeystoreFile` blob (or at least the per‑entry headers) with this master key.

This keeps metadata confidential and allows global rate‑limiting.

---

### F‑3 (Medium) – `SigningKey` zeroisation is ineffective

```rust
impl Zeroize for ZeroizingSigningKey {
    fn zeroize(&mut self) {
        let mut bytes = self.0.to_bytes();
        bytes.zeroize();
    }
}
```

Calling `to_bytes()` creates **a copy**; the internal scalar inside `SigningKey` remains. In addition, `SigningKey` does **not** implement `Zeroize` itself.

*Recommendation.*
- Store the secret as `k256::SecretKey` (which implements `ZeroizeOnDrop`) and construct `SigningKey` only when required for a signing operation.
- Alternatively, wrap the scalar in your own `SecretKeyBytes([u8;32])` that implements `Zeroize`.

---

### F‑4 (Medium) – Derived keys live outside a zeroising container

`derive_key_for_entry()` returns `Vec<u8>` that is then copied (`to_vec()`) by some callers. Only some call‑sites wrap it with `Zeroizing`. Missed callers will leave keys in heap memory after use.

*Recommendation.* Return `Zeroizing<Vec<u8>>` directly and propagate the wrapper. Switch to the ✨`zeroize::Zeroizing`✨ type alias for ergonomics.

---

### F‑5 (Medium) – No password policy or rate limiting

*Issue.* Empty strings are rejected, but **"123"** is accepted and an attacker can perform unlimited offline guesses with parameters visible in the file.

*Recommendation.*
- Enforce a *minimum entropy* policy (e.g. length ≥ 12 and at least 3 character classes).
- Consider scrypt/Argon2 cost‑factor auto‑tuning.
- Re‑introduce exponential back‑off on **online** unlock attempts when/if a global master password is restored (see F‑2).

---

### F‑6 (Medium) – Windows ACL hardening missing

Unix permissions are set to `0o600`, but `#[cfg(windows)]` block is `TODO`.

*Recommendation.* Call *SetNamedSecurityInfoW* or use `windows‑acl` crate to grant `FILE_GENERIC_READ|WRITE` only to the current SID.

---

### F‑7 (Medium) – Error messages leak oracle information

*Issue.* Decryption failures after wrong password yield either `Argon2Error` (if KDF fails) or `InvalidPassword` (if AES‑GCM tag mismatch). An attacker can therefore test guesses offline and distinguish malformed passwords from format errors.

*Recommendation.* Return a **single** opaque error (`WrongCredentials`) for any failure stemming from user input.

---

### F‑8 (Low) – No file integrity/HMAC

If an attacker flips a single bit in the stored *ciphertext*, AES‑GCM detects it, but flipping metadata (e.g. `m_cost`) causes un‑checked behaviour and denial‑of‑service.

*Recommendation.* Add a *file‑level* MAC (or wrap JSON inside an AEAD envelope with associated data = constant string).

---

### F‑9 (Low) – Predictable keystore path

`dirs_next::data_dir()/"mfm/keystore_v1.json"` allows malware to watch or exfiltrate the file.

*Recommendation.* Allow applications to supply a **custom path** outside default roaming profile and/or add an option to store inside OS key storage (Keychain, Credential Vault).

---

### F‑10 (Low) – Atomic write can leave truncated file on power loss

`AtomicFile::write()` first writes to `*.swap` then renames. On some filesystems rename is not atomic across crashes, leaving the keystore missing.

*Recommendation.* Keep timestamped *back‑ups* and verify after write.

---

### F‑11 (Low) – Secrets may be swapped to disk

*Recommendation.* Use `memsec`, `ring::constant_time::verify_slices_are_equal`, or OS calls (`mlock`, `VirtualLock`) to pin critical buffers.

---

### F‑12 (Low) – Potential panic in `from_utf8().unwrap()`

If `DEFAULT_PEPPER` is ever set to non‑UTF‑8 bytes, `.unwrap()` will panic and kill the process.

*Recommendation.* Replace with `from_utf8().expect("pepper must be UTF‑8")` at compile time or avoid conversion.

---

### F‑13 (Info) – Argon2 parameters

`m_cost=65 536 KiB` (≈ 64 MiB) is excellent for desktops but can starve embedded devices. Make them **configurable** at runtime or auto‑tune once.

---

### F‑14 (Info) – Version migration

`version:"1.0.0"` is checked but no migration path is implemented. Future changes will fail with `InvalidFormat`.

*Recommendation.* Plan a **semver‑aware migration layer** now.

---

## 4. Positive Observations

- Uses **Argon2id v1.3** with independent 16‑byte salts – good choice.
- Employs **AES‑256‑GCM** with per‑entry random nonces and AAD bound to UUID+address.
- Secrets inside `Vec<u8>` are wrapped in `Zeroizing` in most critical paths.
- File access is guarded by `fs2` advisory locks and `AtomicFile` to prevent torn writes.
- Comprehensive unit‑test suite exercises typical flows.

---

## 5. Recommended Roadmap

| Phase | Action Items |
|-------|--------------|
| **Immediate** | Remove static pepper, fix zeroisation of signing keys, collapse error variants, add Windows ACLs |
| Short Term | Encrypt metadata with master key, enforce password policy, add integrity MAC |
| Long Term | Memory‑lock secrets, introduce keystore migration layer, configurable KDF tuning |

---

## 6. Conclusion

While the keystore achieves baseline cryptographic confidentiality for stored private keys, addressing the above findings will substantially raise resistance against local compromise and offline attacks. The most critical tasks are **eliminating the static pepper** and **securing metadata**, followed by rigorous secret‑zeroisation and consistent error handling.

Please feel free to reach out for clarification or re‑assessment once fixes are implemented.
