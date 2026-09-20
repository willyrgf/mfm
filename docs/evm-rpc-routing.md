# EVM RPC routing

`EvmPhysicalTarget { chain_id, endpoint_ref }` identifies an observational binding. Native
`EvmBalanceRoute` additionally retains the public endpoint descriptor for configuration-deleted
publication. The native client admits native configuration and constructs shared semantic demand;
Portfolio checks collection semantics without decoding the native route.

The caller's expected route is retained independently of the selected binding. Shared Observe
forwards it unchanged. Native implementations qualify configuration during construction and check
the selected binding before IO, including after cold reconstruction. `EvmResources` implements the
existing environment/discovery and binding contracts with unique sorted resources and derived
binding views. There is no registration loop or separate binding cache.

Before IO, the callback checks intent chain and route against the captured target. A local mismatch
or a subject presented to the wrong selected capability returns `AdapterError::Internal`, enters
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
locator and purpose-bound signer handle remain process-local. Resource construction validates the immutable
epoch, signing purpose, and public-key-derived sender. Each invocation compares the command binding
and exact command value ref before IO, then asks the separate `EvmTransactionProvider` facet to
recheck chain ID and genesis at the selected endpoint.

The transaction provider exposes checked chain-instance, pending-nonce, receipt, canonical-block,
transaction-known and exact-raw-submission operations. The native supporting States and designated
Effect have separate responsibilities:

1. Reservation loads existing authority first. Only an absent reservation requires observing the
   pending nonce and allocating the immutable first winner for the exact Effect/command.
2. Preparation loads that reservation and signs only when prepared bytes are absent. It retains
   the exact raw bytes and hash before any submission and qualifies the first prepared winner.
3. Execution checks the retained descriptor and raw bytes, verifies chain identity, then checks
   the receipt. If absent, it checks transaction-known status and submits the retained bytes only
   when the transaction is absent. The signer is never invoked during execution.
4. Pending awaits private native pacing before returning. Cancellation creates no background task
   and changes no command authority. Actual provider errors pass through unchanged; Runtime owns
   no timer or polling deadline.
5. A present receipt must match the retained transaction and canonical block at its stated number.
   Runtime retains the qualified settlement in Journal before interpretation. Custody retains only
   reservations and prepared bytes, not settlement. Terminal cold replay needs no provider call.

Every retry reuses the retained first winner's Effect ID, command, nonce, signature, hash and raw
bytes. Conflicting or malformed retained authority is rejected with its reviewed causal facts.
Transport duplication is allowed; authorizing a replacement or another semantic transaction is
not. Provider errors and malformed transaction ingress retain their reviewed causes in `Unavailable`; a local binding,
signer, route, or retained-authority mismatch is `Internal` before the affected provider phase.

Anchored contract calls use the transaction route rather than `EvmPhysicalTarget`. The provider
re-observes the named block by number, requires code and calls by the exact block-hash selector, then
re-observes the same block before returning. Missing anchors are `SafeFailure`, codeless targets are
`Rejected`, and authenticated anchor replacement is `IntegrityBlocked`.

Transaction and anchored-route callbacks bind through the same native resource environment as
observational Reads. Portfolio exposes no mutation entry point. Receipt settlement remains scoped
to the pinned non-reorging managed Reth acceptance contract; this cutover does not introduce a
configurable production finality policy. Delayed interval mining exercises native Pending pacing.
