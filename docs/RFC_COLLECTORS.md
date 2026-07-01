# RFC: Collectors And Platform Facts

Status: proposal.

## Summary

Collectors should be first-class MFM workflows that repeatedly observe external sources and record
facts through the existing typed state-machine model. The platform should not introduce a parallel
fact event for collectors. Instead, `FactRecorded` should become the universal primitive for adding
knowledge to MFM.

Every eligible `FactRecorded` event should be projected by the store into a searchable
`PlatformFactRef` index. The run stream remains strict authority. The platform fact index is a
store-maintained projection that makes facts discoverable and reusable across runs and MFM
instances sharing the same store/artifact layer.

In short:

```text
certified workflow state records FactRecorded
  -> append-only run stream event + admitted artifact evidence
  -> store-maintained platform_facts projection
  -> searchable PlatformFactRef
  -> later runs can consume and pin selected facts
```

## Motivation

MFM needs long-lived observation behavior: watching a wallet, waiting for a transaction, following a
chain, polling a protocol, or subscribing to an external event stream. These behaviors are
collector-shaped, but they should not require a separate execution framework.

The existing typed state-machine model already has the right execution primitives:

- operations plan certified graph topology
- states execute finite typed behavior
- capabilities and transports perform live observation
- runtime commits typed events and artifacts atomically
- replay reads recorded evidence only

What is missing is a generalized way for recorded facts to become platform knowledge. Today
`FactRecorded` is treated mainly as attempt-local read evidence. That provenance is valuable and
should be preserved, but the fact should also become discoverable platform knowledge unless the
producer explicitly marks it private.

## Goals

- Reuse operations, states, adapters, transports, runtime, store, and replay primitives.
- Treat `FactRecorded` as the canonical platform knowledge primitive.
- Make facts searchable and reusable through a store-maintained `PlatformFactRef` projection.
- Keep run streams and artifact evidence as strict authority.
- Let collectors be recurring certified workflows, not special daemons with independent semantics.
- Make platform visibility the default for facts, with an explicit private opt-out.
- Preserve replay: consuming runs must pin the facts they use into their own run streams.

## Non-Goals

- Do not add a separate `CollectedFactRecorded` event family as the first design move.
- Do not make live collectors semantic authority.
- Do not let state implementations publish facts through ad hoc side channels.
- Do not parse domain-specific artifact JSON in PostgreSQL triggers.
- Do not put secrets into facts, artifacts, public projections, or diagnostics.
- Do not make mutable "latest fact" rows authoritative.

## Definitions

### Collector

A collector is a recurring certified workflow whose primary purpose is to observe an external source
and record facts.

Collectors are implemented with ordinary MFM primitives. A collector can have an operation that
plans one collection cycle and states that load checkpoints, wait or poll, normalize observations,
record facts, and advance checkpoints.

Example shape:

```text
wallet_tx_collector_cycle
  -> load_checkpoint
  -> wait_or_poll_wallet_source
  -> normalize_observations
  -> record_transaction_facts
  -> record_checkpoint
  -> complete_cycle
```

The app, CLI, REST service, or future scheduler may repeatedly launch or resume such cycles. The
looping process is operational. The durable knowledge is the committed run stream and artifacts.

### Platform Fact

A platform fact is a `FactRecorded` event that is eligible for platform indexing. It is a fact claim
with provenance, not an unqualified global truth.

Examples:

- a wallet transaction was observed by a source
- a wallet transaction was finalized under a finality policy
- a chain head was observed at a given block
- a checkpoint was advanced for a collector cycle
- a price quote was observed from a named source

### PlatformFactRef

`PlatformFactRef` is a store projection row pointing back to the authoritative `FactRecorded` event
and its admitted artifact evidence.

It is not separate authority. It is a searchable reference into existing authority.

## Design Principles

### Facts Are Claims With Provenance

A fact recorded by MFM must carry enough provenance for downstream consumers to decide whether to
trust it.

For `FactRecorded`, the existing provenance is useful:

- certified spec hash
- source run id
- node id
- attempt id
- capability kind and version
- adapter kind and version
- request schema and hash
- response schema and hash
- fact key
- response artifact id

The platform fact projection should preserve this provenance and add store-level search/order
metadata.

### Platform Visibility Is Default

All `FactRecorded` events should become platform-visible unless explicitly marked private.

This opt-out model matches the desired platform knowledge graph: facts are part of the MFM "mind" by
default. Privacy is a semantic visibility control, not a secret-handling mechanism. Secrets must not
enter facts, artifacts, events, public outputs, diagnostics, fixtures, or snapshots at all.

### Authority Remains Append-Only

The platform fact projection must be rebuildable from strict authority:

- `run_events`
- `commits`
- artifact admissions
- run artifact bindings
- canonical event payload bytes

The projection is useful for queries, but it must not replace run-stream validation, artifact
evidence validation, or replay authority.

### Live Search Is Not Replay

A live run may query platform facts. Once it chooses a fact, the chosen reference must be recorded
into that run's own stream as read evidence. Replay then uses the consuming run's pinned evidence,
not the current platform index or a live collector.

## FactRecorded Generalization

`FactRecorded` should gain an explicit visibility policy.

Suggested shape:

```rust
pub enum FactVisibility {
    Platform,
    RunPrivate,
}
```

Default:

```text
FactVisibility::Platform
```

`RunPrivate` means the fact remains valid run-stream evidence for the producing run, but the store
does not expose it through the platform fact index.

This is intentionally small. The first step is not to redesign fact semantics. It is to state that
facts are platform knowledge by default and to let producers opt out.

## PlatformFactRef

Suggested logical shape:

```rust
pub struct PlatformFactRef {
    pub source_run_id: RunId,
    pub source_event_id: EventId,
    pub source_seq: StreamSeq,
    pub source_ordinal: EventOrdinal,

    /// Store-owned time when MFM committed the fact.
    pub recorded_at: Timestamp,

    /// Optional source/domain time when the fact itself says the observation happened.
    pub observed_at: Option<Timestamp>,

    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub attempt_id: AttemptId,
    pub fact_key: FactKey,
    pub request_schema_id: SchemaId,
    pub request_hash: ContentDigest,
    pub response_schema_id: SchemaId,
    pub response_hash: ContentDigest,
    pub artifact_id: ArtifactId,
    pub capability_kind: CapabilityKind,
    pub capability_version: CapabilityVersion,
    pub adapter_kind: AdapterKind,
    pub adapter_version: AdapterVersion,
}
```

`recorded_at` is store-owned and always available. In the current PostgreSQL store it can be derived
from the commit row's `committed_at` value.

`observed_at` is source/domain time. It must not require the store to understand arbitrary
domain-specific artifact JSON. A future typed summary contract may provide it. Until then it may be
absent or carried in typed fact metadata supplied by the runtime/event layer.

Ordering authority should still prefer store order over timestamps. Timestamps are for search,
filtering, dashboards, and human-facing inspection. Store sequence, commit ordering, and event
ordinal remain the deterministic ordering basis.

## Store Projection

The PostgreSQL store should maintain a normal projection table, not a PostgreSQL materialized view.

Conceptual table:

```sql
CREATE TABLE platform_facts (
  source_run_id TEXT NOT NULL,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  spec_hash TEXT NOT NULL,
  recorded_at TIMESTAMPTZ NOT NULL,
  observed_at TIMESTAMPTZ NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  fact_key TEXT NOT NULL,
  request_schema_id TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  response_schema_id TEXT NOT NULL,
  response_hash TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  capability_kind TEXT NOT NULL,
  capability_version TEXT NOT NULL,
  adapter_kind TEXT NOT NULL,
  adapter_version TEXT NOT NULL,
  PRIMARY KEY (source_run_id, source_event_id)
);
```

Useful indexes:

```sql
CREATE INDEX platform_facts_recorded_at_idx
  ON platform_facts (recorded_at);

CREATE INDEX platform_facts_observed_at_idx
  ON platform_facts (observed_at)
  WHERE observed_at IS NOT NULL;

CREATE INDEX platform_facts_fact_key_recorded_at_idx
  ON platform_facts (fact_key, recorded_at DESC);

CREATE INDEX platform_facts_request_idx
  ON platform_facts (request_schema_id, request_hash);

CREATE INDEX platform_facts_response_schema_recorded_idx
  ON platform_facts (response_schema_id, recorded_at DESC);

CREATE INDEX platform_facts_capability_recorded_idx
  ON platform_facts (capability_kind, capability_version, recorded_at DESC);
```

The exact index set should be driven by initial query APIs and measured usage.

### Why Not A Materialized View

PostgreSQL materialized views are refreshed in batches. `REFRESH MATERIALIZED VIEW CONCURRENTLY`
reduces read blocking, but it is still not the per-insert incremental projection MFM needs.

The MFM store already appends commits, run events, and artifact evidence atomically. Platform fact
projection should happen inside that same append transaction. That gives:

- no projection refresh lag for newly committed facts
- atomic event/artifact/fact visibility
- ordinary indexes
- straightforward corruption checks
- rebuildability from strict authority

### Why Not A SQL Trigger For Typed Extraction

The database should not duplicate Rust event-schema decoding logic or parse domain-specific fact
payloads from JSON. The typed store implementation already receives typed event payloads before
inserting `run_events`; it should derive platform fact rows there.

Database triggers should remain focused on database-level invariants such as append-only mutation
guards and timestamp/transaction metadata.

## Collector Execution Model

Collectors should be recurring workflows over bounded state attempts.

A state may wait on a WebSocket or poll a source through a typed read capability, but the wait should
be bounded by certified or configured policy:

- event received
- batch size reached
- timeout reached
- source unavailable
- backoff requested
- cancellation or lease loss

The state returns a typed result and the run commits evidence. Then another cycle may start.

This avoids a single forever-open attempt that holds runtime state for hours without committing
progress. It also makes process crashes ordinary: committed facts/checkpoints survive, and uncommitted
observations are retried.

Example cycle:

```text
1. read checkpoint fact or seed checkpoint
2. wait/poll source through transport-backed capability
3. normalize observations into typed fact responses
4. emit FactRecorded events
5. emit checkpoint fact
6. complete cycle
7. scheduler/app starts the next cycle
```

## Consuming Platform Facts

A consuming workflow should use a read state or capability to query platform facts. The query itself
is live read behavior. The selected result must be recorded in the consuming run stream.

Example:

```text
consumer state queries platform_facts for wallet transaction facts
  -> selects one PlatformFactRef
  -> records selected fact reference as FactRecorded read evidence
  -> downstream states consume the pinned result
```

Replay must not query the live platform fact index for "latest" data. Replay uses the consuming
run's recorded selected reference and retained artifacts.

## Public APIs

Initial query APIs should expose platform facts as references and summaries, not raw authority
internals.

Likely filters:

- fact key prefix or exact fact key
- request schema id
- request hash
- response schema id
- capability kind/version
- adapter kind/version
- source run id
- recorded-at range
- observed-at range
- after cursor
- limit

Public pagination should use opaque cursors. It should not expose internal append XIDs, sort keys,
cursor versions, or store epochs.

## Privacy And Security

Facts are not a secret storage mechanism.

Rules:

- secret-bearing data must never enter facts, artifacts, events, public outputs, diagnostics,
  fixtures, or snapshots
- `RunPrivate` hides a fact from platform search, but does not make secret data acceptable
- public APIs must redact errors and avoid endpoint/auth leakage
- source routing, credentials, RPC URLs, authorization headers, passwords, keystores, and signer
  material remain below typed semantic surfaces

## Rebuild And Verification

`platform_facts` should be rebuildable from strict authority rows.

Required checks:

- every platform fact row points to an existing `FactRecorded` event
- the event is platform-visible
- artifact id matches admitted fact response evidence
- source run/event identity matches the run stream
- response schema/hash matches the event payload
- no private facts appear in the projection

The store should eventually expose a validation/rebuild path for projection corruption, similar in
spirit to strict status/replay projection validation.

## Failure Semantics

Facts and platform projections must remain atomic.

- If artifact admission fails, the `FactRecorded` event and platform fact row must not appear.
- If event insertion fails, the platform fact row must not appear.
- If platform fact projection fails, the commit must fail rather than append an event that should be
  platform-visible but is absent from the projection.
- Retrying the same prepared commit must remain idempotent.
- Rebuilding the projection from run streams must produce the same rows.

## Migration Path

1. Document `FactRecorded` as the platform knowledge primitive.
2. Add `FactVisibility` to `FactRecorded`, defaulting to platform-visible.
3. Add `PlatformFactRef` types and query DTOs.
4. Add the `platform_facts` projection table to the PostgreSQL store.
5. Populate `platform_facts` in the same append transaction that inserts `run_events`.
6. Add query APIs over platform facts.
7. Add tests for atomicity, private filtering, ordering, timestamp filters, idempotent retry,
   projection rebuild, and replay pinning.
8. Build the first collector as a recurring certified workflow that emits ordinary
   platform-visible `FactRecorded` events.

## Open Questions

- Should `observed_at` be a direct field on `FactRecorded`, or should it come from a typed fact
  summary descriptor?
- What is the minimal public query API for the first collector use case?
- Should fact visibility be a simple enum or should it include future named scopes?
- How should cross-store fact export/import prove source store trust scope and retained artifact
  evidence?
- Should collector cycles be scheduled by app assembly, a new scheduler surface, or an explicit CLI
  daemon command first?
- Which fact schemas should be treated as checkpoints versus domain observations?

## Preferred First Implementation

Start with the smallest vertical slice:

- one PostgreSQL projection table
- one `PlatformFactRef` query path
- `FactVisibility::Platform` and `FactVisibility::RunPrivate`
- no `observed_at` extraction unless the event layer carries it explicitly
- no SQL triggers for typed extraction
- one collector-style workflow that records facts and checkpoints using existing state-machine
  primitives

This proves the central model: MFM's knowledge graph is built from ordinary `FactRecorded` events,
not a separate collector event system.
