# RFC: Collectors And Platform Facts

Status: proposal.

## Summary

Collectors should be first-class MFM workflows, but not a separate runtime
primitive. A collector is a recurring certified workflow that uses ordinary
operations, states, adapters, transports, runtime events, and artifact evidence
to observe the outside world and record facts.

`FactRecorded` should become the universal event for adding knowledge to MFM.
There should be no collector-specific fact event such as
`CollectedFactRecorded`. Every valid `FactRecorded` must carry typed fact
subject material through the `MfmFactKey` API. Unless a fact is explicitly marked
private, the store projects it into ordered, indexed, searchable platform fact
tables.

The key architectural point is that states do not hand-author platform search
labels. States record a typed fact claim. The runtime/store derives the
`FactKey`, indexed key fields, and `PlatformFactRef` from the fact key descriptor
and canonical key material.

In short:

```text
state records typed FactRecorded claim
  -> append-only run event + admitted artifact evidence
  -> descriptor + canonical key material derive FactKey and index fields
  -> store-maintained platform fact projection
  -> later runs query, select, and pin facts as read evidence
```

This is a breaking event-contract reset. Existing opaque or ad hoc fact records
are not migrated. Development stores, artifacts, and projections are reset, and
all fact producers move to the typed fact-key API.

## Motivation

MFM needs long-lived observation behavior: watching a wallet, waiting for a
transaction, following a chain head, polling a protocol, subscribing to an event
stream, reading weather observations, or collecting any other external fact.
These behaviors are collector-shaped, but they do not need independent
semantics.

The existing state-machine model already has the useful pieces:

- operations plan deterministic graph topology
- states execute reusable typed behavior
- adapters bind state intent to explicit capabilities
- transports perform live IO behind those capabilities
- the runtime commits typed events and artifacts atomically
- replay reads recorded evidence instead of repeating live observation

What is missing is the generalized contract that makes recorded facts become
platform knowledge. Once every `FactRecorded` carries typed subject material,
the platform can maintain a store-level knowledge index without hardcoding
wallets, weather, prices, chain heads, or any other domain into
`PlatformFactRef`.

## Goals

- Reuse operations, states, adapters, transports, runtime, store, and replay.
- Treat `FactRecorded` as the canonical platform knowledge primitive.
- Require every `FactRecorded` to use typed fact subject material through
  `MfmFactKey`.
- Make platform visibility the default, with an explicit private opt-out.
- Keep platform facts generic across domains.
- Derive `FactKey` and queryable key fields from typed descriptor material, not
  from caller-provided label maps.
- Keep run streams, commits, and artifact evidence as strict authority.
- Make platform fact rows an ordered, indexed, rebuildable store projection.
- Let CLI and REST expose kind-first fact queries that compile to descriptor
  fields.
- Preserve replay by pinning consumed platform facts into the consuming run.

## Non-Goals

- Do not introduce a collector-specific runtime or daemon semantics in v1.
- Do not introduce `CollectedFactRecorded`.
- Do not let states publish facts through ad hoc side channels.
- Do not allow opaque caller-chosen fact keys after this reset.
- Do not make `PlatformFactRef` a domain-specific struct or arbitrary label bag.
- Do not use PostgreSQL JSONB-only search as the v1 indexing model.
- Do not parse domain-specific response artifacts in SQL triggers.
- Do not make mutable "latest fact" rows authoritative.
- Do not migrate legacy fact records or preserve old fact-key shapes.
- Do not put secrets into facts, artifacts, public projections, errors,
  diagnostics, fixtures, or snapshots.

## Definitions

### Collector

A collector is a recurring certified workflow whose primary purpose is to
observe an external source and record facts.

Collectors are built from ordinary operations and states. A collector operation
can plan one bounded collection cycle. States can load checkpoints, wait or
poll through a capability, normalize observations, record facts, advance
checkpoints, and complete.

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

The recurring loop is operational policy. The durable semantics are the
committed run stream and artifacts.

### Fact Kind

A fact kind names the class of claim.

Examples:

```text
wallet.balance
wallet.transaction.observed
chain.head
weather.observation
price.quote
collector.checkpoint
```

Fact kind is the first query dimension for users and tools. It is also part of
the fact key descriptor.

### Fact Subject Material

Fact subject material, also called fact key material, is the typed canonical
data that answers "what is this fact about?" It identifies the subject of the
claim, not the observed value.

For a wallet balance, subject material should include dimensions such as chain,
network, account, asset, and balance kind. It should not include the balance
amount.

For weather in Dublin, subject material might include country, locality,
coordinate cell, provider, and measurement kind. It should not include the
observed temperature or humidity value.

For a transaction observation, subject material might include chain, network,
transaction id, and possibly block height if that fact kind is specifically
about a transaction-in-block observation.

Rule of thumb:

- if a value identifies the subject or is needed as a stable search dimension,
  it belongs in typed subject material
- if a value is the observed result, it belongs in the response artifact
- if observed result fields need first-class search later, model a fact kind
  whose subject intentionally includes those dimensions

### Fact Claim

A fact claim is the event-level assertion made by `FactRecorded`.

It binds:

- visibility
- fact kind
- fact key descriptor identity
- canonical subject material
- derived `FactKey`
- optional source/domain observation time
- request and response schema/hash provenance
- admitted artifact evidence
- producing run, node, attempt, capability, and adapter provenance

The claim is not an unqualified global truth. It is an observation with
provenance.

### Fact Key Descriptor

A fact key descriptor is an immutable, content-addressed description of how a
typed fact subject is canonicalized and indexed.

It declares:

- fact kind
- key schema id
- canonical material shape
- field paths that may be indexed
- scalar type for each indexed field
- allowed query operators for those fields
- size limits and redaction expectations

The descriptor is the source of truth for platform search. State code does not
provide arbitrary search labels.

### PlatformFactRef

`PlatformFactRef` is a store projection row that points back to an authoritative
`FactRecorded` event and its artifact evidence.

It is not independent authority. It is a searchable reference into run-stream
authority.

### Platform Fact

A platform fact is a platform-visible `FactRecorded` claim. Private facts remain
valid run evidence, but they are not exposed through platform fact queries.

### PlatformFactSelectionEvidence

`PlatformFactSelectionEvidence` is private read evidence recorded by a run that
consumes platform facts.

When a live run queries the platform fact index and selects a result, replay must
not repeat that live query. The consuming run pins the selected source fact,
query hash, selection policy, and artifact evidence as run evidence.

## Design Principles

### Collectors Are A Workflow Role

Collectors are different from simple ops or states in purpose, not in primitive
type.

- an operation still plans deterministic graph topology
- a state still performs one reusable unit of execution
- a collector is a workflow that repeatedly uses those primitives to observe and
  record facts

A state can wait on a WebSocket or poll a wallet source, as long as that wait is
through explicit capabilities and bounded by policy. The state should not become
an uncommitted forever-loop.

### Kind Plus Subject Produces Claims

The core model is:

```text
fact kind + typed subject material -> FactKey -> ordered claims
```

Examples:

```text
wallet.balance + {chain, network, account_ref, asset_ref, balance_kind}
weather.observation + {country, locality, coordinate_cell, provider, measure}
chain.head + {chain, network, finality_policy}
```

There can be many claims for the same `FactKey`. "Latest" is a query over
ordered claims, not a mutable authoritative row.

### One Source Of Truth For Search

The fact key descriptor owns the indexable shape. The runtime/store derives
indexed fields from descriptor plus canonical subject material.

This prevents two failure modes:

- hardcoding domain fields such as `chain`, `asset`, `location`, or `measure`
  into `PlatformFactRef`
- letting state code attach arbitrary labels that bypass schema review

### Platform Visibility Is Default

All `FactRecorded` events become platform facts unless explicitly marked
private.

`RunPrivate` means "do not project this claim into platform search." It does not
mean secrets are allowed. Secret-bearing data must never enter fact material,
fact artifacts, events, public outputs, diagnostics, fixtures, or snapshots.

### Authority Remains Append-Only

The authoritative record is still:

- run events
- commits
- artifact admissions and bindings
- canonical event payload bytes

The platform fact tables are a projection. They must be rebuildable and
verifiable from durable authority and content-addressed descriptor/material
data.

### Live Search Is Not Replay

A live run may query platform facts. Once it selects a fact, the selection must
be pinned into the consuming run.

Replay uses the consuming run's pinned selection evidence and retained artifacts,
not the current platform fact index and not a live collector.

## Fact Key Contract

Every `FactRecorded` producer must use typed key material. The runner API should
make the typed path the only convenient path.

Conceptual API:

```rust
pub trait MfmFactKey {
    fn descriptor() -> &'static FactKeyDescriptor;
    fn canonical_key_material(&self) -> PlainCanonicalJsonBytes;
}
```

The descriptor, not the state, defines index fields:

```rust
pub struct FactKeyDescriptor {
    pub fact_kind: FactKind,
    pub key_schema_id: SchemaId,
    pub descriptor_hash: ContentDigest,
    pub fields: &'static [FactKeyFieldDescriptor],
}

pub struct FactKeyFieldDescriptor {
    pub path: FactKeyFieldPath,
    pub value_type: FactKeyFieldValueType,
    pub material_pointer: CanonicalMaterialPointer,
    pub operators: &'static [FactQueryOperator],
}
```

Derivation is deterministic:

```text
descriptor_hash = hash(canonical FactKeyDescriptor)
key_material_hash = hash(canonical subject material)
fact_key = hash("mfm.fact-key.v1", descriptor_hash, key_material_hash)
indexed fields = descriptor fields applied to canonical subject material
```

The canonical subject material must follow the platform hashing rules:

- canonical JSON semantics
- no floats in hashed structures
- bounded size
- no secrets
- stable normalized identifiers
- explicit schema id and descriptor hash

### Example: Wallet Balance Key

```rust
#[derive(MfmFactKey)]
#[mfm_fact(kind = "wallet.balance", schema = "mfm.wallet.balance-key.v1")]
pub struct WalletBalanceKey {
    #[mfm_fact(index)]
    pub chain: ChainRef,
    #[mfm_fact(index)]
    pub network: NetworkRef,
    #[mfm_fact(index)]
    pub account_ref: AccountRef,
    #[mfm_fact(index)]
    pub asset_ref: AssetRef,
    #[mfm_fact(index)]
    pub balance_kind: BalanceKind,
}
```

The amount is not part of the key. The amount is part of the response artifact
for a specific claim.

### Example: Weather Observation Key

```rust
#[derive(MfmFactKey)]
#[mfm_fact(kind = "weather.observation", schema = "mfm.weather.observation-key.v1")]
pub struct WeatherObservationKey {
    #[mfm_fact(index)]
    pub country: CountryCode,
    #[mfm_fact(index)]
    pub locality: Locality,
    #[mfm_fact(index)]
    pub coordinate_cell: CoordinateCell,
    #[mfm_fact(index)]
    pub provider: WeatherProviderRef,
    #[mfm_fact(index)]
    pub measure: WeatherMeasure,
}
```

The measured temperature, humidity, wind speed, or pressure belongs in the
response artifact for the claim. `observed_at` captures source/domain time for
search and ordering.

## FactRecorded Contract

`FactRecorded` remains the event. Its contract changes to require typed fact
claim data.

Conceptual shape:

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

Conceptual claim payload:

```rust
pub struct FactClaim {
    pub visibility: FactVisibility,
    pub fact_kind: FactKind,
    pub key_schema_id: SchemaId,
    pub key_descriptor_hash: ContentDigest,
    pub key_material: PlainCanonicalJsonBytes,
    pub key_material_hash: ContentDigest,
    pub fact_key: FactKey,
    pub observed_at: Option<Timestamp>,
    pub artifact_id: ArtifactId,
    pub artifact_evidence_hash: ContentDigest,
}
```

The existing provenance remains part of `FactRecorded`:

- spec hash
- source run id
- node id
- attempt id
- capability kind and version
- adapter kind and version
- request schema id and hash
- response schema id and hash
- response artifact id and evidence hash

The runtime helper should accept typed key material and response evidence, then
construct the claim:

```rust
recorder.record_fact(
    WalletBalanceKey { /* subject */ },
    response_artifact,
    FactRecordOptions::platform().observed_at(source_time),
)?;
```

There is no valid "old shape" `FactRecorded` after this reset. If code cannot
provide typed subject material through `MfmFactKey`, it cannot record a fact.

Private facts still use the same typed shape. The difference is projection:
private facts do not create platform fact rows or platform key-field rows.

## Platform Projection

The PostgreSQL store should maintain normal projection tables, not a PostgreSQL
materialized view and not JSONB-only search.

The projection is populated in the same append transaction that inserts commits,
admits artifacts, and inserts `run_events`.

### Fact Key Descriptors

```sql
CREATE TABLE fact_key_descriptors (
  descriptor_hash TEXT PRIMARY KEY,
  fact_kind TEXT NOT NULL,
  key_schema_id TEXT NOT NULL,
  descriptor_canonical_bytes BYTEA NOT NULL,
  created_at TIMESTAMPTZ NOT NULL
);
```

This table stores immutable descriptor definitions by hash. A `FactRecorded`
append is valid only when the matching descriptor is available to the store,
either already present or supplied with the append and inserted in the same
transaction.

The descriptor table is not a mutable schema registry. Descriptor changes create
new descriptor hashes and usually new schema ids.

### Platform Fact Keys

```sql
CREATE TABLE platform_fact_keys (
  fact_key TEXT PRIMARY KEY,
  descriptor_hash TEXT NOT NULL,
  fact_kind TEXT NOT NULL,
  key_schema_id TEXT NOT NULL,
  key_material_hash TEXT NOT NULL,
  key_material_canonical_bytes BYTEA NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  FOREIGN KEY (descriptor_hash)
    REFERENCES fact_key_descriptors(descriptor_hash)
    ON DELETE RESTRICT,
  UNIQUE (descriptor_hash, key_material_hash)
);
```

This table stores unique fact subjects. It lets many claims for the same subject
share one indexed key representation.

The canonical descriptor and key material are intentionally persisted as bytes,
not as authoritative JSONB. JSONB can be useful as a derived inspection aid, but
the hashed canonical representation must remain available for validation and
rebuild. If key material is too large to persist inline, the fact-key design is
wrong for v1 and should be redesigned.

### Platform Fact Key Fields

```sql
CREATE TABLE platform_fact_key_fields (
  fact_key TEXT NOT NULL,
  descriptor_hash TEXT NOT NULL,
  field_path TEXT NOT NULL,
  value_type TEXT NOT NULL,
  value_text TEXT NULL,
  value_i64 BIGINT NULL,
  value_u64 NUMERIC(20,0) NULL,
  value_bool BOOLEAN NULL,
  value_timestamp TIMESTAMPTZ NULL,
  value_digest TEXT NULL,
  PRIMARY KEY (fact_key, field_path),
  FOREIGN KEY (fact_key)
    REFERENCES platform_fact_keys(fact_key)
    ON DELETE RESTRICT,
  FOREIGN KEY (descriptor_hash)
    REFERENCES fact_key_descriptors(descriptor_hash)
    ON DELETE RESTRICT
);
```

The store derives these rows from descriptor plus canonical key material. States
do not supply these rows directly.

Useful initial indexes:

```sql
CREATE INDEX platform_fact_key_fields_text_idx
  ON platform_fact_key_fields (descriptor_hash, field_path, value_text);

CREATE INDEX platform_fact_key_fields_i64_idx
  ON platform_fact_key_fields (descriptor_hash, field_path, value_i64);

CREATE INDEX platform_fact_key_fields_u64_idx
  ON platform_fact_key_fields (descriptor_hash, field_path, value_u64);

CREATE INDEX platform_fact_key_fields_timestamp_idx
  ON platform_fact_key_fields (descriptor_hash, field_path, value_timestamp);
```

### Platform Facts

```sql
CREATE TABLE platform_facts (
  source_run_id TEXT NOT NULL,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  commit_id TEXT NOT NULL,
  append_xid BIGINT NOT NULL,
  commit_sort_key TEXT NOT NULL,
  recorded_at TIMESTAMPTZ NOT NULL,
  observed_at TIMESTAMPTZ NULL,
  spec_hash TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  fact_key TEXT NOT NULL,
  descriptor_hash TEXT NOT NULL,
  fact_kind TEXT NOT NULL,
  key_schema_id TEXT NOT NULL,
  key_material_hash TEXT NOT NULL,
  request_schema_id TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  response_schema_id TEXT NOT NULL,
  response_hash TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  artifact_evidence_hash TEXT NOT NULL,
  capability_kind TEXT NOT NULL,
  capability_version TEXT NOT NULL,
  adapter_kind TEXT NOT NULL,
  adapter_version TEXT NOT NULL,
  PRIMARY KEY (source_run_id, source_seq, source_ordinal),
  UNIQUE (source_run_id, source_event_id),
  FOREIGN KEY (fact_key)
    REFERENCES platform_fact_keys(fact_key)
    ON DELETE RESTRICT
);
```

Types and foreign key names are illustrative. The implementation should bind
these rows tightly to existing `run_events`, `commits`, and artifact admission
tables. `artifact_id` alone is not enough; the projected reference must retain
artifact evidence hash/provenance needed to verify the selected claim.

Useful initial indexes:

```sql
CREATE INDEX platform_facts_fact_kind_recorded_idx
  ON platform_facts (fact_kind, recorded_at DESC);

CREATE INDEX platform_facts_fact_key_recorded_idx
  ON platform_facts (fact_key, recorded_at DESC);

CREATE INDEX platform_facts_observed_idx
  ON platform_facts (fact_kind, observed_at DESC)
  WHERE observed_at IS NOT NULL;

CREATE INDEX platform_facts_store_order_idx
  ON platform_facts (append_xid, commit_sort_key, source_ordinal);

CREATE INDEX platform_facts_response_idx
  ON platform_facts (response_schema_id, response_hash);
```

### Projection Algorithm

For each appended `FactRecorded`:

1. Validate the typed fact claim shape.
2. Validate descriptor hash, fact kind, and key schema id.
3. Canonicalize subject material.
4. Recompute `key_material_hash`.
5. Recompute `fact_key`.
6. Derive indexed fields from descriptor plus material.
7. Verify derived fields satisfy descriptor type and size limits.
8. If visibility is `RunPrivate`, insert only the run event and artifacts.
9. If visibility is `Platform`, upsert descriptor, key, key fields, and fact ref
   inside the same append transaction.

If projection fails for a platform-visible fact, the append fails. MFM should not
commit an event that is supposed to be public platform knowledge while omitting
its projection row.

### Ordering And Time

`recorded_at` is store-owned time. In the current PostgreSQL store this should
come from the commit row, not from state code.

`observed_at` is optional source/domain time supplied by the fact producer. It
should be captured as close to the external observation as the source and
adapter can honestly support. If no trustworthy source/domain time exists, it
remains absent.

Ordering authority should prefer store order:

- source run sequence and ordinal order facts inside one run
- commit ordering orders facts across runs
- `append_xid` and `commit_sort_key` are suitable internal ordering inputs
- public cursors should remain opaque

Timestamps are for filters, dashboards, and human inspection. They are not the
only deterministic ordering basis.

## Why Not A Materialized View

PostgreSQL materialized views are refreshed in batches. `REFRESH MATERIALIZED
VIEW CONCURRENTLY` reduces read blocking, but it is still not the per-insert
incremental projection MFM needs.

The append path already has a transaction that inserts commits, admits artifacts,
and inserts run events. Platform fact projection belongs in that transaction.
That gives:

- atomic event/artifact/fact visibility
- no refresh lag for new facts
- ordinary indexes
- deterministic failure behavior
- straightforward validation and rebuild

## Why Not SQL Triggers For Typed Extraction

The database should not duplicate Rust event-schema decoding logic or parse
domain-specific response artifacts.

The typed append path already receives the event payload before inserting
`run_events`. It should validate fact claims, derive keys and fields, and insert
projection rows there.

Database triggers can still enforce database-level invariants such as
append-only mutation guards. They should not be the source of typed domain
extraction.

## Collector Execution Model

Collectors should run as recurring bounded cycles.

A state may wait on a WebSocket, poll a source, or loop through a small internal
state machine while looking for a new observation. The wait must go through an
explicit capability and must be bounded by certified/configured policy:

- event received
- batch size reached
- timeout reached
- source unavailable
- backoff requested
- cancellation or lease loss

The state returns a typed result and the run commits evidence. Another cycle may
then start.

This avoids a single forever-open attempt that holds runtime state for hours
without committing progress. It also makes crashes ordinary: committed
facts/checkpoints survive, and uncommitted observations are retried.

Example cycle:

```text
1. read checkpoint evidence or seed checkpoint
2. wait or poll source through transport-backed capability
3. normalize observations into typed response artifacts
4. emit platform-visible FactRecorded claims
5. emit private/control checkpoint fact
6. complete cycle
7. operational launcher starts or resumes the next ordinary run cycle
```

### Checkpoints

Checkpoint facts can use `FactRecorded`, but they should be modeled carefully.

A checkpoint fact should include subject material such as collector kind, source,
scope, and stream partition. Its response artifact should include high-watermark,
range, predecessor checkpoint, and any finality policy needed for safe resume.

Checkpoint visibility should usually be `RunPrivate` or a future control scope.
Make a checkpoint platform-visible only when other workflows are expected to
query and trust it as shared state.

## Consuming Platform Facts

A workflow can query platform facts through a typed read capability. That live
query is not replay authority.

Flow:

```text
consumer state queries platform_facts
  -> selects a PlatformFactRef
  -> records PlatformFactSelectionEvidence as private read evidence
  -> downstream states consume the pinned evidence
```

The consuming run should not emit a new domain `FactRecorded` merely because it
read an existing platform fact. It records private selection evidence unless it
is making a genuinely new claim.

Conceptual pinned evidence:

```rust
pub struct PlatformFactSelectionEvidence {
    pub source_run_id: RunId,
    pub source_seq: StreamSeq,
    pub source_ordinal: EventOrdinal,
    pub source_event_id: EventId,
    pub store_scope: StoreScopeRef,
    pub fact_kind: FactKind,
    pub fact_key: FactKey,
    pub descriptor_hash: ContentDigest,
    pub key_material_hash: ContentDigest,
    pub response_schema_id: SchemaId,
    pub response_hash: ContentDigest,
    pub artifact_id: ArtifactId,
    pub artifact_evidence_hash: ContentDigest,
    pub query_hash: ContentDigest,
    pub selection_policy_hash: ContentDigest,
}
```

Replay verifies the pinned source fact and artifact evidence. It does not ask
"what is latest now?"

## Public APIs

The public query surface should be kind-first and descriptor-driven.

Initial CLI shape:

```sh
mfm facts kinds
mfm facts describe wallet.balance
mfm facts latest wallet.balance \
  chain=bitcoin \
  network=mainnet \
  account_ref=addr:bc1q... \
  asset_ref=btc \
  balance_kind=confirmed
mfm facts history weather.observation \
  country=IE \
  locality=Dublin \
  measure=temperature
mfm facts show <source-run-id>:<seq>:<ordinal>
mfm facts explain wallet.balance
```

The CLI uses descriptor metadata to validate fields, parse typed values, and
compile the query into generic fact-key-field filters.

Initial REST shape:

```text
GET /v1/facts/kinds
GET /v1/facts/kinds/wallet.balance
GET /v1/facts/wallet.balance/latest?chain=bitcoin&network=mainnet&asset_ref=btc
GET /v1/facts/weather.observation?country=IE&locality=Dublin&measure=temperature
GET /v1/facts/ref/{source_run_id}/{seq}/{ordinal}
```

Raw generic query can exist for tooling:

```sh
mfm facts query \
  --kind wallet.balance \
  --field chain=bitcoin \
  --field network=mainnet \
  --field account_ref=addr:bc1q... \
  --field asset_ref=btc
```

Public pagination should use opaque cursors. The API should not expose raw
append XIDs, commit sort keys, cursor versions, or store epochs.

### PlatformFactRef DTO

The DTO should remain generic:

```rust
pub struct PlatformFactRef {
    pub source_run_id: RunId,
    pub source_seq: StreamSeq,
    pub source_ordinal: EventOrdinal,
    pub source_event_id: EventId,
    pub recorded_at: Timestamp,
    pub observed_at: Option<Timestamp>,
    pub fact_kind: FactKind,
    pub fact_key: FactKey,
    pub key_schema_id: SchemaId,
    pub descriptor_hash: ContentDigest,
    pub key_material_hash: ContentDigest,
    pub request_schema_id: SchemaId,
    pub request_hash: ContentDigest,
    pub response_schema_id: SchemaId,
    pub response_hash: ContentDigest,
    pub artifact_id: ArtifactId,
    pub artifact_evidence_hash: ContentDigest,
    pub capability_kind: CapabilityKind,
    pub capability_version: CapabilityVersion,
    pub adapter_kind: AdapterKind,
    pub adapter_version: AdapterVersion,
}
```

It intentionally does not contain domain columns such as `chain`, `network`,
`asset`, `country`, or `measure`. Those are descriptor-derived key fields joined
through `fact_key`.

## Privacy And Security

Facts are not secret storage.

Rules:

- secret-bearing data must never enter fact subject material, artifacts, events,
  public outputs, diagnostics, fixtures, or snapshots
- private visibility hides a fact from platform search, but does not make
  secret data acceptable
- private facts should not create platform key rows that would leak
  private-only subjects
- public APIs must redact errors and avoid endpoint/auth leakage
- source routing, credentials, RPC URLs, authorization headers, passwords,
  keystores, and signer material remain below typed semantic surfaces

## Rebuild And Verification

Projection rebuild and validation should ship with v1. Idempotent append alone
is not enough because a retry that sees an existing commit may return early and
will not necessarily repair missing projection rows.

Required validation:

- every platform fact row points to an existing `FactRecorded` event
- every projected event is platform-visible
- no private fact creates a platform fact row or private-only key rows
- descriptor hash, fact kind, and schema id match descriptor canonical bytes
- key material hash matches canonical subject material
- `FactKey` matches descriptor hash plus key material hash
- key-field rows match descriptor-derived values
- artifact id and artifact evidence hash match admitted response evidence
- request/response schema and hash match event payload
- source run id, sequence, ordinal, and event id match the run stream
- store ordering columns match the commit row

Rebuild should truncate and repopulate the projection from strict authority and
content-addressed descriptor/material data, then run the same validation checks.

## Failure Semantics

Facts and platform projections must be atomic.

- If descriptor validation fails, the append fails.
- If key derivation fails, the append fails.
- If artifact admission fails, the `FactRecorded` event and projection rows do
  not appear.
- If event insertion fails, projection rows do not appear.
- If platform projection fails, the whole append fails.
- Retrying the same prepared commit remains idempotent.
- Projection rebuild from authority produces the same rows.

## Migration Path

This RFC assumes a destructive development reset, not backward compatibility.
Existing fact records, artifacts, and projections from the old contract are
discarded with the old store baseline.

1. Document `FactRecorded` as the platform knowledge primitive.
2. Add `FactVisibility`.
3. Add typed `MfmFactKey` support and descriptor derivation.
4. Replace ad hoc `FactKey` construction with typed subject material.
5. Require every `FactRecorded` to carry `FactClaim` data.
6. Add descriptor, key, key-field, and platform-fact projection tables.
7. Populate projection tables in the same append transaction as `run_events`.
8. Add projection validation and rebuild.
9. Add kind-first CLI and REST query APIs.
10. Add private read evidence for platform fact selection.
11. Build the first collector as a recurring certified workflow over ordinary
    states.

## Resolved Decisions

- Collectors are recurring workflows over ops and states, not a new runtime
  category.
- `FactRecorded` is the universal fact event.
- There is no `CollectedFactRecorded`.
- Platform visibility is default.
- Every `FactRecorded` must have typed subject material after the reset.
- `FactKey`, indexed fields, and key material hash are descriptor-derived.
- `PlatformFactRef` is generic provenance and ordering, not domain labels.
- Canonical key material is persisted in bounded form for v1.
- `recorded_at` is store-owned commit time.
- `observed_at` is optional source/domain time supplied by the fact producer.
- Platform projection is maintained inside the append transaction.
- PostgreSQL materialized views and SQL artifact-parsing triggers are not the v1
  projection strategy.
- Consuming runs pin selected facts as private read evidence for replay.

## Deferred Questions

- Should descriptors be supplied inline with each first use, admitted as
  descriptor artifacts, or both? The invariant is that rebuild must not depend
  only on whatever Rust code happens to be present later.
- Which typed field operators are required beyond equality and timestamp
  ordering in the first public API?
- Do named visibility scopes need to exist before multi-tenant stores, or is
  `Platform` / `RunPrivate` sufficient until there is a concrete authorization
  model?
- Which checkpoint facts should ever be platform-visible, and should a future
  control scope exist separately from public platform search?
- When a future use case needs search over observed response values, should that
  be modeled as a new fact kind with those dimensions in subject material, or a
  separate response-value indexing system?
- What operational lease/backoff policy should launch recurring collector
  cycles outside the durable state-machine semantics?
- How should cross-store export/import prove source store trust scope, source
  stream authority, and retained artifact evidence?

## Preferred First Implementation

Start with one vertical slice:

- `MfmFactKey` derive for one domain fact key
- mandatory typed `FactClaim` on `FactRecorded`
- `FactVisibility::Platform` and `FactVisibility::RunPrivate`
- descriptor, key, key-field, and platform-fact PostgreSQL projection tables
- append-transaction projection for platform-visible facts
- projection validation and rebuild
- `recorded_at` and optional `observed_at`
- kind-first `mfm facts` query path
- pinned platform fact selection evidence for consumers
- one collector-style workflow that records platform-visible domain facts and
  private checkpoint facts using existing state-machine primitives

This proves the model: MFM's shared knowledge graph is built from ordinary typed
`FactRecorded` events, and collectors are just recurring certified workflows
that produce those events.
