# mfm-signing

`mfm-signing` owns checked transient recoverable-secp256k1 public keys, exact digest/signature
values, public recovery, and the key- and purpose-bound `Secp256k1Signer` interface. A request
contains only one exact 32-byte digest; the selected handle already fixes its immutable purpose and
public key.

These values have no serde or diagnostic surface. Persisted account identity, secret scalars,
custody-provider selection, EVM address derivation, transaction encoding, and provider IO remain
outside this crate.
