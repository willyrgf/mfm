# Plan: Implement RFC_COLLECTORS

Status: planning document.

Source RFC: [`docs/RFC_COLLECTORS.md`](docs/RFC_COLLECTORS.md)

This document plans implementation of collectors and platform facts without changing the RFC.
It is concrete enough to execute in staged PRs, but each gate should still look for the smallest
implementation that preserves the RFC invariants. If a gate uncovers a simpler crate split or
API shape, prefer the simpler shape after verifying the authority, replay, privacy, and boundary
rules below still hold.

## Read-In Summary

The current repository already has an older fact surface:

- `crates/kernel/events/src/lib.rs` has `mfm_events::v1::FactRecorded` with request/response hashes,
  a flat `FactKey`, and a response artifact id.
- `ArtifactRole::FactResponse` exists, but `FactDescriptor` and `FactQueryEvidence` artifact roles
  do not.
- `crates/kernel/runtime/src/runner_kit.rs` lets runners record facts by caller-provided `FactKey`.
- `crates/kernel/store/src/v1/mod.rs` has a simple `FactProjection` keyed by the old fact shape.
- `crates/storages/stream-store-postgres` owns the production append path, schema baseline, and
  rebuild/validation authority.
- `crates/collectors/proof` is categorized as a state crate and acts as a proof domain contract, not
  as the RFC's recurring collector runtime.
- `crates/collectors/btc-jsonrpc-http` is categorized as a transport crate and already contains a
  redacted Bitcoin JSON-RPC HTTP client that can be reused for a narrow first collector after the
  facts kernel exists.

The RFC is intentionally a destructive event-contract reset. The plan assumes no migration of old
fact events, projections, or artifacts.

## Implementation Principles

1. `FactRecorded` is the only fact event. Do not add `CollectedFactRecorded` or any collector-only
   event family.
2. Collectors are recurring certified workflows over ordinary operations, states, adapters,
   transports, runtime events, and artifacts. Do not add a collector runtime primitive in v1.
3. Descriptor authority has two required legs: descriptor bytes must be content-addressed and
   available to the store, and the producing certified node must allow the descriptor hash.
4. Store admission alone is not semantic permission to publish a fact descriptor.
5. `FactKey` is derived only from fact kind plus descriptor-declared subject fields. Result and
   metadata fields are searchable and orderable, but must not affect subject identity.
6. Field extraction must be declarative and kernel-owned. Append, rebuild, query planning, and
   replay must not call arbitrary domain crate code to compute terms.
7. Every fact has explicit visibility. There is no implicit public publication.
8. `RunPrivate` facts are retained as run evidence only and never create fact index rows or term
   rows.
9. `Control` facts are non-public, indexed, cross-run operational evidence available only through
   internal capabilities and scope checks.
10. Public surfaces query only `audience = Platform` under authorized scope and descriptor exposure
    policy.
11. Fact refs and term rows are rebuildable projections. Run stream events, artifact admissions,
    descriptor bytes, subject material, and response artifacts are authority.
12. A live fact query is not replay authority. Consuming runs must pin `FactQueryEvidence` through a
    private `ArtifactReferenced` event using `ArtifactRole::FactQueryEvidence`.
13. Hashed structured data uses canonical JSON, contains no floats, has bounded size, and contains
    no secrets.
14. CLI and REST decode transport input and render public DTOs only. Query compilation belongs in a
    reusable facts service, not in bins.
15. The first collector should prove the model with one narrow bounded cycle, not a general
    collector scheduler.

## Proposed Crate Ownership

Preferred ownership, with simplification allowed if dependency direction stays clean:

| Area | Proposed owner | Notes |
| --- | --- | --- |
| Fact descriptor model, field grammar, visibility, `FactKey`, `FactClaimId`, internal refs, query evidence types | new `crates/kernel/facts` (`mfm-facts`) | Kernel crate, domain-free. Add to workspace with `package.metadata.mfm.category = "kernel"`. |
| Canonical JSON/hash helpers reused by facts | existing `crates/kernel/canonical` and `crates/kernel/values` | Avoid duplicating canonicalization. Add value-path/extraction support in facts or values depending on dependency needs. |
| Normalized event payloads and artifact roles | `crates/kernel/events` | Events may depend on `mfm-facts` if the kernel DAG is updated accordingly, or duplicate only versioned event structs with facts types re-exported intentionally. |
| Store commit contracts, projection traits, query receipt authority, retention hooks | `crates/kernel/store` | Abstract authority and validation rules. No SQLx. |
| Fact derive and authored descriptor emission | `crates/kernel/program-derive`, `crates/kernel/program` | Derive emits descriptor shape and `MfmFactType`; program/spec carries node descriptor allow-lists. |
| Certified descriptor allow-list | `crates/kernel/spec`, `crates/kernel/certify` | Allow-lists are hash-defining spec material. |
| Runtime staging and recorder API | `crates/kernel/runtime` | Runner kit exposes only typed `record_fact<T: MfmFactType>` and query-evidence staging. |
| Postgres physical tables, append projection, query execution, rebuild, validation | `crates/storages/stream-store-postgres` | Owns SQL baseline and production implementation. |
| Evidence-only app fact query/read services and public DTO shaping | `crates/app` | No workflow semantics. No live transports for fact queries. |
| Public commands/routes | `bin/cli`, `bin/rest-api` | Thin transport surfaces over app/facts services. |
| First collector capability contracts | likely new `crates/btc-capabilities` for Bitcoin read authority | Domain capability contract, no live IO. |
| First collector states | prefer `crates/states/btc` | Reusable bounded read/normalize/checkpoint state behavior. Use a collector-specific state crate only if the state semantics truly are not reusable outside collector cycles. |
| First collector operation | likely `crates/ops/btc-chain-head-collector-op` | Deterministic cycle topology only. |
| First collector adapter/transport binding | likely `crates/adapters/btc-jsonrpc` plus existing `crates/collectors/btc-jsonrpc-http` transport | Consider later renaming the existing transport path, but do not block this plan on a rename. |

Do not add a separate `facts-query` crate at first unless `mfm-facts` becomes too broad or a
dependency cycle appears. A `mfm_facts::query` module can own v1 query compilation and canonical
planning. Splitting later is acceptable if the public interfaces are stable.

## Migration And Reset Strategy

This is a destructive development reset, matching the RFC.

1. Update event/store/schema baselines in the same gate that makes old facts invalid.
2. Replace old `FactRecorded` payload parsing/encoding with the normalized payload. Do not support
   a compatibility decoder for historical fact events in production paths.
3. Remove or rewrite tests and fixtures that construct old flat `FactKey` facts.
4. Replace the old simple fact projection in `mfm-store` and Postgres with descriptor/index/term
   projections.
5. Reset Postgres baseline `0001_run_store.sql` rather than adding compatibility migrations for old
   fact tables or rows.
6. Reset any dev artifacts or snapshots that encode old fact events.
7. Document in implementation PRs that existing development stores must be recreated.
8. Do not edit the RFC unless a blocking ambiguity prevents implementation. Record questions in
   this plan and resolve through a follow-up RFC edit only if necessary.

## Gate 1: Facts Kernel Types And Canonical Contracts

### Objective

Create the domain-free facts kernel contract: descriptor model, field extraction grammar, visibility
types, subject namespace/material canonicalization, `FactKey`, `FactClaimId`, normalized fact claim
data, internal fact refs, replay evidence data structures, and public-surface filtering constraints.
This gate should be mostly pure Rust types, validation, canonicalization, and tests.

### Crates And Modules Likely Touched

- Add `crates/kernel/facts`.
- Update workspace `Cargo.toml`.
- Add dependency edges from `mfm-events`, `mfm-store`, `mfm-runtime`, `mfm-program`, and
  `mfm-certify` only as each later gate needs them.
- Use `crates/kernel/canonical` for canonical bytes/digests.
- Use `crates/kernel/values` for `MfmValue` integration and schema descriptors.
- Add cargo metadata contract allow-list for the new kernel crate if required by
  `tests/integration/tests/cargo_metadata_contract.rs`.

### New Or Changed Public Types And Interfaces

In `mfm-facts`:

- `FactKind`
- `FactDescriptor`
- `FactFieldDescriptor`
- `FactFieldId`
- `FactFieldPath`
- `FactFieldAccessor`
- `FactMetadataField`
- `FactFieldValueType`
- `FactCanonicalScalar`
- `FactFieldExposure`
- `FactQueryOperator`
- `FactOrderingDescriptor`
- `FactOrderingTerm`
- `FactOrderingName`
- `FactUnit`
- `FactScale`
- `FactVisibility`
- `FactAudience`
- `FactVisibilityScope`
- `FactSubjectNamespaceV1`
- `FactSubjectNamespaceFieldV1`
- `FactSubjectMaterialV1`
- `FactSubjectValueV1`
- `FactKey`
- `FactClaimId`
- `FactSubjectEvidence`
- `FactRequestEvidence`
- `FactProducerProvenance`
- `FactClaim`
- `InternalFactRef`
- `FactQueryEvidence`
- `CanonicalFactQueryPlan`
- `FactQueryReceipt`
- `FactSelectionEvidence`
- `StoreReadFrontier`
- `StoreReadFrontierType`
- `StoreReceiptAuthentication`
- `StoreReceiptAuthenticationScheme`
- `ReturnedFieldSummaries`
- `QueryResultCardinality`

Gate 1 should define only kernel and replay contracts. Rendered public DTOs such as
`PublicFactRef`, `PublicFactRefId`, `PublicFactDescriptorRef`, and `PublicFactFieldValue` belong to
the app/public surface work in Gate 5, backed by kernel facts contracts and descriptor exposure
policy.

Pure functions:

- `validate_descriptor(&FactDescriptor) -> Result<ValidatedFactDescriptor, FactDescriptorError>`
- `canonical_fact_descriptor_bytes(&FactDescriptor) -> Result<PlainCanonicalJsonBytes, ...>`
- `fact_descriptor_hash(&FactDescriptor) -> Result<ContentDigest, ...>`
- `fact_subject_namespace(&ValidatedFactDescriptor) -> FactSubjectNamespaceV1`
- `fact_subject_namespace_hash(&FactSubjectNamespaceV1) -> ContentDigest`
- `extract_subject_material(...) -> Result<FactSubjectMaterialV1, ...>`
- `subject_material_hash(&FactSubjectMaterialV1) -> ContentDigest`
- `derive_fact_key(namespace_hash, material_hash) -> FactKey`
- `derive_fact_claim_id(run_id, seq, ordinal) -> FactClaimId`
- `extract_terms(descriptor, subject, response, metadata) -> Result<Vec<FactIndexTerm>, ...>`

Keep constructors validating by default. Use private fields where values should not be forgeable
without validation.

### Storage Or Projection Changes

None in this gate, except type definitions that later store/projection gates will consume.

### Tests And Acceptance Criteria

- Golden canonical bytes and hashes for:
  - `FactDescriptor`
  - `FactSubjectNamespaceV1`
  - `FactSubjectMaterialV1`
  - `FactKey`
  - `FactClaimId`
  - `CanonicalFactQueryPlan`
  - `FactQueryReceipt` body hash rules
- Descriptor validation rejects:
  - duplicate `field_id`
  - zero subject fields
  - optional subject fields
  - ordering terms for missing fields
  - ordering terms for non-sortable fields
  - incompatible operator/value-type pairs
  - floats or unsupported scalar encodings
  - unsupported arrays, wildcards, slices, or repeated values
  - field paths whose prefix conflicts with accessor source
  - overlong scalar bytes or excessive numeric precision
- Subject identity tests prove:
  - result-field value changes do not change `FactKey`
  - metadata changes do not change `FactKey`
  - subject field id/type/unit/scale/value changes do change `FactKey`
  - descriptor hash is not embedded in descriptor canonical bytes
  - descriptor path changes do not change `FactKey` when stable subject field ids and values match
- Extraction tests cover required and optional fields:
  - required missing/null fails for indexed facts
  - optional missing/null yields no term
  - scalar type mismatch fails
- Secret/no-float tests use `mfm-values` secret marker and no-float policy where available.
- Public rustdoc exists for all public items because new library crates use `#![warn(missing_docs)]`.
- Focused verification:
  - `cargo test -p mfm-facts`
  - `cargo test -p mfm-canonical`
  - `cargo test -p mfm-values`
  - `cargo test -p mfm-integration-tests --test cargo_metadata_contract`

### Dependencies On Previous Gates

None.

### Risks And Failure Modes

- Too much query execution leaks into the facts kernel. Keep gate 1 to data model, canonicalization,
  validation, extraction, and canonical query/evidence shapes.
- A separate facts crate can widen the kernel dependency DAG. Update architecture metadata checks
  deliberately and keep it domain-free.
- Manual implementations of future fact traits could appear authoritative. This gate should make
  clear that typed trait impls are authoring helpers only; descriptor bytes and certification are
  authority.
- Canonical bytes may accidentally include computed hashes. Tests must reject self-referential
  hash fields.

### Explicitly Out Of Scope

- Derive macro support.
- Certified spec descriptor allow-lists.
- Store append validation.
- Postgres tables.
- Query execution.
- CLI/REST commands.
- Collector workflow implementation.

## Gate 2: Certified Descriptor Emission

### Objective

Make fact descriptors part of certified spec authority. A producing node may emit only descriptors
allowed by its certified spec, and descriptor allow-list changes must affect the certified spec hash.

### Crates And Modules Likely Touched

- `crates/kernel/program-derive`
- `crates/kernel/program`
- `crates/kernel/values`
- `crates/kernel/spec`
- `crates/kernel/certify`
- `crates/kernel/events` for `ArtifactRole::FactDescriptor`
- `crates/kernel/runtime` only enough to carry certified descriptor authority into commit planning
- Existing proof domain tests in `crates/collectors/proof` can be updated as a small conformance
  fact type if useful.

### New Or Changed Public Types And Interfaces

- `MfmFactType` trait, likely in `mfm-program` or `mfm-values` depending on dependency direction:
  - `type Subject: MfmValue`
  - `type Response: MfmValue`
  - `fn descriptor() -> &'static FactDescriptor`
  - `fn subject(&self) -> &Self::Subject`
  - `fn response(&self) -> &Self::Response`
- `#[derive(MfmFactType)]` in `mfm-program-derive`.
- State/program descriptor metadata for node-emitted fact descriptor hashes, for example:
  - `FactDescriptorRef`
  - `NodeFactDescriptorAllowance`
  - `StateDescriptor::emitted_fact_descriptors`
  - `TypedExecutionSpec` node field carrying allowed descriptor hashes and descriptor artifact refs
- Certification registry validation that descriptor canonical bytes, descriptor hash, subject schema
  id, response schema id, and field grammar are valid and hash-defining.
- `ArtifactRole::FactDescriptor` with a closed role contract. Likely schema policy should be exact
  evidence schema or a new exact descriptor schema policy; decide during implementation and test it
  in artifact role contract goldens.

### Storage Or Projection Changes

- No SQL projection yet.
- Descriptor artifact staging/admission may need a generic store/runtime representation before
  Postgres projection exists.

### Tests And Acceptance Criteria

- Derive pass tests:
  - minimal valid fact with subject and response structs
  - result fields orderable by integer/timestamp
  - metadata fields accepted where declared by descriptor annotations
- Derive compile-fail tests reject:
  - subject type with unannotated serialized fields
  - optional subject fields
  - subject arrays/repeated values
  - unsupported result field scalar
  - duplicate field ids
  - missing subject field annotations
  - floats
  - fields with secret-marker names or values in test construction where applicable
- Certification tests prove:
  - descriptor allow-list appears in certified spec material
  - descriptor allow-list changes change the spec hash
  - descriptor bytes/certificate verification fails if descriptor canonical bytes are tampered
  - a node cannot claim a descriptor emitted by another node unless the spec allows it for that node
- Artifact role tests update `ArtifactRole::ALL`, parse/roundtrip tests, schema descriptor goldens,
  and Postgres role tag decoding tests.
- Focused verification:
  - `cargo test -p mfm-program-derive --test derive_ui`
  - `cargo test -p mfm-program-derive --test descriptor_golden`
  - `cargo test -p mfm-certify`
  - `cargo test -p mfm-events`
  - `cargo test -p mfm-store`

### Dependencies On Previous Gates

- Requires `mfm-facts` descriptor, validation, canonicalization, and hash derivation.

### Risks And Failure Modes

- Descriptor allow-lists could be treated as registry-time mutable policy instead of certified spec
  material. Acceptance tests must prove hash changes and persisted verification.
- The derive macro could become the semantic authority. Store/runtime must still validate descriptor
  bytes and node allow-lists later.
- Descriptor artifacts could be admitted globally without retention or same-commit proof. Gate 3
  must close this with append validation.
- Adding `mfm-facts` dependencies to `mfm-events` or `mfm-spec` may create a kernel DAG cycle.
  Resolve by moving only domain-free identity/value types into the lowest viable crate.

### Explicitly Out Of Scope

- Replacing the runtime fact recorder.
- Rejecting old `FactRecorded` appends in Postgres.
- Fact query compiler.
- Public APIs.
- First collector.

## Gate 3: Event Reset And Append Projection

### Objective

Replace the old `FactRecorded` event contract with normalized typed fact claims and maintain
descriptor/index/term projections atomically in the append transaction. After this gate, no
old-shape fact event can be appended on production paths.

### Crates And Modules Likely Touched

- `crates/kernel/events/src/lib.rs`
- `crates/kernel/store/src/v1/mod.rs`
- `crates/kernel/store/src/v1/artifact_refs.rs`
- `crates/kernel/runtime/src/runner_kit.rs`
- `crates/kernel/runtime/src/commit.rs`
- `crates/kernel/runtime/src/history.rs`
- `crates/kernel/runtime/src/artifacts.rs`
- `crates/storages/stream-store-postgres/migrations/0001_run_store.sql`
- `crates/storages/stream-store-postgres/src/run_store/append.rs`
- `crates/storages/stream-store-postgres/src/run_store/projections.rs`
- `crates/storages/stream-store-postgres/src/run_store/event_rows.rs`
- `crates/storages/stream-store-postgres/src/run_store/artifact_admission.rs`
- `crates/storages/stream-store-postgres/src/schema.rs`
- Existing tests in `crates/kernel/runtime/src/tests.rs`, `crates/kernel/store/tests`, and
  `crates/storages/stream-store-postgres/src/run_store/tests.rs`.

### New Or Changed Public Types And Interfaces

- Replace old `mfm_events::v1::FactRecorded` fields with:
  - `FactRecordedPayload { claim: FactClaim }`, or equivalent versioned event shape.
- `FactClaim` includes:
  - `visibility`
  - `fact_descriptor_hash`
  - `subject`
  - `observed_at`
  - optional request evidence
  - response `ArtifactEvidenceRef`
  - producer provenance populated by runtime/certified attempt context
- Runtime recorder API:
  - replace `fact_recorded(fact_key, request, response, binding)` with
    `record_fact<T: MfmFactType>(FactRecordInput<T>)`
  - ensure response artifact bytes are staged from `T::Response` and cannot drift from the response
    used for term extraction
  - keep staged handle return values until append assigns event ids and claim ids
- Store commit result mapping:
  - `StagedFactRecordHandle`
  - `FactRecordCommitResult`
  - `FactClaimId` populated from run id, stream seq, and ordinal after commit
- Projection structs:
  - `FactDescriptorProjection`
  - `FactIndexProjection`
  - `FactIndexTermProjection`
  - validation/rebuild reports

### Storage Or Projection Changes

Replace the old simple fact projection with:

- `fact_descriptor_index`
- `fact_index`
- `fact_index_terms`

Physical schema must preserve these logical columns:

- descriptor hash, artifact id, fact kind, descriptor schema id, subject schema id, response schema
  id, subject namespace hash, compatibility group, created-at/store coordinates
- fact claim id as `(source_run_id, source_seq, source_ordinal)`
- source event id, commit id, store commit order, recorded at, observed at
- audience and visibility scope
- fact kind, descriptor hash, subject namespace hash, fact key, subject material hash
- request schema/hash nullable
- response schema/hash, response artifact id, artifact evidence hash
- capability/adapter provenance
- terms keyed by `fact_claim_id`, descriptor hash, field id, and one typed value column

Append projection rules:

- If descriptor validation fails, append fails.
- If descriptor bytes are unavailable, append fails.
- If descriptor hash is not certified for the producing node, append fails.
- If subject derivation or field extraction fails, append fails.
- If response artifact role/schema/hash does not match, append fails.
- If an indexed fact cannot project index and terms in the same transaction, append fails.
- `RunPrivate` facts admit descriptor and response authority but do not insert `fact_index` or terms.

Rebuild rules:

- Truncate and repopulate descriptor/index/term projections from run stream and artifact authority.
- Run validation after rebuild.
- Treat mismatches as deterministic validation errors with redacted diagnostics.

### Tests And Acceptance Criteria

- Old-shape `FactRecorded` construction fails to compile or fails deserialization on production
  paths after the reset.
- Runtime tests prove:
  - record helper derives subject material and response artifact from the same typed fact value
  - caller cannot supply a precomputed `FactKey`
  - producer provenance comes from runtime context, not caller input
  - post-commit mapping returns `FactClaimId` for fact records only
- Store contract tests prove:
  - append rejects missing descriptor artifact
  - append rejects store-admitted but uncertified descriptor
  - append rejects wrong descriptor hash
  - append rejects mismatched subject material hash
  - append rejects response artifact mismatch
  - append rejects required field extraction failure
  - append rejects multi-claim response artifact in v1
  - append of `RunPrivate` fact creates no index or term rows
  - append of indexed fact creates descriptor, index, and term rows atomically
  - failed projection rolls back event and artifact admissions
  - idempotent retry returns the same committed result and does not duplicate projections
- Postgres tests prove:
  - schema authority validates new tables, indexes, triggers, and absence of stale retired tables
  - projection rows have foreign keys back to authoritative run events/artifacts
  - term predicates join on `FactClaimId`, not `FactKey`
  - rebuild reproduces rows byte-for-byte or reports deterministic differences
- Security tests prove:
  - `RunPrivate` facts do not leak into fact tables
  - subject material is not available through public artifact/read APIs
  - fact append errors redact artifact bytes, endpoints, auth, and secret-like values
- Focused verification:
  - `cargo test -p mfm-events`
  - `cargo test -p mfm-store --test commit_contract --features test-support`
  - `cargo test -p mfm-runtime`
  - `cargo test -p mfm-stream-store-postgres`
  - `cargo test -p mfm-integration-tests --test cargo_metadata_contract`

### Dependencies On Previous Gates

- Gate 1 facts kernel types and extraction.
- Gate 2 certified descriptor emission and allow-list authority.

### Risks And Failure Modes

- Partial projection would make indexed facts visible inconsistently. Keep projection insertion in
  the same append transaction.
- `FactKey` could be incorrectly reused as a term join key. Tests and query planner must join
  predicates by `FactClaimId`.
- Descriptor bytes might be fetched from current Rust code during rebuild. Rebuild must load
  retained descriptor artifacts.
- The reset can break many existing proof/runtime tests. Rewrite tests to the new contract rather
  than adding compatibility shims.
- Public schema rows or diagnostics could expose canonical subject material or internal refs.

### Explicitly Out Of Scope

- Fact query compiler and public query execution.
- Receipt authentication.
- CLI/REST fact commands.
- Collector cycle implementation.
- Checkpoint conflict/lease authority.

## Gate 4: Reusable Query Compiler And Replay Evidence

### Objective

Implement descriptor-scoped fact query compilation, store-authenticated query receipts, and private
replay evidence so live fact reads can be replayed without asking the current store for latest data.

### Crates And Modules Likely Touched

- `crates/kernel/facts` or a later split `crates/kernel/facts-query`
- `crates/kernel/store`
- `crates/kernel/events` for `ArtifactRole::FactQueryEvidence`
- `crates/kernel/runtime` for query evidence recorder/staging
- `crates/kernel/replay` for replay verification interfaces
- `crates/storages/stream-store-postgres` for query execution and receipt signing
- `crates/app` for evidence-only query service assembly

### New Or Changed Public Types And Interfaces

- `FactQueryInput`
- `FactDescriptorSelector`
- `ResolvedFactDescriptor`
- `FactQueryCompiler`
- `StoreFactQueryExecutor`
- `CanonicalFactQueryPlan`
- `FactQueryScope`
- `FactOrdering`
- `FactFilterExpression`
- `FactQueryReceipt`
- `StoreReadFrontier`
- `DescriptorCatalogWatermark`
- `FactProjectionGeneration`
- `StoreCommitWatermark`
- `StoreReceiptAuthentication`
- `StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1`
- `FactSelectionEvidence`
- `FactQueryEvidence`
- `FactQueryEvidenceVerifier`
- `FactRecorder::record_fact_query_evidence`
- `ArtifactRole::FactQueryEvidence`

Keep ownership split: `mfm-facts` may own descriptor resolution inputs, validation, and canonical
query compilation, but query execution and receipt signing live in `mfm-store`,
`crates/storages/stream-store-postgres`, and app assembly. Do not put SQL, store reads, receipt
signing, or projection access behind a facts-kernel service implementation.

The public compiler input can be ergonomic, but the canonical plan must resolve exactly one
descriptor in v1 and must bind:

- store scope
- query scope
- query compiler version
- canonicalizer version
- resolved descriptor hash
- scope decision evidence
- canonical query bytes and hash
- ordering policy
- limit

### Storage Or Projection Changes

- Query execution reads `fact_descriptor_index`, `fact_index`, and `fact_index_terms`.
- Add store metadata for receipt authentication trust root and key id if not already available.
- Add projection generation/rebuild id and descriptor catalog watermark.
- Add retention edges from `FactQueryEvidence` artifacts to:
  - descriptor artifacts
  - returned source events
  - subject material authority
  - response artifacts
  - returned field summaries used by selection
  - receipt authentication material
- Do not insert query evidence artifacts into `fact_index`.

### Tests And Acceptance Criteria

- Query compiler tests:
  - kind-only selector resolves exactly one descriptor or returns stable ambiguity/unknown errors
  - zero-descriptor and multi-descriptor v1 resolution fails
  - public scope compiles only `audience = Platform`
  - internal scope can compile `audience = Control` only through internal capability/service
  - operators are rejected when descriptor field type does not allow them
  - hidden fields are rejected for public filters/orderings
  - query-only fields filter/order but are omitted from public summaries
  - every "latest" helper requires an explicit temporal or progression ordering
  - result ranking is not labeled latest unless descriptor ordering is a progression/time axis
- Store receipt tests:
  - receipt hash excludes `store_receipt_hash` and authentication fields
  - receipt authentication binds canonical plan hash, scope, ordering, limit, returned refs,
    summaries digest, cardinality, result-set digest, and frontier
  - bad signature, unknown key id, unsupported scheme, missing auth, wrong trust root, and tampered
    receipt all fail closed
  - frontier mismatch and projection generation mismatch fail closed
- Replay tests:
  - empty result replay uses authenticated receipt and frontier without live store reads
  - limited result replay uses authenticated receipt and retained refs without live store reads
  - selected indices must be unique, in bounds, and canonical for the selection policy
  - selected summaries digest must match when summaries are used
  - missing retained descriptor/source/response/query evidence artifacts fail replay
  - replay services do not construct live transports, signer providers, keystores, or runtime
    source config
- Postgres query tests:
  - predicates over subject, result, and metadata terms join by `FactClaimId`
  - audience/scope filtering excludes `Control` from public queries
  - exact-ref lookup for non-public facts returns the same redacted not-found class as unknown refs
  - ordering tie-breakers are deterministic by store-owned coordinates
- Focused verification:
  - `cargo test -p mfm-facts`
  - `cargo test -p mfm-store`
  - `cargo test -p mfm-replay`
  - `cargo test -p mfm-runtime`
  - `cargo test -p mfm-stream-store-postgres`
  - `cargo test -p mfm-app`

### Dependencies On Previous Gates

- Gate 1 canonical query/evidence types.
- Gate 3 fact projections and retained authority.

### Risks And Failure Modes

- CLI/REST could implement their own query parser/compiler. Keep all normalization in the shared
  query service and expose thin input DTOs.
- Receipt authentication may become optional for convenience. V1 has no unauthenticated mode.
- Empty results could be treated as cryptographic absence proofs. V1 explicitly uses authenticated
  receipt plus semantic frontier only.
- Public exact-ref lookup could leak non-public existence. Error classes and timing should fail
  closed.
- Query evidence retention can be incomplete if only returned refs are retained. Include descriptor
  and artifact authority needed to replay those refs.

### Explicitly Out Of Scope

- Multi-descriptor query execution.
- Cryptographic absence proofs.
- Remote/federated receipt verification.
- Store signing-key rotation policy beyond the minimal local trust-root contract.
- Public pagination internals beyond opaque cursors.
- Collector checkpoint monotonicity enforcement.

## Gate 5: Public CLI/REST Surfaces

### Objective

Expose descriptor-scoped, kind-first public fact queries through CLI and REST while preserving the
transport-only boundary. Public output must use `PublicFactRef` and descriptor-approved fields only.

### Crates And Modules Likely Touched

- `crates/app/src/lib.rs`
- `crates/app/README.md`
- `bin/cli/src/commands/mod.rs`
- likely new `bin/cli/src/commands/facts.rs` or `bin/cli/src/commands/facts/*`
- `bin/cli/src/presentation`
- `bin/cli/README.md`
- `bin/rest-api/src/lib.rs`
- `bin/rest-api/README.md`
- CLI/REST tests under `bin/cli/tests` and `bin/rest-api/src/tests.rs`

### New Or Changed Public Types And Interfaces

App evidence-only service:

- `FactCatalogService`
- `FactPublicQueryService`
- `FactPublicRefResolver`
- DTOs:
  - `PublicFactKindSummary`
  - `PublicFactDescriptorSummary`
  - `PublicFactExplain`
  - `PublicFactQueryRequest`
  - `PublicFactQueryPage`
  - `PublicFactRef`
  - `PublicFactFieldValue`
  - redacted stable error codes

CLI commands:

- `mfm facts kinds`
- `mfm facts describe <kind>`
- `mfm facts explain <kind>`
- `mfm facts latest <kind> --shape ... --order ...`
- `mfm facts history <kind> --shape ... --order ...`
- `mfm facts top <kind> --shape ... --order ...`
- `mfm facts show <public-ref>`
- `mfm facts query --kind ... --shape ... --where ... --order ...`

REST routes:

- `GET /v1/facts/kinds`
- `GET /v1/facts/kinds/{kind}`
- `GET /v1/facts/{kind}/latest`
- `GET /v1/facts/{kind}`
- `GET /v1/facts/ref/{public_ref}`

### Storage Or Projection Changes

- None beyond Gate 4.
- Public opaque cursors may require app-level cursor encoding. Do not expose raw DB order,
  transaction ids, run ids, event ids, artifact ids, or projection generations.

### Tests And Acceptance Criteria

- CLI tests prove:
  - global `--output-format` and `MFM_OUTPUT_FORMAT` are respected
  - JSON output schemas are stable and documented
  - kind-only query fails with stable ambiguity error when multiple descriptors match
  - `latest` requires explicit ordering
  - `top` and `history` use descriptor ordering policies
  - public output omits internal refs, artifact ids, evidence hashes, subject hashes, response
    hashes, raw run/event coordinates, and response artifacts
  - `QueryOnly` fields filter/order but are omitted from returned summaries
  - unknown and non-public exact refs return the same redacted not-found class
- REST tests prove the same public/security contracts for HTTP status, JSON body shape, and errors.
- App tests prove evidence-only public fact services do not construct live transports, signer
  providers, keystores, or runtime source config.
- Descriptor discovery tests prove `Control` descriptors and `RunPrivate` facts are not disclosed
  through kinds, describe, explain, count, ambiguity, or exact-ref lookup.
- Docs update:
  - `bin/cli/README.md`
  - `bin/rest-api/README.md`
  - possibly `docs/persisted-public-surfaces.md`
- Focused verification:
  - `cargo test -p mfm-app`
  - `cargo test -p mfm --test cli_tests`
  - `cargo test -p mfm --test json_output_integration`
  - `cargo test -p mfm-rest-api`

### Dependencies On Previous Gates

- Gate 4 shared query compiler, query receipt, and public DTO boundary.

### Risks And Failure Modes

- Public commands could expose implementation language such as adapter names, artifact ids, or
  internal refs. Keep output domain-oriented and descriptor-filtered.
- Descriptor discovery could reveal non-public facts. Public discovery must filter to Platform
  audience and exposure policy before returning or forming ambiguity errors.
- CLI/REST might depend directly on SQLx or Postgres internals. They should call app services.
- Public refs might become stable cross-scope correlation handles. Make them opaque, scope-checked,
  and resolvable only through the fact service.

### Explicitly Out Of Scope

- Public mutation APIs for facts.
- Public query of `Control` audience.
- Raw artifact or subject material reads.
- Multi-descriptor public queries.
- Full named-scope authorization beyond the default v1 scope.
- Background collector scheduling.

## Gate 6: First Collector Workflow

### Objective

Prove the RFC model with one recurring bounded collector workflow that records platform-visible
domain facts and internal checkpoint facts using ordinary operations, states, adapters, transports,
runtime events, and query evidence. Do not add collector-specific runtime semantics.

### Proposed First Workflow

Preferred target: a narrow Bitcoin chain-head collector cycle using the existing redacted Bitcoin
JSON-RPC HTTP transport as the live backend.

The cycle records:

- `chain.head` Platform facts:
  - subject: chain, network, source identity if it changes what is observed, head kind
  - result: block height, block hash, provider time if available, observed source status
  - metadata: `recorded_at`, `observed_at`, store order
- `collector.checkpoint` Control facts:
  - subject: collector kind, non-secret semantic source identity, default scope, stream partition,
    chain/network
  - result: high-watermark block height/hash, predecessor checkpoint ref/hash, finality policy
  - metadata: `recorded_at`, store order

Simplification option: if Bitcoin capability/state work is too large for the first proving slice,
use a deterministic proof collector cycle first, then follow immediately with the Bitcoin chain-head
collector. The deterministic slice must still use real `MfmFactType` descriptors, Platform facts,
Control checkpoint facts, and pinned checkpoint query evidence. It must not become the only proof of
external observation behavior.

### Crates And Modules Likely Touched

For the Bitcoin target:

- new `crates/btc-capabilities` for typed read capability contracts
- new or existing transport wiring around `crates/collectors/btc-jsonrpc-http`
- new `crates/states/btc` for bounded read, normalize, record, and checkpoint state contracts;
  choose a collector-named state crate only if the semantics cannot be reused by non-collector
  Bitcoin workflows
- new `crates/adapters/btc-jsonrpc` for runner binding from state intent to Bitcoin capability
  backend
- new `crates/ops/btc-chain-head-collector-op` for deterministic cycle topology
- `crates/app` registration for certification descriptors and runner factories
- `bin/cli` or existing run start entry-point registration only if this collector gets a public
  start command in v1
- integration tests under `tests/integration` or crate-local tests with mocked transport/replay

### New Or Changed Public Types And Interfaces

- `BtcChainHeadFact: MfmFactType`
- `BtcChainHeadSubject: MfmValue`
- `BtcChainHeadResponse: MfmValue`
- `CollectorCheckpointFact: MfmFactType`
- `CollectorCheckpointSubject: MfmValue`
- `CollectorCheckpointResponse: MfmValue`
- `BtcChainHeadReadCapability`
- `BtcChainHeadRequest`
- `BtcChainHeadEvidence`
- `LoadCollectorCheckpointState`
- `ObserveBtcChainHeadState`
- `RecordBtcChainHeadFactState`, if separated from observation
- `RecordCollectorCheckpointState`, if separated from observation
- `BtcChainHeadCollectorConfig`
- `btc_chain_head_collector_cycle_program_draft`

Keep state names reusable and domain-specific. Keep the operation name workflow-specific.

### Storage Or Projection Changes

- No new store primitives.
- Uses `FactRecorded` with `Platform` and `Control` visibility.
- Uses `FactQueryEvidence` for checkpoint selection.
- Retention follows fact authority and query evidence edges.
- No checkpoint monotonicity or exclusive advancement in v1 unless a separate generic lease or
  compare-and-append authority is implemented first.

### Tests And Acceptance Criteria

- Operation tests prove:
  - topology is deterministic from typed config
  - descriptor allow-lists include `chain.head` and `collector.checkpoint`
  - operation does not depend on transport, signer, app, binary, or storage crates
- State tests prove:
  - waits/polls are bounded by certified/configured policy
  - source errors are redacted
  - no RPC URL, auth header, username, password, process-local route, or runtime source ref enters
    typed fact material; source identity in facts is non-secret semantic provenance only
  - response values contain no floats; integer-scaled or strings only
- Adapter/transport tests prove:
  - live backend uses explicit capability only
  - replay verifier uses recorded evidence only
  - chain/source identity mismatches fail with redacted evidence
- Runtime/store integration tests prove:
  - cycle records Platform `chain.head` fact and Control checkpoint fact
  - next cycle queries latest Control checkpoint through the shared query service
  - checkpoint query is pinned as `FactQueryEvidence`
  - crash/retry keeps committed facts/checkpoints and retries only uncommitted observations
  - replay does not call live Bitcoin RPC or current fact index
- Public API tests prove:
  - `chain.head` Platform facts appear in public facts queries
  - `collector.checkpoint` Control facts do not appear in public APIs
- Focused verification:
  - `cargo test -p mfm-btc-capabilities`
  - `cargo test -p mfm-states-btc-collector`
  - `cargo test -p mfm-adapters-btc-jsonrpc`
  - `cargo test -p mfm-op-btc-chain-head-collector`
  - `cargo test -p mfm-app`
  - targeted Postgres parity test with managed or manually started Postgres

### Dependencies On Previous Gates

- Gate 1 fact types/canonicalization.
- Gate 2 descriptor derive and certified allow-lists.
- Gate 3 append projection.
- Gate 4 query evidence and Control fact querying.
- Gate 5 only if the collector's Platform facts must be visible through public CLI/REST in the same
  PR; otherwise Gate 5 can validate generic surfaces before the collector lands.

### Risks And Failure Modes

- First collector could accidentally introduce recurring daemon semantics. Keep recurrence outside
  durable semantics as launcher/backoff policy.
- A long wait could become a forever-open attempt. Bound by event, batch size, timeout, source
  unavailable, backoff, cancellation, or lease loss.
- Checkpoint facts could imply exclusive advancement. RFC defers conflict policy; document duplicate
  checkpoint behavior unless a separate authority is added.
- Bitcoin transport path currently lives under `crates/collectors/...` while categorized as
  `transport`. Do not let the path name justify putting workflow topology into the transport.
- Process-local RPC URL/auth could leak into facts. Add redaction and persisted-surface tests.

### Explicitly Out Of Scope

- Generic collector scheduler or daemon.
- Exclusive checkpoint leases or compare-and-append authority.
- Multi-source collector fanout.
- Wallet balance scans or UTXO collector if the first target is chain head.
- Public write/control APIs for collectors.
- Named-scope authorization beyond default v1 scope.

## Canonicalization And Hashing Test Strategy

Add a dedicated golden suite before store work depends on the hashes.

- Store golden inputs as small, readable Rust fixtures rather than opaque JSON snapshots when
  possible.
- Golden coverage:
  - descriptor canonical bytes and hash
  - descriptor hash exclusion from descriptor canonical bytes
  - subject namespace canonical bytes and hash
  - subject material canonical bytes and hash
  - `FactKey`
  - `FactClaimId`
  - canonical query plan bytes/hash
  - receipt body hash
  - result-set digest
  - query evidence artifact bytes/hash
- Perturbation tests:
  - reorder descriptor fields in input, assert canonical order by `field_id`
  - reorder subject material values, assert canonical order by `field_id`
  - change result field only, assert `FactKey` unchanged
  - change ordering policy only, assert descriptor hash changes but `FactKey` unchanged
  - add descriptor exposure policy change, assert descriptor hash changes but `FactKey` unchanged
  - include float-like values where forbidden, assert validation fails
  - include numeric decimal strings, assert order-preserving encoding or database numeric behavior
    is explicit
- Cross-crate tests:
  - derive-emitted descriptor equals hand-constructed expected descriptor
  - persisted descriptor artifact bytes re-verify without calling domain code
  - projection rebuild from artifacts reproduces term rows

## Privacy And Security Checks

Required checks across gates:

- No secret-bearing value may enter fact subject material, response artifacts, descriptor bytes,
  events, public outputs, diagnostics, fixtures, snapshots, or query evidence.
- `FactVisibility` is mandatory for every fact recording call.
- `RunPrivate` facts never insert `fact_index` or `fact_index_terms` rows.
- `Control` facts are absent from public kinds, describe, explain, query, counts, ambiguity errors,
  direct ref lookup, and public summaries.
- Public fact output never includes:
  - raw `FactClaimId`
  - source run id
  - source sequence
  - event ordinal
  - event id
  - artifact id
  - artifact evidence hash
  - subject material
  - subject material hash
  - response artifact
  - request/response hash
  - capability routing details
  - RPC URL/auth/source runtime config
- Public `QueryOnly` fields are treated as indirectly exposed. Do not mark sensitive values as
  query-only.
- Receipt signatures/MACs must not leak signing key material in errors or debug output.
- Store/app/bin errors must redact artifacts, endpoints, authorization material, passwords,
  keystore paths, signer material, and raw response bodies.
- Add persisted-public-surface inventory updates when CLI/REST fact output lands.

## Replay And Retention Checks

Replay verification must prove:

- Stored certified spec and certificate verify against the compiled registry before replay.
- Descriptor artifacts are loaded from retained artifact authority, not rebuilt from current Rust
  descriptors.
- `FactRecorded` events validate against the descriptor allowed by the producing certified node.
- Response artifacts match event evidence and descriptor response schema.
- Subject material and `FactKey` recompute from retained canonical bytes.
- Query evidence replay verifies:
  - canonical plan hash
  - compiler/canonicalizer versions
  - descriptor resolution
  - scope decision evidence
  - ordering and limit
  - receipt authentication
  - read frontier
  - result-set digest
  - returned refs and summaries
  - selection policy and indices
  - retained source fact authority
- Replay never constructs:
  - live transports
  - signer providers
  - keystores
  - live runtime config
  - current certification registry as outcome policy for already-certified facts
- Retention keeps source events, descriptor bytes, subject material, response artifacts, artifact
  admissions, query evidence artifacts, returned refs, receipts, selection evidence, and trust root
  material needed to verify retained evidence.
- Removing facts from live query surfaces is allowed only after proving no retained query evidence
  depends on their authority, or after retaining the authority despite removing projections.

## Postgres Validation And Rebuild Plan

Postgres remains the production store implementation.

1. Replace the baseline schema directly because the project is pre-production and the RFC declares a
   destructive reset.
2. Add tables and indexes for descriptor index, fact index, and fact terms.
3. Add database-level constraints for append-only behavior, uniqueness, foreign keys, typed value
   exclusivity, byte/numeric bounds, and audience/scope enum values.
4. Keep typed extraction in Rust append/rebuild code, not SQL triggers.
5. Insert descriptor/index/term rows inside the same transaction that admits artifacts and inserts
   run events.
6. Ensure failed projection aborts the transaction.
7. Add schema authority checks for required tables, indexes, triggers, constraints, and absence of
   retired stale fact projection objects.
8. Add an internal/test-only rebuild store method by default. Expose an operator command only after
   a deliberate CLI/REST maintenance contract is designed. The internal method should:
   - truncate fact projection tables
   - fold committed run events in store order
   - load descriptor and response artifacts from retained authority
   - re-extract terms using kernel extractor
   - validate every rebuilt row
   - report deterministic diff counts and first redacted mismatch
9. Add validation that runs without truncating and checks existing rows against authority.
10. Include idempotent append retry tests that do not rely on retry to repair projection rows.
11. Add query execution tests over realistic subject/result/metadata predicates and orderings.

## CLI/REST Contract Impacts

CLI and REST changes are public contracts and must update docs in the same PR.

- New command group: `mfm facts ...`
- New REST route group: `/v1/facts/...`
- JSON outputs must be stable and documented.
- Public query inputs are kind-first and descriptor-scoped; `--shape` or descriptor hash is required
  when kind is ambiguous.
- `latest` is `limit = 1` plus explicit descriptor ordering. Do not implement implicit mutable
  latest rows.
- Public DTOs use `PublicFactRef`, not `InternalFactRef`.
- Public refs are opaque and scope-checked.
- Error codes should distinguish invalid public input from redacted not-found/unauthorized without
  revealing non-public existence.
- Evidence-only fact reads in app/CLI/REST must not parse live runtime config.
- Keep output examples in docs free of secrets, internal ids, raw event coordinates, and artifact
  evidence.

## Likely PR Breakdown

Suggested PR sequence:

1. Add `mfm-facts` canonical types, validation, extraction grammar, and goldens.
2. Add `MfmFactType` derive and descriptor emission with compile-fail tests.
3. Add certified spec node descriptor allow-lists and certification hash tests.
4. Add `ArtifactRole::FactDescriptor` and descriptor artifact admission plumbing.
5. Reset `FactRecorded` event shape in events/store/runtime tests without Postgres projection.
6. Add Postgres descriptor/index/term schema and append projection.
7. Add projection validation and rebuild.
8. Add `ArtifactRole::FactQueryEvidence` and query evidence artifact recording.
9. Add reusable query compiler and canonical plan tests.
10. Add Postgres query execution, receipt authentication, and replay verification.
11. Add app public/internal fact query services and retention edges.
12. Add CLI `mfm facts` commands and docs.
13. Add REST `/v1/facts` routes and docs.
14. Add the first collector capability/state/adapter/op vertical slice.
15. Run final Nix gates and update any repo maps/docs affected by crate additions.

Some adjacent PRs can merge if they stay reviewable, but do not combine infrastructure gates with
the first collector workflow.

## Unresolved Engineering Questions

These are not blocking ambiguities in the RFC, but implementation should answer them explicitly:

1. Should `MfmFactType` live in `mfm-program`, `mfm-values`, or `mfm-facts`? The owner must avoid
   cycles while keeping derive ergonomics reasonable.
2. Should `mfm-events` depend on `mfm-facts`, or should event structs define versioned wrappers that
   convert to facts kernel types? Prefer the option that keeps the kernel DAG simplest.
3. What exact artifact role contract should `FactDescriptor` and `FactQueryEvidence` use for schema,
   semantic type, producer scope, staging, retention, and same-commit policy?
4. How should canonical subject material bytes be retained and protected from generic public reads
   while still being available for rebuild?
5. Where should the local store receipt signing key be configured and stored for v1 so replay can
   verify against the local trust root without exposing key material?
6. What is the minimal `DescriptorCatalogWatermark` and `FactProjectionGeneration` representation
   that proves receipt frontier semantics without overbuilding catalog machinery?
7. Should the first collector use Bitcoin chain head as planned, or should a deterministic proof
   collector land first as a smaller proving slice?
8. Is the existing `crates/collectors/btc-jsonrpc-http` path acceptable while categorized as a
   transport, or should it be renamed before first collector work to reduce architecture confusion?
9. Which exact public fact error codes should be added to CLI/REST contracts for unknown kind,
   ambiguous descriptor, invalid field, forbidden field, invalid ordering, invalid public ref, and
   redacted not found?
10. Should any focused Postgres command expose projection rebuild/validation for operators? Default
    v1 posture is internal/test-only until an operator maintenance contract is deliberately designed.

## Architect Critique Outcome

Architect review found no P0/P1 architecture, privacy, replay, or implementation-readiness issues.
Minor P2 notes were folded into this plan: public DTO ownership is app/public-surface owned, query
execution stays out of the facts kernel, reusable state crate naming is preferred, checkpoint source
identity is explicitly non-secret semantic provenance, and projection rebuild remains internal or
test-only by default.

## Final Readiness Criteria

The RFC implementation is ready for merge only when:

- every gate's acceptance criteria are satisfied or intentionally deferred in docs with RFC-aligned
  scope;
- no old-shape `FactRecorded` production path remains;
- public APIs cannot reveal `Control` or `RunPrivate` facts;
- replay of fact queries uses pinned evidence and retained artifacts only;
- Postgres projection rebuild validates from authoritative run stream and artifacts;
- first collector records facts and checkpoints through ordinary certified workflow primitives;
- `nix run .#check`, `nix run .#test`, and `nix run .#test-db` pass before commit;
- `nix run .#ci` passes before final merge-readiness validation.
