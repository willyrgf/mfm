# mfm-keystore

Encrypted MFM key storage and the generic keystore-backed signing provider.

Import signing keys with `mfm_cli keystore import` first, then reference the keystore entry by
non-secret id or label from runtime code. Raw key retrieval and key wrappers are crate-private;
the public surface supports import, public metadata/listing, deletion, and signing.

Each signing call reads the configured unlock file on a blocking worker into protected storage,
rejects content larger than 64 KiB, strips at most one terminal LF or CRLF, and drops the unlock
secret before returning. Paths and provider failures are redacted.

Docs:

- Keystore module and security model: [`src/keystore/mod.rs`](src/keystore/mod.rs)
- Signing provider: [`src/signer.rs`](src/signer.rs)
- Design contract: [`../../docs/design.md`](../../docs/design.md)
- Architecture overview: [`../../docs/architecture.md`](../../docs/architecture.md)
