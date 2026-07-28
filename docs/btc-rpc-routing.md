# Bitcoin Collection Qualification Target

Status: unregistered capability contract

Bitcoin collection is not registered in the production state/capability catalog. No current app,
CLI, or REST run can execute Bitcoin collection, and no aggregate-reader fallback exists. This
document fixes the graph and qualification evidence required by a future registration change.

Bitcoin RPC endpoints, Basic-auth values, deadlines, and local source resolution remain
process/deployment resources. They must never appear in specs, journal records, retained objects,
facts, public outputs, portable exports, fixtures, or diagnostics.

## Required Graph

A qualified Bitcoin operation must expand exactly this authority shape:

```text
BootstrapBitcoinSource
  -> ScanBitcoinUtxos
  -> ConfirmBitcoinScanAnchor
  -> AggregateBitcoinBalances
```

### Bootstrap

Admission binds one immutable non-secret `routing_generation_ref`, expected semantic source, and
Bitcoin network. It performs no live source validation.

The bootstrap state later performs one audited `getblockchaininfo` operation through the exact
admitted generation. It requires the expected chain and `initialblockdownload == false`. A returned
network mismatch is the typed terminal `source_mismatch` result. Resolution failure is a reviewed
safe failure and cannot trigger route fallback.

### Scan

The scan request contains one bounded, strictly ordered, non-empty set of canonical
`addr(address)` descriptors. The capability performs exactly one:

```text
scantxoutset "start"
```

That collection-valued request is one indivisible application-protocol operation. The capability
must not issue hidden `status`, `abort`, retry, source-reselection, or recovery calls.

The checked result retains the complete bounded UTXO material needed for pure reduction, including
the scan height/hash. `success = false` is the typed terminal `scan_incomplete` result.

### Anchor confirmation

The confirmation state derives its request from the returned scan height and performs one audited
`getblockhash(height)`. The hash must equal the scan anchor. A mismatch is the typed terminal
`anchor_changed` result.

### Pure aggregation

The aggregator performs no IO. It validates exact source/network binding, descriptor coverage,
canonical addresses, hashes, outpoints, scripts, heights, and amounts. JSON amount tokens convert
directly to satoshis with at most eight fractional digits and no sign, exponent, float
intermediary, overflow, or value above `MAX_MONEY`. Address balances and total use checked sums;
zero-balance addresses remain explicit.

The state may then produce ordered typed balances and transition facts. Same-run portfolio
consumption must use graph edges.

## Why Registration Is Blocked

`scantxoutset "start"` can continue provider work after a client timeout or cancellation and shares
Bitcoin Core's global scan resource. Treating it as a read requires qualification that proves:

- it creates no durable domain mutation;
- one invocation has one indivisible snapshot outcome;
- maximum request, response, provider work, and wall-clock bounds are reviewed;
- response loss, cancellation, and delayed reissue are safe;
- concurrent scans and provider cost are explicitly accepted;
- an indeterminate call is never treated as no entry;
- a later invocation receives a fresh MFM authorization and may repeat the full work;
- `scan_busy` proves only that this invocation did not start another scan; and
- no hidden status/abort/retry or source fallback exists.

Until production tests establish all of those properties against the supported Bitcoin Core
behavior, the capability remains absent. If repeat-work safety cannot be established, a separately
designed keyed work executor and audited status-read protocol is required; the generic runtime does
not gain a Bitcoin-specific recovery mode.

## Safe-Failure Contract

Bitcoin uses the EVM read codes plus:

```text
scan_busy
```

Only the exact reviewed Bitcoin Core scan-busy condition—JSON-RPC code `-8` plus the exact-message
classifier—maps to:

```text
Indeterminate
destination
boundary_observation
ScanBusy
```

A near match remains an ordinary `json_rpc_error`. `scan_busy` yields
`InsufficientEvidence`; it does not recover or borrow the result of an earlier authorization.

The retryable HTTP set is `408`, `425`, `429`, `500`, `502`, `503`, `504`, and `507`. Other
numeric destination rejections may become typed `destination_rejected`. Cancellation, transport
failure, and unclassified failure yield `InsufficientEvidence`; malformed, missing, oversized, or
otherwise unrepresentable responses yield `InvalidEvidence`.

Provider messages, bodies, URLs, credentials, and paths are discarded. The only retained
diagnostics are reviewed HTTP status, JSON-RPC numeric code, closed response-invalid kind, or
`ScanBusy`.

## Required Qualification Tests

Registration requires production-path tests covering:

- exact routing-generation resolution across process restart;
- bootstrap source/network mismatch;
- maximum descriptor fan-in and bounded result material;
- lost response before and after possible boundary entry;
- cancellation at every transport boundary;
- exact scan-busy and adversarial near-match classification;
- simultaneous scans from independent processes;
- delayed reissue after timeout and after process loss;
- no status, abort, hidden retry, or provider reselection;
- scan-height hash change;
- malformed, duplicate, overflowing, and noncanonical response material;
- runtime exact-head CAS loss after authorization or observation;
- replay with zero live IO; and
- absence from the product catalog when any qualification fails.

## Future Routing

A future qualified deployment may configure immutable Bitcoin routing generations. Admission may
select only the non-secret generation reference. Endpoint/authentication resolution and
`getblockchaininfo` validation remain post-admission audited work. Evidence-only reads never resolve
the generation.
