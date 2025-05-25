# In‑Process Rust Keystore ‑ Security Audit Report

## Scope

Audit of the `src/keystore` module (crate **mfm\_core** v0.1.29). The library stores Ethereum‐compatible wallet keys in a local JSON file and encrypts each private key with AES‑256‑GCM, using an Argon2‑derived master key.

## Executive Summary

The overall cryptographic design is modern and mostly correct. However, our review uncovered several implementation‑level weaknesses that could leak secret material, weaken password security, or lock users out of their wallets. We rate two issues **High severity** and four **Medium severity**. These should be addressed before production release.

## Detailed Findings

| ID     | Severity   | Title                                              | Description                                                                                                                                                              | Recommendation                                                                                                 |
| ------ | ---------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| **1**  | **HIGH**   | Secret scalar not zero‑wiped                       | `ZeroizingSigningKey::zeroize` only erases a *copy* returned by `to_bytes()`. The real scalar inside `k256::SigningKey` remains in memory.                               | Derive `Zeroize`/`ZeroizeOnDrop` for the inner `SecretKey` or call `self.0.as_nonzero_scalar_mut().zeroize()`. |
| **2**  | **HIGH**   | Legacy keystore migration can brick wallets        | When unlocking a pre‑1.0.0 file lacking a verification tag, the code **accepts any password**, then writes a new tag with that password. All genuine passwords are lost. | Removes any legacy handling code, since we're not supporting backward compatibility.   |
| **3**  | **MEDIUM** | Rate‑limiting resets on restart                    | `unlock_attempts` lives only in RAM; attackers can bypass delays by restarting the process.                                                                              | Persist attempt counter + timestamp to disk, or use OS keystore‑backed lockout.                                |
| **4**  | **MEDIUM** | Static public pepper offers no security            | `DEFAULT_PEPPER` is identical in every build and publicly known.                                                                                                         | Removing it entirely. Argon2's high memory and time costs, combined with a strong unique salt, already provide significant resistance to offline brute-force attacks.   |
| **5**  | **MEDIUM** | Argon2 parameters on the low side                  | Default 64 MiB / 3 passes is crackable (<50 ms on modern GPU).                                                                                                           | Follow OWASP 2024 guidance (≥128 MiB, ≥4 passes); using as default and never allow user‑supplied values below those minima.         |
| **6**  | **MEDIUM** | Windows ACLs not enforced                          | `save_to_disk()` leaves a TODO; other users could read the keystore on Windows.                                                                                          | Remove all Windows support from the code.                                                         |
| **7**  | **LOW**    | Deterministic HKDF salt                            | Salt = `Keccak(master_key‖"entry-key-salt")`; HKDF recommends random, public salt.                                                                                       | Include `entry_uuid` or 32 B random salt for each entry.                                                       |
| **8**  | **LOW**    | Non‑constant‑time tag compare                      | `decrypted == b"ok"` may leak timing.                                                                                                                                    | Use `subtle::ConstantTimeEq`.                                                                                  |
| **9**  | **LOW**    | Limited Rate Limiting Scope | Rate limiting increments only on failed password attempts, not on other failures (e.g., verification tag mismatches), leaving gaps in brute-force protection. | Ensure rate limit works for any decryption failures.                                                                  |
| **9**  | **LOW**    | Multiple `Zeroizing` clones extend secret lifetime | `change_password` clones the old master key several times.                                                                                                               | Hold one mutable copy; overwrite it in place.                                                                  |
| **10** | **INFO**   | Keystore metadata left unencrypted                 | UUIDs, addresses, timestamps are in plaintext.                                                                                                                           | Use full‑file encryption.                                                                        |
| **11** | **INFO**   | Dead dependency (`reqwest`)                        | Unused crate inflates binary and attack surface.                                                                                                                         | Remove from `Cargo.toml`.                                                                                      |
| **11** | **INFO**   | Not achieved full test coverage | Still having parts of the implementations without unit tests coverage on it. | Find gaps on tests and add full cover unit test for the keystore module.                                                                                      |


## Additional recommended improvements

* **No mlock/VirtualLock** – Keys can be paged to disk; evaluate OS memory‑locking APIs.
* **Thread safety** – `Keystore` is not `Send + Sync`; race conditions possible in multi‑threaded apps.
* **Unit‑test KDF parameters** – Tests lower Argon2 cost drastically; ensure production builds restore strong defaults via `cfg(not(test))`.
* **KDF versioning** – Add `kdf_version` field so future parameter bumps won’t break old files.

