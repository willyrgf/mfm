# mfm-evm

Secret-free typed EVM Read capabilities, cumulative balance State semantics, and domain-owned
`EvmPhysicalTarget`. Six observational Read States and two Pure States implement Program contracts
directly. `CollectEvmBalances<K>` is the checked configured child Operation; it deterministically
unrolls the native/token topology per source without exposing declaration counts or indices.

The State definitions and public reusable `CollectEvmBalances<K>` Operation own their
compiled-product inspection IDs and descriptions. This source metadata is not lowered into Program
or included in content identity.

The Portfolio planner derives each checked target binding once and shares that exact `ContentRef`
with C0 and its configured child Operation. Six exact `CapabilityInjection` pairings clone that
binding for their designated Reads. The current policies add no support States and perform no
provider IO or Runtime registration.

Every source in one collection is observed at one pinned block. `ReadInitialAnchor` pins it,
the balance and decimal reads carry it in their intent, and `ConfirmBalanceAnchor` re-reads the
block that same anchor names. An equal number and hash prove the block still stands; a different
hash proves a reorg replaced it. The confirmation depends on the adapter re-observing the named
block rather than the head, which the `EvmProvider` trait rustdoc states as a provider contract.

The domain validates chain/route binding, anchors, quantities, decimal scale, and closed evidence.
Authenticated integrity evidence maps to the distinct `IntegrityBlocked` failure. This crate has no
Runtime, Store, live client, signer, nonce, broadcast, or ambient IO dependency.
