# EVM Read routing

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
returns `AdapterError::Internal`, enters no provider, and appends nothing. Timeout, transport, or
malformed unauthenticated ingress returns `Unavailable`. Accepted provider results are the closed
typed `Read`, `Rejected`, `SafeFailure`, and `IntegrityBlocked` cases. Only authenticated external
integrity evidence becomes durable.

`JsonRpcEvmProvider` applies that map exactly: transport, timeout, non-success status, an oversized
body, and any undecodable result are `Unavailable`; a JSON-RPC error object and an `eth_call` result
of exactly `"0x"` are definite `SafeFailure`; an undecodable or operation-mismatched intent and a
locally malformed address are `Internal` before the first byte of IO. It never produces
`IntegrityBlocked`.

Endpoint identity changes the target ref and Program. Replacing credentials or a client handle
under the same public target does not. Credentials are never target material.

`EvmEndpoint { endpoint_id }` is the domain-owned derivation of that endpoint ref: trusted
composition names the endpoint, `endpoint_ref` canonicalizes the name, and `EvmPhysicalTarget::new`
binds it to the chain ID. The same name therefore derives the same route ref and the same Program in
every process; an RPC URL, credential, or client handle never enters the derivation.

The request bytes a provider receives are the serialized `EvmReadIntent`. A provider decodes them
with the domain's own checked `EvmReadIntent` deserializer and matches `subject()`; it declares no
serde mirror of the wire, so a domain subject change is a compile error rather than silent drift.
