# Implementation Plan: RFC Collectors And Platform Facts

Status: draft implementation plan for `docs/RFC_COLLECTORS.md`.

This plan is intentionally concrete, but it should not force unnecessary
framework expansion. Prefer the smallest implementation that preserves the RFC,
`docs/design.md`, and `docs/architecture.md` invariants. If implementation
finds a simpler path that keeps the same authority, replay, privacy, and store
contracts, take it and update this plan in the same change.

No code is implemented by this document.

## Implementation Principles

- Collectors are ordinary recurring certified workflows. Do not add a collector
  runtime, daemon primitive, or collector-specific event family.
- `FactRecorded` is the only event for platform knowledge. Do not add
  `CollectedFactRecorded`.
- Every fact uses explicit `FactVisibility`. There is no default public
  publication.
- `FactDescriptor` is durable certified authority for fact shape, field
  extraction, field ids, ordering, operators, exposure, units, scales, and
  subject identity.
- Descriptor bytes, descriptor hash, certified node allow-list, event payload,
  admitted artifacts, and append-only run stream coordinates are authority.
  Projections and public DTOs are rebuildable views.
- `FactKey` is derived only from fact kind plus descriptor-declared subject
  material. Result and metadata fields never affect subject identity.
- Field extraction is declarative and kernel-owned. Append, rebuild, and query
  planning must not call arbitrary domain crate code.
- `FactClaimId` identifies one recorded claim and is derived from run stream
  coordinates for every `FactRecorded`, including `RunPrivate` facts.
- Indexed fact joins use `FactClaimId`, not `FactKey`. `FactKey` groups claims
  about the same subject only.
- Live fact queries are not replay authority. Consuming runs must pin
  `FactQueryEvidence` as a private `ArtifactReferenced` event with
  `ArtifactRole::FactQueryEvidence`.
- CLI and REST decode and render only. Query compilation, descriptor
  resolution, scope checks, field validation, and public DTO filtering belong
  in reusable app/kernel services.
- Public APIs must not expose internal refs, artifact ids, artifact evidence
  hashes, canonical subject material, subject hashes, response artifacts, raw
  run/event coordinates, or capability routing details.
- Facts are not secret storage. Visibility and descriptor exposure do not make
  secret-bearing data acceptable.
- This is a destructive development reset. Do not migrate old opaque fact
  events, old `FactKey` shapes, or old projections.

## Proposed Crate Ownership

| Area | Proposed owner | Notes |
| --- | --- | --- |
| Fact contracts and canonical algorithms | new `crates/kernel/facts` | Owns `FactDescriptor`, field descriptors, extraction grammar, visibility, `FactKey`, `FactClaimId`, canonicalization, and validation. Keep Gate 1 narrow; add query plan/receipt/evidence and public DTO types only in later gates. |
| Fact query capability contract | new `crates/fact-capabilities` or an equally narrow capability-contract crate | Owns state-facing read capability specs, request/response evidence types, and redacted errors for indexed fact queries. It may depend on facts kernel types, but not on app, runtime, Postgres, or live store implementations. |
| Fact query compiler and evidence | `crates/kernel/facts` query module or new `crates/kernel/facts-query` | Gate 4 owns canonical query plans, receipts, selection evidence, query evidence, compiler versioning, and replay validation. Split from `crates/kernel/facts` if it keeps dependency direction or review scope cleaner. |
| Event payload reset and artifact roles | `crates/kernel/events` | Owns new normalized `FactRecordedPayload`/`FactClaim` event shape and adds `ArtifactRole::FactDescriptor` plus `ArtifactRole::FactQueryEvidence`. |
| Typed value integration | `crates/kernel/values` | Exposes canonical value/path support needed by descriptor validation and kernel extraction without leaking serde layout as authority. |
| Fact authoring derive | `crates/kernel/program`, `crates/kernel/program-derive` | Owns `MfmFactType`, derive attributes, descriptor generation, subject/response accessors, and derive UI tests. |
| Certified descriptor allow-list | `crates/kernel/spec`, `crates/kernel/certify`, `crates/kernel/program` | Certified specs carry per-producing-node fact descriptor allow-lists. Certification validates descriptor identity and hash-defining inclusion. |
| Runtime recording boundary | `crates/kernel/runtime` | Replaces raw fact payload helpers with a typed `FactRecorder`/runner-kit path that derives fact claim fields from one `T: MfmFactType`. |
| Store contracts | `crates/kernel/store` | Defines append validation, projection/rebuild traits, internal query contracts, retention requirements, and post-commit staged-id to event-id/claim-id mapping. |
| Postgres implementation | `crates/storages/stream-store-postgres` | Owns baseline schema reset, descriptor artifact admission checks, `fact_descriptor_index`, `fact_index`, `fact_index_terms`, append projection, query execution, receipt authentication, validation, and rebuild. |
| App services | `crates/app` | Owns evidence-only fact read services, live/internal control query capability wiring, public ref token mint/resolve, and shared CLI/REST service facade. |
| Public surfaces | `bin/cli`, `bin/rest-api` | Add `mfm facts ...` commands and REST routes that call app services only. Update `bin/cli/README.md` and `bin/rest-api/README.md`. |
| First collector domain | `crates/ops/*`, `crates/states/*`, `crates/adapters/*`, `crates/transports/*` | Build the collector as a normal operation/state/adapter/transport workflow. Do not put reusable protocol IO into a workflow crate. |

## Migration And Reset Strategy

- Treat the RFC as a persisted contract reset while MFM is pre-production.
- Replace the existing `FactRecorded` event shape rather than supporting both
  old and new payloads.
- Replace the current old fact projection keyed by
  `(NodeId, AttemptId, events::FactKey)` with the RFC model:
  `FactClaimId`, descriptor index, fact index, and typed term rows.
- Update `crates/storages/stream-store-postgres/migrations/0001_run_store.sql`
  directly as the new baseline. Do not add compatibility migrations for old
  fact rows.
- Update schema contract/version markers if the store uses one to reject old
  databases.
- Remove or rewrite tests and fixtures that construct old-shape facts manually.
  Tests should fail if old `FactRecorded` JSON can still decode or append.
- All existing fact producers must move to typed `MfmFactType` publication in
  the same gate that removes the old event shape, or be explicitly disabled
  until converted.
- Development users must recreate stores/artifacts after the reset. No legacy
  fact event, artifact, or projection migration is planned.

## Gate 1: Facts Kernel Types And Canonical Contracts

### Objective

Create the domain-free facts kernel contract and canonical algorithms required
by all later gates: descriptors, visibility, field extraction grammar,
subject-only `FactKey`, claim identity, internal/public ref shapes, validation,
and canonical hash test vectors.

### Crates And Modules Likely Touched

- Add `crates/kernel/facts` and workspace metadata.
- Touch `Cargo.toml` and cargo metadata architecture tests.
- Touch `crates/kernel/ids` only if new checked identity wrappers are better
  owned there. Prefer facts-owned wrappers first to keep the reset local.
- Touch `crates/kernel/values` for canonical value path support if existing
  descriptor shapes cannot expose a bounded `CanonicalValuePath` API.
- Add focused tests under `crates/kernel/facts/tests`.

### New Or Changed Public Types And Interfaces

- `FactKind`
- `FactCompatibilityGroup`
- `FactDescriptor`
- `FactFieldDescriptor`
- `FactFieldId`
- `FactFieldPath`
- `FactFieldAccessor::{SubjectPath, ResponsePath, Metadata}`
- `FactMetadataField`
- `FactFieldValueType`
- `FactCanonicalScalar`
- `FactQueryOperator`
- `FactFieldExposure::{Returnable, QueryOnly, Hidden}`
- `FactUnit`, `FactScale`
- `FactOrderingDescriptor`, `FactOrderingTerm`, `FactOrderingName`
- `SortDirection`, `NullOrdering`
- `FactVisibility::{RunPrivate, Indexed { audience, scope }}`
- `FactAudience::{Control, Platform}`
- `FactVisibilityScope::Default`
- `FactSubjectNamespaceV1`
- `FactSubjectNamespaceFieldV1`
- `FactSubjectMaterialV1`
- `FactSubjectValueV1`
- `FactKey`
- `FactClaimId`
- `FactDescriptorError`, `FactExtractionError`, `FactKeyError`
- Functions or small sealed helpers for:
  - descriptor validation
  - descriptor canonical bytes and descriptor hash
  - subject namespace derivation
  - subject material derivation
  - `FactKey` derivation
  - `FactClaimId` derivation from `(RunId, StreamSeq, EventOrdinal)`
  - descriptor-declared extraction over canonical subject/response values

### Storage And Projection Changes

None in this gate. It defines contracts only. No event reset, Postgres table,
append path, or query service should land here unless needed for compile-time
type ownership.

### Tests And Acceptance Criteria

- Golden canonical bytes and hashes for:
  - `FactDescriptor`
  - `FactSubjectNamespaceV1`
  - `FactSubjectMaterialV1`
  - `FactKey`
  - `FactClaimId`
- Descriptor hash goldens prove the descriptor hash is not embedded in the
  descriptor bytes being hashed.
- Compatible descriptor version tests prove `FactKey` stays stable when only
  result fields, metadata fields, paths, operators, exposure, ordering,
  descriptor schema id, response schema id, or compatibility metadata change.
- Incompatible descriptor tests prove `FactKey` changes when fact kind,
  subject field id, subject value type, unit, scale, or subject value changes.
- Validation rejects:
  - duplicate field ids
  - zero subject fields
  - optional subject fields
  - missing required scalar values
  - arrays, wildcards, slices, and repeated fields
  - floats in hashed structures
  - unsupported timestamps, decimals, digests, and numeric bounds
  - ordering over missing or non-sortable fields
  - operator/value type mismatches
  - unbounded scalar lengths or numeric precision
  - secret-marker strings in fact material where the facts kernel can detect
    them
- Extraction tests cover subject, result, and metadata sources without invoking
  domain crate code.
- Cargo metadata checks allow the new facts kernel crate only along the kernel
  dependency DAG.
- All new public items have rustdoc.
- `docs/design.md` and `docs/architecture.md` are updated if the new facts
  kernel changes documented crate ownership or canonical fact authority
  semantics.

### Dependencies On Previous Gates

None.

### Risks And Failure Modes

- Over-designing the extraction grammar into a general expression engine.
- Accidentally deriving identity from serde layout, descriptor paths, response
  fields, metadata fields, or descriptor hash.
- Allowing floats or unbounded values into canonical hashed material.
- Putting query execution or store implementation details in the facts kernel.
- Placing identity wrappers in `ids` too early and widening the blast radius.

### Explicitly Out Of Scope

- `MfmFactType` derive.
- Certified spec descriptor allow-lists.
- New event payloads.
- Postgres projection tables.
- Public CLI/REST.
- Collector workflows.
- Batched or multi-claim response artifacts.
- Multi-valued fields, arrays, geospatial, full-text, prefix, or custom domain
  operators.

## Gate 2: Certified Descriptor Emission

### Objective

Make descriptor-certified typed fact publication possible and make certified
specs declare which fact descriptors each producing node may emit.

### Crates And Modules Likely Touched

- `crates/kernel/program`
- `crates/kernel/program-derive`
- `crates/kernel/values`
- `crates/kernel/spec`
- `crates/kernel/certify`
- `crates/kernel/events` for `ArtifactRole::FactDescriptor`
- `crates/kernel/runtime` for runner/output validation hooks, if needed
- Existing op/state tests that register states emitting facts
- Cargo metadata and trybuild tests

### New Or Changed Public Types And Interfaces

- `MfmFactType: MfmValue`
  - associated `Subject: MfmValue`
  - associated `Response: MfmValue`
  - `descriptor() -> &'static FactDescriptor`
  - `subject(&self) -> &Self::Subject`
  - `response(&self) -> &Self::Response`
  - The static descriptor is an authoring helper only. Runtime append,
    rebuild, replay, and public reads must validate from retained canonical
    descriptor artifact bytes and the certified allow-list, not from the
    current Rust constructor.
- `#[derive(MfmFactType)]`
- Fact derive attributes for:
  - kind
  - schema
  - subject field ids and paths
  - response/result field ids and paths
  - ordering
  - exposure
  - units/scales
  - sortable/operator hints
- Certified spec material for node-level allowed fact descriptor hashes.
- Program builder APIs for a state descriptor to declare emitted fact
  descriptors. Prefer deriving this from typed state registration rather than
  requiring operation authors to hand-code descriptor hashes.
- Certification errors for missing, duplicate, invalid, or uncertified
  descriptor emission.
- Artifact role contract for `ArtifactRole::FactDescriptor`.

### Storage And Projection Changes

- No fact index tables yet.
- Descriptor canonical bytes become admissible framework artifacts by role.
- Store admission alone is not semantic authority. This gate should establish
  that a later append can verify both descriptor bytes and certified node
  allow-list.
- Persisted descriptor artifacts are the primary validation input for store,
  rebuild, replay, and query services. Any in-memory/static descriptor catalog
  is a discovery or authoring convenience, not replay authority.

### Tests And Acceptance Criteria

- A small domain fact derives `MfmFactType` successfully.
- Derive emits descriptor bytes with stable canonical hash.
- Derive emits subject and response accessors that cannot drift from the value
  staged as the response artifact in later gates.
- Descriptor validation tests parse retained canonical descriptor bytes and
  compare them with certified allow-list entries. The tests must not call the
  current `T::descriptor()` constructor for append/rebuild/replay authority.
- Compile-fail tests reject:
  - subject types with unannotated serialized fields
  - optional subject fields
  - unsupported scalar encodings
  - response fields outside the v1 extraction grammar
  - float fields in fact material
  - duplicate field ids
  - invalid operators/orderings/exposure declarations
- `ArtifactRole::FactDescriptor` is added to artifact role policy baselines:
  schema policy, semantic policy, producer policy, staging class, retention
  class, same-commit policy, schema tags, and `ArtifactRole::ALL`.
- Certification tests prove:
  - descriptor allow-list is hash-defining spec material
  - adding/removing/changing an allowed descriptor changes the certified spec
    hash
  - a node cannot certify with malformed descriptor metadata
  - a store-admitted but uncertified descriptor is not sufficient for append
    authority
- Runtime or store-level negative test proves an append using an uncertified
  descriptor is rejected once append wiring exists. If that cannot be wired
  cleanly until Gate 3, leave the test as a Gate 3 acceptance item rather than
  adding a shim.
- Documentation updates are part of the gate:
  - `docs/design.md` for descriptor authority and certified allow-list
    semantics.
  - `docs/architecture.md` for crate ownership and state/operation placement
    expectations.

### Dependencies On Previous Gates

Gate 1 provides facts kernel descriptor, extraction, and canonicalization
contracts.

### Risks And Failure Modes

- Treating the `MfmFactType` implementation as authority instead of descriptor
  bytes plus certified spec allow-list.
- Letting manual trait implementations bypass descriptor admission, extraction,
  canonicalization, or allow-list validation.
- Adding descriptor emission as operation-local ad hoc metadata rather than
  state/descriptor authority.
- Making descriptor artifacts runtime-local instead of durable retained
  authority.
- Expanding derives beyond the v1 scalar/path grammar.

### Explicitly Out Of Scope

- Public fact query APIs.
- Postgres fact index tables.
- Query evidence and receipt signing.
- Collector checkpoint semantics.
- Named scope authorization beyond `Default`.

## Gate 3: Event Reset And Append Projection

### Objective

Replace the old opaque `FactRecorded` shape with normalized typed fact claims
and project indexed facts atomically in Postgres from retained authority.

### Crates And Modules Likely Touched

- `crates/kernel/events`
- `crates/kernel/facts`
- `crates/kernel/store`
- `crates/kernel/runtime`
- `crates/storages/stream-store-postgres`
  - `migrations/0001_run_store.sql`
  - `src/run_store/append.rs`
  - `src/run_store/projections.rs`
  - `src/run_store/artifacts.rs`
  - `src/run_store/artifact_admission.rs`
  - `src/run_store/tests.rs`
- Existing fact producers that currently call
  `RunnerArtifactBuilder::fact_response` and `RunnerPayloadBuilder::fact_recorded`,
  including `crates/adapters/evm-contracts`, `crates/adapters/portfolio`, and
  `crates/collectors/proof` if they remain in the workspace.
- `crates/kernel/runtime/src/runner_kit.rs`,
  `crates/kernel/runtime/src/runners.rs`, and commit validation modules.

### New Or Changed Public Types And Interfaces

- Replace old `events::FactRecorded` fields with:
  - `FactRecordedPayload`
  - `FactClaim`
  - `FactSubjectEvidence`
  - `FactRequestEvidence`
  - `FactProducerProvenance`
- Add or stabilize `FactRecorder`:
  - `record_fact<T: MfmFactType>(FactRecordInput<T>)`
  - `record_fact_query_evidence(FactQueryEvidence)` can be stubbed only if it
    returns a typed not-yet-supported error until Gate 4. Prefer implementing
    it in Gate 4 to keep this gate narrower.
- `FactRecordInput<T: MfmFactType>`
- `FactRecordOptions`
- `StagedFactRecordHandle`
- `FactRecordCommitResult`
- `InternalFactRef` or an implementation-local equivalent for trusted
  store/runtime/query projection rows.
- Runtime helper that derives:
  - descriptor hash
  - descriptor artifact evidence
  - canonical subject material
  - subject material hash
  - fact subject namespace hash
  - `FactKey`
  - response artifact with `ArtifactRole::FactResponse`
  - producer provenance from runtime context
- Post-commit mapping from staged local id to committed `RunEventId` and
  `FactClaimId`.

### Storage And Projection Changes

- Replace old projection snapshot fact map keyed by
  `(NodeId, AttemptId, FactKey)` with claim-oriented structures.
- Add Postgres baseline tables:
  - `fact_descriptor_index`
  - `fact_index`
  - `fact_index_terms`
- Projection rows are inserted in the same append transaction that inserts
  commits, artifact admissions, commit artifact evidence, and run events.
- `RunPrivate` facts commit event/artifact/descriptor authority but create no
  `fact_index` or `fact_index_terms` rows.
- `Indexed` facts insert exactly one `fact_index` row per `FactRecorded` plus
  subject/result/metadata term rows keyed by `FactClaimId`.
- Term joins must use `FactClaimId`; never use `FactKey` as the claim join key.
- Descriptor bytes must be admitted before or during the append that first uses
  them.
- Every `FactRecorded` append must bind descriptor authority through committed
  artifact evidence, not a global cache:
  - the append request must require or admit an `ArtifactRole::FactDescriptor`
    artifact whose canonical bytes hash to `fact_descriptor_hash`
  - the descriptor artifact binding must be part of the same committed run
    append authority, or must reference already committed artifact evidence
    through the store's required-artifact mechanism
  - retention must include the descriptor artifact evidence for every fact
    descriptor used by the run
  - append, rebuild, and replay must be able to find descriptor bytes from
    committed artifact evidence and retained artifacts without consulting the
    current Rust descriptor constructor or a rebuildable descriptor projection
- Projection rebuild must truncate and repopulate descriptor/fact/term
  projections from run stream events, descriptor artifacts, subject material,
  response artifacts, and commit metadata.
- Do not use PostgreSQL materialized views for fact indexing.
- Do not use SQL triggers to decode typed response artifacts or run descriptor
  extraction.

### Tests And Acceptance Criteria

- Old-shape `FactRecorded` cannot decode, construct, append, or pass schema
  golden tests.
- An indexed append commits event, response artifact, descriptor artifact,
  descriptor projection, fact index row, and all term rows atomically.
- A `RunPrivate` append retains authority but creates no fact index or term
  rows.
- Append rejects:
  - missing descriptor bytes
  - missing descriptor artifact commit binding or retention edge
  - descriptor hash mismatch
  - descriptor not allowed by certified node spec
  - fact kind/schema mismatch
  - subject material hash mismatch
  - recomputed `FactKey` mismatch
  - response artifact role other than `FactResponse`
  - response artifact containing multiple claims
  - extraction failures
  - required missing/null fields
  - scalar bound violations
  - event producer provenance inconsistent with the runtime attempt
  - projection insertion failure
- Rebuild tests mutate or remove current Rust descriptor constructors while
  retaining descriptor artifacts, proving rebuild uses retained canonical bytes
  and not in-process static descriptors.
- Negative tests prove a descriptor present in `fact_descriptor_index` or an
  in-memory catalog is insufficient when committed descriptor artifact evidence
  is absent.
- Rollback tests prove descriptor, extraction, artifact, event, and projection
  failures leave no partial authoritative rows or projection rows.
- Idempotent retry of the same prepared commit returns the committed batch and
  does not duplicate fact projections.
- Rebuild from retained authority reproduces projection rows deterministically
  or emits a stable validation report with precise differences.
- Validation catches:
  - fact index row without matching `FactRecorded`
  - `RunPrivate` fact with index rows
  - descriptor hash mismatch
  - subject namespace/material/hash/key mismatch
  - response artifact/evidence mismatch
  - source run id/seq/ordinal/event id mismatch
  - store order mismatch
- SQLx schema drift checks and Postgres parity tests cover new tables.
- Documentation updates are part of the gate:
  - `docs/design.md` for new event schema, descriptor artifact binding,
    retention, and projection authority.
  - `docs/architecture.md` for storage/projection ownership.
  - `docs/persisted-public-surfaces.md` for fact events, descriptor artifacts,
    subject material, response artifacts, and projection rows.

### Dependencies On Previous Gates

Depends on Gate 1 facts kernel contracts and Gate 2 descriptor allow-list
authority.

### Risks And Failure Modes

- Committing an indexed fact without projection rows.
- Treating projection rows as replay or resume authority.
- Recomputing descriptor meaning from current Rust code during rebuild instead
  of retained descriptor bytes.
- Allowing public or control queryability before privacy filters and query
  evidence exist.
- Leaving existing adapters on manual fact keys and thereby preserving the old
  contract.
- Making `FactClaimId` optional for indexed facts or deriving it before store
  coordinates exist.

### Explicitly Out Of Scope

- Public CLI/REST fact query surfaces.
- Store-authenticated query receipts.
- Cryptographic absence proofs.
- Multi-descriptor query execution.
- Exclusive checkpoint advancement or checkpoint conflict resolution.
- Public ref token format.

## Gate 4: Reusable Query Compiler And Replay Evidence

### Objective

Add descriptor-scoped fact query compilation, store execution, authenticated
receipts, state selection evidence, and replayable `FactQueryEvidence` pinned
into consuming runs.

### Crates And Modules Likely Touched

- `crates/kernel/facts` query modules, or a new
  `crates/kernel/facts-query` only if keeping query compilation separate is
  materially simpler.
- New `crates/fact-capabilities` or equivalent narrow capability-contract
  crate for state-facing indexed fact reads.
- `crates/kernel/store`
- `crates/kernel/events` for `ArtifactRole::FactQueryEvidence`
- `crates/kernel/runtime`
- `crates/kernel/replay`
- `crates/storages/stream-store-postgres`
- `crates/adapters/*` for the generic store-backed live fact query runner
  binding and replay verifier binding.
- `crates/app` evidence-only services
- `crates/runtime-config` only for non-secret store receipt trust-root wiring;
  do not put private signing material in typed configs.
- `crates/artifact-capabilities` if new retained artifact read requests are
  needed for query evidence replay.

### New Or Changed Public Types And Interfaces

- `FactQueryInput`
- `FactDescriptorSelector`
- `ResolvedFactDescriptor`
- `FactQueryScope`
- `CanonicalFactQueryPlan`
- `CanonicalFactPredicate`
- `FactOrdering`
- `FactQueryCompiler`
- `FactQueryService`
- `FactQueryReceipt`
- `StoreReadFrontier`
- `StoreReadFrontierType`
- `DescriptorCatalogWatermark`
- `FactProjectionGeneration`
- `StoreCommitWatermark`
- `StoreReceiptAuthentication`
- `StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1`
- `StoreIdentity`, `StoreKeyId`
- `ReturnedFieldSummaries`
- `QueryResultCardinality`
- `FactSelectionEvidence`
- `FactQueryEvidence`
- `StagedFactQueryEvidenceHandle`
- State-facing capability contract:
  - `FactQueryReadCapability`
  - `FactQueryRequest`
  - `FactQueryResponse`
  - `FactQueryCapabilityError`
  - capability kind similar to `mfm.fact.query.read`
- Certified consuming-state config/policy types:
  - descriptor selector or exact descriptor hash
  - fact query scope and audience
  - predicates
  - ordering policy
  - limit
  - selection policy
  - returned summary requirements
- Live adapter binding that executes canonical plans against the store query
  service and returns store-authenticated receipts.
- Replay adapter binding that answers only from pinned `FactQueryEvidence` and
  retained artifacts.
- Internal read capability for `FactAudience::Control`; public app services use
  the same compiler/query execution but are not the state capability boundary.

The descriptor selector, scope, predicates, ordering, limit, and selection
policy used by a consuming workflow must be hash-defining certified state config
or certified descriptor material for that consuming node. A live runner may
materialize that policy into a capability request, but it must not invent or
mutate outcome-affecting query semantics at runtime.

### Storage And Projection Changes

- Add store read APIs that execute canonical plans over the fact projection.
- Add descriptor catalog watermark and fact projection generation/rebuild id
  tracking.
- Add receipt signing support:
  - `crates/storages/stream-store-postgres` owns the receipt authentication
    scheme implementation and binds receipts to store identity, trust scope,
    schema contract version, descriptor/projection watermarks, key id, and
    public verification key fingerprint
  - the private receipt signing key is supplied to the live store/query service
    as a process-local `FactReceiptSigner` or equivalent trait object from app
    assembly; it is never typed workflow config and is never persisted in run
    events, artifacts, public output, diagnostics, fixtures, or snapshots
  - the public verification key/trust root is stored as non-secret store
    metadata or an append-only store trust table so evidence-only replay can
    verify receipts without constructing live runtime config or secret-bearing
    signer providers
  - evidence-only app/replay services get a verifier over store metadata and
    retained receipt bytes, not a private signer
  - no private receipt signing key is stored in typed configs, events,
    artifacts, public output, diagnostics, fixtures, or snapshots
- Add `ArtifactRole::FactQueryEvidence` with role policy baselines.
- `record_fact_query_evidence` admits canonical `FactQueryEvidence` as a
  private artifact and emits the existing generic `ArtifactReferenced` event.
  It does not insert into `fact_index`.
- Query evidence creates retention edges for:
  - the evidence artifact
  - receipt authority
  - selected and returned `InternalFactRef`s
  - source run events
  - descriptor artifacts
  - canonical subject material
  - response artifacts
  - artifact admission/binding evidence
- V1 does not retain omitted matching facts for empty or limited result sets.
  The authenticated receipt plus semantic read frontier is replay authority for
  those decisions.

### Tests And Acceptance Criteria

- Public and internal callers use the same compiler and canonicalizer.
- State-facing fact reads go through `FactQueryReadCapability`; states do not
  depend on app, `mfm-store` implementation types, SQLx, or Postgres.
- Adapter tests prove live execution binds the capability to store query
  execution and replay binds it to pinned `FactQueryEvidence`.
- Cargo metadata checks prove fact-consuming state crates do not depend on app,
  store implementations, Postgres, transports, or operation crates.
- Certification tests prove descriptor selector, scope, predicates, ordering,
  limit, and selection policy are hash-defining certified config/material for
  every consuming node that can query facts.
- V1 planning resolves exactly one descriptor before execution.
- Compilation rejects:
  - zero descriptor matches
  - more than one descriptor match
  - unknown fields
  - hidden public fields
  - operator/value type mismatches
  - missing explicit ordering for `latest`
  - invalid limits
  - invalid scope/audience combinations
- Query execution filters by audience and scope before result construction.
- Public execution cannot see `Control` or `RunPrivate` facts through counts,
  ambiguity errors, exact-ref lookup, or descriptor discovery.
- Ordering tests cover store commit order, observed time, result progression
  fields, null handling, and deterministic tie-breakers.
- Receipt tests reject:
  - missing authentication
  - unsupported scheme
  - unknown key id
  - bad signature/MAC
  - wrong store trust root
  - store identity or trust-scope mismatch
  - key fingerprint mismatch
  - canonical query hash mismatch
  - descriptor resolution mismatch
  - scope decision mismatch
  - ordering or limit mismatch
  - frontier mismatch
  - result-set digest mismatch
  - returned summary digest mismatch
- Selection tests reject duplicated, out-of-bounds, unsorted, or policy-invalid
  selected indices and selected-summary mismatches.
- Replay rejects query evidence when the canonical query plan, scope, ordering,
  limit, descriptor selector/resolution, or selection policy does not match the
  certified consuming node policy exactly.
- Replay tests prove empty and limited results replay from pinned evidence
  without querying the current fact index.
- Replay service construction remains evidence-only and cannot construct live
  transports, signer providers, keystores, mutable certification registries, or
  live runtime config.
- Retention tests prove query evidence keeps all source authority needed for
  replay even if live fact index rows are later removed from query surfaces.
- Redaction tests prove receipt signer config errors do not leak private key
  paths, key material, environment variable names that resolve secrets, or
  credential-bearing diagnostics.
- Wrong-trust-root tests prove receipts from another store/trust scope/key are
  rejected.
- Documentation updates are part of the gate:
  - `docs/design.md` for fact query capability, receipt authentication,
    replay authority, and retention semantics.
  - `docs/architecture.md` for capability/adapter/app boundaries.
  - `docs/persisted-public-surfaces.md` for query evidence, receipts, returned
    refs, summaries, and receipt signer diagnostics.

### Dependencies On Previous Gates

Depends on Gate 3 projection tables and normalized fact authority.

### Risks And Failure Modes

- Letting CLI/REST implement independent query parsing or compilation.
- Treating database cursors or pagination tokens as replay authority.
- Failing to authenticate empty results or limited result sets.
- Trusting current descriptor registry instead of retained descriptor bytes
  during replay.
- Creating a receipt signing key path that leaks secrets into persisted or
  public surfaces.
- Constructing live fact queries during replay.
- Letting states call app/store/Postgres services directly instead of using a
  typed read capability and adapter binding.
- Treating non-certified query or selection policy as runtime convenience.

### Explicitly Out Of Scope

- Cryptographic absence proofs.
- Multi-descriptor query execution.
- Remote/federated receipts.
- Receipt key rotation policy beyond accepting a configured v1 local trust
  root.
- Full named-scope authorization beyond `Default`.
- Public pagination internals beyond opaque cursors.

## Gate 5: Public CLI/REST Surfaces

### Objective

Expose descriptor-scoped, kind-first public fact queries through thin CLI and
REST surfaces backed by shared app services and the reusable query compiler.

### Crates And Modules Likely Touched

- `crates/app/src/facts.rs` or equivalent app service module
- `bin/cli/src/commands/facts/*`
- `bin/cli/src/presentation/*`
- `bin/rest-api/src/*`
- `bin/cli/README.md`
- `bin/rest-api/README.md`
- `docs/persisted-public-surfaces.md`
- CLI/REST integration tests and JSON output contract tests

### New Or Changed Public Types And Interfaces

- App service:
  - `FactsReadService`
  - `FactsDescribeService`
  - `FactsExplainService`
  - `PublicFactRefResolver`
- CLI commands:
  - `mfm facts kinds`
  - `mfm facts describe <kind>`
  - `mfm facts explain <kind>`
  - `mfm facts latest <kind> ...`
  - `mfm facts history <kind> ...`
  - `mfm facts top <kind> ...`
  - `mfm facts query ...`
  - `mfm facts show <public-ref>`
- REST routes:
  - `GET /v1/facts/kinds`
  - `GET /v1/facts/kinds/{kind}`
  - `GET /v1/facts/{kind}/latest`
  - `GET /v1/facts/{kind}`
  - `GET /v1/facts/ref/{public_ref}`
- Public DTOs:
  - `PublicFactRef`
  - `PublicFactRefId`
  - `PublicFactDescriptorRef`
  - `PublicFactFieldValue`
  - stable redacted error codes for ambiguity, invalid field/operator,
    unsupported ordering, invalid scope, invalid public ref, not found, and
    authorization failure.

### Storage And Projection Changes

- No new semantic storage beyond Gate 4 query/read support.
- Add opaque public ref mint/resolve support in the app/query service. The
  public ref must be versioned and scope-checked, and must not encode raw
  artifact ids, raw `FactClaimId`, run ids, event ids, subject hashes, or
  stable cross-scope correlation handles in plaintext.
- Public pagination uses opaque cursors only. Do not expose transaction ids,
  append xid, store epochs, commit sort keys, or raw store order coordinates.

### Tests And Acceptance Criteria

- CLI text and JSON output match documented contracts.
- REST responses match documented route contracts.
- Public results include only descriptor `Returnable` fields.
- `QueryOnly` fields can filter/order but are omitted from public summaries.
- `Hidden` fields cannot filter, order, or return through public APIs.
- Public output never exposes:
  - `InternalFactRef`
  - artifact ids
  - artifact evidence hashes
  - canonical subject material
  - subject hashes
  - request/response hashes
  - response artifacts
  - raw run ids
  - event ids
  - source sequence/ordinal
  - capability routing details
- `Control` and `RunPrivate` facts are invisible through:
  - `kinds`
  - `describe`
  - `explain`
  - query results
  - latest/history/top
  - exact public ref lookup
  - error messages
- Unknown public refs and refs for non-public facts return the same redacted
  not-found class.
- Kind-only query fails with a stable ambiguity error unless it resolves to
  exactly one descriptor or deterministic compatibility group.
- CLI/REST use app services and do not depend directly on SQLx or storage
  implementation internals.
- Evidence-only fact read services do not parse live runtime config or
  construct live transports/signers.
- Documentation updates are part of the gate:
  - `bin/cli/README.md` for command, text output, JSON output, and error
    contracts.
  - `bin/rest-api/README.md` for route, response, and error contracts.
  - `docs/design.md` for public fact read authority and public DTO
    non-authority.
  - `docs/architecture.md` for public naming and binary/API boundaries.
  - `docs/persisted-public-surfaces.md` for every persisted or returned fact
    field.

### Dependencies On Previous Gates

Depends on Gate 4 query compiler, query execution, and public/internal scope
semantics.

### Risks And Failure Modes

- Leaking internal refs or raw authority through convenience DTOs.
- Letting public descriptor discovery reveal `Control` descriptors.
- Adding CLI-specific query compilation that diverges from REST/runtime.
- Treating public refs as artifact read authority.
- Returning `QueryOnly` fields and assuming they are private.
- Creating pagination cursors that expose store internals.

### Explicitly Out Of Scope

- Public named-scope authorization beyond default scope.
- Public raw artifact reads for facts.
- Cross-store export/import.
- Public access to `Control` facts.
- Domain-specific fact DTOs such as wallet- or weather-specific structs.

## Gate 6: First Collector Workflow

### Objective

Prove the model with one recurring bounded collector-style workflow that uses
ordinary operations, states, adapters, transports, run events, platform facts,
control checkpoint facts, and pinned query evidence. Do not add collector
runtime semantics.

### Proposed First Workflow

Default target: a Bitcoin wallet balance collector cycle, because the repo
already has a Bitcoin JSON-RPC HTTP client and the RFC examples repeatedly use
wallet balances.

Proposed shape:

```text
btc_wallet_balance_collector_cycle
  -> query_control_checkpoint
  -> poll_wallet_source
  -> normalize_balance_observation
  -> record_wallet_balance_fact
  -> record_checkpoint_fact
  -> complete_cycle
```

If this proves too broad for the first mergeable vertical slice, use the
existing proof domain to build a non-production collector acceptance harness,
but keep it clearly marked as a harness. The RFC should not be considered fully
implemented until a real external observation workflow records both platform
facts and control checkpoint facts.

### Crates And Modules Likely Touched

- Capability contract crate for Bitcoin read authority, if one does not
  already exist. Prefer `crates/btc-capabilities` over putting capability
  traits in a collector or transport crate.
- Transport crate for Bitcoin JSON-RPC under `crates/transports/*`. Before a
  Bitcoin collector uses the existing `crates/collectors/btc-jsonrpc-http`
  code, move/rename that reusable protocol IO into a transport crate or replace
  it with a proper transport implementation. Do not keep a transport-like
  collector crate as a temporary architecture exception.
- State crate for reusable wallet balance collection semantics, likely under
  `crates/states/*`.
- Adapter crate under `crates/adapters/*` to bind state intent to the Bitcoin
  capability and fact recorder.
- Operation crate under `crates/ops/*-op` to plan one bounded collector cycle.
- `crates/app` to register the operation, runner, capability binding, and
  internal control fact query capability.
- CLI/REST start surfaces only if a public command/route is needed to launch
  the collector cycle.

### New Or Changed Public Types And Interfaces

- `WalletBalanceFact` implementing `MfmFactType`.
- `WalletBalanceSubject` with subject fields such as chain, network,
  account_ref, asset_ref, and balance_kind.
- `WalletBalanceObservation` with result fields such as amount in satoshis and
  observed block height.
- `CollectorCheckpointFact` implementing `MfmFactType`.
- `CollectorCheckpointSubject` with collector kind, source ref, visibility
  scope, and stream partition.
- `CollectorCheckpointObservation` with high-watermark, predecessor checkpoint,
  observed range, and finality policy.
- Typed read state config for bounded polling/waiting:
  - source ref
  - timeout
  - maximum batch size
  - retry/backoff policy
  - finality/confirmation policy if relevant
- Internal query/selection policy for choosing the checkpoint. This must be
  recorded as `FactQueryEvidence`.

### Storage And Projection Changes

- No collector-specific storage.
- Platform wallet balance observations are indexed with
  `audience = Platform`.
- Checkpoints are indexed with `audience = Control`.
- Checkpoint query evidence is retained as a private `FactQueryEvidence`
  artifact in the consuming run.
- If exclusive checkpoint advancement is required, do not fake it with
  descriptor policy. Add a separate generic lease, compare-and-append, or
  side-effect authority first, or defer competing-writer support.

### Tests And Acceptance Criteria

- One run cycle records at least one platform-visible domain fact.
- The same run or next run records a `Control` checkpoint fact.
- The next cycle queries the checkpoint through the internal control query
  capability and pins `FactQueryEvidence`.
- Crash/retry tests prove committed facts and checkpoints survive, while
  uncommitted observations are retried.
- Replay of the collector cycle uses pinned query evidence and recorded
  response artifacts only. It does not query the live fact index or live
  Bitcoin source.
- Public fact APIs can query platform wallet balance facts and cannot see
  checkpoint facts.
- Capability, adapter, state, and operation crate dependencies satisfy
  architecture metadata checks.
- No RPC URLs, credentials, auth headers, password paths, or source routing
  secrets enter typed configs, facts, artifacts, events, diagnostics, fixtures,
  snapshots, CLI output, or REST output.
- The operational recurring loop is outside durable semantics: an external
  launcher starts or resumes bounded cycle runs.
- Documentation updates are part of the gate:
  - `docs/architecture.md` if any Bitcoin capability/transport/state/adapter
    ownership is added or renamed.
  - `docs/design.md` only if collector execution changes runtime, replay, or
    store semantics; an ordinary workflow should not need a design contract
    change.
  - CLI/REST docs if a public collector launch command or route is added.

### Dependencies On Previous Gates

Depends on Gates 1 through 5. Gate 4 is required for checkpoint query evidence.
Gate 5 is required if public users must inspect the produced platform facts
through CLI/REST.

### Risks And Failure Modes

- Turning the collector loop into a forever-open attempt that does not commit
  progress.
- Putting Bitcoin JSON-RPC IO into a state or operation crate.
- Publishing checkpoints as public facts by accident.
- Treating checkpoint facts as conflict prevention when competing collectors
  can write the same partition.
- Persisting runtime source routing or credentials in typed fact material.
- Adding a collector-specific runtime scheduler instead of using ordinary runs.

### Explicitly Out Of Scope

- Background worker-pool dispatch.
- Feed-driven automatic dispatch.
- Long-lived daemon semantics.
- Exclusive checkpoint advancement unless a generic authority is implemented.
- Multi-source reconciliation.
- Public checkpoint APIs.
- Cross-store fact export/import.

## Canonicalization And Hashing Test Strategy

- Keep canonical hash vectors in kernel tests near the type that owns the
  canonical contract.
- Use canonical JSON through `mfm-canonical`; never hash serde's default string
  output directly.
- Assert no hashed structure contains floats.
- Assert descriptor hash excludes the descriptor hash field itself.
- Assert `FactSubjectNamespaceV1.fields` and `FactSubjectMaterialV1.values` are
  sorted by `field_id` before canonicalization.
- Add fixtures proving path changes do not affect `FactKey` when stable subject
  field ids, value types, units, scales, and values are unchanged.
- Add fixtures proving result fields such as amount, block number, or
  temperature do not affect `FactKey`.
- Add fixtures proving metadata fields such as recorded time, observed time,
  and store order do not affect `FactKey`.
- Add malformed canonical bytes tests for descriptor, subject material, and
  query evidence.
- Add store/rebuild tests that recompute all hashes from retained authority,
  not current Rust descriptor constructors.
- Avoid snapshot tests that bless large opaque JSON without checking the
  semantic pieces that matter.

## Privacy And Security Checks

- Extend persisted/public surface review to include fact descriptors, subject
  material, fact responses, query evidence, receipts, public refs, CLI output,
  REST output, and diagnostics.
- Reuse and extend secret-marker guards from `mfm-values` where appropriate,
  while documenting that marker checks are not proof of safety.
- Add negative fixtures for common secret-bearing names and values:
  password, passphrase, mnemonic, private key, seed phrase, API key, access
  token, authorization header, bearer token, RPC URL with credentials, password
  file path, keystore path.
- Verify `RunPrivate` facts create no `fact_index` or `fact_index_terms` rows.
- Verify `Control` facts are queryable only through internal capabilities and
  never through public app/CLI/REST services.
- Verify descriptor exposure:
  - `Returnable` may appear in public summaries
  - `QueryOnly` may filter/order but not return
  - `Hidden` cannot cross the public boundary
- Verify public errors are redacted and fail closed for non-public facts.
- Verify exact-ref lookup cannot distinguish unknown refs from refs to
  non-public facts.
- Verify evidence-only read/replay/public-output paths do not parse live
  runtime config, construct transports, construct signer providers, open
  keystores, or resolve environment variables.
- Receipt signing private material must remain process-local or in a secret
  store and must not be persisted in typed semantic surfaces.

## Replay And Retention Checks

- Indexed facts remain queryable only while retained authority exists:
  source event payload, commit metadata, descriptor bytes, subject material,
  response artifact, artifact admission/binding evidence, request/response
  schema/hash evidence, and producer provenance.
- `FactQueryEvidence` retention edges must preserve all returned refs,
  descriptors, source events, subject material, response artifacts, receipt
  authority, returned summaries, and selection evidence needed for replay.
- Replay must verify:
  - stored certified spec and certificate
  - descriptor hashes and retained descriptor bytes
  - canonical query plan and hash
  - compiler/canonicalizer versions
  - scope decision evidence
  - ordering and limit
  - store receipt authentication
  - result-set digest
  - returned refs and summaries
  - selection policy hash and selected indices
  - source event/artifact authority for selected facts
- Replay must not:
  - query the live fact index
  - call live collectors
  - construct live transports
  - construct live signers
  - call mutable registries or external policy oracles for outcome-affecting
    decisions
- Garbage collection must either preserve authority for queryable facts and
  pinned query evidence, or first remove the fact from live query surfaces and
  prove no retained query evidence depends on it.

## Postgres Validation And Rebuild Plan

- Add fact projection insertion to the append transaction after event and
  artifact evidence are validated, but before commit.
- Keep typed extraction in Rust append/rebuild code, not SQL triggers.
- Add database constraints for:
  - `fact_claim_id` uniqueness
  - source event foreign keys to `run_events`
  - descriptor artifact evidence references
  - response artifact evidence references
  - audience and scope enums
  - one typed value column per term row where practical
  - non-negative byte lengths and bounded text/numeric sizes
- Add useful indexes for:
  - `(fact_claim_id, field_id)`
  - `(fact_descriptor_hash, field_id, typed_value)`
  - `(fact_descriptor_hash, field_id, sortable_value)`
  - audience/scope/kind/recorded-time
  - audience/scope plus `FactKey`
  - store commit order tie-breakers
- Add `FactProjectionGeneration` and descriptor catalog watermark tracking.
- Implement validation that checks every required invariant listed in the RFC
  "Rebuild And Verification" section.
- Implement rebuild as truncate-and-repopulate of fact projections from strict
  authority in a transaction or a clearly staged repair mode. The rebuild
  command/API can be internal/test-only for v1, but the code path should ship
  because append idempotency does not repair missing projection rows.
- Add deterministic validation reports with stable error categories so tests do
  not parse display strings.
- Run SQLx prepare/schema drift checks and managed Postgres parity tests after
  schema changes.

## CLI And REST Contract Impacts

- `mfm facts` becomes a new public CLI namespace and must respect
  `--output-format` plus `MFM_OUTPUT_FORMAT`.
- JSON output shapes are public API and need documented examples in
  `bin/cli/README.md`.
- REST routes and response/error envelopes need documentation in
  `bin/rest-api/README.md`.
- Public fact queries are descriptor-scoped even when the user starts with a
  kind. If kind resolution is ambiguous, return a stable ambiguity error and
  require `--shape` or descriptor hash.
- `latest` is syntactic convenience for `limit = 1` plus an explicit temporal
  or progression ordering policy. Ranking by amount/price/score/temperature is
  `top` or ordinary query wording, not automatically latest.
- CLI/REST should share app DTOs and query compilation. Do not duplicate field
  parsing or ordering semantics in each transport.
- Public ref output is not artifact read authority and must be opaque.
- Update public surface inventory and tests when adding any persisted or
  returned field.

## Likely PR Breakdown

1. Add `crates/kernel/facts` with descriptor, visibility, extraction, identity,
   canonicalization, and golden tests.
2. Add `MfmFactType` derive support and descriptor artifact role.
3. Add certified spec node-level descriptor allow-lists and certification hash
   tests.
4. Reset `FactRecorded` event payload and runtime runner-kit fact recorder.
5. Convert or disable existing old-shape fact producers.
6. Add Postgres fact projection tables, append projection, validation, and
   rebuild.
7. Add reusable query compiler, descriptor resolution, ordering, and public vs
   control scope checks.
8. Add store query execution, receipt authentication, query evidence artifacts,
   replay verification, and retention edges.
9. Add app facts read services and public ref handling.
10. Add CLI and REST public fact surfaces with docs and integration tests.
11. Add first collector workflow vertical slice.
12. Run final `nix run .#check`, `nix run .#test`, `nix run .#test-db`, and
    `nix run .#ci` for merge readiness after major integration.

Some adjacent items can merge together when the code would otherwise be
unbuildable, but avoid hiding multiple authority changes in one review.

## Unresolved Engineering Questions

- Should the reusable query compiler live entirely in `crates/kernel/facts` for
  v1, or should it be split into a `crates/kernel/facts-query` crate? Start
  with one facts kernel crate unless dependency pressure proves otherwise.
- What exact versioned format should `PublicFactRefId` use? It must be opaque,
  scope-checked, non-authoritative, and non-correlating across scopes.
- Where should the v1 local receipt signing public trust root live, and how is
  the private signing key supplied without entering typed semantic surfaces?
- Should descriptor alias/shape registration for CLI `--shape` live in app
  assembly, the facts kernel catalog, or certified descriptor artifacts?
- Which real collector should count as the first production collector if the
  Bitcoin workflow is too broad for the first vertical slice?
- Should `crates/collectors/btc-jsonrpc-http` be moved under
  `crates/transports/*`, renamed, or kept temporarily as a transport-like crate
  while preserving dependency boundaries?
- What minimal internal/admin surface should trigger fact projection validation
  and rebuild in v1?
- How should retention cleanup prove that no retained `FactQueryEvidence`
  depends on a fact before removing it from live query surfaces?
- Does the existing artifact role staging/retention policy need a new
  descriptor-specific class, or can `FactDescriptor` use an existing
  value-artifact class with stricter validation?
- Do `record_fact_query_evidence` and `record_fact` share one recorder trait in
  runtime, or should query evidence recording stay on a narrower artifact
  recorder interface until Gate 4?

## Architect Review Notes

This section records review-loop outcomes. Blocking or major architecture,
privacy, replay, or implementation-readiness issues must be resolved before the
plan is considered ready.

- Pending initial architect critique.
