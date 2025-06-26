# Security Audit Implementation TODO - Post Updated C_SECURITY_AUDIT.md Review

**Audit Date**: June 26, 2025  
**Version Reviewed**: 0.1.29  
**Status**: Implementation-specific vulnerabilities identified in otherwise solid codebase

---

## 🎉 **Acknowledged Security Strengths** 

The audit specifically recognizes our strong security foundation:
- ✅ Extensive use of `zeroize` / `zeroize_on_drop` 
- ✅ MAC covers KDF parameters + algorithm string + protected data (eliminates downgrade attacks)
- ✅ `!Send + !Sync` enforced via `PhantomData<*const ()>` (prevents cross-thread use)
- ✅ File writes use atomic rename plus exclusive lock (excellent robustness)
- ✅ AES-GCM AAD binds entry UUID and Ethereum address (thwarts key-swap attacks)

**Overall Assessment**: *"With the fixes above applied, the library provides a solid local keystore suitable for production wallets"*

---

## 🚨 **CRITICAL PRIORITY** (Production Blockers)

### C-1: Residual Key Material in Cryptographic Objects
- [ ] **Fix `aes-gcm` key zeroization**
  - Wrap `Aes256Gcm::new(Key::from_slice())` in zeroizing newtype
  - Implement custom `Drop + Zeroize` for AES key material
  - Consider switching to `aes-gcm-siv` or `ring::aead::LessSafeKey` which expose zeroizing constructors
  
- [ ] **Fix `ring::hmac::Key` zeroization**  
  - Wrap `ring::hmac::Key` in zeroizing newtype with custom Drop
  - Ensure all HMAC key material is explicitly wiped
  - Document that ALL ephemeral keying material must be wiped

**Impact**: Memory forensics can recover master keys, per-entry keys, audit-signing keys after lock/shutdown

### C-2: Verification-nonce Outside MAC Protection  
- [ ] **Include `verification_nonce` in MAC input**
  - Modify MAC calculation to include `AuthenticatedKeystoreEnvelope::verification_nonce`
  - Update MAC verification to include nonce in protected data
  - Ensure backward compatibility or migration strategy
  
**Impact**: Attacker can flip verification nonce → permanent keystore DoS (ransomware scenario)

### C-4: Deterministic Nonce Derivation Breaks Forward Secrecy
- [ ] **Add independent random salt for nonce derivation**
  - Generate additional 256-bit random secret at keystore creation  
  - Store this salt MAC-protected in envelope
  - Use both master key AND random salt for HMAC nonce derivation
  - **Alternative**: Switch to `rand::OsRng` + collision detection (simpler)
  
**Impact**: Master key compromise allows re-computing all historical nonces → known-plaintext attacks

---

## ⚠️ **HIGH PRIORITY** (Production Impact)

### H-3: Unlocked File Reads (Race Conditions)
- [ ] **Add shared locks for reading**
  - Take shared lock when reading keystore files
  - Keep lock for entire read-parse lifetime
  - Ensure compatibility with existing exclusive write locks
  - Add thorough concurrency tests
  
**Impact**: Concurrent reads can see partial writes → crash/silent data loss

---

## 🔧 **MEDIUM PRIORITY** (Hardening)

### M-1: Weak Zeroization Pattern
- [ ] **Fix `ZeroizingSigningKey::zeroize()` constant pattern**
  - Replace `[1;32]` dummy key with `rand::OsRng` garbage
  - Call `zeroize()` twice or use random overwrite
  
### M-2: u64 Counter Overflow
- [ ] **Extend nonce counter to u128
  - Switch from `u64` to `u128` global nonce counter
  
### M-3: Weak Session Concurrency Detection
- [ ] **Improve session concurrency heuristics**
  - Replace timing-based detection with thread-id/process-id binding
  - Use atomic fencing instead of elapsed time checks
  
### M-4: Small Salt Size  
- [ ] **Increase master key salt to 32 bytes**
  - Bump from 16 to 32 random bytes for new keystores
  - Maintain backward compatibility for existing keystores
  

## 🎯 **Implementation Priority**

### Phase 1: Critical Security Fixes (Week 1)
1. **C-1**: Fix cryptographic object zeroization  
2. **C-2**: Include verification-nonce in MAC
3. **C-4**: Add independent nonce derivation salt

### Phase 2: High-Impact Fixes (Week 2)  
2. **H-3**: Add shared file locks for reads
3. **H-1**: Add memory usage profiles/checks

### Phase 3: Hardening & Polish (Week 3)
1. **M-1 to M-5**: Medium-priority hardening fixes
2. **I-1 to I-5**: Code quality improvements
3. Comprehensive testing and validation

---

## 📊 **Risk Assessment**

**Current State**: *"Solid local keystore foundation with implementation-specific vulnerabilities"*

**With Critical Fixes**: Production-ready for local wallet applications  
**Without Critical Fixes**: Risk of key compromise through memory forensics and DoS attacks

**Target**: Address C-1, C-2, C-4, H-2, H-3 for production deployment readiness

---

## 🔗 **Quick Reference Links**

- **Audit Source**: `/mfm/C_SECURITY_AUDIT.md`
- **Critical Issues**: C-1 (zeroization), C-2 (MAC coverage), C-4 (nonce derivation)
- **Patch Checklist**: Lines 77-86 in audit report
- **Positive Acknowledgments**: Lines 60-65 (validates our previous security work)

---

*This TODO reflects findings from the June 26, 2025 security audit of our improved codebase. The audit acknowledges significant security improvements while identifying specific implementation vulnerabilities requiring attention for production readiness.*
