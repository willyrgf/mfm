# mfm-evm

Secret-free typed EVM Read capabilities, cumulative balance State semantics, and domain-owned
`EvmPhysicalTarget`. Six observational Read States and two Pure States implement Program contracts
directly. `CollectEvmBalances<K>` is the checked configured child Operation; it deterministically
unrolls the native/token topology per source without exposing declaration counts or indices.

Six exact `CapabilityInjection` pairings derive the target binding for their designated Reads. The
current policies add no support States and perform no provider IO or Runtime registration.

The domain validates chain/route binding, anchors, quantities, decimal scale, and closed evidence.
Authenticated integrity evidence maps to the distinct `IntegrityBlocked` failure. This crate has no
Runtime, Store, live client, signer, nonce, broadcast, or ambient IO dependency.
