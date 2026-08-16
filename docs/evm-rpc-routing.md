# EVM Read routing

`EvmPhysicalTarget { chain_id, endpoint_ref }` is the sole public route identity. The EVM domain owns
its checked schema/value/content ref. Portfolio planning requires targets strictly sorted and unique
by chain ID and places the selected binding ref in every Read declaration and C0 route.

Trusted composition pairs each target with one opaque provider handle and calls
`register_evm_reads`. The installer derives the binding ref and registers the three surviving Read
capabilities directly. One target serves all six current Read State occurrences; no call ID, role
map, descriptor wrapper, or live assembly contribution exists.

Before IO, the callback checks intent chain and route against the captured target. A local mismatch
returns `ReadAdapterError::Internal`, enters no provider, and appends nothing. Timeout, transport, or
malformed unauthenticated ingress returns `Unavailable`. Accepted provider results are the closed
typed `Read`, `Rejected`, `SafeFailure`, and `IntegrityBlocked` cases. Only authenticated external
integrity evidence becomes durable.

Endpoint identity changes the target ref and Program. Replacing credentials or a client handle
under the same public target does not. Credentials are never target material.
