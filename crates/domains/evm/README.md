# mfm-evm

Secret-free typed EVM Read and transaction Effect contracts, cumulative balance semantics, and
domain-owned route identities. `EvmAddress` and `EvmHash` retain fixed raw bytes and expose
infallible exact byte access; their strict string wires remain lowercase `0x` hexadecimal.
`EvmU256` owns canonical decimal EVM words, while canonical base64 values retain
`CanonicalBytes` directly and chain IDs and gas limits retain `NonZeroU64`.

`EvmTransaction<C, R>::new(binding)` authors one transaction using a static `TransactionRecipe<C>`.
Capability injection expands its `ExecuteEvmTransaction<C, R>` into reservation, preparation,
execution, and Pure outcome projection. `CreateAt<Slot>`, `CallCreatedAt<CallSlot, DeploymentSlot>`,
and `CallAt<Slot>` construct commands from checked plans and the selected successful creation.
`CheckedCallPlan` has no target; `CheckedTargetCallPlan` adds a required checked address for an
ordinary call. Plan and command construction share byte, gas, and fee validation. The reserve
State constructs the complete command during deterministic prepare, before reservation or IO.

Each State replaces the recipe's field using `ContextSlot`, retaining every sibling. The field
accumulates the exact command and reservation, full preparation evidence, full settlement, and
sealed `Created` or `Called` projection. `ReservedEvmTransaction` and `PreparedEvmTransaction`
remain live capability command descriptors. Cumulative fact constructors and decoding preserve
command/reference, nonce-domain, settlement-nonce/hash/action, and projection agreement. Runtime
establishes the provenance of Effect identities. Only authenticated reversion reaches
the typed failure carrying the executed context; internal mismatches retain native causes.
Executable identity commits to stage, implementation version, explicit recipe identity, ordered
slot identities, and outcome mode. Changing a selected same-typed source changes identity.

`custody` owns the reusable asynchronous nonce-reservation and opaque signed-byte retention port.
The live adapter supplies signer/provider IO and PostgreSQL supplies atomic persistence. Custody
returns immutable first prepared winners; Runtime commits transaction settlement before interpretation. The reserve
State EffectId identifies custody throughout the sequence. Raw signed bytes have no serde or debug
surface. No State performs IO, and Runtime has no EVM-specific logic.

Products own named context records, selected recipe connections, ABI decoding, and terminal
report policy. They register the transaction State family through live EVM's pure
`register_evm_transaction_states::<C, R>` helper, separately from explicit IO adapter registration.
Terminal reports use the existing execution facts and settlement outcome. Product failure reports
retain plans once with the executed evidence prefix and derive their failure reason.

`CheckedObservationPlan` checks route and calldata before admission. `ObserveAt<ObservationSlot,
ConfigurationSlot>` supplies the selected completed call's target and receipt anchor, rejecting
cross-field chain/route mismatches before provider entry. `ReadAnchoredContractCall<C, R>`
constructs intent during prepare and retains the exact intent and accepted evidence in
`AnchoredObservationFacts`. Failure contexts also retain rejected, safe-failure, or
integrity-blocked evidence. The live provider algorithm remains outside this crate.

Six balance Read States and one consolidation Pure State implement the cumulative balance contract.
`CollectEvmBalances<K>::new(binding, request)` authors native/token State sequences
from the complete checked request. Root validation compares the request, route and initial work state
with the supplied input before Program construction. There is no runtime selector State or Match
payload. Initial-anchor interpretation advances the ordinary data stage using the checked active
source; the work cursor derives that source from the completed prefix.

Planning retains semantic recovery allowances only. Actual values, complete frames and accumulated
run sizes are checked by their owning boundaries; domain planners provide no lifecycle estimate.

Operational failures retain typed causes and the complete executed State input, including caller
continuations. Read failures retain the exact intent; pending Effect views retain the prepared
command and EffectId. Operational causes distinguish provider, authority and signer unavailability.
Runtime owns phase and settlement authority; States provide no separate failure contextualizer.

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

Checked identity products expose named fields; commands and correlated facts retain checked constructors.

Transaction factories accept `u128` fees. Plans and complete commands share nested `parameters`
with `binding`, `value`, `gas_limit`, and checked `fees` (`priority` and `maximum` decimal strings).

Exact EVM errors implement `ClassifyError`: duplicate-safe Read timeouts, rate limits and unavailable
observations are Retryable; AnchorChanged is InputInvalidated; authenticated integrity blocks and
unavailable sources are Permanent. Transaction provider/authority failures remain OutcomeUnknown,
while signer unavailability before wire retention is Retryable. Authenticated reversion stays a
Permanent domain settlement failure. Classification does not authorize another command.

Callers own handler selection, checkpoint regions and finite allowances. `AnchorChanged` retains
the previous and observed anchors and rejects equal anchors during qualification.

Balance metadata has one checked correlation constructor. `EvmBalanceContext::new` and its native
Runtime decoder retain constructor errors with their metadata location and reviewed cause. Empty
and overlong correlations remain distinct; rejected text is withheld. Ordinary Serde and native
decoding share construction, while the native route avoids Serde's custom-error conversion.
Other checked value families still use their existing decoder boundaries pending their owner cutovers.
