# mfm-signing

`mfm-signing` owns checked public signing values, exact transient digest/signature types, public
recoverable-secp256k1 verification, and the key- and purpose-bound `Signer` interface. A signer
request contains only one exact 32-byte digest; the selected handle already fixes its immutable
purpose, signer route, algorithm, and public key instance.

`PublicSigningKey` and `PublicSignerIdentity` are secret-free content-addressed MFM values. Secret
scalars, signer implementation state, EVM address derivation, transaction encoding, and provider
IO remain outside this crate.
