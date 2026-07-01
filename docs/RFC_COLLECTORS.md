# RFC: Collectors And Platform Facts

Status: proposal.

## Summary

Collectors should be first-class MFM workflows that repeatedly observe external sources and record
facts through the existing typed state-machine model. The platform should not introduce a parallel
fact event for collectors. Instead, `FactRecorded` should become the universal primitive for adding
knowledge to MFM.

Every valid `FactRecorded` event must carry typed fact-key publication evidence. Platform-visible
facts are projected by the store into a searchable `PlatformFactRef` index. The run stream remains
strict authority. The platform fact index is a store-maintained projection that makes facts
discoverable and reusable across runs and MFM instances sharing the same store/artifact layer.

This is a breaking event-contract reset. Existing opaque/ad hoc fact keys are not preserved by a
compatibility layer. Development stores and artifacts are reset, and all fact producers move to the
typed fact-key API.

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

What is missing is a generalized way for recorded facts to become platform knowledge. `FactRecorded`
should remain attempt-provenanced evidence, but the event contract should now require typed fact-key
material so the same event can also become discoverable platform knowledge unless the producer
explicitly marks it private.

## Goals

- Reuse operations, states, adapters, transports, runtime, store, and replay primitives.
- Treat `FactRecorded` as the canonical platform knowledge primitive.
- Require every `FactRecorded` to use typed fact-key material through the `MfmFactKey` API.
- Make facts searchable and reusable through a store-maintained `PlatformFactRef` projection.
- Derive searchable domain fields from typed fact-key material, not from ad hoc labels or
  domain-specific `PlatformFactRef` fields.
- Keep run streams and artifact evidence as strict authority.
- Let collectors be recurring certified workflows, not special daemons with independent semantics.
- Make platform visibility the default for facts, with an explicit private opt-out.
- Preserve replay: consuming runs must pin the facts they use into their own run streams.

## Non-Goals

- Do not add a separate `CollectedFactRecorded` event family as the first design move.
- Do not make live collectors semantic authority.
- Do not let state implementations publish facts through ad hoc side channels.
- Do not parse domain-specific artifact JSON in PostgreSQL triggers.
- Do not make `PlatformFactRef` a domain-specific struct or an arbitrary label bag.
- Do not preserve opaque legacy fact keys or add compatibility shims for old fact records.
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

App assembly, CLI, or REST may repeatedly start or resume collector cycles through the ordinary run
surfaces. The looping process is operational. The durable knowledge is the committed run stream and
artifacts. There is no collector-specific daemon or special CLI/REST execution API in the v1 design.

### Platform Fact

A platform fact is a platform-visible `FactRecorded` event. It is a fact claim with provenance, not
an unqualified global truth.

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

### Fact Key Material

Fact key material is the typed, canonical input used to derive a `FactKey`.

It answers the question "what is this fact about?" without including the observed value itself. For
example, a wallet balance key should identify chain, network, account, asset, and balance kind, but
not the balance amount. A weather observation key should identify the place and measurement kind,
but not the measured temperature.

Fact key material is mandatory for every `FactRecorded` event. It should be typed and
schema-described. The platform derives both the stable `FactKey` and the searchable key fields from
that typed material. An opaque caller-chosen fact key is not valid in the new contract.

Conceptual contract:

```rust
pub trait MfmFactKey {
    fn key_schema_id() -> SchemaId;
    fn canonical_key_material(&self) -> PlainCanonicalJsonBytes;
    fn searchable_fields(&self) -> Vec<FactKeyField>;
}
```

`searchable_fields` is not an arbitrary label map. It is a bounded, schema-declared projection of
the key material into generic scalar fields the store can index.

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

### Search Comes From Fact Keys

`PlatformFactRef` should remain generic. Domain search fields should come from the typed fact-key
material that generated `fact_key`.

This avoids two bad outcomes:

- hardcoding domains such as wallet balances, weather, prices, or chain heads into the platform
  fact reference
- adding arbitrary labels that bypass schema review and become an unbounded query surface

The generic platform only needs to understand:

- fact key schema id
- canonical fact key hash
- schema-declared searchable key fields
- fact provenance
- artifact evidence

Domain-specific CLI commands can still be ergonomic. They map friendly arguments into typed
fact-key schema fields.

### Live Search Is Not Replay

A live run may query platform facts. Once it chooses a fact, the chosen reference must be recorded
into that run's own stream as read evidence. Replay then uses the consuming run's pinned evidence,
not the current platform index or a live collector.

## FactRecorded Generalization

`FactRecorded` must carry typed publication evidence. Visibility is part of that evidence.

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

Every `FactRecorded` must also carry enough typed key metadata for the store to validate and project
facts without understanding domain artifact payloads:

```rust
pub struct FactPublicationEvidence {
    pub visibility: FactVisibility,
    pub fact_key: FactKey,
    pub key_schema_id: SchemaId,
    pub key_material_hash: ContentDigest,
    pub searchable_fields: Vec<FactKeyField>,
    pub observed_at: Option<Timestamp>,
}
```

The runtime or runner helper should build this from typed fact-key material. The PostgreSQL store
then copies these already-typed fields into projection tables. It does not parse arbitrary response
artifacts or request JSON to discover domain search keys.

The canonical key material is required at fact-publication time, but v1 does not persist the raw key
material inline in `FactRecorded` and does not retain a separate key-material artifact. The event
persists the schema id, canonical material hash, derived `FactKey`, and schema-declared searchable
fields. Persisting raw key material can be revisited for an audit/export use case, but it is not
needed for the initial projection.

There is no valid "old shape" `FactRecorded` after this change. If a runner cannot provide
`FactPublicationEvidence`, it cannot record a fact.

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
    pub fact_key_schema_id: SchemaId,
    pub fact_key_material_hash: ContentDigest,
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

`observed_at` is optional source/domain time carried directly by `FactRecorded` publication evidence.
It should be captured as close to the external observation as the fact producer can honestly support.
If the source provides a timestamp that is part of the fact semantics, use that. Otherwise, a live
adapter or transport may supply the time at the successful observation or normalization boundary.
State logic must still avoid ambient clocks, and if no trustworthy source or runner timestamp exists,
`observed_at` remains absent.

The store must not derive `observed_at` by parsing arbitrary domain-specific artifact JSON. It only
copies the typed value supplied by the event layer into the projection.

Ordering authority should still prefer store order over timestamps. Timestamps are for search,
filtering, dashboards, and human-facing inspection. Store sequence, commit ordering, and event
ordinal remain the deterministic ordering basis.

`PlatformFactRef` intentionally does not include fields such as `chain`, `network`, `asset`,
`location`, or `measure`. Those live in a companion key-field projection generated from typed
fact-key material.

## Searchable Fact Key Fields

Searchable fact key fields are generic scalar projections of typed fact-key material.

Conceptual shape:

```rust
pub struct FactKeyField {
    pub path: FactKeyFieldPath,
    pub value: FactKeyFieldValue,
}

pub enum FactKeyFieldValue {
    Text(String),
    I64(i64),
    U64(u64),
    Bool(bool),
    Timestamp(Timestamp),
    Digest(ContentDigest),
}
```

The field path should be schema-declared and stable, for example:

```text
chain
network
account_ref
asset_ref
balance_kind
country
locality
measure
```

The platform treats these as generic typed fields, not domain semantics. Domain crates own the
fact-key schema and decide which fields are searchable.

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
  fact_key_schema_id TEXT NOT NULL,
  fact_key_material_hash TEXT NOT NULL,
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

Searchable key fields should use a companion projection table:

```sql
CREATE TABLE platform_fact_key_fields (
  source_run_id TEXT NOT NULL,
  source_event_id TEXT NOT NULL,
  fact_key_schema_id TEXT NOT NULL,
  field_path TEXT NOT NULL,
  value_type TEXT NOT NULL,
  value_text TEXT NULL,
  value_i64 BIGINT NULL,
  value_u64 NUMERIC(20,0) NULL,
  value_bool BOOLEAN NULL,
  value_timestamp TIMESTAMPTZ NULL,
  value_digest TEXT NULL,
  PRIMARY KEY (source_run_id, source_event_id, field_path),
  FOREIGN KEY (source_run_id, source_event_id)
    REFERENCES platform_facts(source_run_id, source_event_id)
    ON DELETE RESTRICT
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

CREATE INDEX platform_fact_key_fields_text_idx
  ON platform_fact_key_fields (fact_key_schema_id, field_path, value_text);

CREATE INDEX platform_fact_key_fields_i64_idx
  ON platform_fact_key_fields (fact_key_schema_id, field_path, value_i64);

CREATE INDEX platform_fact_key_fields_timestamp_idx
  ON platform_fact_key_fields (fact_key_schema_id, field_path, value_timestamp);
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
inserting `run_events`; it should derive platform fact rows and fact-key-field rows there.

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
7. app, CLI, or REST starts or resumes the next ordinary run cycle
```

Collector cycles do not require a new scheduler surface, daemon command, or REST execution mode in
the first implementation. They are normal certified runs. Operational code may choose when to launch
the next cycle, but that choice is not durable semantic authority.

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

The v1 public surface should be deliberately small:

- CLI: `mfm facts query`
- REST: `GET /v1/facts`
- required `key_schema` / `--key-schema`
- repeatable schema-declared field equality filters
- optional exact `fact_key` / `--fact-key`
- opaque cursor
- limit

The initial API lists fact references by key schema and field filters. It does not need capability,
adapter, source-run, request/response, observed-at, or range filters until the first collector use
case proves they are necessary.

Public pagination should use opaque cursors. It should not expose internal append XIDs, sort keys,
cursor versions, or store epochs.

Generic CLI examples:

```sh
mfm facts query \
  --key-schema mfm.wallet.balance.v1 \
  --field chain=bitcoin \
  --field network=mainnet \
  --field account_ref=addr:bc1q...

mfm facts query \
  --key-schema mfm.weather.observation.v1 \
  --field country=IE \
  --field locality=Dublin \
  --field measure=temperature
```

REST examples:

```text
GET /v1/facts?key_schema=mfm.wallet.balance.v1&field=chain%3Dbitcoin&field=network%3Dmainnet

GET /v1/facts?key_schema=mfm.weather.observation.v1&field=country%3DIE&field=measure%3Dtemperature
```

Domain-specific commands may provide friendlier syntax, but they should compile down to the same
typed fact-key schema and field filters.

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
- fact key schema/hash and searchable fields match the event payload
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

This RFC assumes a destructive development reset, not backward compatibility. Existing fact records,
artifacts, and projections are discarded with the old store baseline.

1. Document `FactRecorded` as the platform knowledge primitive.
2. Add mandatory `FactPublicationEvidence` to `FactRecorded`, including `FactVisibility`,
   `MfmFactKey` material, key schema, material hash, searchable fields, and optional `observed_at`.
3. Replace ad hoc `FactKey` construction with typed fact-key material support and schema-declared
   searchable key fields.
4. Add `PlatformFactRef` types and query DTOs.
5. Add the `platform_facts` and `platform_fact_key_fields` projection tables to the PostgreSQL
   store.
6. Populate both projection tables in the same append transaction that inserts `run_events`.
7. Add the initial CLI and REST query APIs over platform facts and fact-key fields.
8. Add tests for atomicity, private filtering, key-field filtering, ordering, timestamp filters,
   idempotent retry, projection rebuild, and replay pinning.
9. Build the first collector as a recurring certified workflow that emits ordinary
   platform-visible `FactRecorded` events.

## Resolved Decisions

- `observed_at` is an optional direct field in `FactRecorded` publication evidence. The store copies
  it into `platform_facts`; it never extracts it from domain artifacts.
- Typed fact-key material is required at publication time, but v1 persists only the schema id,
  material hash, derived `FactKey`, and schema-declared searchable fields.
- `FactKeyFieldValue` v1 supports `Text`, `I64`, `U64`, `Bool`, `Timestamp`, and `Digest`.
- The initial public query API is `mfm facts query` and `GET /v1/facts`, listing fact references by
  key schema and equality field filters with opaque cursor pagination.
- Fact visibility is a simple `Platform` / `RunPrivate` enum in v1. Named scopes are deferred until
  there is a concrete authorization model.
- Cross-store fact export/import is deferred. It is not required for the first platform fact
  projection or collector workflow.
- Collector cycles run through ordinary app, CLI, and REST run start/resume flows. There is no v1
  collector daemon, scheduler surface, or special execution API.

## Deferred Questions

- Which fact schemas should be treated as checkpoints versus domain observations? The first
  implementation can model checkpoints as ordinary `FactRecorded` events with dedicated checkpoint
  schemas, but the final taxonomy likely needs transport-, adapter-, and domain-specific review.
- How should a future cross-store fact export/import bundle prove source store trust scope, source
  run/event identity, stream authority, and retained artifact evidence?

## Preferred First Implementation

Start with the smallest vertical slice:

- one PostgreSQL projection table
- one companion key-field projection table
- one `PlatformFactRef` query path
- one typed fact-key material path with schema-declared searchable fields
- mandatory `FactPublicationEvidence` on every `FactRecorded`
- `FactVisibility::Platform` and `FactVisibility::RunPrivate`
- optional direct `observed_at` on `FactPublicationEvidence`, copied into projection without artifact
  extraction
- no SQL triggers for typed extraction
- one collector-style workflow that records facts and checkpoints using existing state-machine
  primitives
- collector cycles launched and resumed as ordinary runs through existing app, CLI, and REST surfaces

This proves the central model: MFM's knowledge graph is built from ordinary `FactRecorded` events,
not a separate collector event system.
