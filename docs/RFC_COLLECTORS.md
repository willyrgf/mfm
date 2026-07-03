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
  -> FactKey from kind-scoped subject material
  -> fact_index row
  -> fact_index_terms rows from subject/result/metadata fields
  -> FactQueryReceipt pinned by FactQueryEvidence
```

`FactDescriptor` binds the fact kind, subject type, response type, field
definitions, ordering policies, and exposure rules. Only descriptor fields with
`source = Subject` define subject material and participate in `FactKey`
derivation. Result and metadata fields make observed values searchable and
orderable without changing subject identity.

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
domain into internal refs or public DTOs.

## Goals

- Reuse operations, states, adapters, transports, runtime, store, and replay.
- Treat `FactRecorded` as the canonical platform knowledge primitive.
- Author facts through one derive-owned `MfmFactType`.
- Use one durable `FactDescriptor` per fact shape.
- Treat `FactDescriptor` as certified authority allowed by the certified spec
  for the producing node, not merely store-admitted data.
- Derive `FactKey` only from fact kind and canonical subject material.
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
- Do not make `InternalFactRef` or `PublicFactRef` domain-specific structs or
  arbitrary label bags.
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
multiple descriptor versions.

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
- schema version metadata

The descriptor hash is computed from canonical descriptor bytes. The descriptor
hash is not embedded inside those canonical bytes.

### Fact Subject

The subject is the stable identity of what the fact is about. It is typed
canonical material described by `source = Subject` fields in `FactDescriptor`.
In v1, every subject field is required identity material. If a value should be
searchable but should not change identity, it must be modeled as a result or
metadata field, not as a subject field.

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
fact_subject_namespace_hash = hash(canonical FactSubjectNamespaceV1)
subject_material_hash = hash(canonical FactSubjectMaterialV1)
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

Subject identity is based on stable semantic field ids and typed values, not on
Rust struct layout, schema paths, or extraction paths. Descriptor paths and
extractions can evolve compatibly without changing `FactKey` when the subject
field ids, value types, units, scales, and values are unchanged.

### Fact Field

A fact field is a descriptor-declared typed field that can produce index terms.
Terms are projection data, not authority.

Field source is defined by `FactFieldExtraction`:

```rust
pub enum FactFieldExtraction {
    SubjectPath(CanonicalValuePath),
    ResponsePath(CanonicalValuePath),
    Metadata(FactMetadataField),
}
```

- `SubjectPath` fields answer "what is this fact about?"
- `ResponsePath` fields answer "what did this claim observe?"
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

### FactClaimId

`FactClaimId` is the stable identity for one recorded claim. Every
`FactRecorded` event derives one `FactClaimId`, whether the fact is indexed or
`RunPrivate`. Projection rows use the same id; they do not create it.

V1 uses the run-stream event coordinate:

```rust
pub struct FactClaimId {
    pub source_run_id: RunId,
    pub source_seq: StreamSeq,
    pub source_ordinal: EventOrdinal,
}
```

`FactClaimId` identifies a claim. `FactKey` identifies the claim subject. Query
joins for result and metadata terms must use `FactClaimId`; `FactKey` is only
for grouping claims about the same subject.

### InternalFactRef

`InternalFactRef` is a rebuildable store projection row for an indexed
`FactRecorded` claim. It points back to the authoritative run event and
artifact evidence.

It is not independent authority. It is a searchable reference into run-stream
authority, and it is not a public API DTO.

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
- query scope
- read frontier/watermark
- canonical query plan
- store-authenticated query receipt
- state-owned selection evidence

Replay uses this evidence and retained artifacts. It does not ask the current
store "what is latest now?"

The authority carrier is the existing generic `ArtifactReferenced` run event.
For fact query evidence, the referenced artifact must use
`ArtifactRole::FactQueryEvidence` and contain canonical `FactQueryEvidence`
bytes. The event, artifact id, artifact evidence hash, and artifact bytes are
retained as run-private replay evidence. They are not projected into
`fact_index`.

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

Every `FactRecorded` producer must use descriptor-certified typed fact
publication. In Rust v1, the authoring path is a derive-owned fact type, and
the runner API should make this the only available path for fact publication.

Conceptual API:

```rust
pub trait MfmFactType: MfmValue {
    type Subject: MfmValue;
    type Response: MfmValue;

    fn descriptor() -> &'static FactDescriptor;
    fn subject(&self) -> &Self::Subject;
    fn response(&self) -> &Self::Response;
}
```

`MfmFactType` is derive-owned, but not enforced through an inaccessible private
supertrait. The derive macro emits the descriptor artifact with
`ArtifactRole::FactDescriptor`, subject/response extractions, and compile-time
shape checks. The public recorder API accepts `T: MfmFactType`, but type
implementation is not semantic authority. Descriptor bytes, descriptor hash,
certified node allow-list, and append validation are the only authority for fact
publication.

Manual implementations are unsupported for production fact publication. They
can compile only if they satisfy the same trait surface, but they cannot bypass
descriptor admission, certified allow-list validation, canonicalization, field
extraction, or projection checks.

V1 derive checks must reject subject types with unannotated serialized fields,
optional subject fields, unsupported scalar encodings, or response fields that
cannot be represented by the v1 extraction grammar.

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
    pub fields: &'static [FactFieldDescriptor],
    pub orderings: &'static [FactOrderingDescriptor],
}

pub struct FactFieldDescriptor {
    pub field_id: FactFieldId,
    pub path: FactFieldPath,
    pub value_type: FactFieldValueType,
    pub extraction: FactFieldExtraction,
    pub operators: &'static [FactQueryOperator],
    pub exposure: FactFieldExposure,
    pub unit: Option<FactUnit>,
    pub scale: Option<FactScale>,
    pub sortable: bool,
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
    pub direction: SortDirection,
    pub nulls: NullOrdering,
    pub tie_breaker: bool,
}
```

`field_id` is descriptor-owned stable identity. `path` is a human-readable and
schema-facing pointer that may evolve across descriptor versions. Index rows,
ordering policies, compatibility rules, and query evidence bind to `field_id`.

`extraction` is the single source of field extraction authority. Its variant
defines whether the field is `Subject`, `Result`, or `Metadata`. In v1, every
`SubjectPath` field is required and participates in `FactKey`; result and
metadata fields never participate in `FactKey`.

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

Descriptor validation must reject:

- duplicate `field_id` values
- zero `SubjectPath` fields
- optional `SubjectPath` fields
- descriptor paths whose prefix conflicts with the extraction source
- ordering terms that reference missing or non-sortable fields
- fields whose allowed operators conflict with their value type
- unsupported timestamp, decimal, digest, numeric bound, or normalization rules

Ordering policies must define type comparison, null handling, and deterministic
tie-breakers. Store order should be the final fallback tie-breaker.

Descriptor hashing:

```text
fact_descriptor_hash = hash(canonical FactDescriptor bytes)
fact_subject_namespace_hash = hash(canonical FactSubjectNamespaceV1)
subject_material_hash = hash(canonical FactSubjectMaterialV1)
fact_key = hash(
  "mfm.fact-key.v1",
  fact_subject_namespace_hash,
  subject_material_hash
)
```

Canonical `FactSubjectNamespaceV1`:

```rust
pub struct FactSubjectNamespaceV1 {
    pub version: &'static str,              // "mfm.fact-subject-namespace.v1"
    pub fact_kind: FactKind,
    pub fields: Vec<FactSubjectNamespaceFieldV1>,
}

pub struct FactSubjectNamespaceFieldV1 {
    pub field_id: FactFieldId,
    pub value_type: FactFieldValueType,
    pub unit: Option<FactUnit>,
    pub scale: Option<FactScale>,
}

pub struct FactSubjectMaterialV1 {
    pub version: &'static str,              // "mfm.fact-subject-material.v1"
    pub values: Vec<FactSubjectValueV1>,
}

pub struct FactSubjectValueV1 {
    pub field_id: FactFieldId,
    pub value_type: FactFieldValueType,
    pub value: FactCanonicalScalar,
}
```

Both vectors are sorted by `field_id` before canonicalization. The namespace
includes only descriptor fields whose extraction is `SubjectPath`; the material
contains only values extracted for those fields from `T::Subject`. This excludes
result fields, metadata fields, schema paths, extraction paths, ordering
policies, allowed operators, exposure policy, descriptor schema id, response
schema id, and descriptor hash. Those excluded fields may
change query behavior or descriptor identity, but they do not change subject
identity.

Compatible descriptor versions share `FactKey` when they keep the same
`FactSubjectNamespaceV1` and produce the same `FactSubjectMaterialV1`. A
descriptor that changes subject identity must change the relevant subject
`field_id`, value type, unit, scale, fact kind, or subject value.

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

V1 operator floor:

- equality for descriptor-exposed scalar fields
- comparison for timestamp fields
- comparison and ordering for descriptor-declared sortable integer or decimal
  fields
- no prefix, full-text, array, geospatial, or custom domain operators

### Example: Wallet Balance

```rust
#[derive(MfmFactType)]
#[mfm_fact(kind = "wallet.balance", schema = "mfm.wallet.balance.v1")]
pub struct WalletBalanceFact {
    pub subject: WalletBalanceSubject,
    pub response: WalletBalanceObservation,
}

pub struct WalletBalanceSubject {
    #[mfm_fact(field = "subject.chain")]
    pub chain: ChainRef,
    #[mfm_fact(field = "subject.network")]
    pub network: NetworkRef,
    #[mfm_fact(field = "subject.account_ref")]
    pub account_ref: AccountRef,
    #[mfm_fact(field = "subject.asset_ref")]
    pub asset_ref: AssetRef,
    #[mfm_fact(field = "subject.balance_kind")]
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
    #[mfm_fact(field = "subject.country")]
    pub country: CountryCode,
    #[mfm_fact(field = "subject.locality")]
    pub locality: Locality,
    #[mfm_fact(field = "subject.coordinate_cell")]
    pub coordinate_cell: CoordinateCell,
    #[mfm_fact(field = "subject.measure")]
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
pub struct FactRecordedPayload {
    pub claim: FactClaim,
}

pub struct FactClaim {
    pub visibility: FactVisibility,
    pub fact_descriptor_hash: ContentDigest,
    pub subject: FactSubjectEvidence,
    pub observed_at: Option<Timestamp>,
    pub request: Option<FactRequestEvidence>,
    pub response: ArtifactEvidenceRef,
    pub producer: FactProducerProvenance,
}

pub struct FactSubjectEvidence {
    pub subject_material: PlainCanonicalJsonBytes,
    pub subject_material_hash: ContentDigest,
    pub fact_key: FactKey,
}

pub struct FactRequestEvidence {
    pub request_schema_id: SchemaId,
    pub request_hash: ContentDigest,
}

pub struct FactProducerProvenance {
    pub spec_hash: ContentDigest,
    pub node_id: NodeId,
    pub attempt_id: AttemptId,
    pub capability_kind: CapabilityKind,
    pub capability_version: CapabilityVersion,
    pub adapter_kind: AdapterKind,
    pub adapter_version: AdapterVersion,
}
```

The event envelope and commit row are the only source for source run id, stream
sequence, event ordinal, source event id, recorded time, commit id, and store
commit order. `FactClaimId` is derived from that envelope. The `FactClaim`
payload above is the only source for visibility, descriptor hash, subject,
request, response artifact evidence, and producer-provenance fields. Fact kind,
schemas, subject namespace hash, field definitions, and query metadata are
derived from the retained descriptor during append and rebuild. Projection rows
copy or derive from these authoritative sources; they do not introduce
independent truth.

`FactProducerProvenance` is populated from runtime and certified attempt
context. It is not caller-provided fact input.

The v1 runtime helper uses two canonical materials from the same typed fact
value:

- `T::Subject` is normalized into `FactSubjectMaterialV1`, then canonicalized
  into `FactSubjectEvidence.subject_material`
- `T::Response` alone is admitted as an `ArtifactEvidenceRef` with
  `role = ArtifactRole::FactResponse`

The response artifact canonical bytes are the source for response decoding,
field extraction, response hash, artifact id, and artifact evidence. The helper
must not accept an independent typed response and unrelated response artifact
that can drift.

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

The helper canonicalizes `subject`, stages a response artifact from `response`,
computes hashes, admits the artifact, derives index terms, and records
`FactRecorded` from those same materials.

There is no valid old-shape `FactRecorded` after this reset. If code cannot
provide a derive-owned `MfmFactType`, it cannot record a fact.

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

## Main Types And Interfaces

These are planning interfaces, not a new collector runtime.

Recording:

```rust
pub trait FactRecorder {
    fn record_fact<T: MfmFactType>(
        &mut self,
        input: FactRecordInput<T>,
    ) -> Result<StagedFactRecordHandle, FactRecordError>;

    fn record_fact_query_evidence(
        &mut self,
        evidence: FactQueryEvidence,
    ) -> Result<StagedFactQueryEvidenceHandle, FactRecordError>;
}

pub struct StagedFactRecordHandle {
    pub local_id: StagedRecordId,
}

pub struct StagedFactQueryEvidenceHandle {
    pub local_id: StagedRecordId,
}

pub struct FactRecordCommitResult {
    pub staged: StagedRecordId,
    pub run_event_id: RunEventId,
    pub fact_claim_id: Option<FactClaimId>,
}
```

`record_fact` emits one `FactRecordedPayload` run event and derives one
`FactClaimId`. For indexed facts, it also projects one `fact_index` row plus its
`fact_index_terms` in the same append transaction. `record_fact_query_evidence`
admits canonical `FactQueryEvidence` as a private artifact and emits the
existing generic `ArtifactReferenced` run event with
`ArtifactRole::FactQueryEvidence`.

Recorder calls stage evidence before commit. They do not return `FactClaimId` or
`RunEventId`, because those are derived from append-assigned run-stream
coordinates. The append/commit result maps staged handles to committed event ids
and, for `FactRecorded` events, `FactClaimId`. `fact_claim_id` is optional on
`FactRecordCommitResult` only because the same commit mapping can include staged
query-evidence artifact references, which are not claims.

Descriptor catalog, internal to facts kernel/store planning:

```rust
pub trait FactDescriptorCatalog {
    fn admit_descriptor(
        &mut self,
        descriptor: &'static FactDescriptor,
    ) -> Result<DescriptorAdmission, FactDescriptorError>;

    fn resolve_descriptor(
        &self,
        selector: FactDescriptorSelector,
    ) -> Result<ResolvedFactDescriptor, FactDescriptorError>;
}
```

Store/query boundary:

```rust
pub trait FactQueryService {
    fn compile(
        &self,
        input: FactQueryInput,
    ) -> Result<CanonicalFactQueryPlan, FactQueryError>;

    fn execute(
        &self,
        plan: &CanonicalFactQueryPlan,
    ) -> Result<FactQueryReceipt, FactQueryError>;
}
```

Projection/rebuild, internal to store planning:

```rust
pub trait FactProjectionStore {
    fn project_fact_recorded(
        &mut self,
        envelope: &RunEventEnvelope,
        payload: &FactRecordedPayload,
    ) -> Result<(), FactProjectionError>;

    fn rebuild_fact_projection(&mut self) -> Result<FactProjectionReport, FactProjectionError>;
}
```

CLI, REST API, collectors, and ordinary consuming states should use only the
recording and query contracts. Descriptor admission and projection/rebuild are
kernel/store implementation obligations, not application-facing extension
points. No caller should implement descriptor parsing, field extraction, query
compilation, or projection logic independently.

## Type Inventory

This inventory is for planning. Crate placement can change during
implementation as long as ownership boundaries stay intact.

| Type or role | Status | Primary owner | Notes |
| --- | --- | --- | --- |
| `ArtifactReferenced` | existing | `crates/kernel/events` | Generic run event used for query evidence artifacts. |
| `ArtifactEvidenceRef` | existing | `crates/kernel/events`, `crates/kernel/store` | Reused for response, descriptor, and query-evidence artifacts. |
| `ArtifactRole::FactResponse` | existing | `crates/kernel/events` | Role for one fact response artifact per claim in v1. |
| `ArtifactRole::FactDescriptor` | new | `crates/kernel/events` | Role for durable descriptor authority artifacts. |
| `ArtifactRole::FactQueryEvidence` | new | `crates/kernel/events` | Role for canonical private query-evidence artifacts. |
| `MfmFactType` | new | `crates/kernel/program`, `crates/kernel/program-derive` | Rust v1 authoring path for descriptor-certified typed facts. |
| `FactDescriptor` and field/order descriptors | new | `crates/kernel/facts` | Semantic authority for subject identity, extraction, operators, exposure, and ordering. |
| `FactFieldExtraction` | new | `crates/kernel/facts` | Declarative kernel-owned extraction grammar. |
| `FactVisibility`, `FactAudience`, `FactVisibilityScope` | new | `crates/kernel/facts` | Explicit visibility and query audience contract. |
| `FactRecordedPayload` and `FactClaim` | new | `crates/kernel/events`, `crates/kernel/facts` | Normalized claim payload for every `FactRecorded`. |
| `FactSubjectNamespaceV1`, `FactSubjectMaterialV1` | new | `crates/kernel/facts` | Canonical subject identity inputs for `FactKey`. |
| `FactKey` | new shape | `crates/kernel/facts` | Subject-only key after the destructive reset. |
| `FactClaimId` | new | `crates/kernel/facts` | Derived for every `FactRecorded` from run-stream coordinates. |
| `InternalFactRef` | new/internal | `crates/kernel/facts`, store | Full trusted projection/replay ref; may use the existing implementation name `FactRef`. |
| `PublicFactRef` and `PublicFactRefId` | new/public | `crates/app`, `bin/cli`, `bin/rest-api` | Filtered public DTO and opaque public ref id. |
| `FactQueryInput` | new | facts-query crate/service | Transport-decoded input before canonical planning. |
| `CanonicalFactQueryPlan` | new | facts-query crate/service | Canonical single-descriptor v1 query plan. |
| `FactQueryReceipt` | new | store/facts-query | Store-owned authenticated returned-set evidence. |
| `FactSelectionEvidence` | new | runtime/facts-query | State-owned evidence for selecting from receipt results. |
| `StoreReadFrontier`, `StoreReadFrontierType` | new | store/facts-query | Semantic snapshot/prefix frontier. |
| `StoreReceiptAuthentication`, `StoreReceiptAuthenticationScheme` | new | store/facts-query | V1 uses `LocalEd25519Sha256JcsV1`. |
| `ReturnedFieldSummaries`, `QueryResultCardinality` | new | facts-query crate/service | Pinned summaries/cardinality used by replay and selection. |
| `DescriptorCatalogWatermark`, `FactProjectionGeneration` | new | store | Store-owned watermarks for receipt/frontier validation. |
| Multi-descriptor query execution | deferred | facts-query crate/service | V1 resolves one descriptor before planning. |
| Checkpoint lease/compare-and-append authority | deferred | store/runtime | Needed only for exclusive checkpoint advancement. |

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

The descriptor artifact uses `ArtifactRole::FactDescriptor`. If that artifact
role does not exist when this RFC is implemented, add it to the existing generic
artifact role set. The descriptor artifact must be admitted before or during the
first append that records a fact using it. Admission alone is not enough. The
producing node's certified spec must also allow the descriptor hash.

### fact_descriptor_index

Stores searchable descriptor metadata and lookup pointers to descriptor
authority. This table is a rebuildable descriptor catalog projection/cache, not
a semantic peer of the descriptor artifact.

Conceptual columns:

```text
descriptor_hash
descriptor_artifact_id
descriptor_artifact_evidence_hash
fact_kind
descriptor_schema_id
subject_schema_id
response_schema_id
fact_subject_namespace_hash
created_at
```

A `FactRecorded` append is valid only when the descriptor is available to the
store, admitted by the producing run, and certified for the producing node. The
descriptor may already be present in the store-wide catalog or be supplied in
the same transaction, but store admission is necessary, not sufficient.

### run_fact_descriptor_admissions

Stores the per-run link from `RunAdmitted.fact_descriptor_artifacts` to the
store-wide descriptor catalog. This table is rebuildable projection/cache data;
the semantic authority remains the run stream plus retained descriptor artifact
evidence.

Conceptual columns:

```text
run_id
descriptor_hash
descriptor_artifact_id
descriptor_artifact_evidence_hash
source_seq
source_ordinal
source_event_id
commit_id
```

Canonical subject material is retained authority. It must be bounded,
non-secret, and unavailable through generic public read APIs. Public access to
subject information goes through descriptor-approved subject fields and exposure
policy only.

### fact_index

Stores one indexed claim per `FactRecorded` with `FactVisibility::Indexed`.

Conceptual columns:

```text
fact_claim_id
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

`request_schema_id` and `request_hash` are nullable when the fact is derived
without a live request. Response artifact fields are required in v1.

Public platform APIs must read through a strict view or query boundary filtered
to `audience = Platform` and an allowed `visibility_scope`. Internal
collector/runtime capabilities may query `audience = Control`.

### fact_index_terms

Stores generic typed field terms for indexed facts.

Logical shape:

```text
fact_claim_id
fact_descriptor_hash
field_id
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

`field_id` is the stable descriptor-owned identity for a queryable extracted
field. `field_path`, source category, value type, unit, scale, operator support,
and exposure policy come from the retained descriptor. They may be physically
denormalized for query speed, but the descriptor remains their semantic
authority. `fact_key`, source run coordinates, audience, visibility scope, and
store order are joined through `fact_index` by `fact_claim_id`; they may also be
physically denormalized when the implementation can rebuild and validate them.

Useful logical indexes:

```text
(fact_claim_id, field_id)
(fact_descriptor_hash, field_id, typed_value)
(fact_descriptor_hash, field_id, sortable_value)
```

These are term-table access paths. End-user queries also need joined access
through `fact_index`, such as audience/scope/kind/recorded-time, audience/scope
plus `FactKey`, or store commit order. A physical implementation may satisfy
those with joins against `fact_index`, partial indexes per audience, or safe
denormalization onto term rows.

Term joins that combine subject, result, and metadata predicates for the same
claim must join on `fact_claim_id`. `fact_key` must not be used as the claim
join key because it groups all claims for the same subject.

### Projection Algorithm

For each appended `FactRecorded`:

1. Validate the typed fact claim shape.
2. Admit or load the `FactDescriptor`.
3. Verify the descriptor hash is allowed by the certified spec for the
   producing node.
4. Validate fact kind, subject schema id, response schema id, and descriptor
   hash.
5. Canonicalize subject material.
6. Derive `FactClaimId` from the run-stream event coordinate.
7. Recompute subject material hash and fact subject namespace hash.
8. Recompute `FactKey` from fact subject namespace hash and subject material
   hash.
9. Validate that the response artifact contains exactly one claim payload in v1.
10. Derive subject terms from subject material using the kernel extractor.
11. Derive result terms from the typed response artifact
    using the kernel extractor.
12. Derive metadata terms from claim/store metadata using the kernel extractor.
13. Verify field id, type, unit, scale, operator, exposure, size, and scalar
    constraints.
14. If visibility is `RunPrivate`, admit descriptor evidence and insert only the
    run event and artifacts; do not insert fact index or term rows.
15. If visibility is `Indexed`, upsert descriptor index, fact index, and index
    terms inside the same append transaction.

If projection fails for an indexed fact, the append fails. MFM should not commit
an event that is supposed to be cross-run discoverable while omitting its
projection rows.

## Ordering And Query Semantics

Every query that asks for "latest" or otherwise depends on order must name an
ordering policy. Different policies are different semantics.

Ordering terms are canonical structures:

```text
field_id
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

Authority carrier:

```rust
pub struct ArtifactReferenced {
    pub spec_hash: SpecHash,
    pub node_id: Option<NodeId>,
    pub attempt_id: Option<AttemptId>,
    pub artifact_ref: ArtifactEvidenceRef,
}
```

The consuming run uses the existing generic `ArtifactReferenced` event. Its
`artifact_ref.role` must be `ArtifactRole::FactQueryEvidence`, and the artifact
bytes are canonical `FactQueryEvidence`. This event is run-private evidence: it
is append-only run authority for replay and retention, but it is never inserted
into `fact_index`. If `ArtifactRole::FactQueryEvidence` does not exist when this
RFC is implemented, add that artifact role to the existing generic artifact
role set rather than creating a fact-specific event.

Conceptual query evidence:

```rust
pub struct FactQueryEvidence {
    pub plan: CanonicalFactQueryPlan,
    pub receipt: FactQueryReceipt,
    pub selection: FactSelectionEvidence,
}

pub struct CanonicalFactQueryPlan {
    pub store_scope: StoreScopeRef,
    pub query_scope: FactQueryScope,
    pub query_compiler_version: Version,
    pub canonicalizer_version: Version,
    pub resolved_descriptors: Vec<ContentDigest>,
    pub scope_decision_evidence: ScopeDecisionEvidence,
    pub canonical_query: PlainCanonicalJsonBytes,
    pub canonical_query_hash: ContentDigest,
    pub ordering: FactOrdering,
    pub limit: Option<u64>,
}

pub struct FactQueryScope {
    pub audience: FactAudience,
    pub scope: FactVisibilityScope,
}

pub struct FactQueryReceipt {
    pub read_frontier: StoreReadFrontier,
    pub frontier_type: StoreReadFrontierType,
    pub returned_refs: Vec<InternalFactRef>,
    pub returned_field_summaries: Option<ReturnedFieldSummaries>,
    pub result_set_digest: ContentDigest,
    pub result_cardinality: QueryResultCardinality,
    pub store_receipt_hash: ContentDigest,
    pub store_receipt_authentication: StoreReceiptAuthentication,
}

pub struct StoreReadFrontier {
    pub store_scope: StoreScopeRef,
    pub query_scope: FactQueryScope,
    pub descriptor_catalog_watermark: DescriptorCatalogWatermark,
    pub projection_generation: FactProjectionGeneration,
    pub max_included_store_commit_order: StoreCommitOrder,
    pub commit_watermark: StoreCommitWatermark,
}

pub struct StoreReceiptAuthentication {
    pub store_identity: StoreIdentity,
    pub scheme: StoreReceiptAuthenticationScheme,
    pub key_id: Option<StoreKeyId>,
    pub signature_or_mac: Bytes,
}

pub enum StoreReceiptAuthenticationScheme {
    LocalEd25519Sha256JcsV1,
}

pub struct FactSelectionEvidence {
    pub selection_policy_hash: ContentDigest,
    pub selected_indices: Vec<u64>,
    pub selected_summaries_digest: Option<ContentDigest>,
}
```

`FactQueryScope` is read-specific. `RunPrivate` is not queryable through the fact
index and therefore cannot appear in a canonical query plan.

V1 query execution resolves exactly one descriptor hash before planning. Kind
or compatibility-group selectors are discovery and convenience inputs; the
canonical plan must contain one resolved descriptor. Multi-descriptor query
execution is deferred.

The receipt is store-owned evidence for what the store returned for the
canonical query plan. Selection is state-owned evidence for what the workflow
chose from the returned set. `selected_indices` index into `returned_refs`,
which prevents a state from selecting facts outside the returned set. Selection
indices must be canonical: unique, in bounds, and sorted unless the selection
policy explicitly preserves a different order. Returned refs are enough only
when replay can deterministically recompute every value used by the selection
policy from retained artifacts. If a selection policy depends on returned field
summaries, `selected_summaries_digest` must pin the exact summaries used by that
policy.

V1 does not retain every omitted matching fact for limited queries and does not
produce cryptographic absence proofs for empty results. Instead, v1 treats a
store-authenticated `FactQueryReceipt` plus semantic `StoreReadFrontier` as the
replay authority for empty and bounded result-set decisions. The receipt
authentication must bind the canonical query plan, resolved descriptor set,
visibility/scope decision, ordering policy, limit, returned refs, returned
summaries digest, result cardinality, result-set digest, and read frontier.
Ordinary database cursors or pagination tokens are not replay authority.
Cryptographic absence proofs can be added later without changing
`FactQueryEvidence`.

`store_receipt_hash` is computed over the canonical receipt body plus the
canonical query plan hash. It excludes `store_receipt_hash` and
`store_receipt_authentication` themselves. V1 has no unauthenticated receipt
mode. The v1 baseline scheme is
`StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1`: the local store
signs `store_receipt_hash` with a configured store signing key, records the
store identity and key id, and replay verifies the signature against the
configured local store trust root. Remote federation, key rotation policy, and
alternative receipt schemes are deferred. `result_set_digest` is computed over
the ordered returned refs and returned field summaries digest; it does not
include state selection evidence.

`StoreReadFrontier` is semantic, not a PostgreSQL cursor. At minimum it must
bind the store scope, query audience/scope, descriptor catalog watermark, fact
projection generation or rebuild id, maximum included store commit order, and a
commit watermark. The receipt proves query evaluation over a complete
authorized projection snapshot or prefix at the named generation and watermark
for that scope and ordering.

Replay verifies the pinned source facts, artifact evidence, canonical query,
ordering policy, store receipt authentication, result-set digest, selection
policy, selected indices, selected summaries digest, scope decision, descriptor
resolution, query compiler/canonicalizer versions, and read frontier. Selection
replay runs over the returned refs and pinned summaries from the authenticated
receipt; it does not ask the live store to repeat the query.

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

mfm facts show <public-ref>
mfm facts explain wallet.balance
```

`mfm facts describe wallet.balance` lists known descriptors/schema versions for
the kind. A kind-only query is allowed only when the kind resolves
unambiguously. Otherwise the CLI must return an ambiguity error and require
`--shape` or a descriptor hash.

`--shape` is a user-facing selector for one `FactDescriptor`: it may be a schema
alias, compatibility-group version, or descriptor hash, but execution must
resolve it to one descriptor before planning the query. V1 public queries
execute against one resolved descriptor. Kind-only or compatibility-group
commands may discover that descriptor, but they must fail with a stable
ambiguity error if they cannot resolve exactly one descriptor.

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
GET /v1/facts/ref/{public_ref}
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

Public `kinds`, `describe`, `explain`, query, latest, history, top, and exact-ref
endpoints all pass through the same query boundary: `audience = Platform`,
authorized `FactVisibilityScope`, descriptor exposure policy, and stable
redacted errors. They must fail closed and must not disclose `Control` or
`RunPrivate` existence through counts, ambiguity messages, direct ref lookup,
timing-shaped errors, or descriptor discovery.

### InternalFactRef And PublicFactRef DTOs

The internal ref DTO should remain generic. It is a trusted runtime/store/replay
object, not a public API output:

```rust
pub struct InternalFactRef {
    pub fact_claim_id: FactClaimId,
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
    pub request_schema_id: Option<SchemaId>,
    pub request_hash: Option<ContentDigest>,
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

The implementation may keep the existing internal type name `FactRef`, but the
public contract must treat it as `InternalFactRef`. It intentionally does not
contain domain columns such as `chain`, `network`, `asset`, `country`,
`measure`, `amount`, `temperature`, or `block_number`. Those are
descriptor-derived fields joined through `fact_claim_id`.

The decomposed source run id, sequence, and ordinal are the storage
presentation of `FactClaimId`. They are not independent identity fields, and
they are not public fact identifiers.

Public API output uses a smaller presentation DTO:

```rust
pub struct PublicFactRef {
    pub public_ref: PublicFactRefId,
    pub fact_kind: FactKind,
    pub descriptor: PublicFactDescriptorRef,
    pub recorded_at: Timestamp,
    pub observed_at: Option<Timestamp>,
    pub fields: Vec<PublicFactFieldValue>,
}
```

`PublicFactRefId` is opaque, scope-checked, and only resolvable through the fact
query service. It must not be an artifact id, raw `FactClaimId`, source run
coordinate, subject hash, or stable cross-scope correlation handle. Public fact
output must not expose `artifact_id`, artifact evidence hashes, canonical
subject material, subject hashes, request/response hashes, raw run ids, event
ids, or capability routing details. Returning a public ref never grants artifact
read authority.

Field values may be returned only according to descriptor exposure policy:

- `Returnable`: may be displayed and returned by public APIs
- `QueryOnly`: may be used as a filter/order term but omitted from returned
  summaries
- `Hidden`: retained for authority/rebuild or internal use, but not queried or
  returned publicly

`QueryOnly` is not a privacy feature. Query predicates, ordering, cardinality,
and timing can make values inferable. Use it only for fields that are acceptable
to expose indirectly through query behavior.

Allowed operators decide equality, range, prefix, or ordering support. Exposure
does not encode operator semantics. `Control` and other internal queries still
require internal capabilities and scope checks; descriptor exposure controls
what can cross the public boundary, not whether private data is safe to store.

## Privacy And Security

Facts are not secret storage.

Rules:

- secret-bearing data must never enter fact subject material, artifacts, events,
  public outputs, diagnostics, fixtures, or snapshots
- explicit visibility controls projection and query access, but does not make
  secret data acceptable
- `RunPrivate` facts must not create fact index or term rows
- `Control` audience facts are cross-run discoverable only through internal
  operational capabilities and their scopes
- public APIs must expose only descriptor-approved fields and must not return
  canonical subject material, response artifacts, internal refs, artifact ids,
  artifact evidence hashes, subject hashes, or raw run/event coordinates
- there should be no generic public raw-subject-material read path
- public APIs must redact errors and avoid endpoint/auth leakage; exact-ref and
  descriptor-discovery endpoints must fail closed for non-public facts
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

If a consuming run records an `ArtifactReferenced` event with
`ArtifactRole::FactQueryEvidence`, retention also follows the query evidence
edge. The reference event, evidence artifact, store-authenticated receipt,
state-owned selection evidence, referenced `InternalFactRef`s, descriptor
artifacts, source events, subject material, response artifacts, and receipt
authority must remain available for replay even if the fact is later removed
from live query surfaces. V1 retention does not need to keep every omitted
matching fact for empty or limited result-set replay; the authenticated receipt
and semantic read frontier are the authority for that decision.

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
- every fact index row and term row uses the `FactClaimId` derived from source
  run id, sequence, and ordinal
- public platform APIs expose only authorized `audience = Platform` rows
- no `RunPrivate` fact creates fact index or term rows
- descriptor hash matches descriptor canonical bytes
- descriptor hash is allowed by the producing node's certified spec
- fact subject namespace hash matches canonical `FactSubjectNamespaceV1`
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

Fact queries and query evidence must fail closed.

- If query input resolves to zero descriptors, or more than one descriptor in
  v1, compilation fails.
- If scope authorization fails, query execution fails without revealing whether
  matching `Control` or `RunPrivate` facts exist.
- If a public exact-ref lookup resolves only to a non-public fact, the endpoint
  returns the same class of not-found/redacted error as an unknown ref.
- If the query compiler, canonicalizer version, descriptor hash, scope decision,
  ordering policy, or limit cannot be replayed exactly, replay fails.
- If `FactQueryEvidence` artifact admission fails, the consuming run does not
  commit the evidence event.
- If receipt authentication is absent, uses an unsupported scheme, references
  an unknown key id, fails signature verification, or does not match the
  configured store trust root, recording/replay rejects the evidence.
- If frontier type, projection generation, descriptor catalog watermark,
  maximum included store order, or commit watermark cannot be verified,
  recording/replay rejects the evidence.
- If returned refs, returned summaries, result cardinality, or result-set digest
  do not match the authenticated receipt, recording/replay rejects the evidence.
- If selected indices are duplicated, out of bounds, non-canonical for their
  selection policy, or inconsistent with `selected_summaries_digest`,
  recording/replay rejects the evidence.
- If retained source events, descriptors, subject material, response artifacts,
  or artifact evidence cannot be verified, replay fails.

## Migration Path

This RFC assumes a destructive development reset, not backward compatibility.
Existing fact records, artifacts, and projections from the old contract are
discarded with the old store baseline.

1. Document `FactRecorded` as the platform knowledge primitive.
2. Add explicit scoped `FactVisibility` with `RunPrivate` and indexed
   `FactAudience::{Control, Platform}`.
3. Add derive-owned `MfmFactType` support.
4. Add certified `FactDescriptor` admission and validation against the
   producing node's certified spec.
5. Replace ad hoc `FactKey` construction with typed subject material.
6. Require every `FactRecorded` to carry normalized `FactRecordedPayload` data.
7. Add `ArtifactRole::FactDescriptor` descriptor authority artifacts plus
   logical `fact_descriptor_index`, `run_fact_descriptor_admissions`, `fact_index`, and `fact_index_terms`
   projections.
8. Populate projections in the same append transaction as `run_events`.
9. Add projection validation and rebuild.
10. Add reusable facts-query compilation and canonicalization.
11. Add `ArtifactRole::FactQueryEvidence` artifacts referenced by private
    generic `ArtifactReferenced` events, with canonical `FactQueryEvidence`,
    `CanonicalFactQueryPlan`, `FactQueryReceipt`, and `FactSelectionEvidence`.
12. Add descriptor-scoped, kind-first CLI and REST query APIs over the reusable
    query compiler and `PublicFactRef` DTOs.
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
- Every `FactRecorded` must use a derive-owned typed `MfmFactType` after the
  reset.
- Fact descriptors are certified authority: a node may emit only descriptors
  allowed by its certified spec.
- `FactDescriptor` is the durable shape for subject identity, result fields,
  metadata fields, ordering, operators, units/scales, and exposure.
- Field extraction is declarative and kernel-owned; append/rebuild must not call
  arbitrary domain crate code.
- Fields have stable descriptor-owned ids; paths are UX/schema pointers.
- `FactKey` is derived only from canonical `FactSubjectNamespaceV1` and
  canonical subject material.
- Observed-result indexing is in v1 through descriptor-declared result fields.
- Metadata fields are in v1 for fields such as `recorded_at`, `observed_at`,
  and store order.
- Descriptor canonical bytes are durable content-addressed framework artifacts
  with `ArtifactRole::FactDescriptor`, admitted before or during the first
  append that needs them.
- Descriptor hashes are not embedded inside descriptor canonical bytes.
- `InternalFactRef` is generic provenance and ordering, not domain labels;
  public APIs return `PublicFactRef`.
- Canonical subject material is persisted in bounded form for v1.
- Persisted subject material is authority/rebuild material, not public output.
- Projection for indexed facts is maintained inside the append transaction.
- PostgreSQL materialized views and SQL artifact-parsing triggers are not the v1
  projection strategy.
- Consuming runs pin `FactQueryEvidence` as private read evidence for replay.
- Query compilation lives in a reusable facts-query crate/service; CLI and REST
  remain transport adapters.
- Query evidence stores a canonical query plan, store-owned query receipt, and
  state-owned selection evidence, not only hashes.
- Query evidence authority is a run-private generic `ArtifactReferenced` event
  pointing at an admitted canonical `FactQueryEvidence` artifact with
  `ArtifactRole::FactQueryEvidence`.
- V1 fact queries resolve exactly one descriptor before planning.
- V1 receipt authentication has no unauthenticated mode and uses
  `LocalEd25519Sha256JcsV1` against a configured local store trust root.
- V1 empty and limited query replay trusts a store-authenticated receipt plus a
  semantic read frontier; omitted matching facts are not retained as proof.
- Query ordering policy is explicit and hashed into query evidence.
- Queryable indexed facts require retention of their verification authority.
- Query evidence creates retention edges for the facts and artifacts it pins.
- V1 supports one claim per response artifact.
- Compatible descriptor versions share `FactKey` only when they preserve the
  same stable subject namespace and subject material values.
- V1 checkpoint facts are ordinary `Control` facts; checkpoint conflict
  enforcement is outside fact descriptors.

## V1 Deferrals

- Batched or multi-claim response artifacts.
- Cryptographic absence proofs for fact queries.
- Multi-descriptor query execution.
- Multi-valued fields and array extraction.
- Full named-scope authorization beyond `FactVisibilityScope::Default`.
- Public pagination internals beyond opaque cursors.
- Remote/federated receipt verification and receipt signing-key rotation
  policy.
- Subject-term deduplication as a required physical storage shape.
- Cross-store fact export/import.
- Generic lease or compare-and-append authority for exclusive checkpoint
  advancement.

## Deferred Questions

- Which typed field operators should be added after the v1 floor?
- Which checkpoint facts should ever be platform-visible rather than `Control`?
- What operational lease/backoff policy should launch recurring collector
  cycles outside the durable state-machine semantics?

## Implementation Ownership

Suggested crate placement for the first planning pass:

- `crates/kernel/facts`: fact descriptors, field descriptors, extraction
  grammar, `FactKey`, `FactClaimId`, `InternalFactRef`, query
  scope/plan/receipt/selection types, and validation logic.
- `crates/kernel/events`: new `FactRecordedPayload`, normalized `FactClaim`
  shape, `ArtifactRole::FactDescriptor`, and
  `ArtifactRole::FactQueryEvidence` on the existing generic artifact-reference
  event path.
- `crates/kernel/values`: shared `MfmValue` canonical value integration needed
  by fact descriptor derivation and extraction.
- `crates/kernel/program` and `crates/kernel/program-derive`: typed fact derive
  support and certified node emission metadata.
- `crates/kernel/spec` and `crates/kernel/certify`: descriptor allow-list in
  certified spec material.
- `crates/kernel/store`: abstract append/query/rebuild contracts for fact
  authority and fact indexes.
- `crates/storages/stream-store-postgres`: descriptor artifact admission,
  `fact_descriptor_index`, `run_fact_descriptor_admissions`, `fact_index`, `fact_index_terms`, append projection,
  query execution, and rebuild.
- `crates/app`, `bin/cli`, and `bin/rest-api`: transport-level command/API
  surfaces over the reusable facts-query contract and `PublicFactRef` output.

## Preferred First Implementation

Build this in gates, while keeping one vertical slice as the north star.

Gate 1: facts kernel types and canonical contracts.

- `FactVisibility::{RunPrivate, Indexed { audience, scope }}`
- `FactAudience::{Control, Platform}`
- `FactDescriptor`, `FactFieldDescriptor`, `FactFieldExtraction`, field ids, and
  ordering descriptors
- canonical `FactSubjectNamespaceV1`, `FactSubjectMaterialV1`, and kind-scoped
  `FactKey` derivation
- `FactClaimId` derived for every `FactRecorded` from run-stream event
  coordinates
- staged fact/evidence handles and post-commit id mapping
- v1 field extraction grammar and validation
- `FactRecordInput`, `FactRecordedPayload`, normalized `FactClaim`,
  `InternalFactRef`, `PublicFactRef`
- existing generic `ArtifactEvidenceRef` usage for fact responses and query
  evidence artifacts
- canonical hash golden tests and descriptor compatibility tests

Done when:

- canonical hash golden tests cover descriptor, subject namespace, subject
  material, `FactKey`, and `FactClaimId`
- invalid subject/result shapes fail with stable typed errors
- `FactClaimId` derivation works for indexed and `RunPrivate` facts
- the type inventory can be mapped to concrete crates without changing the RFC
  contract

Gate 2: certified descriptor emission.

- one `MfmFactType` derive for a domain fact
- descriptor artifact generation/admission
- certified spec descriptor allow-list per producing node
- runtime append rejection for descriptors not allowed by certified spec
- compile-fail tests for invalid derive shapes
- certification tests proving descriptor allow-list changes affect spec hashes

Done when:

- derive emits `ArtifactRole::FactDescriptor` descriptor artifacts
- certified specs include descriptor allow-lists per producing node
- appending with a store-admitted but uncertified descriptor is rejected
- descriptor allow-list changes alter certified spec hashes

Gate 3: event reset and append projection.

- mandatory normalized `FactRecordedPayload` on `FactRecorded`
- `FactClaimId` derivation for every `FactRecorded`, independent of whether the
  fact is indexed
- `T::Subject` to `FactSubjectEvidence.subject_material`
- `T::Response` to an `ArtifactEvidenceRef` with
  `ArtifactRole::FactResponse`
- v1 one-claim-per-response-artifact rule
- logical `fact_descriptor_index`, `run_fact_descriptor_admissions`, `fact_index`, and `fact_index_terms`
  projections in PostgreSQL
- subject, result, and metadata term extraction in the append transaction
- append rollback tests for descriptor, extraction, artifact, and projection
  failures
- projection validation and rebuild parity tests

Done when:

- no old-shape `FactRecorded` can be appended after the reset
- indexed append commits event, artifacts, descriptor projection, fact index,
  and term rows atomically
- `RunPrivate` appends retain authority but create no fact index or term rows
- rebuild from retained authority reproduces projection rows byte-for-byte or
  reports deterministic validation differences
- descriptor, extraction, artifact, event, and projection failures roll back the
  whole append

Gate 4: reusable query compiler and replay evidence.

- reusable facts-query crate/service
- canonical query and ordering representation
- explicit ordering policies for store commit order, observed time, and
  descriptor-declared result fields
- semantic `StoreReadFrontier`
- read-only `FactQueryScope`
- `CanonicalFactQueryPlan` with descriptor resolution,
  compiler/canonicalizer version, scope decision evidence, canonical query,
  ordering, and limit
- `FactQueryReceipt` as store-owned evidence with frontier type, returned refs,
  optional field summaries, result cardinality, result-set digest, and store
  receipt authentication
- minimum `StoreReceiptAuthentication` contract and trust-root verification
- `FactSelectionEvidence` as state-owned evidence with selected indices and
  selected-summaries digest
- `ArtifactRole::FactQueryEvidence` artifact admission and private generic
  `ArtifactReferenced` event emission
- v1 empty/limited result-set semantics over authenticated receipts
- retention edges from query evidence

Done when:

- public and internal callers share the same query compiler/canonicalizer
- v1 query planning rejects zero-descriptor and multi-descriptor resolution
- receipt tampering, unsupported auth schemes, unknown key ids, bad signatures,
  frontier mismatches, and result-set digest mismatches are rejected
- empty and limited query results replay from authenticated receipts without
  live store reads
- invalid selected indices or selected-summary digests reject query evidence
- query evidence creates retention edges for source refs, descriptors,
  artifacts, receipts, and selection evidence

Gate 5: public surfaces.

- descriptor-scoped, kind-first `mfm facts` query path
- REST query surface over the same query compiler
- public `PublicFactRef` output filtered to `audience = Platform`
- internal `Control` query capability for runtime/collector workflows
- public/Control visibility rejection tests

Done when:

- public CLI and REST output never expose internal refs, artifact ids, artifact
  evidence hashes, subject hashes, raw run/event coordinates, or response
  artifacts
- `kinds`, `describe`, `explain`, query, latest/history/top, and exact-ref
  endpoints all fail closed for `Control` and `RunPrivate` facts
- `QueryOnly` fields can filter/order but are not returned in public summaries
- direct public ref lookup returns the same redacted not-found class for
  unknown and non-public refs

Gate 6: first collector workflow.

- one collector-style workflow that records platform-visible domain facts using
  existing state-machine primitives
- ordinary `Control` checkpoint facts with partition and high-watermark fields
- certified checkpoint query/selection policy pinned by `FactQueryEvidence`
- operational launcher/backoff policy kept outside durable fact semantics

Done when:

- one recurring workflow records platform-visible domain facts using ordinary
  operations, states, adapters, transports, and run events
- the workflow resumes from an indexed `Control` checkpoint selected through a
  pinned query evidence artifact
- crash/retry behavior preserves committed facts/checkpoints and retries only
  uncommitted observations
- no collector-specific runtime primitive or fact event is introduced

This proves the model: MFM's shared knowledge graph is built from ordinary typed
`FactRecorded` events, and collectors are recurring certified workflows that
produce those events.
