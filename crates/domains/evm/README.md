# mfm-evm

Secret-free typed EVM Read and transaction Effect contracts, cumulative balance semantics, and
domain-owned route identities. `EvmAddress`, `EvmHash`, and `EvmU256` are the only provider-facing
address, hash, and EVM-word concepts. Their strict transparent wires preserve lowercase hexadecimal
and canonical decimal forms across balance, transaction, receipt, and anchored-call values.

`EvmTransactionEffect` executes one nonce-free, fixed type-2, empty-access-list command shared by
the independent `CreateEvmContract<K>` and `CallEvmContract<K>` States. Their checked contexts admit
only the matching Create or Call action. Creation completion exposes the command binding, receipt
anchor, created address, and transaction hash; call completion exposes the binding, receipt anchor,
target, and hash. Their action-specific failures retain caller context and the minimal revert, while
settlement evidence keeps its exact `EffectId`, nonce, confirmation, and revert contract unchanged.

`EvmContractCallContext::for_created_contract` consumes a confirmed creation and fixes the next
call's binding and target from that evidence. `AnchoredContractCallContext::for_confirmed_call`
consumes a confirmed call and fixes the observation route, target, and receipt anchor. The general
checked constructors remain available for explicit alternate policy. EVM supplies no lifecycle
Operation: consumers keep deployment, call, and observation as explicit Program nodes.

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
