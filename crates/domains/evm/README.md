# mfm-evm

Secret-free typed EVM Read capabilities, cumulative balance State semantics, and domain-owned
`EvmPhysicalTarget`. Six observational Read States and two Pure States implement Program contracts
directly. `CollectEvmBalances<K>` is the checked configured child Operation; it deterministically
unrolls the native/token topology per source without exposing declaration counts or indices.

The Portfolio planner derives each checked target binding once and shares that exact `ContentRef`
with C0 and its configured child Operation. Six exact `CapabilityInjection` pairings clone that
binding for their designated Reads. The current policies add no support States and perform no
provider IO or Runtime registration.

The domain validates chain/route binding, anchors, quantities, decimal scale, and closed evidence.
Authenticated integrity evidence maps to the distinct `IntegrityBlocked` failure. This crate has no
Runtime, Store, live client, signer, nonce, broadcast, or ambient IO dependency.
