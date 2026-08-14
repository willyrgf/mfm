# Capability and binding inventory

This is the checked-in U3/U5 preflight artifact for the two admitted entry points. It records the
provider assertion boundary, the finite evidence algebra, and the immutable binding material that
can survive in durable history. No secret, credential, raw provider payload, or freshness lineage
is part of the descriptor.

## Reachable capabilities

| Entry point | State capability | Provider owner | Intent | Raw ingress | Authentication and correlation | Evidence projection | Fact mode | Entry rule |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `mfm.portfolio/snapshot@1` | `EvmCapability<2>` (chain), `<6>` (anchor), and `<7>` (balance) Reads | `mfm-evm-live::EvmAdapterBinding` and its `EvmProvider` | `EvmReadIntent` with exact chain, source, anchor, and asset intent | `EvmProviderResponse::Read`, `Rejected`, `SafeFailure`, or `IntegrityBlocked` | Complete execution binding, physical target, State/capability/adapter refs, call id, and echoed operation; each closed capability family and `EvmReadEvidence::validate_for` bind the private complete subject and returned value | Bounded typed chain/anchor/raw-unit/decimal evidence or a closed non-value outcome | `NoPriorFacts` | Read, three total attempts |
| `mfm.evm/submit-transaction@1` | `EvmCapability<0>` (nonce reservation) | `mfm-evm-live::EvmAdapterBinding` and its `WalletNonceAuthority` | `NonceReservationIntent` | Durable nonce reservation result | Exact State/capability/binding/target, tenant/sender/nonce-domain effect domain, and committed call id | `Reserved`, `Rejected`, or `IntegrityBlocked`; acknowledgement unknown remains unresolved | `NoPriorFacts` | one-entry `EffectMode`, one total attempt |
| `mfm.evm/submit-transaction@1` | `EvmCapability<1>` (broadcast) | `mfm-evm-live::EvmAdapterBinding`, signer, and `EvmProvider` | Complete `BroadcastIntent` | `Broadcast`, `Rejected`, `PossibleEntry`, or `IntegrityBlocked` | Complete execution binding, sender, nonce domain, target, public signer key instance, call id, operation, and candidate id | `Returned`, `Rejected`, or `IntegrityBlocked`; `PossibleEntry` remains unresolved and emits no evidence | `NoPriorFacts` | one-entry `EffectMode`, one total attempt |
| `mfm.evm/submit-transaction@1` | `EvmCapability<3>` (submission status: receipt, finality, and canonical block) Read | `mfm-evm-live::EvmAdapterBinding` and its `EvmProvider` | `EvmReadIntent` bound to broadcast hash, finalized head, or receipt block | Same bounded read response algebra | Exact binding/target/call/operation and intent-derived private read subject | Bounded receipt/finality/canonical-block evidence | `NoPriorFacts` | Read, three total attempts |

Authenticated JSON-RPC/provider material is a provider assertion, not independent chain truth. The
adapter drops raw response bytes and maps transport ambiguity to `UnresolvedClassification`; a
generic transport error never becomes definite pre-entry evidence. `IntegrityBlocked` is accepted
only through the capability-certified failure route and grants no retry authority.

## Immutable binding descriptor

`mfm-program::single_trust::BindingDescriptor` retains exactly these public propositions:

- State implementation identity;
- capability contract identity, when the State is Access;
- adapter implementation identity, when the State is Access;
- complete physical target/route identity;
- Effect domain, when the State is Effect; and
- public signer key-instance identity, when one exists.

`BindingDescriptor::content_ref` uses the `mfm.execution-binding` schema and canonical descriptor
bytes. `EvmAdapterBinding::binding_matches` compares the complete physical-target content identity,
not an endpoint-only or currentness field. Release, promotion, generation, revocation, leases, and
resource-currentness lineage have no consumer in the final surface.

## Evidence and retention disposition

| Material | Hot owner | Durable form | Retained after conclusion |
| --- | --- | --- | --- |
| Canonical intent | `PreparedExecution` / `CommittedCall` | `StatePrepared` value/object | Yes, as the preparation identity |
| Provider response bytes | Adapter future only | None | No |
| Accepted evidence | Call-bound accepted wrapper | `StateConcluded::Access` value/object | Only the bounded domain evidence enum |
| Integrity code | Call-bound integrity wrapper | Bounded evidence plus declared failure outcome | Stable redacted code only |
| Signed transaction bytes / credentials | Provider and signer internals | None | No |
| Prior-fact request/selection | Preparation-bound continuation | Preparation objects and Access conclusion selection | Exact structural refs only |

The inventory is implemented by `crates/domains/evm/src/lib.rs`,
`crates/live/evm/src/lib.rs`, `crates/kernel/program/src/single_trust.rs`, and
`crates/kernel/runtime/src/single_trust.rs`. Focused evidence includes the EVM balance matrix,
the exact live-adapter closure target, the App submission/Portfolio cutover target, Store
preparation/conclusion tests, and the Runtime access lifecycle counter regression. Those checks
separately count provider entry, accepted ingress, preparation, and State interpretation before a
durable conclusion is produced.
