# EVM Routing Generations And Audited Reads

Status: production transport and state-graph contract for EVM-backed reads

EVM endpoints, authorization values, connection pools, and local routing configuration are
process/deployment resources. They are never typed workflow configuration and never appear in a
spec, journal record, retained object, fact, public output, portable export, durable fixture, or
diagnostic. Live transport tests may generate ephemeral authorization sentinels only inside
zeroizing owners and must not retain or print them.

## Immutable Routing Generation

For each certified semantic EVM binding, admission selects one already qualified immutable
non-secret `routing_generation_ref`. The root retains that reference with the expected semantic
network and chain identity.

Admission may check local syntax and select the generation. It must not:

- contact an endpoint;
- call `eth_chainId`;
- validate live source health;
- inspect current block state;
- rotate or rank sources; or
- silently substitute another generation.

After admission, every EVM call resolves the exact recorded generation. If it cannot be resolved,
the audited bootstrap call records the reviewed failure. Resume never reinterprets a mutable route
label and never falls back to a newer generation. A new run may select a different generation.

The generation's secret-bearing material remains below the capability boundary:

- HTTP(S) endpoint;
- optional HTTP authorization;
- connection and request limits;
- secret-source locations; and
- concrete transport pool/session resources.

Only reviewed source-scope and implementation identities may appear in access audit.

## Production Read Graph

The aggregate EVM reader is not a capability. The reusable operation expands one certified graph:

```text
BootstrapEvmSource
  -> ReadEvmInitialAnchor
  -> one node per independently meaningful RPC request
       ReadNativeBalance
       ReadTokenMetadata
       ReadTokenBalance
  -> ConfirmEvmAnchor
  -> AggregateEvmBalances
```

The exact state names may remain private, but the audit units and edges are mandatory.

### Bootstrap

The bootstrap state authors one immutable request containing the admitted routing generation and
expected semantic source/chain binding. Its one audited capability operation resolves that
generation and calls the exact chain-identity protocol operation. A returned mismatch is a typed
`source_mismatch` domain result; it is not a route fallback trigger.

The typed bootstrap output carries only reviewed non-secret binding evidence required by later
nodes. Endpoint and credential material remain in the private capability implementation.

### Initial anchor

The anchor state performs one audited read that obtains the exact canonical number/hash pair.
Block numbers retain the full U256 range as canonical decimal strings. The returned typed anchor is
an explicit input edge for every later anchored request.

### Fan-out

Each independently meaningful metadata or balance request is its own certified state occurrence and
its own authorization/observation pair:

- native balance: one account at the exact anchor;
- token metadata: one contract/field request at the exact anchor;
- token balance: one account/contract call at the exact anchor.

Requests use the anchor hash through EIP-1898 with `requireCanonical: true`. Token metadata may be
deduplicated by deterministic planning/dataflow, but a capability call cannot hide several
application-protocol operations behind one authorization.

Runtime may execute ready nodes concurrently across repeated `drive_once` callers, but each drive
performs at most one audited operation. Response completion order does not alter certified node or
aggregation order.

### Final anchor confirmation

The final state performs one audited number-to-hash read and requires the hash to equal the initial
anchor. Asking for the old hash again is not a canonicality check. Anchor drift is a typed returned
semantic failure.

### Pure aggregation

Aggregation performs no IO. It receives the bootstrap, anchor, metadata, balance, and confirmation
values through exact graph bindings. It verifies:

- complete certified source coverage;
- unique results for every request;
- exact source, chain, and routing-generation agreement;
- exact common anchor;
- canonical addresses and U256 quantities;
- token decimal bounds; and
- deterministic output/fact order.

It then produces typed EVM balance values and any transition fact emissions. Portfolio states
consume same-run values directly through graph edges.

## Authorization And Observation

Every EVM protocol operation follows:

```text
pure request authorship
  -> ExternalAccessAuthorized
  -> one affine AuthorizedAccess call
  -> ExternalAccessObserved
  -> later state settlement
```

The transport performs no implicit retry or source rotation. `InsufficientEvidence` may cause a
later `drive_once` to append a new authorization for the same immutable request.

Only these safe-failure codes are admitted by the frozen EVM contract:

```text
routing_generation_unavailable
configuration_invalid
request_invalid
access_cancelled
transport_failed
http_status
json_rpc_error
response_invalid
response_missing_result
response_too_large
unclassified_failure
```

The only typed diagnostics are reviewed HTTP status, JSON-RPC numeric code, or closed
response-invalid kind. Provider messages, response bodies, URLs, authorization, and paths are
discarded.

The retryable HTTP set is `408`, `425`, `429`, `500`, `502`, `503`, `504`, and `507`. The
retryable JSON-RPC set is `-32603`, `-32001`, `-32002`, and `-32005`. Those observations yield
`InsufficientEvidence`; nonretryable numeric destination rejection may become the closed typed
`destination_rejected` failure. Invalid/unrepresentable responses yield `InvalidEvidence`.

Cancellation or transport failure after boundary entry may be `Indeterminate`; that audit outcome
does not invent an EVM result.

## Transport Contract

The reusable EVM transport owns:

- exact generation resolution;
- bounded HTTP(S) request/response IO;
- one private allowlist of the six read and five wallet operations, with no raw method/parameter
  seam;
- direct exact-capacity request encoding into owner-backed zeroizing bodies;
- non-empty valid authorization values bounded to 16 KiB and consumed into sensitive owner-backed
  headers;
- a 512 KiB signed-envelope limit checked after encoding and again before HTTP, plus one
  preallocated zeroizing 1 MiB response buffer;
- exact HTTP `200` acceptance;
- redirects and implicit retries disabled;
- strict borrowed JSON-RPC 2.0 envelope/id/result-or-error validation, including semantic duplicate
  detection across escaped key aliases and trailing-input rejection;
- bounded lexical projection of ignored values at depth 64 and 4096 items per container;
- a 128 KiB raw-result limit for reads and 16 KiB encoded/decoded error-message limit;
- checked Alloy-backed hash, address, bytes, quantity, block, and call decoding;
- semantic response-size limits in addition to an outer transport limit; and
- the closed safe-failure mapping.

It receives operation-only typed requests. It knows neither the portfolio graph nor journal/store
authority. A private live adapter consumes runtime's affine access authority and calls the
transport; the public reusable read API remains runtime-agnostic. Wallet mutation is exposed only
through `EvmWalletExecutor<Store>`, which receives this concrete transport only through the sealed
wallet qualification. The executor cannot accept an independently constructed transport. No public
raw wallet RPC client, response, error, target wrapper, or target-entry descriptor exists.

## Replay And Inspection

Recorded verification checks authorization/observation linkage, request/response schemas, graph
lineage, anchors, outputs, facts, and closure without invoking state callbacks.

Exact reproduction reruns the admitted pure request authors and reducers from retained typed values.
Candidate comparison runs only explicitly identified candidate callbacks. No replay mode resolves a
routing generation, opens HTTP, reads current routing config, or appends.

`ReadPublic` exposes only certified portfolio output. Exact EVM requests, observations, safe
diagnostics, and facts require separately authorized trace, audit, replay, or export access.

## Mutation Separation

This routing/read graph by itself grants no transaction, signer, nonce, or raw-broadcast authority.
The registered wallet effect reuses only the exact-generation stateless transport privately
retained by its wallet qualification. That live clone shares the original transport runtime and
route catalog, while the secret-free canonical qualification proof, reference, and debug form
exclude endpoints, authorization, and transport internals. Its target-entry authority,
sender/nonce ownership, guarded signer, delivery audit, replacement policy, and terminal evidence
come from the separately qualified durable executor described in `docs/evm-transactions.md`.
