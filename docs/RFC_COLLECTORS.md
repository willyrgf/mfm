# RFC: Collectors And Platform Facts

Status: proposal.

## Summary

Collectors should be first-class MFM workflows, but not a separate runtime
primitive. A collector is a recurring certified workflow that uses ordinary
operations, states, adapters, transports, runtime events, and artifact evidence
to observe the outside world and record facts.

`FactRecorded` should become the universal event for adding knowledge to MFM.
There should be no collector-specific fact event such as
`CollectedFactRecorded`. Every valid `FactRecorded` must declare explicit
visibility and carry typed fact subject material through the `MfmFactKey` API.
`Platform` facts become public platform knowledge. `Control` facts become
non-public cross-run operational knowledge. `RunPrivate` facts remain local run
evidence.

The key architectural point is that states do not hand-author platform search
labels. States record a typed fact claim. The runtime/store derives the
`FactKey`, indexed key fields, and `PlatformFactRef` from the fact key descriptor
and canonical key material.

In short:

```text
state records typed FactRecorded claim
  -> append-only run event + admitted artifact evidence
  -> descriptor + canonical key material derive FactKey and index fields
  -> store-maintained projected fact tables for Platform/Control visibility
  -> later runs query and pin PlatformFactQueryEvidence for replay
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
- Require explicit fact visibility; no implicit public publication.
- Add a non-public `Control` visibility for cross-run operational facts such as
  collector checkpoints.
- Keep platform facts generic across domains.
- Derive `FactKey` and queryable key fields from typed descriptor material, not
  from caller-provided label maps.
- Derive queryable observed-result fields from typed response artifacts through
  descriptor-owned result indexes.
- Make ordering policies descriptor-owned so queries can order by store commit,
  observation time, block number, provider time, price time, or other typed
  result fields.
- Keep run streams, commits, and artifact evidence as strict authority.
- Make projected fact rows an ordered, indexed, rebuildable store projection.
- Let CLI and REST expose kind-first fact queries that compile to descriptor
  fields.
- Preserve replay by pinning projected fact queries into the consuming run.

## Non-Goals

- Do not introduce a collector-specific runtime or daemon semantics in v1.
- Do not introduce `CollectedFactRecorded`.
- Do not let states publish facts through ad hoc side channels.
- Do not allow opaque caller-chosen fact keys after this reset.
- Do not default facts into public platform search.
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

- if a value identifies the stable subject of the claim, it belongs in typed
  subject material
- if a value is the observed result, it belongs in the response artifact
- if observed result fields need first-class search or ordering, add a separate
  descriptor-owned result index derived from typed response artifacts in Rust
  append logic, not to the subject key

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
- public exposure policy for those fields
- size limits and redaction expectations

The key descriptor is the source of truth for subject search. State code does
not provide arbitrary search labels.

Descriptor canonical bytes are content-addressed framework artifacts produced
by derive or registration. On first append, the store must already have the
descriptor or receive and admit it in the same append transaction. Rebuild uses
stored descriptor bytes, not whatever Rust code happens to exist later.

### Fact Result Descriptor

A fact result descriptor is an immutable, content-addressed description of which
typed response fields may be indexed or used for ordering.

It declares:

- fact kind
- response schema id
- response field paths that may be indexed
- scalar type for each indexed result field
- allowed query operators for those fields
- public exposure policy for those fields
- ordering policies that may use those fields
- size limits and redaction expectations

Result descriptors do not participate in `FactKey` derivation. They index the
observed claim payload for a specific recorded fact. This keeps subject identity
stable while still making observed facts useful to query.

Result descriptor canonical bytes are admitted and retained like fact key
descriptor bytes. The store derives result index rows from typed response
artifacts in Rust append logic; SQL never parses domain response JSON.

### PlatformFactRef

`PlatformFactRef` is the public reference view over a `Platform`
`ProjectedFactRef`. It points back to an authoritative `FactRecorded` event and
its artifact evidence.

It is not independent authority. It is a searchable reference into run-stream
authority.

### Platform Fact

A platform fact is a `Platform` `FactRecorded` claim. `RunPrivate` facts remain
local run evidence. `Control` facts are cross-run operational evidence, but they
are not exposed through public platform fact queries.

### Control Fact

A control fact is a non-public, cross-run discoverable `FactRecorded` claim. It
is intended for operational state such as collector checkpoints. Control facts
are queryable through internal runtime/collector capabilities, not through
public platform fact APIs.

### PlatformFactQueryEvidence

`PlatformFactQueryEvidence` is private read evidence recorded by a run that
queries platform or control facts.

When a live run queries projected facts, replay must not repeat that live query.
The consuming run pins the canonical query, ordering policy, limit, store
watermark, selection policy, selected source facts, and empty-result/cardinality
statement as run evidence.

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
ordered claims under an explicit ordering policy, not a mutable authoritative
row.

### One Source Of Truth For Search And Ordering

The fact key descriptor owns the indexable subject shape. The fact result
descriptor owns the indexable observed-result shape and ordering policies. The
runtime/store derives subject fields from canonical subject material and result
fields from typed response artifacts.

This prevents two failure modes:

- hardcoding domain fields such as `chain`, `asset`, `location`, or `measure`
  into `PlatformFactRef`
- letting state code attach arbitrary labels that bypass schema review
- stuffing observed values such as amount, temperature, price, or block number
  into `FactKey` just because callers need to search or sort by them

### Visibility Is Explicit

Every `FactRecorded` declares visibility. There is no implicit public
publication.

Conceptual visibility:

```rust
pub enum FactVisibility {
    RunPrivate,
    Control,
    Platform,
}
```

`RunPrivate` means "keep this as local run evidence only."

`Control` means "make this discoverable across runs for internal operational
workflows, but do not expose it through public platform fact APIs."

`Platform` means "make this discoverable through public platform fact APIs,
subject to descriptor exposure policy."

Visibility does not make secrets acceptable. Secret-bearing data must never
enter fact material, fact artifacts, events, public outputs, diagnostics,
fixtures, or snapshots.

### Authority Remains Append-Only

The authoritative record is still:

- run events
- commits
- artifact admissions and bindings
- canonical event payload bytes

The projected fact tables are a projection. They must be rebuildable and
verifiable from durable authority and content-addressed descriptor/material
data.

### Live Search Is Not Replay

A live run may query platform or control facts. The query result, including
empty results and bounded result sets, must be pinned into the consuming run.

Replay uses the consuming run's pinned query evidence and retained artifacts,
not the current projected fact index and not a live collector.

## Fact Descriptor Contracts

Every `FactRecorded` producer must use typed key material. The runner API should
make the typed path the only available path for fact publication.

Conceptual API:

```rust
pub trait MfmFactKey: private::Sealed {
    type Material: MfmValue;

    fn descriptor() -> &'static FactKeyDescriptor;
    fn material(&self) -> &Self::Material;
}

pub trait MfmFactResult: private::Sealed {
    type Response: MfmValue;

    fn result_descriptor() -> Option<&'static FactResultDescriptor>;
}
```

`MfmFactKey` should be derive-only or otherwise sealed outside the framework.
State code should not return raw canonical bytes. Framework-owned
canonicalization turns typed `MfmValue` material into canonical bytes and
enforces schema id, descriptor hash, no-float hashing rules, size limits,
redaction constraints, and field extraction.

`MfmFactResult` follows the same rule for typed response artifacts. Framework
code extracts descriptor-approved result fields from typed response values and
rejects a fact append when a declared result index cannot be derived safely.

The descriptors, not the state, define index fields:

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
    pub exposure: FactFieldExposure,
}

pub enum FactFieldExposure {
    Returnable,
    SearchOnly,
    EqualityOnly,
    Redacted,
}

pub struct FactResultDescriptor {
    pub fact_kind: FactKind,
    pub response_schema_id: SchemaId,
    pub descriptor_hash: ContentDigest,
    pub fields: &'static [FactResultFieldDescriptor],
    pub orderings: &'static [FactOrderingDescriptor],
}

pub struct FactResultFieldDescriptor {
    pub path: FactResultFieldPath,
    pub value_type: FactFieldValueType,
    pub response_pointer: TypedResponsePointer,
    pub operators: &'static [FactQueryOperator],
    pub exposure: FactFieldExposure,
}

pub struct FactOrderingDescriptor {
    pub name: FactOrderingName,
    pub terms: &'static [FactOrderingTerm],
}

pub enum FactOrderingTerm {
    StoreCommitOrder,
    ObservedAt,
    ResultField {
        path: FactResultFieldPath,
        direction: SortDirection,
    },
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

### Observed-Result Indexing

Observed-result indexing means indexing values from the response artifact, not
from the fact subject.

Examples:

- all wallet balance claims where `amount > 1 BTC`
- all weather observations where `temperature_celsius < 0`
- all price quotes where `price_usd > 100000`
- all chain head claims above a block height when block height is an observed
  response value rather than part of the subject

These are core platform queries. V1 should support them when the fact result
descriptor declares the response field, scalar type, operators, exposure policy,
and ordering behavior.

The constraint is separation, not deferral:

- subject indexes answer "what is this fact about?"
- result indexes answer "what value did this claim observe?"
- ordering policies define how claims are sorted for `latest`, history, and
  bounded result windows

Result indexes are derived from typed response artifacts in Rust append logic,
separate from `FactKey`, so response values do not distort subject identity.

## FactRecorded Contract

`FactRecorded` remains the event. Its contract changes to require typed fact
claim data.

Conceptual shape:

```rust
pub enum FactVisibility {
    RunPrivate,
    Control,
    Platform,
}
```

There is no default. Callers must choose visibility explicitly.

Conceptual claim payload:

```rust
pub struct FactClaim {
    pub visibility: FactVisibility,
    pub fact_kind: FactKind,
    pub key_schema_id: SchemaId,
    pub key_descriptor_hash: ContentDigest,
    pub result_descriptor_hash: Option<ContentDigest>,
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
    FactRecordOptions::new(FactVisibility::Platform).observed_at(source_time),
)?;
```

There is no valid "old shape" `FactRecorded` after this reset. If code cannot
provide typed subject material through `MfmFactKey`, it cannot record a fact.

Private and control facts still use the same typed shape. The difference is
projection and access: `RunPrivate` facts do not create projected fact rows,
`Control` facts create non-public projected fact rows, and `Platform` facts
create public projected fact rows.

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

### Fact Result Descriptors

```sql
CREATE TABLE fact_result_descriptors (
  descriptor_hash TEXT PRIMARY KEY,
  fact_kind TEXT NOT NULL,
  response_schema_id TEXT NOT NULL,
  descriptor_canonical_bytes BYTEA NOT NULL,
  created_at TIMESTAMPTZ NOT NULL
);
```

This table stores immutable result indexing and ordering definitions by hash.
A `FactRecorded` append that names a result descriptor is valid only when the
matching descriptor is available to the store, either already present or
supplied with the append and inserted in the same transaction.

### Projected Fact Keys

```sql
CREATE TABLE projected_fact_keys (
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

### Projected Fact Key Fields

```sql
CREATE TABLE projected_fact_key_fields (
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
    REFERENCES projected_fact_keys(fact_key)
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
CREATE INDEX projected_fact_key_fields_text_idx
  ON projected_fact_key_fields (descriptor_hash, field_path, value_text);

CREATE INDEX projected_fact_key_fields_i64_idx
  ON projected_fact_key_fields (descriptor_hash, field_path, value_i64);

CREATE INDEX projected_fact_key_fields_u64_idx
  ON projected_fact_key_fields (descriptor_hash, field_path, value_u64);

CREATE INDEX projected_fact_key_fields_timestamp_idx
  ON projected_fact_key_fields (descriptor_hash, field_path, value_timestamp);
```

### Projected Fact Result Fields

```sql
CREATE TABLE projected_fact_result_fields (
  source_run_id TEXT NOT NULL,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  result_descriptor_hash TEXT NOT NULL,
  field_path TEXT NOT NULL,
  value_type TEXT NOT NULL,
  value_text TEXT NULL,
  value_i64 BIGINT NULL,
  value_u64 NUMERIC(20,0) NULL,
  value_bool BOOLEAN NULL,
  value_timestamp TIMESTAMPTZ NULL,
  value_decimal TEXT NULL,
  value_digest TEXT NULL,
  PRIMARY KEY (source_run_id, source_seq, source_ordinal, field_path),
  FOREIGN KEY (result_descriptor_hash)
    REFERENCES fact_result_descriptors(descriptor_hash)
    ON DELETE RESTRICT
);
```

Result field rows are per recorded fact because each claim may observe different
values for the same subject. The store derives these rows from the fact result
descriptor plus typed response artifact evidence. States do not supply result
rows or labels directly.

Useful initial indexes:

```sql
CREATE INDEX projected_fact_result_fields_text_idx
  ON projected_fact_result_fields (result_descriptor_hash, field_path, value_text);

CREATE INDEX projected_fact_result_fields_i64_idx
  ON projected_fact_result_fields (result_descriptor_hash, field_path, value_i64);

CREATE INDEX projected_fact_result_fields_u64_idx
  ON projected_fact_result_fields (result_descriptor_hash, field_path, value_u64);

CREATE INDEX projected_fact_result_fields_timestamp_idx
  ON projected_fact_result_fields (result_descriptor_hash, field_path, value_timestamp);

CREATE INDEX projected_fact_result_fields_decimal_idx
  ON projected_fact_result_fields (result_descriptor_hash, field_path, value_decimal);
```

### Projected Facts

```sql
CREATE TABLE projected_facts (
  source_run_id TEXT NOT NULL,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  commit_id TEXT NOT NULL,
  append_xid BIGINT NOT NULL,
  commit_sort_key TEXT NOT NULL,
  recorded_at TIMESTAMPTZ NOT NULL,
  observed_at TIMESTAMPTZ NULL,
  visibility TEXT NOT NULL,
  spec_hash TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  fact_key TEXT NOT NULL,
  descriptor_hash TEXT NOT NULL,
  result_descriptor_hash TEXT NULL,
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
    REFERENCES projected_fact_keys(fact_key)
    ON DELETE RESTRICT,
  FOREIGN KEY (result_descriptor_hash)
    REFERENCES fact_result_descriptors(descriptor_hash)
    ON DELETE RESTRICT
);
```

Types and foreign key names are illustrative. The implementation should bind
these rows tightly to existing `run_events`, `commits`, and artifact admission
tables, and `projected_fact_result_fields` should be tied to the same projected
fact identity. `artifact_id` alone is not enough; the projected reference must
retain artifact evidence hash/provenance needed to verify the selected claim.

`projected_facts` contains both `Platform` and `Control` rows. Public platform
fact APIs should read through a strict view or query boundary filtered to
`visibility = 'Platform'`. Internal collector/runtime capabilities may query
`Control` rows. `RunPrivate` facts are not inserted into projected fact tables.

Useful initial indexes:

```sql
CREATE INDEX projected_facts_fact_kind_recorded_idx
  ON projected_facts (visibility, fact_kind, recorded_at DESC);

CREATE INDEX projected_facts_fact_key_recorded_idx
  ON projected_facts (visibility, fact_key, recorded_at DESC);

CREATE INDEX projected_facts_observed_idx
  ON projected_facts (visibility, fact_kind, observed_at DESC)
  WHERE observed_at IS NOT NULL;

CREATE INDEX projected_facts_store_order_idx
  ON projected_facts (visibility, append_xid, commit_sort_key, source_ordinal);

CREATE INDEX projected_facts_response_idx
  ON projected_facts (visibility, response_schema_id, response_hash);

CREATE INDEX projected_facts_result_descriptor_idx
  ON projected_facts (visibility, result_descriptor_hash, fact_kind);
```

### Projection Algorithm

For each appended `FactRecorded`:

1. Validate the typed fact claim shape.
2. Validate key descriptor hash, fact kind, and key schema id.
3. Validate result descriptor hash and response schema id when result indexing is
   declared.
4. Canonicalize subject material.
5. Recompute `key_material_hash`.
6. Recompute `fact_key`.
7. Derive subject index fields from key descriptor plus material.
8. Derive result index fields from result descriptor plus typed response
   artifact when a result descriptor is present.
9. Verify derived fields satisfy descriptor type, exposure, operator, and size
   limits.
10. If visibility is `RunPrivate`, insert only the run event and artifacts.
11. If visibility is `Control` or `Platform`, upsert descriptors, key, key
    fields, result fields, and projected fact ref inside the same append
    transaction.

If projection fails for a `Control` or `Platform` fact, the append fails. MFM
should not commit an event that is supposed to be cross-run discoverable while
omitting its projection row.

### Ordering And Time

`recorded_at` is store-owned time. In the current PostgreSQL store this should
come from the commit row, not from state code.

`observed_at` is optional source/domain time supplied by the fact producer. It
should be captured as close to the external observation as the source and
adapter can honestly support. If no trustworthy source/domain time exists, it
remains absent.

Every query that asks for "latest" or otherwise depends on order must name an
ordering policy. Different policies are different semantics:

- `StoreCommitOrderDesc`: newest committed fact first, using commit/store order
- `ObservedAtDesc`: newest source/domain observation time first, tie-broken by
  store order
- `ResultFieldDesc(path)` / `ResultFieldAsc(path)`: ordered by a
  result-descriptor field such as `block_number`, `provider_timestamp`,
  `price_time`, `amount`, or `temperature`
- named domain policies such as `chain-finalized-height-desc` when a result
  descriptor defines them as one or more ordering terms

V1 should support `StoreCommitOrderDesc` as the default only when the caller did
not request result or domain semantics. V1 should also support ordering by
descriptor-declared timestamp, integer, unsigned integer, decimal-string, and
digest result fields when the descriptor allows ordering for that field. The
selected ordering policy is part of the canonical query and is hashed into query
evidence.

Store order remains the deterministic tie-breaker:

- source run sequence and ordinal order facts inside one run
- commit ordering orders facts across runs
- `append_xid` and `commit_sort_key` are suitable internal ordering inputs
- public cursors should remain opaque

## Why Not A Materialized View

PostgreSQL materialized views are refreshed in batches. `REFRESH MATERIALIZED
VIEW CONCURRENTLY` reduces read blocking, but it is still not the per-insert
incremental projection MFM needs.

The append path already has a transaction that inserts commits, admits artifacts,
and inserts run events. Projected fact insertion belongs in that transaction.
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
4. emit `Platform` FactRecorded claims
5. emit control checkpoint fact
6. complete cycle
7. operational launcher starts or resumes the next ordinary run cycle
```

### Checkpoints

Checkpoint facts can use `FactRecorded`, but they should be modeled carefully.

A checkpoint fact should include subject material such as collector kind, source,
scope, and stream partition. Its response artifact should include high-watermark,
range, predecessor checkpoint, and any finality policy needed for safe resume.

Checkpoint visibility should usually be `Control`, because the next collector
cycle needs cross-run discovery without publishing operational subjects through
public platform fact APIs. Use `RunPrivate` only when the next cycle already has
an exact pinned predecessor reference. Make a checkpoint `Platform` only when
other public workflows are expected to query and trust it as shared knowledge.

## Consuming Projected Facts

A workflow can query platform or control facts through a typed read capability.
That live query is not replay authority.

Flow:

```text
consumer state queries projected facts
  -> selects zero or more refs under an explicit policy
  -> records PlatformFactQueryEvidence as private read evidence
  -> downstream states consume the pinned evidence
```

The consuming run should not emit a new domain `FactRecorded` merely because it
read an existing fact. It records private query evidence unless it is making a
genuinely new claim.

Conceptual query evidence:

```rust
pub struct PlatformFactQueryEvidence {
    pub store_scope: StoreScopeRef,
    pub read_watermark: StoreReadWatermark,
    pub visibility: FactVisibility,
    pub canonical_query_hash: ContentDigest,
    /// Key and result descriptor hashes/schemas that bound the query.
    pub descriptor_scope: FactDescriptorScope,
    pub ordering_policy_hash: ContentDigest,
    pub limit: Option<u64>,
    pub selection_policy_hash: ContentDigest,
    pub selected_refs: Vec<ProjectedFactRef>,
    pub result_cardinality: QueryResultCardinality,
}
```

This evidence covers selected facts, bounded result sets, and empty-result
branches. For v1, a store-trusted read watermark is enough to support replayable
"no result at this point" decisions. Cryptographic absence proofs can be
deferred.

Replay verifies the pinned source facts, artifact evidence, canonical query,
ordering policy, selection policy, and read watermark. It does not ask "what is
latest now?"

## Public APIs

The public query surface should be kind-first and descriptor-driven. Kind-only
commands are convenience commands; execution semantics are descriptor- or
schema-scoped.

Initial CLI shape:

```sh
mfm facts kinds
mfm facts describe wallet.balance
mfm facts latest wallet.balance \
  --descriptor <descriptor-hash> \
  --result-descriptor <result-descriptor-hash> \
  --order result:observed_block_number:desc \
  chain=bitcoin \
  network=mainnet \
  account_ref=addr:bc1q... \
  asset_ref=btc \
  balance_kind=confirmed
mfm facts history weather.observation \
  --schema mfm.weather.observation-key.v1 \
  --result-schema mfm.weather.observation-result.v1 \
  --order observed-at-desc \
  country=IE \
  locality=Dublin \
  measure=temperature \
  --result temperature_celsius.lt=0
mfm facts show <source-run-id>:<seq>:<ordinal>
mfm facts explain wallet.balance
```

`mfm facts describe wallet.balance` lists known descriptors/schema versions for
the kind. A kind-only query is allowed only when the kind resolves
unambiguously, or when a deterministic compatibility group is registered.
Otherwise the CLI must return an ambiguity error and require `--descriptor` or
`--schema`.

The CLI uses key and result descriptor metadata to validate fields, parse typed
values, enforce field exposure policy, choose an explicit ordering policy, and
compile the query into generic key-field and result-field filters.

Initial REST shape:

```text
GET /v1/facts/kinds
GET /v1/facts/kinds/wallet.balance
GET /v1/facts/wallet.balance/latest?descriptor=...&result_descriptor=...&order=result%3Aobserved_block_number%3Adesc&chain=bitcoin&network=mainnet&asset_ref=btc
GET /v1/facts/weather.observation?schema=mfm.weather.observation-key.v1&result_schema=mfm.weather.observation-result.v1&order=observed-at-desc&country=IE&locality=Dublin&measure=temperature&result=temperature_celsius.lt%3D0
GET /v1/facts/ref/{source_run_id}/{seq}/{ordinal}
```

Raw generic query can exist for tooling:

```sh
mfm facts query \
  --kind wallet.balance \
  --descriptor <descriptor-hash> \
  --result-descriptor <result-descriptor-hash> \
  --order result:amount_sat:desc \
  --field chain=bitcoin \
  --field network=mainnet \
  --field account_ref=addr:bc1q... \
  --field asset_ref=btc \
  --result amount_sat.gt=100000000
```

Public pagination should use opaque cursors. The API should not expose raw
append XIDs, commit sort keys, cursor versions, or store epochs.

### ProjectedFactRef DTO

The internal DTO should remain generic:

```rust
pub struct ProjectedFactRef {
    pub source_run_id: RunId,
    pub source_seq: StreamSeq,
    pub source_ordinal: EventOrdinal,
    pub source_event_id: EventId,
    pub recorded_at: Timestamp,
    pub observed_at: Option<Timestamp>,
    pub visibility: FactVisibility,
    pub fact_kind: FactKind,
    pub fact_key: FactKey,
    pub key_schema_id: SchemaId,
    pub descriptor_hash: ContentDigest,
    pub result_descriptor_hash: Option<ContentDigest>,
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
`asset`, `country`, `measure`, `amount`, `temperature`, or `block_number`.
Those are descriptor-derived key fields joined through `fact_key` and
descriptor-derived result fields joined through the source fact identity.

Public `PlatformFactRef` output is a filtered view of `ProjectedFactRef` for
`visibility = Platform`. It must not return canonical key material wholesale.
Field values may be returned only according to descriptor exposure policy:

- `Returnable`: may be displayed and returned by public APIs
- `SearchOnly`: may be used as a filter but omitted from returned summaries
- `EqualityOnly`: may be matched exactly, but not ranged, listed, or displayed
- `Redacted`: may be retained for authority/rebuild but not queried or returned

## Privacy And Security

Facts are not secret storage.

Rules:

- secret-bearing data must never enter fact subject material, artifacts, events,
  public outputs, diagnostics, fixtures, or snapshots
- explicit visibility controls projection and query access, but does not make
  secret data acceptable
- `RunPrivate` facts should not create projected key rows that would leak
  private-only subjects
- `Control` facts are cross-run discoverable only through internal operational
  capabilities
- public APIs must expose only descriptor-approved subject/result fields and
  must not return canonical key material or response artifacts wholesale
- public APIs must redact errors and avoid endpoint/auth leakage
- source routing, credentials, RPC URLs, authorization headers, passwords,
  keystores, and signer material remain below typed semantic surfaces

## Retention

A queryable projected fact must not outlive the material needed to verify it.

While a `Platform` or `Control` fact remains queryable, the store must retain:

- source run event payload and commit metadata
- descriptor canonical bytes
- result descriptor canonical bytes when present
- key material canonical bytes
- derived key-field rows
- derived result-field rows
- request and response schema/hash evidence
- response artifact
- artifact admission/binding evidence
- capability and adapter provenance

Garbage collection must either keep these materials or first remove the
projected fact from all query surfaces. A projected fact ref that cannot be
verified is corruption.

## Rebuild And Verification

Projection rebuild and validation should ship with v1. Idempotent append alone
is not enough because a retry that sees an existing commit may return early and
will not necessarily repair missing projection rows.

Required validation:

- every projected fact row points to an existing `FactRecorded` event
- every projected event is `Platform` or `Control`
- public platform APIs expose only `Platform` rows
- no `RunPrivate` fact creates projected fact rows or private-only key rows
- descriptor hash, fact kind, and schema id match descriptor canonical bytes
- result descriptor hash and response schema id match result descriptor canonical
  bytes when present
- key material hash matches canonical subject material
- `FactKey` matches descriptor hash plus key material hash
- key-field rows match descriptor-derived values
- result-field rows match response-artifact-derived values
- artifact id and artifact evidence hash match admitted response evidence
- request/response schema and hash match event payload
- source run id, sequence, ordinal, and event id match the run stream
- store ordering columns match the commit row

Rebuild should truncate and repopulate the projection from strict authority and
content-addressed descriptor/material data, then run the same validation checks.

## Failure Semantics

Facts and projected fact rows must be atomic.

- If descriptor validation fails, the append fails.
- If key derivation fails, the append fails.
- If declared result-field derivation fails, the append fails.
- If artifact admission fails, the `FactRecorded` event and projection rows do
  not appear.
- If event insertion fails, projection rows do not appear.
- If `Platform` or `Control` projection fails, the whole append fails.
- Retrying the same prepared commit remains idempotent.
- Projection rebuild from authority produces the same rows.

## Migration Path

This RFC assumes a destructive development reset, not backward compatibility.
Existing fact records, artifacts, and projections from the old contract are
discarded with the old store baseline.

1. Document `FactRecorded` as the platform knowledge primitive.
2. Add explicit `FactVisibility` with `RunPrivate`, `Control`, and `Platform`.
3. Add sealed/derive-owned `MfmFactKey` support and descriptor derivation.
4. Replace ad hoc `FactKey` construction with typed subject material.
5. Require every `FactRecorded` to carry `FactClaim` data.
6. Add key descriptor, result descriptor, key-field, result-field, and
   projected-fact tables.
7. Populate projection tables in the same append transaction as `run_events`.
8. Add projection validation and rebuild.
9. Add descriptor-scoped, kind-first CLI and REST query APIs.
10. Add private `PlatformFactQueryEvidence`.
11. Build the first collector as a recurring certified workflow over ordinary
    states.

## Resolved Decisions

- Collectors are recurring workflows over ops and states, not a new runtime
  category.
- `FactRecorded` is the universal fact event.
- There is no `CollectedFactRecorded`.
- Visibility is explicit; there is no default public publication.
- `Control` is the non-public cross-run visibility for collector checkpoints and
  shared operational state.
- Every `FactRecorded` must have typed subject material after the reset.
- `MfmFactKey` canonicalization is framework-owned, sealed, and derive-oriented.
- `FactKey`, indexed subject fields, and key material hash are key-descriptor
  derived.
- Indexed result fields and result-based ordering policies are
  result-descriptor derived from typed response artifacts.
- Descriptor canonical bytes are durable content-addressed framework artifacts
  admitted before or during the first append that needs them.
- `PlatformFactRef` is generic provenance and ordering, not domain labels.
- Canonical key material is persisted in bounded form for v1.
- Persisted key material is authority/rebuild material, not public output.
- `recorded_at` is store-owned commit time.
- `observed_at` is optional source/domain time supplied by the fact producer.
- Projection for `Platform` and `Control` facts is maintained inside the append
  transaction.
- PostgreSQL materialized views and SQL artifact-parsing triggers are not the v1
  projection strategy.
- Consuming runs pin `PlatformFactQueryEvidence` as private read evidence for
  replay.
- Query ordering policy is explicit and hashed into query evidence.
- V1 supports observed-result indexing for descriptor-declared response fields.
- Queryable projected facts require retention of their verification material.

## Deferred Questions

- Which typed field operators are required beyond equality and timestamp
  ordering in the first public API?
- Do named visibility scopes need to exist before multi-tenant stores, or is the
  v1 `Platform` / `Control` / `RunPrivate` enum sufficient until there is a
  concrete authorization model?
- Which checkpoint facts should ever be platform-visible rather than `Control`?
- Which result field scalar types and operators are needed beyond exact,
  comparison, timestamp, and decimal-string ordering in the first API?
- What operational lease/backoff policy should launch recurring collector
  cycles outside the durable state-machine semantics?
- How should cross-store export/import prove source store trust scope, source
  stream authority, and retained artifact evidence?

## Preferred First Implementation

Start with one vertical slice:

- `MfmFactKey` derive for one domain fact key
- mandatory typed `FactClaim` on `FactRecorded`
- explicit `FactVisibility::{RunPrivate, Control, Platform}`
- descriptor admission as content-addressed framework artifacts
- key descriptor, result descriptor, key-field, result-field, and projected-fact
  PostgreSQL projection tables
- append-transaction projection for `Platform` and `Control` facts
- projection validation and rebuild
- `recorded_at` and optional `observed_at`
- descriptor-scoped, kind-first `mfm facts` query path
- explicit ordering policies for store commit order, observed time, and
  descriptor-declared result fields
- pinned `PlatformFactQueryEvidence` for consumers
- one collector-style workflow that records platform-visible domain facts and
  control checkpoint facts using existing state-machine primitives

This proves the model: MFM's shared knowledge graph is built from ordinary typed
`FactRecorded` events, and collectors are just recurring certified workflows
that produce those events.
