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
  -> FactKey from subject only
  -> FactFacets from subject/result/metadata
  -> ProjectedFactClaim
  -> FactQueryEvidence
```

`FactDescriptor` binds the fact kind, subject type, response type, facet
definitions, ordering policies, and exposure rules. Only the descriptor's
subject section participates in `FactKey` derivation. Result and metadata facets
make observed values searchable and orderable without changing subject identity.

In short:

```text
state records typed FactRecorded claim
  -> append-only run event + admitted artifact evidence
  -> FactDescriptor derives subject/result/metadata facets
  -> FactKey identifies the stable subject
  -> ProjectedFactClaim makes Platform/Control facts queryable
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
domain into `PlatformFactRef`.

## Goals

- Reuse operations, states, adapters, transports, runtime, store, and replay.
- Treat `FactRecorded` as the canonical platform knowledge primitive.
- Author facts through one sealed/derive-oriented `MfmFactType`.
- Use one durable `FactDescriptor` per fact shape.
- Derive `FactKey` only from the descriptor's subject section and canonical
  subject material.
- Support observed-result indexing in v1 through descriptor-owned result
  facets.
- Support metadata facets for store/claim fields such as `recorded_at`,
  `observed_at`, and store order.
- Require explicit fact visibility; no implicit public publication.
- Add non-public `Control` visibility for cross-run operational facts such as
  collector checkpoints.
- Keep projected facts generic across domains.
- Keep run streams, commits, descriptors, subject material, and artifact
  evidence as strict authority.
- Make projected fact claims and facet terms an ordered, indexed, rebuildable
  store projection.
- Let CLI and REST expose kind-first fact queries that compile to descriptor
  facets.
- Preserve replay by pinning `FactQueryEvidence` into the consuming run.

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

Fact kind is the first user-facing query dimension. A fact kind can have
multiple descriptor versions or compatibility groups.

### FactDescriptor

`FactDescriptor` is the durable, content-addressed description of a fact shape.

It binds:

- fact kind
- subject schema id
- response schema id
- subject section that derives `FactKey`
- subject facets, result facets, and metadata facets
- field operators, type semantics, units/scales, and exposure policy
- ordering policies and tie-breakers
- compatibility group or schema version metadata

The descriptor hash is computed from canonical descriptor bytes. The descriptor
hash is not embedded inside those canonical bytes.

### Fact Subject

The subject is the stable identity of what the fact is about. It is typed
canonical material described by the subject section of `FactDescriptor`.

For a wallet balance, the subject can include chain, network, account, asset,
and balance kind. It should not include the balance amount.

For weather in Dublin, the subject can include country, locality, coordinate
cell, provider when provider changes the identity of the observation, and
measurement kind. It should not include the observed temperature.

For a transaction observation, the subject can include chain, network, and
transaction id. Block height belongs in the subject only when the fact kind is
specifically about a transaction-in-block identity; otherwise it is an observed
result facet.

Provider or source belongs in the subject only when it changes what the fact is
about. Otherwise it belongs in provenance, result facets, or metadata facets.

### FactKey

`FactKey` is the stable content-derived key for a fact subject.

Derivation:

```text
subject_shape_hash = hash(canonical descriptor subject section)
subject_material_hash = hash(canonical subject material)
fact_key = hash("mfm.fact-key.v1", subject_shape_hash, subject_material_hash)
```

Only the subject section participates in `FactKey`. Adding a result facet such
as `amount_sat`, `block_number`, or `temperature_celsius_milli` must not change
the identity of the same wallet balance or weather subject.

### FactFacet

A fact facet is a descriptor-declared typed index term. Facets are projection
data, not authority.

Facet source:

```rust
pub enum FactFacetSource {
    Subject,
    Result,
    Metadata,
}
```

- `Subject` facets answer "what is this fact about?"
- `Result` facets answer "what did this claim observe?"
- `Metadata` facets answer "how, when, or where was this claim recorded?"

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

V1 facets are scalar and single-valued per owner. If a future fact needs
multi-valued facets, the model should add an explicit `field_ordinal` or a
dedicated repeated-value shape.

### Fact Claim

A fact claim is the event-level assertion made by `FactRecorded`.

It binds:

- explicit visibility
- fact kind
- fact descriptor hash
- subject shape hash
- canonical subject material
- derived `FactKey`
- optional source/domain observation time
- request and response schema/hash provenance
- admitted artifact evidence
- response binding when an artifact contains multiple claim payloads
- producing run, node, attempt, capability, and adapter provenance

The claim is not an unqualified global truth. It is an observation with
provenance.

### ProjectedFactClaim

`ProjectedFactClaim` is a rebuildable store projection row for a `Platform` or
`Control` `FactRecorded` claim. It points back to the authoritative run event
and artifact evidence.

It is not independent authority. It is a searchable reference into run-stream
authority.

### PlatformFactRef

`PlatformFactRef` is the public reference view over a `Platform`
`ProjectedFactClaim`.

Public APIs never expose `Control` claims and never return canonical subject
material or response artifacts wholesale.

### Control Fact

A control fact is a non-public, cross-run discoverable `FactRecorded` claim. It
is intended for operational state such as collector checkpoints. Control facts
are queryable through internal runtime/collector capabilities, not through
public platform fact APIs.

### FactQueryEvidence

`FactQueryEvidence` is private read evidence recorded by a run that queries
projected facts.

It pins:

- store scope
- visibility scope
- read frontier/watermark
- canonical query bytes and hash
- canonical ordering policy
- limit
- returned refs
- selected refs
- result set digest
- empty-result or cardinality statement

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
fact kind + typed subject material -> FactKey -> ordered claims
```

There can be many claims for the same `FactKey`. "Latest" is not a mutable row;
it is a query with `limit = 1` under an explicit temporal or progression
ordering policy.

Use `top`, `ranked`, or ordinary query wording for ranking by values such as
largest balance or highest price. Those are valid result ordering policies, but
they are not automatically "latest" unless the descriptor says the field is a
progression/time axis.

### One Descriptor, Three Facet Sources

`FactDescriptor` is the single source of truth for search and ordering.

It defines:

- subject facets from canonical subject material
- result facets from typed response artifacts
- metadata facets from claim/store metadata
- ordering policies over subject, result, and metadata facets
- exposure policy for public output

State code does not attach arbitrary labels. SQL does not parse domain JSON.
The runtime/store derives facets from typed values using the descriptor.

### Observed Results Are First-Class

Observed-result indexing is in scope for v1.

Examples:

- all wallet balance claims where `amount_sat > 100000000`
- all weather observations where `temperature_celsius_milli < 0`
- all price quotes where `price_usd_micro > 100000000000`
- latest chain head by `result.block_number desc`

The constraint is separation:

- subject facets identify what the fact is about
- result facets index what this claim observed
- metadata facets index claim/store properties

Do not stuff observed values into `FactKey` just because callers need to query
or sort by them.

### Visibility Is Explicit

Every `FactRecorded` declares visibility. There is no implicit public
publication.

Conceptual visibility:

```rust
pub enum FactVisibility {
    RunPrivate,
    Control { scope: FactVisibilityScope },
    Platform { scope: FactVisibilityScope },
}
```

`RunPrivate` means "keep this as local run evidence only."

`Control` means "make this discoverable across runs for internal operational
workflows, but do not expose it through public platform fact APIs."

`Platform` means "make this discoverable through public platform fact APIs,
subject to scope authorization and descriptor exposure policy."

V1 can start with a single default scope, but the shape should include scope so
collector, source, or tenant boundaries are not retrofitted later.

Visibility does not make secrets acceptable. Secret-bearing data must never
enter fact material, fact artifacts, events, public outputs, diagnostics,
fixtures, or snapshots.

### Authority Remains Append-Only

The authoritative record is still:

- run events
- commits
- descriptor canonical bytes
- canonical subject material
- artifact admissions and bindings
- canonical event payload bytes

Projected claims and facet terms are query projections. They must be rebuildable
and verifiable from durable authority.

### Live Search Is Not Replay

A live run may query platform or control facts. The query result, including
empty results and bounded result sets, must be pinned into the consuming run as
`FactQueryEvidence`.

Replay uses pinned query evidence and retained artifacts, not the current
projected fact index and not a live collector.

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

Framework-owned canonicalization turns typed subject values into canonical bytes
and extracts descriptor-approved facets from typed subject, response, and
metadata values. Manual implementations must not return raw canonical bytes or
ad hoc labels.

Conceptual descriptor shape:

```rust
pub struct FactDescriptor {
    pub fact_kind: FactKind,
    pub descriptor_schema_id: SchemaId,
    pub subject_schema_id: SchemaId,
    pub response_schema_id: SchemaId,
    pub compatibility_group: Option<FactCompatibilityGroup>,
    pub subject: FactSubjectDescriptor,
    pub facets: &'static [FactFacetDescriptor],
    pub orderings: &'static [FactOrderingDescriptor],
}

pub struct FactSubjectDescriptor {
    pub subject_shape_id: SchemaId,
    pub fields: &'static [FactSubjectFieldDescriptor],
}

pub struct FactSubjectFieldDescriptor {
    pub path: FactSubjectFieldPath,
    pub value_type: FactFacetValueType,
    pub required: bool,
}

pub struct FactFacetDescriptor {
    pub source: FactFacetSource,
    pub path: FactFacetPath,
    pub value_type: FactFacetValueType,
    pub pointer: FactFacetPointer,
    pub operators: &'static [FactQueryOperator],
    pub exposure: FactFieldExposure,
    pub unit: Option<FactUnit>,
    pub scale: Option<FactScale>,
    pub sortable: bool,
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
    pub source: FactFacetSource,
    pub path: FactFacetPath,
    pub value_type: FactFacetValueType,
    pub direction: SortDirection,
    pub nulls: NullOrdering,
    pub tie_breaker: bool,
}
```

Ordering policies must define type comparison, null handling, and deterministic
tie-breakers. Store order should be the final fallback tie-breaker.

Descriptor hashing:

```text
fact_descriptor_hash = hash(canonical FactDescriptor bytes)
subject_shape_hash = hash(canonical subject section bytes)
subject_material_hash = hash(canonical subject material)
fact_key = hash("mfm.fact-key.v1", subject_shape_hash, subject_material_hash)
```

The descriptor hash and subject shape hash are computed values. They must not be
embedded inside the canonical bytes they hash.

The canonical subject material and descriptor bytes must follow platform hashing
rules:

- canonical JSON semantics
- no floats in hashed structures
- bounded size
- no secrets
- stable normalized identifiers
- explicit schema ids

For numeric result facets, prefer integer-scaled values such as `amount_sat`,
`temperature_celsius_milli`, or `price_usd_micro`. If a decimal facet is needed,
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
    #[mfm_fact(facet = "subject.chain")]
    pub chain: ChainRef,
    #[mfm_fact(facet = "subject.network")]
    pub network: NetworkRef,
    #[mfm_fact(facet = "subject.account_ref")]
    pub account_ref: AccountRef,
    #[mfm_fact(facet = "subject.asset_ref")]
    pub asset_ref: AssetRef,
    #[mfm_fact(facet = "subject.balance_kind")]
    pub balance_kind: BalanceKind,
}

pub struct WalletBalanceObservation {
    #[mfm_fact(facet = "result.amount_sat", orderable)]
    pub amount_sat: u64,
    #[mfm_fact(facet = "result.block_number", orderable)]
    pub block_number: u64,
}
```

`FactKey` is derived from `WalletBalanceSubject`. `amount_sat` and
`block_number` are result facets for filtering and ordering claims.

### Example: Weather Observation

```rust
#[derive(MfmFactType)]
#[mfm_fact(kind = "weather.observation", schema = "mfm.weather.observation.v1")]
pub struct WeatherObservationFact {
    pub subject: WeatherObservationSubject,
    pub response: WeatherObservationResult,
}

pub struct WeatherObservationSubject {
    #[mfm_fact(facet = "subject.country")]
    pub country: CountryCode,
    #[mfm_fact(facet = "subject.locality")]
    pub locality: Locality,
    #[mfm_fact(facet = "subject.coordinate_cell")]
    pub coordinate_cell: CoordinateCell,
    #[mfm_fact(facet = "subject.measure")]
    pub measure: WeatherMeasure,
}

pub struct WeatherObservationResult {
    #[mfm_fact(facet = "result.temperature_celsius_milli", orderable)]
    pub temperature_celsius_milli: i64,
    #[mfm_fact(facet = "result.provider_time", orderable)]
    pub provider_time: Timestamp,
}
```

Temperature is an observed result facet, not part of subject identity.

## FactRecorded Contract

`FactRecorded` remains the event. Its contract changes to require typed fact
claim data.

Conceptual shape:

```rust
pub struct FactClaim {
    pub visibility: FactVisibility,
    pub fact_kind: FactKind,
    pub fact_descriptor_hash: ContentDigest,
    pub subject_shape_hash: ContentDigest,
    pub subject_schema_id: SchemaId,
    pub response_schema_id: SchemaId,
    pub subject_material: PlainCanonicalJsonBytes,
    pub subject_material_hash: ContentDigest,
    pub fact_key: FactKey,
    pub observed_at: Option<Timestamp>,
    pub artifact_id: ArtifactId,
    pub artifact_evidence_hash: ContentDigest,
    pub response_binding: Option<ResponseBindingEvidence>,
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

The runtime helper should accept a typed fact type and response evidence, then
construct the claim:

```rust
recorder.record_fact(
    WalletBalanceFact {
        subject,
        response,
    },
    response_artifact,
    FactRecordOptions::new(FactVisibility::Platform { scope }).observed_at(source_time),
)?;
```

There is no valid old-shape `FactRecorded` after this reset. If code cannot
provide a sealed `MfmFactType`, it cannot record a fact.

### Response Binding

If one response artifact contains exactly one claim payload, the artifact id and
response hash may be enough. If one artifact contains a batch of observations,
each `FactRecorded` must include binding evidence for the slice it claims:

- typed response pointer
- slice/index path
- slice canonical hash
- schema id for the slice when different from the full response

This lets result facets be rebuilt for each claim without guessing which part of
a batch artifact produced the fact.

## Projection Model

The PostgreSQL store should maintain normal projection tables, not a PostgreSQL
materialized view and not JSONB-only search.

The projection is populated in the same append transaction that inserts commits,
admits artifacts, and inserts `run_events`.

The RFC describes logical storage. The implementation may split or combine
physical tables for performance, but it must preserve the same invariants,
foreign-key relationships, rebuildability, and visibility filtering.

### fact_descriptors

Stores immutable descriptor artifacts.

Conceptual columns:

```text
descriptor_hash
fact_kind
descriptor_schema_id
subject_schema_id
response_schema_id
subject_shape_hash
compatibility_group
descriptor_canonical_bytes
created_at
```

A `FactRecorded` append is valid only when the descriptor is available to the
store, either already present or supplied and admitted in the same transaction.

### fact_subjects

Stores stable subject identity.

Conceptual columns:

```text
fact_key
subject_shape_hash
fact_kind
subject_schema_id
subject_material_hash
subject_material_canonical_bytes
created_at
```

`fact_subjects` lets many claims for the same subject share one identity and one
set of subject facets.

### fact_claims

Stores one projected `Platform` or `Control` claim per visible `FactRecorded`.

Conceptual columns:

```text
source_run_id
source_seq
source_ordinal
source_event_id
commit_id
append_xid
commit_sort_key
recorded_at
observed_at
visibility_kind
visibility_scope
fact_kind
fact_descriptor_hash
subject_shape_hash
fact_key
subject_material_hash
request_schema_id
request_hash
response_schema_id
response_hash
artifact_id
artifact_evidence_hash
response_binding_hash
capability_kind
capability_version
adapter_kind
adapter_version
```

`RunPrivate` facts are not inserted into `fact_claims`.

Public platform APIs must read through a strict view or query boundary filtered
to `visibility_kind = Platform` and an allowed `visibility_scope`. Internal
collector/runtime capabilities may query `Control` scopes.

### fact_terms

Stores generic typed facet terms.

Logical shape:

```text
owner_kind              // subject | claim
fact_key                // for subject-owned terms
source_run_id           // for claim-owned terms
source_seq              // for claim-owned terms
source_ordinal          // for claim-owned terms
fact_descriptor_hash
facet_source            // subject | result | metadata
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

Subject facets may be stored once per `fact_key`. Result and metadata facets are
stored per claim because each claim can observe different values. A physical
implementation may use separate `fact_subject_terms` and `fact_claim_terms`
tables if that gives cleaner constraints or indexes.

Facet rows are projection data. They are rebuildable from descriptors, subject
material, response artifacts, and claim metadata.

Useful logical indexes:

```text
(fact_descriptor_hash, facet_source, field_path, typed_value)
(visibility_kind, visibility_scope, fact_kind, recorded_at desc)
(visibility_kind, visibility_scope, fact_key, recorded_at desc)
(visibility_kind, visibility_scope, fact_descriptor_hash, field_path, sortable_value)
(append_xid, commit_sort_key, source_ordinal)
```

These are logical access paths. A physical implementation may satisfy them with
separate subject/claim term tables, joins against `fact_claims`, or safe
denormalization of visibility and fact kind onto claim-owned term rows.

### Projection Algorithm

For each appended `FactRecorded`:

1. Validate the typed fact claim shape.
2. Admit or load the `FactDescriptor`.
3. Validate fact kind, subject schema id, response schema id, and descriptor
   hash.
4. Canonicalize subject material.
5. Recompute subject material hash and subject shape hash.
6. Recompute `FactKey` from subject shape hash and subject material hash.
7. Validate response binding against the response artifact.
8. Derive subject facets from subject material.
9. Derive result facets from the typed response artifact or response slice.
10. Derive metadata facets from claim/store metadata.
11. Verify facet type, unit, scale, operator, exposure, size, and scalar
    constraints.
12. If visibility is `RunPrivate`, admit descriptor evidence and insert only the
    run event and artifacts; do not insert projected claims or facet terms.
13. If visibility is `Control` or `Platform`, upsert descriptor, subject, facet
    terms, and projected claim inside the same append transaction.

If projection fails for a `Control` or `Platform` fact, the append fails. MFM
should not commit an event that is supposed to be cross-run discoverable while
omitting its projection rows.

## Ordering And Query Semantics

Every query that asks for "latest" or otherwise depends on order must name an
ordering policy. Different policies are different semantics.

Ordering terms are canonical structures:

```text
source       // metadata | result | subject
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

Public cursors are opaque. Internal order can use append transaction ordering,
commit sort key, source sequence, and source ordinal as deterministic
tie-breakers.

## Why Not A Materialized View

PostgreSQL materialized views are refreshed in batches. `REFRESH MATERIALIZED
VIEW CONCURRENTLY` reduces read blocking, but it is still not the per-insert
incremental projection MFM needs.

The append path already has a transaction that inserts commits, admits
artifacts, and inserts run events. Projected fact insertion belongs in that
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
`run_events`. It should validate fact claims, derive facets, and insert
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
4. emit Platform FactRecorded claims
5. emit Control checkpoint fact
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

## Consuming Projected Facts

A workflow can query platform or control facts through a typed read capability.
That live query is not replay authority.

Flow:

```text
consumer state queries projected facts
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
    pub store_scope: StoreScopeRef,
    pub visibility: FactVisibility,
    pub read_frontier: StoreReadFrontier,
    pub canonical_query: PlainCanonicalJsonBytes,
    pub canonical_query_hash: ContentDigest,
    pub ordering: FactOrdering,
    pub limit: Option<u64>,
    pub returned_refs: Vec<ProjectedFactRef>,
    pub selected_refs: Vec<ProjectedFactRef>,
    pub result_set_digest: ContentDigest,
    pub result_cardinality: QueryResultCardinality,
    pub selection_policy_hash: ContentDigest,
}
```

This evidence covers selected facts, bounded result sets, and empty-result
branches. For v1, a store-trusted read frontier is enough to support replayable
"no result at this point" decisions. Cryptographic absence proofs can be
deferred.

Replay verifies the pinned source facts, artifact evidence, canonical query,
ordering policy, selection policy, returned refs, selected refs, result set
digest, and read frontier.

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

The CLI uses descriptor metadata to validate fields, parse typed values, enforce
field exposure policy, choose an explicit ordering policy, and compile the query
into generic facet filters.

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
    pub fact_descriptor_hash: ContentDigest,
    pub subject_shape_hash: ContentDigest,
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
Those are descriptor-derived facets joined through `fact_key` or source claim
identity.

Public `PlatformFactRef` output is a filtered view of `ProjectedFactRef` for
`visibility = Platform`. It must not return canonical subject material or
response artifacts wholesale.

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
- `RunPrivate` facts should not create projected subject/claim terms that would
  leak private-only subjects
- `Control` facts are cross-run discoverable only through internal operational
  capabilities and their scopes
- public APIs must expose only descriptor-approved facets and must not return
  canonical subject material or response artifacts wholesale
- public APIs must redact errors and avoid endpoint/auth leakage
- source routing, credentials, RPC URLs, authorization headers, passwords,
  keystores, and signer material remain below typed semantic surfaces

## Retention

A queryable projected fact must not outlive the authority needed to verify it.

While a `Platform` or `Control` fact remains queryable, the store must retain:

- source run event payload and commit metadata
- descriptor canonical bytes
- canonical subject material bytes
- request and response schema/hash evidence
- response artifact
- response binding evidence for batched artifacts
- artifact admission/binding evidence
- capability and adapter provenance

Facet rows and projected claim rows are not authority. They are rebuildable
projection data. Garbage collection must either keep authority materials or
first remove the projected fact from all query surfaces.

## Rebuild And Verification

Projection rebuild and validation should ship with v1. Idempotent append alone
is not enough because a retry that sees an existing commit may return early and
will not necessarily repair missing projection rows.

Required validation:

- every projected claim points to an existing `FactRecorded` event
- every projected claim is `Platform` or `Control`
- public platform APIs expose only authorized `Platform` rows
- no `RunPrivate` fact creates projected subject/claim terms
- descriptor hash matches descriptor canonical bytes
- subject shape hash matches descriptor subject section bytes
- subject material hash matches canonical subject material
- `FactKey` matches subject shape hash plus subject material hash
- response binding matches the admitted response artifact
- subject facets match descriptor-derived subject values
- result facets match descriptor-derived response values
- metadata facets match claim/store metadata
- artifact id and artifact evidence hash match admitted response evidence
- request/response schema and hash match event payload
- source run id, sequence, ordinal, and event id match the run stream
- store ordering columns match the commit row

Rebuild should truncate and repopulate projections from strict authority and
then run the same validation checks.

## Failure Semantics

Facts and projected fact rows must be atomic.

- If descriptor validation fails, the append fails.
- If subject derivation fails, the append fails.
- If declared facet derivation fails, the append fails.
- If response binding validation fails, the append fails.
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
2. Add explicit scoped `FactVisibility` with `RunPrivate`, `Control`, and
   `Platform`.
3. Add sealed/derive-owned `MfmFactType` support.
4. Add durable `FactDescriptor` admission.
5. Replace ad hoc `FactKey` construction with typed subject material.
6. Require every `FactRecorded` to carry `FactClaim` data.
7. Add logical `fact_descriptors`, `fact_subjects`, `fact_claims`, and
   `fact_terms` projections.
8. Populate projections in the same append transaction as `run_events`.
9. Add projection validation and rebuild.
10. Add descriptor-scoped, kind-first CLI and REST query APIs.
11. Add private `FactQueryEvidence`.
12. Build the first collector as a recurring certified workflow over ordinary
    states.

## Resolved Decisions

- Collectors are recurring workflows over ops and states, not a new runtime
  category.
- `FactRecorded` is the universal fact event.
- There is no `CollectedFactRecorded`.
- Visibility is explicit and scoped; there is no default public publication.
- `Control` is the non-public cross-run visibility for collector checkpoints and
  shared operational state.
- Every `FactRecorded` must use a sealed typed `MfmFactType` after the reset.
- `FactDescriptor` is the durable shape for subject identity, result facets,
  metadata facets, ordering, operators, units/scales, and exposure.
- `FactKey` is derived only from the descriptor subject section and canonical
  subject material.
- Observed-result indexing is in v1 through descriptor-declared result facets.
- Metadata facets are in v1 for fields such as `recorded_at`, `observed_at`,
  and store order.
- Descriptor canonical bytes are durable content-addressed framework artifacts
  admitted before or during the first append that needs them.
- Descriptor hashes are not embedded inside descriptor canonical bytes.
- `PlatformFactRef` is generic provenance and ordering, not domain labels.
- Canonical subject material is persisted in bounded form for v1.
- Persisted subject material is authority/rebuild material, not public output.
- Projection for `Platform` and `Control` facts is maintained inside the append
  transaction.
- PostgreSQL materialized views and SQL artifact-parsing triggers are not the v1
  projection strategy.
- Consuming runs pin `FactQueryEvidence` as private read evidence for replay.
- Query evidence stores canonical query material, not only hashes.
- Query ordering policy is explicit and hashed into query evidence.
- Queryable projected facts require retention of their verification authority.

## Deferred Questions

- Which typed field operators are required beyond equality, comparison,
  timestamp ordering, and descriptor-declared sortable numeric fields in the
  first public API?
- Do named visibility scopes need a full authorization model in v1, or is a
  single default scope enough while preserving the scoped enum shape?
- Which checkpoint facts should ever be platform-visible rather than `Control`?
- Should physical storage use one `fact_terms` table or split subject and claim
  terms for stronger constraints and performance?
- What exact response-binding shape is needed for large batched artifacts?
- What operational lease/backoff policy should launch recurring collector
  cycles outside the durable state-machine semantics?
- How should cross-store export/import prove source store trust scope, source
  stream authority, and retained artifact evidence?

## Preferred First Implementation

Start with one vertical slice:

- one `MfmFactType` derive for a domain fact
- mandatory typed `FactClaim` on `FactRecorded`
- explicit scoped `FactVisibility::{RunPrivate, Control, Platform}`
- descriptor admission as content-addressed framework artifacts
- logical `fact_descriptors`, `fact_subjects`, `fact_claims`, and `fact_terms`
  projections in PostgreSQL
- subject facets, result facets, and metadata facets
- append-transaction projection for `Platform` and `Control` facts
- projection validation and rebuild
- explicit ordering policies for store commit order, observed time, and
  descriptor-declared result fields
- descriptor-scoped, kind-first `mfm facts` query path
- pinned `FactQueryEvidence` for consumers
- one collector-style workflow that records platform-visible domain facts and
  control checkpoint facts using existing state-machine primitives

This proves the model: MFM's shared knowledge graph is built from ordinary typed
`FactRecorded` events, and collectors are recurring certified workflows that
produce those events.
