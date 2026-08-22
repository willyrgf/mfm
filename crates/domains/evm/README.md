# mfm-evm

Secret-free typed EVM Read and transaction Effect contracts, cumulative balance semantics, and
domain-owned route identities. `EvmAddress`, `EvmHash`, and `EvmU256` are the only provider-facing
address, hash, and EVM-word concepts. Their strict transparent wires preserve lowercase hexadecimal
and canonical decimal forms across balance, transaction, receipt, and anchored-call values.

`EvmTransactionEffect` executes one nonce-free, fixed type-2, empty-access-list command through
`ExecuteEvmTransaction<K>`. The command contains a complete chain/route/epoch/wallet binding,
create-or-call action, value, nonzero gas limit, and ordered fee pair. Its settlement evidence binds
the exact `EffectId`, reserved nonce, transaction hash, receipt anchor, and one closed terminal
result. Caller context is retained outside the command and projected unchanged into a minimal
confirmation or revert.

`EvmAnchoredContractCallRead` and `ReadAnchoredContractCall<K>` carry one exact transaction-route
reference, target, bounded calldata, and block anchor. Returned evidence retains only that anchor
and bounded return bytes; the State maps the other evidence variants to the closed rejected,
safe-failure, or integrity-blocked reasons. The live provider algorithm is deliberately outside
this crate.

Six balance Read States and two Pure States continue to implement the cumulative balance contract.
`CollectEvmBalances<K>` deterministically unrolls the native/token topology per source without
exposing declaration counts or indices. Its work cursor derives the active source from the completed
prefix instead of duplicating source values in every stage.

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

The domain validates chain/route binding, anchors, quantities, byte bounds, action/result shape,
decimal scale, and closed evidence. Authenticated integrity evidence maps to the distinct
`IntegrityBlocked` failure. This crate depends only on public signing identity contracts; it has no
Runtime, Store, live client, signer handle, nonce authority, broadcast, or ambient IO dependency.
