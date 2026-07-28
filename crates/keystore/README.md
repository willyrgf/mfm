# mfm-keystore

Encrypted MFM key storage and the generation-guarded deterministic wallet
signer.

Import signing keys with `mfm_cli keystore import` first, then reference the keystore entry by
non-secret id or label from runtime code. Raw key retrieval and key wrappers are crate-private;
the public surface supports import, public metadata/listing, deletion, and signing.

`KeystoreSignerProvider` accepts one exact verified public binding and one
mandatory deployment-supplied generation guard. The binding fixes the provider,
algorithm, deterministic profile, account, durable wallet generation, fence
attestation, and provider/ACL proof excluding general direct-sign access. Every
signing call checks the caller's expected generation and awaits that guard
immediately before any key-file or private-key access. There is no permissive
guard and the provider does not implement the general direct-sign traits.

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
