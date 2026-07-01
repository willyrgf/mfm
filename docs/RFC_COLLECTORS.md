# RFC: Collectors And Platform Facts

Status: proposal.

## Summary

Collectors should be first-class MFM workflows, but not a separate runtime
primitive. A collector is a recurring certified workflow that uses ordinary
operations, states, adapters, transports, runtime events, and artifact evidence
to observe the outside world and record facts.

`FactRecorded` is the universal event for adding knowledge to MFM. There should
be no collector-specific fact event such as `CollectedFactRecorded`. Every valid
`FactRecorded` must declare explicit visibility and carry a typed fact claim
described by one durable `FactDescriptor`.

The simplified model is:

```text
FactRecorded
  -> FactDescriptor
  -> FactKey from kind-scoped subject fields
  -> fact_index row
  -> fact_index_terms rows from subject/result/metadata fields
  -> FactQueryReceipt pinned by FactQueryEvidence
```

`FactDescriptor` binds the fact kind, subject type, response type, field
definitions, ordering policies, and exposure rules. Only descriptor fields with
`source = Subject` participate in `FactKey` derivation. Result and metadata
fields make observed values searchable and orderable without changing subject
identity.

In short:

```text
state records typed FactRecorded claim
  -> append-only run event + admitted artifact evidence
  -> FactDescriptor derives subject/result/metadata index terms
  -> FactKey identifies the kind-scoped stable subject
  -> fact_index makes indexed facts queryable by audience
  -> later runs query and pin FactQueryEvidence for replay
```

This is a breaking event-contract reset. Existing opaque or ad hoc fact records
are not migrated. Development stores, artifacts, and projections are reset, and
all fact producers move to the typed fact API.

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
platform knowledge. Once every `FactRecorded` is typed by a durable
`FactDescriptor`, the platform can maintain a generic store-level knowledge
index without hardcoding wallets, weather, prices, chain heads, or any other
domain into `FactRef`.

## Goals

- Reuse operations, states, adapters, transports, runtime, store, and replay.
- Treat `FactRecorded` as the canonical platform knowledge primitive.
- Author facts through one sealed/derive-oriented `MfmFactType`.
- Use one durable `FactDescriptor` per fact shape.
- Treat `FactDescriptor` as certified authority allowed by the certified spec
  for the producing node, not merely store-admitted data.
- Derive `FactKey` only from fact kind, descriptor subject fields, and canonical
  subject material.
- Support observed-result indexing in v1 through descriptor-owned result
  fields.
- Support metadata fields for store/claim fields such as `recorded_at`,
  `observed_at`, and store order.
- Require explicit fact visibility; no implicit public publication.
- Add non-public indexed `Control` audience for cross-run operational facts such
  as collector checkpoints.
- Keep indexed fact refs generic across domains.
- Keep field extraction declarative and kernel-owned so append/rebuild never
  calls arbitrary domain crate code.
- Keep run streams, commits, descriptors, subject material, and artifact
  evidence as strict authority.
- Make fact refs and terms an ordered, indexed, rebuildable store projection.
- Let CLI and REST expose kind-first fact queries that compile to descriptor
  fields.
- Preserve replay by pinning `FactQueryEvidence` into the consuming run.

## Non-Goals

- Do not introduce a collector-specific runtime or daemon semantics in v1.
- Do not introduce `CollectedFactRecorded`.
- Do not let states publish facts through ad hoc side channels.
- Do not allow opaque caller-chosen fact keys after this reset.
- Do not default facts into public platform search.
- Do not make `FactRef` a domain-specific struct or arbitrary label bag.
- Do not use PostgreSQL JSONB-only search as the v1 indexing model.
- Do not parse domain-specific response artifacts in SQL triggers.
- Do not let the store or projection rebuild call domain crate code for field
  extraction.
- Do not let CLI or REST own fact query compilation.
- Do not make mutable "latest fact" rows authoritative.
- Do not support multi-claim response artifacts in v1.
- Do not put checkpoint conflict policy into fact descriptors in v1.
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

Fact kind is the first user-facing query dimension. A fact kind can have
multiple descriptor versions or compatibility groups.

### FactDescriptor

`FactDescriptor` is the durable, content-addressed description of a fact shape.

It binds:

- fact kind
- subject schema id
- response schema id
- fields that derive `FactKey`
- subject fields, result fields, and metadata fields
- field operators, type semantics, units/scales, and exposure policy
- ordering policies and tie-breakers
- compatibility group or schema version metadata

The descriptor hash is computed from canonical descriptor bytes. The descriptor
hash is not embedded inside those canonical bytes.

### Fact Subject

The subject is the stable identity of what the fact is about. It is typed
canonical material described by `source = Subject` fields in `FactDescriptor`.

For a wallet balance, the subject can include chain, network, account, asset,
and balance kind. It should not include the balance amount.

For weather in Dublin, the subject can include country, locality, coordinate
cell, provider when provider changes the identity of the observation, and
measurement kind. It should not include the observed temperature.

For a transaction observation, the subject can include chain, network, and
transaction id. Block height belongs in the subject only when the fact kind is
specifically about a transaction-in-block identity; otherwise it is an observed
result field.

Provider or source belongs in the subject only when it changes what the fact is
about. Otherwise it belongs in provenance, result fields, or metadata fields.

### FactKey

`FactKey` is the stable content-derived key for a fact subject.

Derivation:

```text
fact_subject_namespace_hash = hash(canonical { fact_kind, key_part subject fields })
subject_material_hash = hash(canonical subject material)
fact_key = hash(
  "mfm.fact-key.v1",
  fact_subject_namespace_hash,
  subject_material_hash
)
```

`FactKey` is kind-scoped. Two fact kinds can use the same subject material
without accidentally sharing identity. Adding a result field such as
`amount_sat`, `block_number`, or `temperature_celsius_milli` must not change the
identity of the same wallet balance or weather subject.

### Fact Field

A fact field is a descriptor-declared typed field that can produce index terms.
Terms are projection data, not authority.

Field source:

```rust
pub enum FactFieldSource {
    Subject,
    Result,
    Metadata,
}
```

- `Subject` fields answer "what is this fact about?"
- `Result` fields answer "what did this claim observe?"
- `Metadata` fields answer "how, when, or where was this claim recorded?"

Examples:

```text
subject.chain = bitcoin
subject.account_ref = addr:bc1q...
result.amount_sat = 150000000
result.block_number = 850000
result.temperature_celsius_milli = -2500
metadata.recorded_at = 2026-07-01T...
metadata.observed_at = 2026-07-01T...
```

V1 fields are scalar and single-valued per indexed fact. If a future fact needs
multi-valued fields, the model should add an explicit `field_ordinal` or a
dedicated repeated-value shape.

### Fact Claim

A fact claim is the event-level assertion made by `FactRecorded`.

It binds:

- explicit visibility
- fact kind
- fact descriptor hash
- fact subject namespace hash
- canonical subject material
- derived `FactKey`
- optional source/domain observation time
- request and response schema/hash provenance
- admitted artifact evidence
- producing run, node, attempt, capability, and adapter provenance

The claim is not an unqualified global truth. It is an observation with
provenance.

### FactRef

`FactRef` is a rebuildable store projection row for an indexed `FactRecorded`
claim. It points back to the authoritative run event and artifact evidence.

It is not independent authority. It is a searchable reference into run-stream
authority.

### Control Fact

A control fact is an indexed `FactRecorded` claim with `audience = Control`. It
is non-public and cross-run discoverable for operational state such as collector
checkpoints. Control facts are queryable through internal runtime/collector
capabilities, not through public platform fact APIs.

### FactQueryEvidence

`FactQueryEvidence` is private read evidence recorded by a run that queries
indexed facts.

It pins:

- store scope
- visibility scope
- read frontier/watermark
- canonical query plan
- canonical query receipt
- selection policy hash

Replay uses this evidence and retained artifacts. It does not ask the current
store "what is latest now?"

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
fact kind + typed subject fields + typed subject material -> FactKey -> ordered claims
```

There can be many claims for the same `FactKey`. "Latest" is not a mutable row;
it is a query with `limit = 1` under an explicit temporal or progression
ordering policy.

Use `top`, `ranked`, or ordinary query wording for ranking by values such as
largest balance or highest price. Those are valid result ordering policies, but
they are not automatically "latest" unless the descriptor says the field is a
progression/time axis.

### One Descriptor, Three Field Sources

`FactDescriptor` is the single source of truth for search and ordering.

It defines:

- subject fields from canonical subject material
- result fields from typed response artifacts
- metadata fields from claim/store metadata
- ordering policies over subject, result, and metadata fields
- exposure policy for public output

State code does not attach arbitrary labels. SQL does not parse domain JSON.
The runtime/store derives index terms from typed values using the descriptor.

### Observed Results Are First-Class

Observed-result indexing is in scope for v1.

Examples:

- all wallet balance claims where `amount_sat > 100000000`
- all weather observations where `temperature_celsius_milli < 0`
- all price quotes where `price_usd_micro > 100000000000`
- latest chain head by `result.block_number desc`

The constraint is separation:

- subject fields identify what the fact is about and derive `FactKey`
- result fields index what this claim observed
- metadata fields index claim/store properties

Do not stuff observed values into `FactKey` just because callers need to query
or sort by them.

### Visibility Is Explicit

Every `FactRecorded` declares visibility. There is no implicit public
publication.

Conceptual visibility:

```rust
pub enum FactVisibility {
    RunPrivate,
    Indexed {
        audience: FactAudience,
        scope: FactVisibilityScope,
    },
}

pub enum FactAudience {
    Control,
    Platform,
}
```

`RunPrivate` means "keep this as local run evidence only."

`audience = Control` means "make this discoverable across runs for internal
operational workflows, but do not expose it through public platform fact APIs."

`audience = Platform` means "make this discoverable through public platform
fact APIs, subject to scope authorization and descriptor exposure policy."

V1 uses one concrete default scope:

```text
FactVisibilityScope::Default
```

The enum keeps scope in the contract so collector, source, or tenant boundaries
are not retrofitted later. Full named-scope authorization is deferred, but v1
still records scope decisions in query receipts. Public APIs may query only
`audience = Platform` in the default scope. Internal runtime/collector
capabilities may query `audience = Control` in the default scope.

Visibility does not make secrets acceptable. Secret-bearing data must never
enter fact material, fact artifacts, events, public outputs, diagnostics,
fixtures, or snapshots.

### Authority Remains Append-Only

The authoritative record is still:

- run events
- commits
- descriptor canonical bytes as admitted framework artifacts
- canonical subject material
- artifact admissions and bindings
- canonical event payload bytes

Fact refs and index terms are query projections. They must be rebuildable and
verifiable from durable authority.

### Live Search Is Not Replay

A live run may query platform or control facts. The query result, including
empty results and bounded result sets, must be pinned into the consuming run as
`FactQueryEvidence`.

Replay uses pinned query evidence and retained artifacts, not the current fact
index and not a live collector.

## Facts Kernel Contract

The platform should define a small facts kernel contract reused by states,
runners, stores, CLI, REST, and collectors. This is not a new runtime or
collector framework. It is the shared contract for typed fact publication,
projection, query compilation, and replay evidence.

The facts kernel owns:

- certified `FactDescriptor` admission and validation
- descriptor-declared field extraction
- stable field ids and field compatibility rules
- generic query compilation from user/API filters into canonical field queries
- ordering normalization and tie-breaker rules
- projection/rebuild validation rules
- `FactQueryEvidence` shape and canonicalization

Certified workflow specs must declare which fact descriptor hashes each node may
emit. Runner bindings may enforce that contract, but they are not an independent
source of semantic authority. Store admission proves descriptor bytes are
available; it does not by itself prove the certified workflow was allowed to
publish that descriptor. An append is valid only when the `FactRecorded`
descriptor is both:

- content-addressed and available to the store
- allowed by the certified spec for the producing node

## FactDescriptor Contract

Every `FactRecorded` producer must use a sealed/derive-owned fact type. The
runner API should make this the only available path for fact publication.

Conceptual API:

```rust
pub trait MfmFactType: private::Sealed {
    type Subject: MfmValue;
    type Response: MfmValue;

    fn descriptor() -> &'static FactDescriptor;
}
```

Framework-owned canonicalization turns typed subject values into canonical bytes.
Field extraction must be descriptor-declared and kernel-owned. Append and
rebuild must not call domain crate code to compute terms. The descriptor must
lower to a small generic extraction language over canonical subject values,
canonical response values, and claim/store metadata.

Manual implementations must not return raw canonical bytes, ad hoc labels, or
precomputed terms as unchecked authority.

Conceptual descriptor shape:

```rust
pub struct FactDescriptor {
    pub fact_kind: FactKind,
    pub descriptor_schema_id: SchemaId,
    pub subject_schema_id: SchemaId,
    pub response_schema_id: SchemaId,
    pub compatibility_group: Option<FactCompatibilityGroup>,
    pub fields: &'static [FactFieldDescriptor],
    pub orderings: &'static [FactOrderingDescriptor],
}

pub struct FactFieldDescriptor {
    pub field_id: FactFieldId,
    pub source: FactFieldSource,
    pub path: FactFieldPath,
    pub value_type: FactFieldValueType,
    pub extraction: FactFieldExtraction,
    pub operators: &'static [FactQueryOperator],
    pub exposure: FactFieldExposure,
    pub unit: Option<FactUnit>,
    pub scale: Option<FactScale>,
    pub sortable: bool,
    pub key_part: bool,
    pub required: bool,
}

pub enum FactFieldExposure {
    Returnable,
    QueryOnly,
    Hidden,
}

pub struct FactOrderingDescriptor {
    pub name: FactOrderingName,
    pub terms: &'static [FactOrderingTerm],
}

pub struct FactOrderingTerm {
    pub field_id: FactFieldId,
    pub source: FactFieldSource,
    pub path: FactFieldPath,
    pub value_type: FactFieldValueType,
    pub direction: SortDirection,
    pub nulls: NullOrdering,
    pub tie_breaker: bool,
}
```

`field_id` is descriptor-owned stable identity. `path` is a human-readable and
schema-facing pointer that may evolve across descriptor versions. Index rows,
ordering policies, compatibility rules, and query evidence bind to `field_id`.

`key_part = true` is valid only for `source = Subject`. The ordered set of
required subject fields with `key_part = true` defines subject key material for
the descriptor. Result and metadata fields never participate in `FactKey`.

V1 extraction grammar:

```rust
pub enum FactFieldExtraction {
    SubjectPath(CanonicalValuePath),
    ResponsePath(CanonicalValuePath),
    Metadata(FactMetadataField),
}
```

V1 extraction rules:

- paths are absolute object paths over canonical JSON-like `MfmValue` data
- path components are object field names only
- arrays, repeated values, wildcards, and slices are not supported in v1
- extracted values must be scalar: string, bool, signed integer, unsigned
  integer, timestamp string, numeric decimal string, or digest string
- floats are invalid
- no implicit scalar coercion is allowed except descriptor-declared
  integer-to-decimal widening for `value_numeric`
- `required = true` missing/null values fail append for indexed facts
- `required = false` missing/null values produce no index term
- extracted scalar byte length and numeric precision must be bounded by the
  descriptor and facts-kernel defaults
- extracted values must match `value_type`, `unit`, `scale`, and `sortable`
  encoding rules exactly
- extraction language version is part of descriptor canonical bytes

This is intentionally smaller than a general expression language. It is
deterministic, bounded, and replayable from retained authority.

Ordering policies must define type comparison, null handling, and deterministic
tie-breakers. Store order should be the final fallback tie-breaker.

Descriptor hashing:

```text
fact_descriptor_hash = hash(canonical FactDescriptor bytes)
fact_subject_namespace_hash = hash(canonical { fact_kind, key_part subject fields })
subject_material_hash = hash(canonical subject material)
fact_key = hash(
  "mfm.fact-key.v1",
  fact_subject_namespace_hash,
  subject_material_hash
)
```

The descriptor hash and fact subject namespace hash are computed values. They
must not be embedded inside the canonical bytes they hash.

The canonical subject material and descriptor bytes must follow platform hashing
rules:

- canonical JSON semantics
- no floats in hashed structures
- bounded size
- no secrets
- stable normalized identifiers
- explicit schema ids

For numeric result fields, prefer integer-scaled values such as `amount_sat`,
`temperature_celsius_milli`, or `price_usd_micro`. If a decimal field is needed,
its descriptor must define an order-preserving canonical encoding or use a
bounded database numeric type. Raw text ordering is not valid for numeric
semantics.

### Example: Wallet Balance

```rust
#[derive(MfmFactType)]
#[mfm_fact(kind = "wallet.balance", schema = "mfm.wallet.balance.v1")]
pub struct WalletBalanceFact {
    pub subject: WalletBalanceSubject,
    pub response: WalletBalanceObservation,
}

pub struct WalletBalanceSubject {
    #[mfm_fact(field = "subject.chain", key)]
    pub chain: ChainRef,
    #[mfm_fact(field = "subject.network", key)]
    pub network: NetworkRef,
    #[mfm_fact(field = "subject.account_ref", key)]
    pub account_ref: AccountRef,
    #[mfm_fact(field = "subject.asset_ref", key)]
    pub asset_ref: AssetRef,
    #[mfm_fact(field = "subject.balance_kind", key)]
    pub balance_kind: BalanceKind,
}

pub struct WalletBalanceObservation {
    #[mfm_fact(field = "result.amount_sat", orderable)]
    pub amount_sat: u64,
    #[mfm_fact(field = "result.block_number", orderable)]
    pub block_number: u64,
}
```

`FactKey` is derived from `WalletBalanceSubject`. `amount_sat` and
`block_number` are result fields for filtering and ordering claims.

### Example: Weather Observation

```rust
#[derive(MfmFactType)]
#[mfm_fact(kind = "weather.observation", schema = "mfm.weather.observation.v1")]
pub struct WeatherObservationFact {
    pub subject: WeatherObservationSubject,
    pub response: WeatherObservationResult,
}

pub struct WeatherObservationSubject {
    #[mfm_fact(field = "subject.country", key)]
    pub country: CountryCode,
    #[mfm_fact(field = "subject.locality", key)]
    pub locality: Locality,
    #[mfm_fact(field = "subject.coordinate_cell", key)]
    pub coordinate_cell: CoordinateCell,
    #[mfm_fact(field = "subject.measure", key)]
    pub measure: WeatherMeasure,
}

pub struct WeatherObservationResult {
    #[mfm_fact(field = "result.temperature_celsius_milli", orderable)]
    pub temperature_celsius_milli: i64,
    #[mfm_fact(field = "result.provider_time", orderable)]
    pub provider_time: Timestamp,
}
```

Temperature is an observed result field, not part of subject identity.

## FactRecorded Contract

`FactRecorded` remains the event. Its contract changes to require typed fact
claim data.

The recording API accepts only producer-owned input:

```rust
pub struct FactRecordInput<T: MfmFactType> {
    pub fact: T,
    pub visibility: FactVisibility,
    pub observed_at: Option<Timestamp>,
}
```

The runtime/store derives and seals the normalized claim fields. Callers do not
provide hashes, keys, descriptor ids, artifact evidence hashes, or schema ids by
hand.

Conceptual normalized event shape:

```rust
pub struct FactClaim {
    pub visibility: FactVisibility,
    pub fact_kind: FactKind,
    pub fact_descriptor_hash: ContentDigest,
    pub fact_subject_namespace_hash: ContentDigest,
    pub subject_schema_id: SchemaId,
    pub response_schema_id: SchemaId,
    pub subject_material: PlainCanonicalJsonBytes,
    pub subject_material_hash: ContentDigest,
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

The v1 runtime helper uses one canonical material source: one typed fact value
stages one response artifact and records one claim. The artifact canonical bytes
are the source for response decoding, field extraction, hashes, and artifact
evidence. The helper must not accept an independent typed response and unrelated
response artifact that can drift.

```rust
recorder.record_fact(
    WalletBalanceFact {
        subject,
        response,
    },
    FactRecordOptions::new(FactVisibility::Indexed {
        audience: FactAudience::Platform,
        scope,
    })
    .observed_at(source_time),
)?;
```

The helper stages the response artifact from the typed `WalletBalanceFact`
material, computes the canonical response hash, admits the artifact, derives
index terms, and records `FactRecorded` from that same material.

There is no valid old-shape `FactRecorded` after this reset. If code cannot
provide a sealed `MfmFactType`, it cannot record a fact.

### Batched Response Binding

V1 supports one claim per response artifact. Multi-claim response artifacts are
deferred.

When introduced, batched artifacts will need binding evidence for each claimed
slice:

- typed response pointer
- slice/index path
- slice canonical hash
- schema id for the slice when different from the full response

Until that contract exists, a fact recorder must split observations into one
typed response artifact per `FactRecorded`.

## Projection Model

The PostgreSQL store should maintain normal projection tables, not a PostgreSQL
materialized view and not JSONB-only search.

The projection is populated in the same append transaction that inserts commits,
admits artifacts, and inserts `run_events`.

The RFC describes logical storage. The implementation may split or combine
physical tables for performance, but it must preserve the same invariants,
foreign-key relationships, rebuildability, and visibility filtering.

### Descriptor Authority

Descriptor canonical bytes are durable content-addressed framework artifacts.
They are authority, not a rebuildable projection. A rebuild must be able to load
the descriptor bytes from retained artifact authority; it must not depend on the
current Rust code to reconstruct descriptor meaning.

The descriptor artifact must be admitted before or during the first append that
records a fact using it. Admission alone is not enough. The producing node's
certified spec must also allow the descriptor hash.

### fact_descriptor_index

Stores searchable descriptor metadata and lookup pointers to descriptor
authority.

Conceptual columns:

```text
descriptor_hash
descriptor_artifact_id
fact_kind
descriptor_schema_id
subject_schema_id
response_schema_id
fact_subject_namespace_hash
compatibility_group
created_at
```

A `FactRecorded` append is valid only when the descriptor is available to the
store and certified for the producing node. The descriptor may already be
present or be supplied and admitted in the same transaction, but store admission
is necessary, not sufficient.

Canonical subject material is retained authority. It must be bounded,
non-secret, and unavailable through generic public read APIs. Public access to
subject information goes through descriptor-approved subject fields and exposure
policy only.

### fact_index

Stores one indexed claim per `FactRecorded` with `FactVisibility::Indexed`.

Conceptual columns:

```text
source_run_id
source_seq
source_ordinal
source_event_id
commit_id
store_commit_order
recorded_at
observed_at
audience
visibility_scope
fact_kind
fact_descriptor_hash
fact_subject_namespace_hash
fact_key
subject_material_hash
request_schema_id
request_hash
response_schema_id
response_hash
artifact_id
artifact_evidence_hash
capability_kind
capability_version
adapter_kind
adapter_version
```

`RunPrivate` facts are not inserted into `fact_index`.

Public platform APIs must read through a strict view or query boundary filtered
to `audience = Platform` and an allowed `visibility_scope`. Internal
collector/runtime capabilities may query `audience = Control`.

### fact_index_terms

Stores generic typed field terms for indexed facts.

Logical shape:

```text
source_run_id
source_seq
source_ordinal
fact_key
fact_descriptor_hash
field_source            // subject | result | metadata
field_id
field_path
value_type
unit
scale
value_text
value_i64
value_u64
value_bool
value_timestamp
value_numeric
value_digest
sortable_value
```

V1 stores subject, result, and metadata terms per indexed fact. Subject terms
may duplicate across claims with the same `FactKey`; deduplicating them into a
separate subject-term table is a future physical optimization, not v1 semantics.

Term rows are projection data. They are rebuildable from descriptors, subject
material, response artifacts, and claim metadata.

Useful logical indexes:

```text
(fact_descriptor_hash, field_id, typed_value)
(audience, visibility_scope, fact_kind, recorded_at desc)
(audience, visibility_scope, fact_key, recorded_at desc)
(audience, visibility_scope, fact_descriptor_hash, field_id, sortable_value)
(store_commit_order, source_run_id, source_seq, source_ordinal)
```

These are logical access paths. A physical implementation may satisfy them with
joins against `fact_index`, partial indexes per audience, or safe
denormalization of audience and fact kind onto term rows.

### Projection Algorithm

For each appended `FactRecorded`:

1. Validate the typed fact claim shape.
2. Admit or load the `FactDescriptor`.
3. Verify the descriptor hash is allowed by the certified spec for the
   producing node.
4. Validate fact kind, subject schema id, response schema id, and descriptor
   hash.
5. Canonicalize subject material.
6. Recompute subject material hash and fact subject namespace hash.
7. Recompute `FactKey` from fact subject namespace hash and subject material
   hash.
8. Validate that the response artifact contains exactly one claim payload in v1.
9. Derive subject terms from subject material using the kernel extractor.
10. Derive result terms from the typed response artifact
    using the kernel extractor.
11. Derive metadata terms from claim/store metadata using the kernel extractor.
12. Verify field id, type, unit, scale, operator, exposure, size, and scalar
    constraints.
13. If visibility is `RunPrivate`, admit descriptor evidence and insert only the
    run event and artifacts; do not insert fact index or term rows.
14. If visibility is `Indexed`, upsert descriptor index, fact index, and index
    terms inside the same append transaction.

If projection fails for an indexed fact, the append fails. MFM should not commit
an event that is supposed to be cross-run discoverable while omitting its
projection rows.

## Ordering And Query Semantics

Every query that asks for "latest" or otherwise depends on order must name an
ordering policy. Different policies are different semantics.

Ordering terms are canonical structures:

```text
source       // metadata | result | subject
field_id
path
value_type
direction
nulls
tie_breakers
```

Examples:

- `store_commit_desc`: metadata store order
- `observed_at_desc`: metadata/source observation time, tie-broken by store
  order
- `result:block_number:desc`: result progression order for chain observations
- `result:provider_time:desc`: provider timestamp order
- `result:amount_sat:desc`: ranking/top balance claims, not necessarily latest

`latest` means `limit 1` plus an explicit temporal or progression ordering.
Ranking by amount, price, score, or temperature should use `top`, `ranked`, or
ordinary query wording unless the descriptor declares the field as a progression
axis.

Exact-subject queries and grouped latest queries are distinct:

```text
latest wallet.balance for one fact_key
latest-per-subject wallet.balance where subject.asset_ref = btc
top wallet.balance by result.amount_sat desc
```

Public cursors are opaque. Internal order should use store-owned order
coordinates such as `store_commit_order`, commit id, source run id, source
sequence, and source ordinal as deterministic tie-breakers.

## Why Not A Materialized View

PostgreSQL materialized views are refreshed in batches. `REFRESH MATERIALIZED
VIEW CONCURRENTLY` reduces read blocking, but it is still not the per-insert
incremental projection MFM needs.

The append path already has a transaction that inserts commits, admits
artifacts, and inserts run events. Fact index insertion belongs in that
transaction.

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
`run_events`. It should validate fact claims, derive index terms, and insert
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
4. emit indexed Platform FactRecorded claims
5. emit indexed Control checkpoint fact
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
public workflows are expected to query and trust it as shared knowledge.

V1 checkpoint facts are ordinary `Control` facts. Their descriptor should expose
partition identity and high-watermark fields, and the next collector cycle
should use a certified query/selection policy to choose the checkpoint it will
resume from. That query is pinned as `FactQueryEvidence`.

The v1 facts kernel does not enforce checkpoint monotonicity inside
`FactDescriptor`. If a collector needs exclusive advancement across competing
writers, that must be modeled as a separate generic lease, compare-and-append,
or side-effect authority before the checkpointed collector is implemented.

## Consuming Indexed Facts

A workflow can query platform or control facts through a typed read capability.
That live query is not replay authority.

Flow:

```text
consumer state queries indexed facts
  -> store returns zero or more refs under canonical query/order/limit
  -> state selects zero or more refs under selection policy
  -> records FactQueryEvidence as private read evidence
  -> downstream states consume the pinned evidence
```

The consuming run should not emit a new domain `FactRecorded` merely because it
read an existing fact. It records private query evidence unless it is making a
genuinely new claim.

Conceptual query evidence:

```rust
pub struct FactQueryEvidence {
    pub plan: CanonicalFactQueryPlan,
    pub receipt: FactQueryReceipt,
    pub selection_policy_hash: ContentDigest,
}

pub struct CanonicalFactQueryPlan {
    pub store_scope: StoreScopeRef,
    pub visibility: FactVisibility,
    pub query_compiler_version: Version,
    pub canonicalizer_version: Version,
    pub resolved_descriptors: Vec<ContentDigest>,
    pub scope_decision_evidence: ScopeDecisionEvidence,
    pub canonical_query: PlainCanonicalJsonBytes,
    pub canonical_query_hash: ContentDigest,
    pub ordering: FactOrdering,
    pub limit: Option<u64>,
}

pub struct FactQueryReceipt {
    pub read_frontier: StoreReadFrontier,
    pub frontier_type: StoreReadFrontierType,
    pub returned_refs: Vec<FactRef>,
    pub returned_field_summaries: Option<ReturnedFieldSummaries>,
    pub selected_refs: Vec<FactRef>,
    pub result_set_digest: ContentDigest,
    pub result_cardinality: QueryResultCardinality,
}
```

This evidence covers selected facts, bounded result sets, and empty-result
branches. Returned refs are enough only when replay can deterministically
recompute every value used by the selection policy from retained artifacts. If a
selection policy depends on returned summaries, the evidence must pin those
summary values or their digest.

For v1, a store-trusted read frontier is enough to support replayable "no result
at this point" decisions only if the frontier is a semantic fact-read frontier.
It must bind the store scope, resolved descriptor set, visibility/scope decision,
ordering policy, and maximum indexed store order included by the read. Ordinary
database cursors or pagination tokens are not replay authority. Cryptographic
absence proofs can be deferred.

Replay verifies the pinned source facts, artifact evidence, canonical query,
ordering policy, selection policy, returned refs, selected refs, result set
digest, scope decision, descriptor resolution, query compiler/canonicalizer
versions, and read frontier.

## Public APIs

The public query surface should be kind-first and descriptor-driven. Kind-only
commands are convenience commands; execution semantics are descriptor-scoped.

Initial CLI shape:

```sh
mfm facts kinds
mfm facts describe wallet.balance
mfm facts latest wallet.balance \
  --shape mfm.wallet.balance.v1 \
  --order result:block_number:desc \
  --subject chain=bitcoin \
  --subject network=mainnet \
  --subject account_ref=addr:bc1q... \
  --subject asset_ref=btc \
  --subject balance_kind=confirmed

mfm facts history weather.observation \
  --shape mfm.weather.observation.v1 \
  --order metadata:observed_at:desc \
  --subject country=IE \
  --subject locality=Dublin \
  --subject measure=temperature \
  --result temperature_celsius_milli.lt=0

mfm facts top wallet.balance \
  --shape mfm.wallet.balance.v1 \
  --order result:amount_sat:desc \
  --subject asset_ref=btc \
  --limit 20

mfm facts show <source-run-id>:<seq>:<ordinal>
mfm facts explain wallet.balance
```

`mfm facts describe wallet.balance` lists known descriptors/schema versions for
the kind. A kind-only query is allowed only when the kind resolves
unambiguously, or when a deterministic compatibility group is registered.
Otherwise the CLI must return an ambiguity error and require `--shape` or a
descriptor hash.

`--shape` is a user-facing selector for one `FactDescriptor`: it may be a schema
alias, compatibility-group version, or descriptor hash, but execution must
resolve it to one descriptor before planning the query.

CLI and REST must not own query compilation. They decode transport input and
render output. A reusable facts-query crate or service resolves descriptors,
validates fields, parses typed values, enforces exposure policy, normalizes
ordering, and compiles input into canonical field queries and
`CanonicalFactQueryPlan`.

Initial REST shape:

```text
GET /v1/facts/kinds
GET /v1/facts/kinds/wallet.balance
GET /v1/facts/wallet.balance/latest?shape=mfm.wallet.balance.v1&order=result%3Ablock_number%3Adesc&subject=chain%3Dbitcoin&subject=asset_ref%3Dbtc
GET /v1/facts/weather.observation?shape=mfm.weather.observation.v1&order=metadata%3Aobserved_at%3Adesc&subject=country%3DIE&result=temperature_celsius_milli.lt%3D0
GET /v1/facts/ref/{source_run_id}/{seq}/{ordinal}
```

Raw generic query can exist for tooling:

```sh
mfm facts query \
  --kind wallet.balance \
  --shape mfm.wallet.balance.v1 \
  --order result:amount_sat:desc \
  --where subject.chain.eq=bitcoin \
  --where subject.asset_ref.eq=btc \
  --where result.amount_sat.gt=100000000
```

Public pagination should use opaque cursors. The API should not expose raw
database transaction ids, cursor versions, or store epochs.

### FactRef DTO

The ref DTO should remain generic:

```rust
pub struct FactRef {
    pub source_run_id: RunId,
    pub source_seq: StreamSeq,
    pub source_ordinal: EventOrdinal,
    pub source_event_id: EventId,
    pub recorded_at: Timestamp,
    pub observed_at: Option<Timestamp>,
    pub visibility: FactVisibility,
    pub fact_kind: FactKind,
    pub fact_descriptor_hash: ContentDigest,
    pub fact_subject_namespace_hash: ContentDigest,
    pub fact_key: FactKey,
    pub subject_material_hash: ContentDigest,
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
Those are descriptor-derived fields joined through `fact_key` or source claim
identity.

Public API output is a filtered `FactRef` view for `audience = Platform`. It
must not return canonical subject material or response artifacts wholesale.

Field values may be returned only according to descriptor exposure policy:

- `Returnable`: may be displayed and returned by public APIs
- `QueryOnly`: may be used as a filter/order term but omitted from returned
  summaries
- `Hidden`: retained for authority/rebuild or internal use, but not queried or
  returned publicly

Allowed operators decide equality, range, prefix, or ordering support. Exposure
does not encode operator semantics.

## Privacy And Security

Facts are not secret storage.

Rules:

- secret-bearing data must never enter fact subject material, artifacts, events,
  public outputs, diagnostics, fixtures, or snapshots
- explicit visibility controls projection and query access, but does not make
  secret data acceptable
- `RunPrivate` facts should not create index terms that would leak private-only
  subjects
- `Control` audience facts are cross-run discoverable only through internal
  operational capabilities and their scopes
- public APIs must expose only descriptor-approved fields and must not return
  canonical subject material or response artifacts wholesale
- there should be no generic public raw-subject-material read path
- public APIs must redact errors and avoid endpoint/auth leakage
- source routing, credentials, RPC URLs, authorization headers, passwords,
  keystores, and signer material remain below typed semantic surfaces

## Retention

A queryable indexed fact must not outlive the authority needed to verify it.

While an indexed fact remains queryable, the store must retain:

- source run event payload and commit metadata
- descriptor canonical bytes
- canonical subject material bytes
- request and response schema/hash evidence
- response artifact
- artifact admission/binding evidence
- capability and adapter provenance

If a consuming run records `FactQueryEvidence`, retention also follows the query
evidence edge. The referenced `FactRef`s, descriptor artifacts, source events,
subject material, response artifacts, and receipt authority must remain
available for replay even if the fact is later removed from live query surfaces.

Index terms and fact index rows are not authority. They are rebuildable
projection data. Garbage collection must either keep authority materials or
first remove the indexed fact from all query surfaces and prove that no retained
query evidence still depends on it.

## Rebuild And Verification

Projection rebuild and validation should ship with v1. Idempotent append alone
is not enough because a retry that sees an existing commit may return early and
will not necessarily repair missing projection rows.

Required validation:

- every fact index row points to an existing `FactRecorded` event
- every fact index row has `FactVisibility::Indexed`
- public platform APIs expose only authorized `audience = Platform` rows
- no `RunPrivate` fact creates fact index or term rows
- descriptor hash matches descriptor canonical bytes
- descriptor hash is allowed by the producing node's certified spec
- fact subject namespace hash matches descriptor fact kind and key-part subject
  fields
- subject material hash matches canonical subject material
- `FactKey` matches fact subject namespace hash plus subject material hash
- the response artifact contains exactly one claim payload in v1
- subject terms match descriptor-derived subject values
- result terms match descriptor-derived response values
- metadata terms match claim/store metadata
- artifact id and artifact evidence hash match admitted response evidence
- request/response schema and hash match event payload
- source run id, sequence, ordinal, and event id match the run stream
- store order coordinates match the commit row

Rebuild should truncate and repopulate projections from strict authority and
then run the same validation checks.

## Failure Semantics

Facts and fact index rows must be atomic.

- If descriptor validation fails, the append fails.
- If the descriptor is not certified for the producing node, the append fails.
- If subject derivation fails, the append fails.
- If declared field extraction fails, the append fails.
- If the response artifact does not satisfy the v1 single-claim rule, the append
  fails.
- If artifact admission fails, the `FactRecorded` event and projection rows do
  not appear.
- If event insertion fails, projection rows do not appear.
- If indexed projection fails, the whole append fails.
- Retrying the same prepared commit remains idempotent.
- Projection rebuild from authority produces the same rows.

## Migration Path

This RFC assumes a destructive development reset, not backward compatibility.
Existing fact records, artifacts, and projections from the old contract are
discarded with the old store baseline.

1. Document `FactRecorded` as the platform knowledge primitive.
2. Add explicit scoped `FactVisibility` with `RunPrivate` and indexed
   `FactAudience::{Control, Platform}`.
3. Add sealed/derive-owned `MfmFactType` support.
4. Add certified `FactDescriptor` admission and validation against the
   producing node's certified spec.
5. Replace ad hoc `FactKey` construction with typed subject material.
6. Require every `FactRecorded` to carry `FactClaim` data.
7. Add descriptor authority artifacts plus logical `fact_descriptor_index`,
   `fact_index`, and `fact_index_terms` projections.
8. Populate projections in the same append transaction as `run_events`.
9. Add projection validation and rebuild.
10. Add reusable facts-query compilation and canonicalization.
11. Add private `FactQueryEvidence` with `CanonicalFactQueryPlan` and
    `FactQueryReceipt`.
12. Add descriptor-scoped, kind-first CLI and REST query APIs over the reusable
    query compiler.
13. Build the first collector as a recurring certified workflow over ordinary
    states.

## Resolved Decisions

- Collectors are recurring workflows over ops and states, not a new runtime
  category.
- `FactRecorded` is the universal fact event.
- There is no `CollectedFactRecorded`.
- Visibility is explicit and scoped; there is no default public publication.
- `Control` is the non-public indexed audience for collector checkpoints and
  shared operational state.
- Every `FactRecorded` must use a sealed typed `MfmFactType` after the reset.
- Fact descriptors are certified authority: a node may emit only descriptors
  allowed by its certified spec.
- `FactDescriptor` is the durable shape for subject identity, result fields,
  metadata fields, ordering, operators, units/scales, and exposure.
- Field extraction is declarative and kernel-owned; append/rebuild must not call
  arbitrary domain crate code.
- Fields have stable descriptor-owned ids; paths are UX/schema pointers.
- `FactKey` is derived only from fact kind, key-part subject fields, and
  canonical subject material.
- Observed-result indexing is in v1 through descriptor-declared result fields.
- Metadata fields are in v1 for fields such as `recorded_at`, `observed_at`,
  and store order.
- Descriptor canonical bytes are durable content-addressed framework artifacts
  admitted before or during the first append that needs them.
- Descriptor hashes are not embedded inside descriptor canonical bytes.
- `FactRef` is generic provenance and ordering, not domain labels.
- Canonical subject material is persisted in bounded form for v1.
- Persisted subject material is authority/rebuild material, not public output.
- Projection for indexed facts is maintained inside the append transaction.
- PostgreSQL materialized views and SQL artifact-parsing triggers are not the v1
  projection strategy.
- Consuming runs pin `FactQueryEvidence` as private read evidence for replay.
- Query compilation lives in a reusable facts-query crate/service; CLI and REST
  remain transport adapters.
- Query evidence stores a canonical query plan and query receipt, not only
  hashes.
- Query ordering policy is explicit and hashed into query evidence.
- Queryable indexed facts require retention of their verification authority.
- Query evidence creates retention edges for the facts and artifacts it pins.
- V1 supports one claim per response artifact.
- V1 checkpoint facts are ordinary `Control` facts; checkpoint conflict
  enforcement is outside fact descriptors.

## V1 Deferrals

- Batched or multi-claim response artifacts.
- Cryptographic absence proofs for fact queries.
- Multi-valued fields and array extraction.
- Full named-scope authorization beyond `FactVisibilityScope::Default`.
- Public pagination internals beyond opaque cursors.
- Subject-term deduplication as a required physical storage shape.
- Cross-store fact export/import.
- Generic lease or compare-and-append authority for exclusive checkpoint
  advancement.

## Deferred Questions

- Which typed field operators are required beyond equality, comparison,
  timestamp ordering, and descriptor-declared sortable numeric fields in the
  first public API?
- Which checkpoint facts should ever be platform-visible rather than `Control`?
- What operational lease/backoff policy should launch recurring collector
  cycles outside the durable state-machine semantics?

## Implementation Ownership

Suggested crate placement for the first planning pass:

- `crates/kernel/facts`: fact descriptors, field descriptors, extraction
  grammar, `FactKey`, `FactRef`, query plan/receipt types, and validation logic.
- `crates/kernel/events`: new `FactRecorded` event payload and normalized
  `FactClaim` shape.
- `crates/kernel/values`: shared `MfmValue` canonical value integration needed
  by fact descriptor derivation and extraction.
- `crates/kernel/program` and `crates/kernel/program-derive`: typed fact derive
  support and certified node emission metadata.
- `crates/kernel/spec` and `crates/kernel/certify`: descriptor allow-list in
  certified spec material.
- `crates/kernel/store`: abstract append/query/rebuild contracts for fact
  authority and fact indexes.
- `crates/storages/stream-store-postgres`: descriptor artifact admission,
  `fact_descriptor_index`, `fact_index`, `fact_index_terms`, append projection,
  query execution, and rebuild.
- `crates/app`, `bin/cli`, and `bin/rest-api`: transport-level command/API
  surfaces over the reusable facts-query contract.

## Preferred First Implementation

Build this in gates, while keeping one vertical slice as the north star.

Gate 1: facts kernel types and canonical contracts.

- `FactVisibility::{RunPrivate, Indexed { audience, scope }}`
- `FactAudience::{Control, Platform}`
- `FactDescriptor`, `FactFieldDescriptor`, field ids, and ordering descriptors
- kind-scoped `FactKey` derivation
- v1 field extraction grammar and validation
- `FactRecordInput`, normalized `FactClaim`, `FactRef`
- canonical hash golden tests and descriptor compatibility tests

Gate 2: certified descriptor emission.

- one `MfmFactType` derive for a domain fact
- descriptor artifact generation/admission
- certified spec descriptor allow-list per producing node
- runtime append rejection for descriptors not allowed by certified spec
- compile-fail tests for invalid derive shapes
- certification tests proving descriptor allow-list changes affect spec hashes

Gate 3: event reset and append projection.

- mandatory normalized `FactClaim` on `FactRecorded`
- v1 one-claim-per-response-artifact rule
- logical `fact_descriptor_index`, `fact_index`, and `fact_index_terms`
  projections in PostgreSQL
- subject, result, and metadata term extraction in the append transaction
- append rollback tests for descriptor, extraction, artifact, and projection
  failures
- projection validation and rebuild parity tests

Gate 4: reusable query compiler and replay evidence.

- reusable facts-query crate/service
- canonical query and ordering representation
- explicit ordering policies for store commit order, observed time, and
  descriptor-declared result fields
- semantic `StoreReadFrontier`
- `CanonicalFactQueryPlan` with descriptor resolution,
  compiler/canonicalizer version, scope decision evidence, canonical query,
  ordering, and limit
- `FactQueryReceipt` with frontier type, returned refs, selected refs, optional
  field summaries, result cardinality, and result-set digest
- retention edges from query evidence

Gate 5: public surfaces.

- descriptor-scoped, kind-first `mfm facts` query path
- REST query surface over the same query compiler
- public `FactRef` output filtered to `audience = Platform`
- internal `Control` query capability for runtime/collector workflows
- public/Control visibility rejection tests

Gate 6: first collector workflow.

- one collector-style workflow that records platform-visible domain facts using
  existing state-machine primitives
- ordinary `Control` checkpoint facts with partition and high-watermark fields
- certified checkpoint query/selection policy pinned by `FactQueryEvidence`
- operational launcher/backoff policy kept outside durable fact semantics

This proves the model: MFM's shared knowledge graph is built from ordinary typed
`FactRecorded` events, and collectors are recurring certified workflows that
produce those events.
