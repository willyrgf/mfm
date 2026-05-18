# mfm_core

Security-sensitive primitives for MFM (keystore, Ethereum private-key signing, and config models).

Configuration models never load plaintext private-key files. Import signing keys with
`mfm_cli keystore import` first, then reference the keystore entry by non-secret id or label from
runtime code.

Docs:

- Ethereum private-key helpers: [`src/crypto.rs`](src/crypto.rs)
- Keystore module: [`src/keystore/README.md`](src/keystore/README.md)
- Design contract: [`../../docs/design.md`](../../docs/design.md)
- Architecture overview: [`../../docs/architecture.md`](../../docs/architecture.md)
