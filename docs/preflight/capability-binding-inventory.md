# Capability and binding inventory

This is the checked-in U3/U5 preflight artifact for the two admitted entry points. It records the
provider assertion boundary, the finite evidence algebra, and the immutable binding material that
can survive in durable history. No secret, credential, raw provider payload, or freshness lineage
is part of the descriptor.

## Reachable capabilities

| Entry point | State capability | Provider owner | Intent | Raw ingress | Authentication and correlation | Evidence projection | Fact mode | Entry rule |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `mfm.portfolio/snapshot@1` | `ReadBalance` | `mfm-live/evm::EvmAdapterBinding` and its `EvmProvider` | `EvmReadIntent` | `EvmProviderResponse::Read`, `Rejected`, `SafeFailure`, or `IntegrityBlocked` | Complete `ExecutionBindingRef`, physical target, State/capability/adapter refs, call id, and echoed operation; `EvmReadEvidence::validate_for` binds operation and subject | `Returned`, `Rejected`, `SafeFailure`, or `IntegrityBlocked`; only bounded value/anchor or stable redacted code survives | `NoPriorFacts` | Read, three total attempts |
| `mfm.portfolio/snapshot@1` | `ReadWalletNonceStatus` | `mfm-live/evm::EvmAdapterBinding` and its `EvmProvider` | `EvmReadIntent` | Same bounded provider response algebra | Same exact binding and call/operation correlation | Same strict EVM read evidence, interpreted by the State implementation | `NoPriorFacts` | Read, three total attempts |
| `mfm.portfolio/snapshot@1` | `ReadLatestAnchor` | `mfm-live/evm::EvmAdapterBinding` and its `EvmProvider` | `EvmReadIntent` | Same bounded provider response algebra | Same exact binding and call/operation correlation | Same strict EVM read evidence, interpreted by the State implementation | `NoPriorFacts` | Read, three total attempts |
| `mfm.evm/submit-transaction@1` | `ReadWalletNonceStatus` | `mfm-live/evm::EvmAdapterBinding` and its `EvmProvider` | `EvmReadIntent` | Same bounded provider response algebra | Same exact binding and call/operation correlation | Same strict EVM read evidence | `NoPriorFacts` | Read, three total attempts |
| `mfm.evm/submit-transaction@1` | `BroadcastTransaction` | `mfm-live/evm::EvmAdapterBinding` and its `EvmProvider` | `BroadcastIntent` | `EvmProviderResponse::Broadcast`, `Rejected`, `PossibleEntry`, or `IntegrityBlocked` | Complete `ExecutionBindingRef`, sender, nonce domain, target, State/capability/adapter refs, call id, echoed broadcast operation, and candidate id | `Returned`, `Rejected`, or `IntegrityBlocked`; authenticated `PossibleEntry` remains unresolved and emits no evidence; no signed bytes or raw response survives | `NoPriorFacts` | one-entry `EffectMode`, one total attempt |

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
`crates/kernel/runtime/src/single_trust.rs`. The focused checks are
`admission_and_result_deserialization_reenter_domain_validation`,
`adapter_future_panics_are_contained_as_unresolved`, the Store preparation/conclusion tests, and
the Runtime access lifecycle counter regression. That regression separately counts provider entry,
accepted ingress, preparation, and State interpretation and requires one of each before a durable
conclusion is produced.
