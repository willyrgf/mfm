# mfm-evm

Secret-free typed EVM Read capabilities, cumulative balance State semantics, and domain-owned
`EvmPhysicalTarget`. Six observational Read States and two Pure States implement Program contracts
directly. The native/token fragment is deterministically unrolled per source.

The domain validates chain/route binding, anchors, quantities, decimal scale, and closed evidence.
Authenticated integrity evidence maps to the distinct `IntegrityBlocked` failure. This crate has no
Runtime, Store, live client, signer, nonce, broadcast, or ambient IO dependency.
