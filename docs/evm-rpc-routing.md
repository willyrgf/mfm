# EVM RPC routing

`EvmPhysicalTarget { chain_id, endpoint_ref }` is the sole public route identity. The EVM domain owns
its checked schema/value/content ref. Portfolio planning requires targets strictly sorted and unique
by chain ID and places the selected binding ref in every Read declaration and C0 route.

At source-authoring time, each of the six supported capability/State pairs owns an explicit
`CapabilityInjection` implementation. The current policies are identity policies: they derive the
same target binding and emit exactly the designated Read. This policy is deterministic topology
only; provider handles, credentials, adapter registration, and IO remain live Runtime concerns.

Trusted composition pairs each target with one opaque provider handle and calls
`register_evm_reads`. The installer derives the binding ref and registers the three surviving Read
capabilities directly. One target serves all six current Read State occurrences; no call ID, role
map, descriptor wrapper, or live assembly contribution exists.

Before IO, the callback checks intent chain and route against the captured target. A local mismatch
or a subject presented to the wrong registered capability returns `AdapterError::Internal`, enters
no provider, and appends nothing. The callback passes the checked intent and Runtime's exact
canonical intent value ref to `EvmReadProvider`; no operation string or serialized domain-intent
transport exists. Timeout, transport, or malformed unauthenticated ingress returns `Unavailable`.
Every accepted evidence outcome carries that exact ref. Only authenticated external integrity
evidence becomes durable.

`JsonRpcEvmProvider` applies that map exactly: transport, timeout, non-success status, an oversized
body, every JSON-RPC error object, unexpected null, and any undecodable result are `Unavailable`.
An `eth_call` result of exactly `"0x"` is `SafeFailure` only for the broad token-interface reads.
The generic RPC ingress uses exact version/id envelopes and typed request parameters and response
DTOs; structurally incomplete error objects do not parse. Broad reads never produce
`IntegrityBlocked`.

Endpoint identity changes the target ref and Program. Replacing credentials or a client handle
under the same public target does not. Credentials are never target material.

`EvmEndpoint { endpoint_id }` is the domain-owned derivation of that endpoint ref: trusted
composition names the endpoint, `endpoint_ref` canonicalizes the name, and the `EvmPhysicalTarget` fields
binds it to the chain ID. The same name therefore derives the same route ref and the same Program in
every process; an RPC URL, credential, or client handle never enters the derivation.

The broad provider method receives `&EvmReadIntent`; the anchored method receives
`&AnchoredContractCallIntent`. Both receive `&ContentRef` for the exact Runtime-qualified intent,
and every returned or terminal evidence constructor requires that ref. A domain contract change is
therefore a compile error at the provider boundary rather than silent wire drift.

Transaction identity is stricter than observational routing. `EvmChainInstance` binds a nonzero
chain ID to the expected genesis hash, and `EvmTransactionRoute` adds the endpoint ref. A complete
Effect binding additionally fixes the transaction-authority epoch and sender account. The provider
locator and purpose-bound signer handle remain process-local. Registration validates the immutable
epoch, signing purpose, and public-key-derived sender. Each invocation compares the command binding
and exact command value ref before IO, then asks the separate `EvmTransactionProvider` facet to
recheck chain ID and genesis at the selected endpoint.

The transaction provider exposes only checked chain-instance, pending-nonce, receipt,
canonical-block, and exact-raw-submission operations. Execution has one loop-free sequence per
caller invocation:

1. Load the append-only authority and return settled evidence immediately when it already exists.
2. For any non-settled state, verify chain identity once. Observe the pending nonce only when a
   reservation is absent, then reserve or compare the exact
   Effect ID and command reference.
3. Sign the fixed type-2, empty-access-list transaction only when prepared bytes are absent, and
   retain its exact raw bytes and hash before provider submission.
4. Check the receipt first. A null receipt permits at most one submission of the retained bytes and
   a matching submission response returns `Pending`, so the caller must resume. A transport
   failure, dropped acknowledgement, malformed response, or mismatched hash returns `Unavailable`.
5. On a later invocation, validate one present receipt, require its block identity to equal the
   provider's current canonical block at that number, and retain the resulting settlement.

Every retry therefore reuses the retained first winner's Effect ID, command, nonce, signature, hash,
and raw bytes. A different concurrent Prepared or Settled candidate returns `Unavailable` and is
resolved by reloading before any signer or provider retry.
Transport duplication is allowed; authorizing a replacement or another semantic transaction is
not. Provider errors and malformed transaction ingress are redacted `Unavailable`; a local binding,
signer, route, or retained-authority mismatch is `Internal` before the affected provider phase.

Anchored contract calls use the transaction route rather than `EvmPhysicalTarget`. The provider
re-observes the named block by number, requires code and calls by the exact block-hash selector, then
re-observes the same block before returning. Missing anchors are `SafeFailure`, codeless targets are
`Rejected`, and authenticated anchor replacement is `IntegrityBlocked`.

Transaction and anchored-route callbacks are intentionally absent from production
`ComposedRuntime`. Their version 1 receipt settlement is fixed to the pinned, non-reorging managed
Reth fixture; it is not a configurable production finality policy.
