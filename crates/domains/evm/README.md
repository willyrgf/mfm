# mfm-evm

Secret-free typed EVM Read and transaction Effect contracts, cumulative balance semantics, and
domain-owned route identities. `EvmAddress` and `EvmHash` retain fixed raw bytes and expose
infallible exact byte access; their strict string wires remain lowercase `0x` hexadecimal.
`EvmU256` owns canonical decimal EVM words, while canonical base64 values retain
`CanonicalBytes` directly and chain IDs and gas limits retain `NonZeroU64`.

`EvmTransactionEffect` expands the designated `ExecuteEvmTransaction<K>` into reservation,
preparation, execution, and Pure outcome projection States using ordinary capability hooks.
`create` and `call` factories check payload bounds, nonzero gas, fee ceilings, and ordering.
`EvmTransactionContext<K, T>` carries caller context through checked reserved, prepared, and
executed descriptors. Reservation binds the original command reference and nonce domain;
preparation exposes only the retained wire hash. Execution evidence binds its own EffectId,
nonce, hash, and action. Projection preserves caller context and returns typed success or reversion.

`custody` owns the reusable asynchronous nonce-reservation and opaque signed-byte retention port.
The live adapter supplies signer/provider IO and PostgreSQL supplies atomic persistence. Custody
returns immutable first prepared winners; Journal alone retains transaction settlement. The reserve
State EffectId identifies custody throughout the graph. Raw signed bytes have no serde or debug
surface. No State performs IO, and Runtime has no EVM-specific logic.

When a product needs action-specific chaining, its own Pure State projects the transaction
completion into the next checked command or anchored-call context. EVM supplies no lifecycle
Operation or creation-to-call/call-to-observation bridge: consumers keep deployment, call, and
observation as explicit Program nodes and own their policy.

`EvmAnchoredContractCallRead` and `ReadAnchoredContractCall<K>` carry one exact transaction-route
reference, target, bounded calldata, and block anchor in their own exact intent type. Every
evidence variant carries Runtime's exact canonical intent value ref; returned evidence additionally
retains the anchor and bounded return bytes. The State maps the other variants to the closed
rejected, safe-failure, or integrity-blocked reasons. The live provider algorithm is deliberately
outside this crate.

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
block rather than the head, which the typed `EvmReadProvider` contract requires.

The broad balance intent contains only chain, route, and one of six typed subjects; the subject is
the operation discriminator. `EvmTokenDecimals` admits only 0 through 30. Broad and anchored
capability binders reject evidence carrying any other intent value ref before validating the typed
result relationship. The domain also validates anchors, quantities, byte bounds, action/result
shape, and closed evidence. Authenticated integrity evidence maps to the distinct
`IntegrityBlocked` failure. This crate has no signing dependency and no Runtime, Store, live client,
signer handle, nonce authority, broadcast, or ambient IO dependency.
