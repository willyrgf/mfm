# mfm-evm

Secret-free native EVM contracts and deterministic implementations for shared Chain capabilities.
This crate owns EVM planning, ABI encoding/decoding, route qualification and exact native outcomes.
It has no provider, signer, Store or ambient IO dependency. Live resources and clients belong to
[`mfm-evm-live`](../../live/evm/README.md); Portfolio owns collection semantics.

Typed shared Operations compile into a complete immutable Program. Native transaction injection
adds nonce reservation and preparation before the designated shared Effect. Native recipes consume
maintained deployment or configuration requests, qualify the pinned scalar artifact and construct
checked EIP-1559 commands through `Eip1559Options`. Existing-address calls consume the actual
checked deployed predecessor. No context-slot macro, plan wrapper, registration loop or Runtime
root mapper is required. See [capability authoring](../../../docs/capability-authoring.md).

`EvmAddress` and `EvmHash` retain fixed bytes with strict lowercase hexadecimal wires. `EvmU256`
owns canonical decimal words; `CanonicalBytes` owns canonical base64. Commands have bounded
Create or Call actions, nonzero gas, canonical `u128` fees and an empty access list. A transaction
binding fixes ledger/genesis, endpoint, authority epoch and sender. Native qualification checks
request, command, reservation, nonce, hash and settlement action without replacing exact originals.

`custody` owns asynchronous reservation and opaque signed-byte retention. The live adapter supplies
signer/provider IO and PostgreSQL supplies atomic persistence. Custody returns the first immutable
prepared winner. Runtime retains settlement before interpretation; the reservation Effect identity
continues to identify custody. Raw signed bytes have no serde or debug surface.

`EvmContractReadImplementation` implements shared scalar observation. The caller's independent
expected route is retained in `ContractExecutionConfig` and forwarded by shared Observe into
`ReadContractValue`. One native qualifier checks configuration during construction and the selected
binding before provider IO, including cold loading. Same-ledger endpoint mismatch is a local
binding error with expected/actual references, no provider call and no append. Anchored native
outcomes distinguish success, rejection, safe failure and authenticated integrity block, retaining
the exact native original. Scalar decoding checks the exact getter ABI and 32-byte result.

Balance injection owns native preparation, initial anchor, quantity, decimals and confirmation
States around shared observation. Every source in a collection uses one pinned block. Confirmation
re-observes that block number and compares its hash, rather than inspecting only the latest head.
Shared Chain owns decimal arithmetic and cumulative semantic contexts. Native State definitions own
both discovery metadata and exact failure decoder dispatch; metadata is not part of Program identity.
Public route descriptors retain enough information for configuration-deleted cold publication.

Broad and anchored binders qualify the exact intent reference and native result relationship.
Checked constructors validate anchors, quantities, bounds and action shapes. Only authenticated
external evidence can become `IntegrityBlocked`; local mismatch remains Internal. Selected native
codec hooks preserve Decode/Execute/Encode and complete causal diagnostics through Capabilities.

Duplicate-safe observation timeouts, rate limits and unavailability are Retryable. Anchor changes
invalidate input; authenticated integrity blocks and unavailable sources are Permanent. Transaction
provider/authority failures remain OutcomeUnknown, signer unavailability before wire retention is
Retryable, and authenticated reversion is Permanent. Classification never authorizes another command.

Transaction operational error v4 includes transaction-known provider observations and persisted
causes for authority/signer failures. EVM operational error v3 preserves selected provider details.
Changed schemas and dependent State/capability identities reject superseded contracts; there is no
compatibility decoder or history rewrite. Managed lifecycle acceptance covers native 42/84 results,
cold exact commands, delayed settlement, cancellation and cause custody.
