# mfm-keystore

Encrypted MFM key storage and the generation-guarded deterministic wallet
signer.

Import signing keys with `mfm_cli keystore import` first, then reference the keystore entry by
non-secret id or label from runtime code. Raw key retrieval and key wrappers are crate-private;
the public surface supports import, public metadata/listing, deletion, and signing.

Decrypted private-key bytes never cross an ordinary by-value `[u8; 32]` constructor.
AES-GCM output is written into a zeroizing allocation and ownership of that same
protected container is transferred into the crate-private `SecureKey`. Every decrypt
error path drops zeroizing buffers without secret-bearing diagnostics. `Keystore` is
deliberately neither `Send` nor `Sync`.

`KeystoreSignerProvider` accepts one exact structurally verified public binding
and one mandatory deployment-supplied generation guard, but is not itself
production qualification. Consuming `qualify` checks that guard and verifies
the actual keystore key, compressed public key, and account on a blocking
worker. Only that path mints the non-cloneable `QualifiedKeystoreSigner`.
The provider and its transitive guard must explicitly declare that attestation
is observational: consuming quota, approval, anti-replay state, billing credit,
rate-limit capacity, or any other externally meaningful semantic state makes
the signer ineligible. Eligibility is checked before the guard or key is
accessed. The `v2` signer opens only an existing keystore and decrypts through a
token-gated crate-private path that never initializes storage, appends the
keystore audit log, acquires a mutation lock, or saves. The superseded audited
key getter is deleted; authenticated historical `v1` `GetPrivateKey` audit
records remain decodable and MAC-covered but have no current producer. The consuming handoff
repeats complete key-identity, generation,
fence, direct-sign exclusion, and Read-eligibility qualification before minting
the opaque `QualifiedReadSigningProvider` accepted by live assembly. The exact
semantic signer identity/contract travels with that handoff. The semantic signer identity is
key-specific and remains stable across compatible physical generations; the
application verifies it against the qualified public key and account. Public
bindings and raw guarded providers cannot mint this authority.

Every signing call rechecks Read eligibility and the caller's expected generation, then awaits the
guard and rechecks eligibility again immediately before any key-file or private-key access. There
is no permissive guard, Effect fallback, or general direct-sign implementation.

The observational cutover changes the provider implementation identity from
`mfm.signing.keystore.rfc6979.v1` to `mfm.signing.keystore.rfc6979.v2`. Existing
physical release histories retain their `v1` certificate and append a `v2`
successor, even when both releases name the same durable key-generation target.

Each guarded call reads the configured unlock file on a blocking worker into
protected storage, rejects content larger than 64 KiB, strips at most one
terminal LF or CRLF, and drops the unlock secret before returning. Signing
requests, results, signatures, copied digests, and unlock material are
non-cloneable or zeroizing as applicable. Paths and provider failures are
redacted; raw signatures and signed envelopes are not persistence surfaces.

Docs:

- Keystore module and security model: [`src/keystore/mod.rs`](src/keystore/mod.rs)
- Signing provider: [`src/signer.rs`](src/signer.rs)
- Design contract: [`../../docs/design.md`](../../docs/design.md)
- Architecture overview: [`../../docs/architecture.md`](../../docs/architecture.md)
