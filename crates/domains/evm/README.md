# mfm-evm

Secret-free typed EVM Read and transaction Effect contracts, cumulative balance semantics, and
domain-owned route identities. `EvmAddress` and `EvmHash` retain fixed raw bytes and expose
infallible exact byte access; their strict string wires remain lowercase `0x` hexadecimal.
`EvmU256` owns canonical decimal EVM words, while canonical base64 values retain
`CanonicalBytes` directly and chain IDs and gas limits retain `NonZeroU64`.

`EvmTransactionEffect` executes one nonce-free, fixed type-2, empty-access-list command through the
single generic `ExecuteEvmTransaction<K>` State. The action is a private command detail: complete
`create` and `call` factories check byte bounds, nonzero gas, the `u128` fee ceiling, and
priority-fee ordering, and checked deserialization enforces the same contract. One checked
`EvmTransactionContext<K>` admits either complete command.

Settlement evidence is one `EvmTransactionReceipt` plus a closed Created, Called, or Reverted
outcome, together with the exact `EffectId` and nonce. The Effect binder rejects an opposite
successful action before interpretation. A successful State completion preserves caller context
and binding, retains the shared receipt, and projects either the created address or checked call
target. A reversion preserves caller context and that same receipt.

When a product needs action-specific chaining, its own Pure State projects the transaction
completion into the next checked command or anchored-call context. EVM supplies no lifecycle
Operation or creation-to-call/call-to-observation bridge: consumers keep deployment, call, and
observation as explicit Program nodes and own their policy.

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
`IntegrityBlocked` failure. This crate has no signing dependency and no Runtime, Store, live
client, signer handle, nonce authority, broadcast, or ambient IO dependency.
