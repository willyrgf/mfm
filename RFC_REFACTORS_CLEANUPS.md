# RFC: Refactors, Cleanups, And Larger Abstraction Bets

Status: draft

This document records candidate refactors discovered during a read-only architecture pass over the
MFM repository. It separates incremental cleanups from deeper abstraction changes. The distinction
matters: most obvious cleanups remove drift and improve placement, but they do not fundamentally
change the LOC profile. A 30-50% reduction is only plausible if MFM changes how typed kernel
contracts, event transitions, and tests are authored.

## Executive Summary

The current architecture is not obviously wrong. The core model is coherent:

- operations plan typed specs
- states own reusable semantics
- adapters bind state intent to capabilities
- transports implement reusable protocol IO
- runtime/store/replay enforce certified authority from append-only events

The larger issue is implementation strategy. MFM currently hand-maintains the same typed contract
knowledge across spec, events, runtime, store, replay, app, adapters, and tests. That explicitness
protects correctness, but it costs a lot of code and creates drift risk.

The clearest architectural thesis is:

> keep the typed authority model, but stop hand-writing every projection, verifier, event
> descriptor, artifact requirement, and fixture mutation. Move to declarative protocol
> definitions that generate or derive the repeated code.

Without that shift, the realistic LOC reduction from local refactors is closer to 5-15%. With it,
30% may be possible across production plus tests. A 50% reduction would likely require either
generated code being excluded from reviewed LOC, or dropping coverage/detail that currently carries
real correctness value.

## Current LOC Shape

Approximate local measurement: `139852` Rust LOC.

Largest concentrations:

- `crates/kernel/runtime`: about 22k LOC, including an 11k LOC test file
- `crates/kernel/certify`: about 9.7k LOC
- `crates/kernel/store`: about 8.7k LOC plus 5.3k LOC store contract tests
- `crates/kernel/program`: about 7.7k LOC
- `crates/app`: about 7.4k LOC
- `crates/kernel/spec` and `crates/kernel/events`: about 8.3k LOC combined
- `tests/integration/src/test_support.rs`: about 3.9k LOC

This suggests the biggest reduction opportunities are not in ordinary Rust factoring. They are in
generated contracts, generic event/projection machinery, and reusable test scenario builders.

## Fundamental Abstraction Bets

The second architecture pass sharpened the thesis. MFM does not mainly suffer from bad crate
boundaries. It suffers from repeated protocol authority: the same durable facts are expressed as
typed program descriptors, certified spec DTOs, event payloads, schema descriptors, store codecs,
projection folds, runtime indexes, replay views, app service helpers, adapter runner ceremony, and
test fixtures.

The goal should not be to make these contracts dynamic. The goal should be to keep the same typed
authority model while authoring each durable protocol once.

### 1. Define The Kernel Contract Protocol Declaratively

Current evidence:

- `crates/kernel/events/src/lib.rs` hand-defines event payload structs, event schemas, event
  descriptor fields, artifact requirement extraction, enum tag lists, and string wrappers.
- `crates/kernel/spec/src/lib.rs` hand-writes canonical JSON and strict parse logic for persisted
  spec DTOs.
- `crates/kernel/store/src/lib.rs` hand-writes event payload codec mappings and parsing.
- `crates/kernel/store/src/v1/projection.rs` hand-matches payloads into projection state.
- `crates/kernel/replay/src/lib.rs` and `crates/kernel/runtime/src/*` independently re-check parts
  of the same payload contracts.
- Tests hand-build many event payloads again.

One concrete drift signal from the review: `TypedSpecCertificate` exists as an artifact role in the
event and store codec surfaces, while the event schema role tag list appears to omit
`typed_spec_certificate`. That is exactly the kind of issue a single contract source should prevent.

Proposed direction:

Create a declarative kernel contract source for v1 spec and event families. It should be able to
produce or derive:

- persisted event payload structs and enum tag tables
- event schema descriptors and field descriptors
- canonical JSON encoders and strict parsers for spec DTOs
- payload codec mappings for the store
- artifact requirement extraction
- projection transition skeletons
- public output and artifact evidence matchers
- golden fixture builders for valid baseline events

This could be implemented as:

- a declarative Rust macro inside the kernel, or
- checked schema definitions plus generated Rust committed to the repository, or
- a build step that fails when generated output is stale

Expected impact:

- Potentially removes 8k-15k hand-written LOC across events/spec/store/replay/runtime/tests.
- More importantly, event shape, schema descriptors, artifact requirements, codecs, canonical JSON,
  and projection matching become one contract.

Correctness requirements:

- Generated or derived code must preserve canonical JSON and public schema stability.
- Event names, field names, enum variants, and string encodings must be byte-stable unless a
  deliberate persisted-format migration is made.
- Generated code must be reviewable. If generated code is committed, CI should check it is fresh.
- The schema source must be treated as typed kernel authority, not as loose metadata.

Verification:

- `cargo test -p mfm-events`
- `cargo test -p mfm-spec`
- `cargo test -p mfm-store`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-runtime`
- descriptor, canonical JSON, and fixture-hash golden tests before and after the migration

### 2. Promote Artifact Roles To A Typed Contract Table

Current evidence:

- Event payloads expose artifact evidence.
- Store keeps a near-parallel evidence representation with producer and semantic fields.
- Runtime classifies roles into staged bindings, side-effect phases, retained artifacts, and same
  commit requirements.
- Runtime history and commit validation repeat role and producer matching.

Proposed direction:

Introduce an `ArtifactRoleContract` table or typed `ArtifactEvidence<Role, ProducerScope>` model.
Each role should define:

- stable role tag
- required schema id and semantic id behavior
- allowed producer scope
- staging category
- same-commit requirements
- retained artifact behavior
- event payload lens used to extract the role from a payload

Expected impact:

- Removes 2k-4k hand-written LOC across runtime/history/commit/replay/store.
- Eliminates a major class of schema and role drift.

Correctness requirements:

- Preserve same-commit admission rules.
- Preserve producer scoping.
- Keep config, spec, certificate, and retained-artifact exceptions explicit.
- Do not make artifact metadata authoritative when the append-only stream remains the authority.

Verification:

- artifact role matrix golden tests
- artifact tamper tests
- store projection and rebuild tests
- replay tests for missing, wrong-role, and wrong-producer artifacts

### 3. Make Side-Effect Execution A Generic FSM And Driver

Current evidence:

- Side-effect event variants are generic, but phase legality is repeated in events, runtime, store,
  replay, and tests.
- EVM lifecycle runner phase logic is currently in `crates/app/src/evm_contracts.rs`.
- Proof implements a similar protocol in `crates/transports/proof/src/lib.rs`.
- `SideEffectState` already models domain associated types such as intent, idempotency input,
  submission, receipt, and confirmation, but runners still manually assemble most saga events.

Proposed direction:

Split this into two layers:

1. A typed side-effect FSM that owns legal transitions, terminal classes, required artifacts,
   resource-lane preconditions, replay classification, and uncertainty boundaries.
2. A reusable side-effect driver that owns the common runner protocol:
   - intent persistence
   - claim and fencing token creation
   - invocation prepared/started transitions
   - submission observed/unknown/not-submitted transitions
   - receipt observed
   - confirmation observed
   - ambiguity/failure handling
   - retained artifact construction for standard phases

Adapters should implement only the domain callbacks:

- build intent
- build idempotency input
- prepare invocation
- reconstruct prepared invocation
- submit
- read receipt or recovery evidence
- build confirmation
- map confirmation to state output

Expected impact:

- Removes hundreds of lines from each side-effect adapter.
- Could remove 3k-8k LOC over time if EVM, proof, future Bitcoin, and future exchange adapters all
  share the same protocol driver.
- More importantly, the durable side-effect reliability model becomes one verified protocol.

Correctness requirements:

- Runtime/store remain the commit authority. The driver must only return runner payloads and staged
  artifacts.
- No signed raw transactions or secret-bearing data may cross typed semantic surfaces.
- Resource claims, idempotency keys, replay verifier ids, ledger purposes, and invocation epochs
  must remain explicit.
- Replay semantics must not depend on live providers.
- Ambiguous submission and unknown receipt states must remain first-class states, not errors hidden
  inside adapter code.

Verification:

- `cargo test -p mfm-store --test commit_contract side_effect`
- `cargo test -p mfm-runtime side_effect`
- `cargo test -p mfm-adapters-evm-contracts`
- `cargo test -p mfm-transports-proof`
- side-effect replay, resume, ambiguity, and recovery tests

### 4. Create A Shared Committed-Run Authority View

Current evidence:

- Store builds projection snapshots from committed stream envelopes.
- Runtime rebuilds history views and separately extracts configs, artifacts, side effects, public
  outputs, and completion state.
- Replay rebuilds projection and then builds its own indexes.
- App validates run streams and builds status/output views through separate helper paths.

Proposed direction:

Expose a reusable `CommittedRunIndex` or `VerifiedRunHistoryView` built by folding committed
envelopes once. It should include:

- config, spec, and certificate refs
- artifact indexes
- fact indexes
- side-effect ledger state
- public output index
- resource-lane state
- completion and failure state
- validation diagnostics and corruption classes

Runtime and replay can wrap this shared view differently:

- runtime gets scheduling and resume authority
- replay gets evidence-only authority
- app gets status, stream, and public-output presentation helpers

Expected impact:

- Removes 4k-7k LOC across store/runtime/replay/app over time.
- Reduces the risk that replay, runtime, and app disagree about a run.

Correctness requirements:

- Projections remain derived, not independent authority.
- Replay must never gain live capabilities, stores, signers, or app assembly from the shared view.
- Full authoritative stream validation must happen before range filtering or presentation.
- Public output remains render-only authority.

Verification:

- corruption fixture tests against store, runtime, replay, and app
- old/new differential projection tests
- route parity tests for resume/status/public output/replay
- tampered stream/spec/artifact rejection tests

### 5. Promote Certified Graph Views To Typestate Authority

Current evidence:

- `CertifiedRuntimeSpec` already indexes certified nodes and cells.
- Program `StateNodeSpec` and persisted `NodeSpec` mirror many fields.
- Certification lowers draft graphs into certified vectors, then runtime and replay rebuild lookup
  indexes.
- Framework lifecycle nodes are sealed framework states, but their roles are still branch-matched
  repeatedly.

Proposed direction:

Introduce shared indexed `SpecGraph<Stage>` views for:

- draft/lowered
- validated
- certified
- runtime
- replay

Add a static `FrameworkNodeContract` table for:

- framework node kind
- config kind
- descriptor identity
- receipt schema and semantic ids
- node-id derivation
- lifecycle ordering
- built-in runner binding

Expected impact:

- Removes 3k-5k LOC from certify/runtime/replay over time.
- Reduces raw vector scans and framework lifecycle branch drift.

Correctness requirements:

- Preserve private constructors and certified authority.
- Preserve hash-defining spec bytes.
- Genesis/bootstrap framework nodes must stay explicit.
- Replay graph views must remain evidence-only.

Verification:

- certified spec hash golden tests
- framework lifecycle topology corruption tests
- authority UI tests that prevent raw spec/envelope construction paths
- `cargo test -p mfm-certify`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`

### 6. Extract A Runtime Runner Kit

Current evidence:

- Descriptor registration and binding repeat across proof transport, portfolio adapter, and EVM
  app runners.
- Runner code repeats typed input materialization, canonical artifact staging, `CellProduced`,
  `FactRecorded`, public output projection, and retention refs.
- This code is runtime protocol ceremony, not domain logic.

Proposed direction:

Create a small `runner-kit` crate or kernel/runtime support module for:

- typed runner registration
- executable descriptor binding
- node input materialization
- canonical JSON artifact staging
- staged artifact retention refs
- `CellProduced` event construction
- `FactRecorded` event construction
- public output projection helpers
- golden comparison helpers for emitted runner payloads

Expected impact:

- Removes 500-900 LOC now across proof, portfolio, and EVM runners.
- Becomes more valuable as adapter count grows.

Correctness requirements:

- Preserve explicit adapter binding.
- Preserve typed schema checks and canonical JSON hashing.
- Avoid a loose `serde_json::Value` runner abstraction.
- Do not move domain execution semantics into app.

Verification:

- golden tests comparing emitted runner events and artifacts before and after
- `cargo test -p mfm-adapters-portfolio`
- `cargo test -p mfm-transports-proof`
- `cargo test -p mfm-adapters-evm-contracts`
- replay tests for emitted artifacts

### 7. Add Program Package And State Declaration Surfaces

Current evidence:

- Workflow op crates repeat state registry, operation registry, certification descriptor
  registration, draft compilation, config artifact selection, and public output binding.
- State crates repeat `StateSpec` identity boilerplate, adapter bindings, version strings, and
  capability identity.

Proposed direction:

Add two related surfaces:

1. A framework-owned `ProgramPackage` or `WorkflowDescriptor` builder for:
   - state registry
   - operation registry
   - certification descriptor registration
   - deterministic config artifact selection
   - draft compilation
   - public output binding
2. A `StateSpec`/capability derive for:
   - namespace
   - state name
   - version
   - effect
   - config/input/output schema
   - adapter binding
   - capability kind

Expected impact:

- Removes 500-1100 LOC across current op/state/adapter slices.
- More importantly, future workflow crates become mostly deterministic topology and domain code.

Correctness requirements:

- Generated descriptors must be byte-for-byte compatible unless a deliberate version bump is made.
- Graph expansion semantics must remain visible in op crates.
- Config refs, scope ids, semantic ids, and certified spec contents must remain explicit.
- Macro diagnostics must stay clear and test-backed.

Verification:

- descriptor-id golden tests
- certified spec fixture comparisons
- proc-macro and trybuild tests
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

### 8. Treat Test Scenarios As First-Class Specifications

Current evidence:

- Runtime tests are large because they hand-mutate certified specs and event streams.
- Store contract tests hand-build many event payloads.
- Integration support carries reusable but verbose run fixtures.
- Proof conformance and replay tests duplicate stream corruption helpers.
- Trybuild harness setup repeats across kernel crates.

Proposed direction:

Create declarative scenario builders for:

- certified spec fixtures
- side-effect ledger timelines
- stream corruption cases
- retention artifacts
- public-output render paths
- manual resolution prefixes
- backend store contract cases
- CLI/REST public contract scenarios
- keystore tamper matrices

The target is not to hide assertions. The target is to make each test describe the behavior under
test instead of retyping the kernel setup.

Expected impact:

- Potentially removes 5k-12k test LOC.
- Makes new correctness tests cheaper to add.
- Makes legal/illegal transition matrices visible as data instead of scattered hand-built streams.

Correctness requirements:

- Builders must drive the real store/runtime/replay APIs, not synthetic projection mutation.
- Builders must expose all semantically important fields when a test needs them.
- Do not bury assertions about secrets, output stability, or append-only stream behavior.
- Keep negative tests precise and readable.

Verification:

- `cargo test -p mfm-runtime --lib`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-integration-tests --test proof_transport_conformance`
- affected trybuild tests with no unexpected `.stderr` churn

### 9. Collapse Duplicate Service Surfaces

Current evidence:

- `RunServices` and `AsyncRunServices` duplicate app behavior.
- Sync and async store surfaces create a repeated architectural axis.
- CLI and REST repeat launch-and-render, stream range filtering, and response-envelope logic.
- REST rebuilds production service wiring in request paths.

Proposed direction:

Make async services primary and provide an async in-memory store adapter. Move common
transport-neutral behavior into `mfm-app`:

- production service builder
- verified run context loader
- launch-and-maybe-render helper
- stream range filter
- public output rendering helper
- response envelope types if a shared public JSON contract is desired

Expected impact:

- Removes about 300-700 LOC.
- More importantly, keeps CLI and REST output semantics aligned and forces all routes through one
  verified authority path.

Correctness requirements:

- Validate full authoritative streams before range filtering.
- Public output remains render-only authority.
- Do not leak HTTP status concerns into CLI JSON.
- Async locking and memory-store behavior must stay explicit.

Verification:

- `cargo test -p mfm-app`
- `cargo test -p mfm`
- REST integration tests

### 10. Reconsider Projection Persistence Boundaries

Current evidence:

- The design says projections are rebuildable and not authority.
- Store owns a broad projection snapshot.
- Postgres has projection tables and loader/rebuild paths.

Proposed direction:

Explore whether projections should be:

- purely ephemeral indexes built from append-only committed streams, or
- one compact persisted read-model snapshot with explicit rebuild verification

This is not a first migration target, but it is a fundamental architectural axis. It may reduce
store complexity if run status and resource-lane performance can stay acceptable.

Expected impact:

- Potentially large store simplification, but riskier than the contract/codegen work.
- Could remove a repeated persistence layer if compact read models replace many specialized
  projection tables.

Correctness requirements:

- Append-only streams remain the source of authority.
- Resource-lane concurrency behavior must remain correct.
- Status and resume performance must be measured, not assumed.
- Postgres parity must remain explicit.

Verification:

- store contract tests
- Postgres parity tests with `DATABASE_URL`
- projection rebuild differential tests
- runtime resume/status performance checks

## Deep-Dive Exploration Backlog

This section expands the ten bets into implementation-shape notes. The intent is to make each bet
ready to become a smaller focused RFC. These notes are still docs-only planning; they deliberately
avoid committing to concrete APIs before golden baselines exist.

### Cross-Cutting Acceptance Gates

Any refactor in this RFC should satisfy these gates before old code is removed:

- descriptor ids, schema ids, event names, artifact role tags, spec hashes, public-output schemas,
  and config artifact identities remain byte-stable unless explicitly versioned
- generated or table-driven behavior is compared against the existing hand-written behavior before
  replacement
- converted tests still exercise production store/runtime/replay/app entry points
- full stream validation happens before stream filtering, status rendering, replay, resume, or
  public-output rendering
- public outputs remain render surfaces, not authority
- projections remain derived or verified caches, not independent authority
- replay paths stay evidence-only and cannot acquire live providers, signers, stores, transports,
  registries, or app builders
- app continues to assemble services and capabilities, not own domain execution semantics
- no helper crate weakens architecture namespace or cargo metadata contracts
- no generated fixture or scenario data contains floats in hash-defining surfaces or secrets in
  persisted/logged surfaces

### Deep Dive: Kernel Contract Protocol

Target shape:

- one v1 kernel protocol declaration treated as typed authority for event payload families, spec
  DTOs, nested schema descriptors, artifact lenses, role tags, store codecs, and projection inputs
- generated or derived Rust, not a replacement of Rust types with loose schema data
- declaration-owned event tags, field names, cardinality, enum variants, canonical JSON
  encoder/parser contracts, event-to-artifact requirement lenses, store payload codecs, projection
  inputs, and baseline fixture builders

Ownership boundary:

- the contract source should stay in kernel crates and remain domain-free
- `mfm-store` remains commit authority; generated code can describe shape and requirements, but
  must not admit commits by itself
- runtime and replay can consume generated views, but replay remains evidence-only
- persisted schema names, field names, enum tags, and canonical JSON bytes are persisted contracts

Migration slice:

1. Add goldens for event schema descriptors, role tags, payload canonical JSON, spec canonical
   JSON, spec hashes, and event artifact requirements.
2. Introduce the declaration in check-only mode and assert that it reproduces current descriptors,
   codecs, requirement lenses, and canonical bytes.
3. Migrate one small event family end to end with old/new differential tests.
4. Move store codec parse/encode and artifact requirement extraction behind generated APIs.
5. Move projection fold inputs behind generated transition descriptors.
6. Delete hand-written mappings only after byte-identical goldens pass or after an explicit v2
   persisted-format migration.

Hard invariants:

- every payload field that carries artifact evidence has exactly one generated requirement lens
- every enum variant accepted by parsers appears in schema descriptors and role/tag goldens
- existing authority stages remain intact: untrusted bytes to typed data to certification to
  runtime/store/replay authority
- private constructors and certified authority types remain non-forgeable

Open questions:

- should the contract source be a Rust macro, checked schema file plus committed generated Rust, or
  build-time generator?
- should ownership live in `mfm-events`, a new kernel contract crate, or split event/spec
  declarations?
- should the `typed_spec_certificate` descriptor omission be corrected in v1 with golden churn, or
  treated as a v2 schema migration?
- how much projection behavior should be generated versus only generating typed transition inputs?

### Deep Dive: Artifact Role Contract Table

Target shape:

Introduce `ArtifactRoleContract` as the single table behind artifact role tags, producer rules,
schema/semantic requirements, runtime staging classes, retention behavior, replay authorization,
and event requirement matching.

| Role group | Schema / semantic contract | Producer scope | Runtime / replay contract |
|---|---|---|---|
| `typed_execution_spec`, `typed_spec_certificate` | no value semantic id; schema absent today | none | launch evidence from `RunStarted`; retained as run-start authority |
| `typed_config` | schema required; semantic absent | none | launch/config `ArtifactReferenced`; materialized only after certified verification |
| `seed_input` | schema and semantic required | seed only | `RunStarted.seed_cells`; seed materialization |
| `state_output` | schema and semantic required | node only | staged state output; `CellProduced` plus same-commit terminal attempt completion |
| `fact_response` | response schema required; semantic absent | node only | read evidence from `FactRecorded`; replay reads recorded fact only |
| `public_output` | public schema required; semantic absent | render node | rendered cache artifact; not resume/replay authority by itself |
| side-effect phase roles | phase schema required where payload carries one; semantic absent | node only | side-effect phase evidence; ambiguity remains paired with same-commit attempt failure |
| manual resolution roles | certified manual schemas required; semantic absent | none | manual prefix proof authority only after signature/quorum verification |
| `redacted_diagnostic` | schema required when carried as artifact evidence; semantic absent | node when emitted by attempt | diagnostic evidence only; must remain redacted |
| `retention_manifest` | schema and semantic absent | framework retention node when produced | `RetentionManifestProjected` plus same-commit retention refs |

Ownership boundary:

- the table describes evidence contracts; it does not make artifact metadata independent authority
- artifact stores verify bytes and metadata, but the append-only stream remains authority for
  whether evidence is admissible
- runtime staging, store admission, replay authorization, artifact capability requests, and
  filesystem artifact-store metadata parsing should use the same table
- role exceptions should be table rows, not scattered `match` fallthroughs

Migration slice:

1. Add the table behind existing role string APIs and assert identical tags.
2. Add a role matrix golden covering tag, schema policy, semantic policy, producer scope, staging
   class, retention class, and same-commit policy.
3. Route event requirement generation through role contracts.
4. Route runtime staging and side-effect phase classification through role contracts.
5. Route replay artifact authorization and artifact-store metadata validation through role
   contracts.
6. Remove duplicate role string parsers and role classification matches after differential tests
   pass.

Hard invariants:

- same-commit admission rules for terminal cells, public-output receipts, side-effect ambiguity,
  and retention manifests are preserved
- producer scoping rejects node/seed swaps and mixed producer evidence
- config, spec, certificate, manual-resolution, diagnostic, and retention exceptions stay explicit
- signed raw transactions and bearer mutation material never become retained typed artifacts

Open questions:

- should `ArtifactRoleContract` live in `mfm-events` with role tags, or closer to `mfm-store`
  admission rules?
- should schema policy distinguish `absent`, `required`, `optional`, and `phase-derived`?
- should retention categories be part of this table now, or added after retention manifest
  generation?

### Deep Dive: Side-Effect FSM And Driver

Target shape:

- `SideEffectFsm`: framework-owned transition model derived from the same legal phases as
  `mfm-store::SideEffectLedgerState`; no IO, no commits, no domain callbacks
- `SideEffectDriver<S, A>`: adapter-facing helper that inspects current ledger projection and
  returns runner output proposals
- `SideEffectDomainAdapter<S>`: typed callbacks for prepare/reconstruct invocation, submit or
  recover submission, read receipt, build confirmation, and map domain errors
- `SideEffectEvidenceBuilder`: canonical artifact and payload builder for intent, prepared
  invocation, submission, not-submitted proof, unknown submission, receipt, confirmation, ambiguity,
  failure, and final output
- state-owned behavior stays in `SideEffectState`: intent, idempotency input, and output from
  confirmation

Ownership boundary:

- the driver proposes runner payloads only; runtime/store still validate, stage, and commit
- the FSM must not become a parallel authority; it should wrap or be generated beside store
  typestate
- live providers remain in adapters/transports
- replay uses only retained evidence
- raw signed transactions and secret material remain transient below typed surfaces
- app may assemble runners, but EVM lifecycle execution logic belongs in
  `crates/adapters/evm-contracts`

Migration slice:

1. Extract runner-kit artifact/payload helpers first, with no behavior change.
2. Add FSM transition tests over existing side-effect ledger phases.
3. Port deterministic proof side-effect runner to the driver and compare emitted event/artifact
   goldens.
4. Move EVM lifecycle runners from app into the EVM adapter and preserve executable identity, or
   version it deliberately.
5. Port EVM deploy/configure to the driver, including resume from started, unknown, submission,
   receipt, and confirmation phases.
6. Add not-submitted, ambiguity, touched-set, and remediation cases before using the driver for new
   side-effect adapters.

Hard invariants:

- `InvocationStarted` remains the durable uncertainty boundary
- side-effect output is legal only after confirmation
- idempotency input, ledger key, claim owner, fencing token, epoch, and ledger purpose remain
  explicit evidence
- forward ambiguity and terminal side-effect failure keep same-commit attempt-failure rules
- resource lanes and saga engagement remain store/runtime owned
- canonical JSON hashes, schema ids, semantic ids, and artifact roles remain byte-stable

Open questions:

- should claim owner and fencing token generation move fully behind runtime/store APIs, or stay as
  deterministic driver proposals validated by store?
- should prepared-invocation artifacts require schema ids everywhere?
- should v1 support one ledger per side-effect node attempt, or multiple ledgers before a concrete
  use case lands?
- how should replay verifier identity be registered: adapter callback, side-effect contract field,
  or runner binding metadata?

### Deep Dive: Shared Committed-Run Authority View

Target shape:

- promote the existing committed stream plus runtime history fold into one sealed
  `VerifiedRunHistoryView` consumed by runtime, replay, and app
- expose typed indexes for run-start evidence, config/spec/certificate refs, seed cells, artifact
  refs, facts, attempts, cells, side-effect ledgers, resource lanes, public outputs, retention,
  completion, and validation diagnostics

Ownership boundary:

- store owns envelopes, sequence/ordinal order, atomic commit grouping, artifact admission, and
  projection rebuild
- runtime adds certified-spec-aware execution and resume validation
- replay consumes an evidence-only wrapper and must not gain live capabilities, stores, signers,
  app builders, or transport construction
- app receives presentation helpers only

Migration slice:

1. Add old/new differential goldens around committed streams, runtime run views, replay broker, and
   app status/output paths.
2. Extract a shared fold behind current APIs.
3. Route replay and app loaders through the shared fold.
4. Remove duplicate projection and index construction from replay and app.

Hard invariants:

- spec hash, run id, committed artifact requirements, retained bytes, and `RunStarted` evidence
  agree before authority is minted
- raw event vectors should not cross runtime/replay/app boundaries as trusted input
- projections remain derived
- validation diagnostics are stable enough for tests, but not necessarily public API unless
  deliberately promoted

Open questions:

- should the shared fold live in `mfm-runtime`, `mfm-store`, or a new kernel history crate?
- should resource-lane state be part of the run-local view or exposed as separate store admission
  state?
- should diagnostics be stable public error classes or internal validation reasons?

### Deep Dive: Certified Graph Typestate Views

Target shape:

- generalize existing certified graph and runtime spec views into `SpecGraph<Stage>` with private
  constructors for draft/lowered/validated/certified/runtime/replay stages
- keep raw `TypedExecutionSpec` as hash-defining persisted data
- make graph views typed indexes over validated data, not new persisted data
- add `FrameworkNodeContract` for bootstrap, render, retention, complete, resolve, and bridge
  framework nodes

Ownership boundary:

- certify mints `SpecGraph<Certified>` and framework lifecycle authority from registry-validated
  specs
- runtime derives `SpecGraph<Runtime>` with scheduler indexes and erased runner lookup
- replay derives `SpecGraph<Replay>` with evidence-only lookup
- callers should not scan raw spec vectors when a certified view exists

Migration slice:

1. Introduce stage marker types and wrap the existing certified graph.
2. Move runtime node/cell/topological indexes behind `SpecGraph<Runtime>`.
3. Replace repeated framework `match` logic with `FrameworkNodeContract`.
4. Tighten UI tests against raw graph construction.

Hard invariants:

- stage-changing conversions require prior authority objects
- spec canonical bytes and spec hashes do not change
- framework genesis/bootstrap and lifecycle tail nodes remain explicit certified nodes
- replay views cannot expose runner invocation or live capability construction

Open questions:

- should `SpecGraph<Runtime>` own cloned indexes or borrow from `SpecGraph<Certified>`?
- should remediation links become first-class graph edges?
- should framework contracts be static Rust data, generated from the kernel protocol source, or
  derived from spec helpers?

### Deep Dive: Runtime Runner Kit

Target shape:

- `RunnerOutputBuilder`: typed construction of runner output batches
- `RunnerArtifactBuilder`: canonical JSON bytes, digest, artifact id, evidence, staging, and
  retention refs
- `RunnerPayloadBuilder`: helpers for `CellProduced`, `FactRecorded`, public-output payloads, and
  side-effect evidence payloads
- `RunnerInputLoader`: optional adapter-side helpers for certified config and materialized input
  cell reads
- `RunnerRegistrationBuilder`: typed descriptor lookup plus explicit executable identity binding
- `RunnerGolden`: deterministic golden view of staged artifacts and runner payloads

Ownership boundary:

- the kit reduces ceremony; it does not schedule, write stores, mint certified specs, or bypass
  commit planning
- keep `RunnerEventPayload` as the only runner payload surface
- avoid loose `serde_json::Value` execution
- adapter binding remains explicit and reviewable
- split kernel-safe builders from adapter-only IO helpers if dependency direction requires it

Migration slice:

1. Extract private helpers matching current proof/portfolio/EVM output exactly.
2. Port proof read/pure runners and lock output goldens.
3. Port portfolio read/pure runners.
4. Port EVM validate and mutation final-output helpers.
5. Add registration helpers only after executable identity preservation is covered by tests.
6. Delete duplicated local artifact, retention, cell, and fact builders.

Hard invariants:

- artifact evidence role, producer node, byte length, digest, schema id, and semantic id are
  unchanged
- `CellProduced` matches the certified output cell
- facts preserve request/response hashes, capability ids, adapter ids, and fact keys
- retention refs remain tied to staged artifacts and same-commit evidence
- framework lifecycle events remain runtime-derived, not runner-authored

Open questions:

- should this live as `crates/kernel/runtime` support, a new `crates/adapters/runner-kit`, or a
  split of both?
- should input loading be included, or should the first kit only build artifacts/payloads?
- should side-effect payload builders live in this kit or only in the side-effect driver?
- how should executable identity construction be made reusable without hiding replay authority?

### Deep Dive: Program Package And State Declarations

Target shape:

- `ProgramPackage` in `mfm-program`: authoring-time package metadata, state/operation type lists,
  registry snapshots, root scope defaults, public-output binding keys, and draft builders
- certifier helpers in `mfm-certify`: register package descriptors into `CertificationRegistry`,
  certify drafts, and gather launch config artifacts required by the certified spec
- `StateSpec` derive in `mfm-program-derive`: emit stable state kind/version/name, effect, caps,
  adapter binding ids, and default `ValidatedConfig<T>::into_inner()` construction where applicable

Ownership boundary:

- package declarations belong to operation/authoring surfaces, not app, runtime, store, CLI, or REST
- state declarations belong to state crates and may name adapter identities, but must not
  instantiate adapters, transports, signers, stores, or live IO
- `mfm-program` must not depend on `mfm-certify`; certifier convenience lives in `mfm-certify` or
  op crates
- package helpers must not infer graph semantics from naming; scope ids, operation keys, seed keys,
  public-output keys, saga policy, and config refs remain explicit

Migration slice:

1. Capture descriptor maps, package registry summaries, certified spec hashes, config artifact
   sets, and public-output schema goldens.
2. Add hand-written `ProgramPackage` wrappers around proof first, then portfolio, then EVM, with no
   descriptor changes.
3. Centralize descriptor registration and config artifact selection.
4. Introduce `StateSpec` derive only after descriptor-id goldens exist.
5. Require trybuild diagnostics and explicit opt-outs for custom constructors.

Hard invariants:

- descriptor ids, spec hashes, schema ids, event names, role tags, and config artifact identities
  remain byte-stable unless deliberately versioned
- graph expansion semantics remain visible in op crates
- generated descriptors are deterministic and sorted for golden review
- macros provide clear diagnostics for ambiguous or forbidden declarations

Open questions:

- should package declarations be macros, typed builders, or generated checked Rust?
- how should package `source_package_refs` be derived without build-environment nondeterminism?
- should config artifact selection live in `mfm-certify`, `mfm-spec`, or an op-support crate?

### Deep Dive: Scenario And Test DSL

Target shape:

- `ProgramScenario`: typed package, seeds, saga policy, expected descriptors, expected public
  outputs
- `RunTimelineScenario`: scheduler steps, attempt outcomes, side-effect phase timeline, retained
  artifacts
- `CorruptionScenario`: explicit stream/artifact/spec mutation plus expected rejection class
- `PublicContractScenario`: CLI/REST/status/public-output inputs plus expected stable JSON
- `StoreBackendScenario`: same logical commit sequence against memory and Postgres backends
- `KeystoreTamperCase`: setup, mutation, operation phase, expected error, and file-unchanged
  assertion

Ownership boundary:

- scenario builders are dev/test-only; production crates must not depend on scenario crates
- the DSL must drive `prepare_run_launch`, scheduler execution, prepared commits, store append APIs,
  replay authority, and public-output rendering
- synthetic stream edits are allowed only for corruption, migration, repair, and low-level store
  contract fixtures
- scenario builders must not mutate projections except in explicitly named repair/corruption tests

Migration slice:

1. Add data-only scenario specs and crate-local helpers without creating dev-dependency cycles.
2. Convert proof transport conformance corruption helpers first.
3. Convert side-effect ledger and replay rejection tests.
4. Convert public-output and backend store contract cases.
5. Delete old fixture plumbing only after old/new differential tests prove equivalent streams,
   projections, replay results, and public JSON.

Hard invariants:

- scenario builders expose semantically important fields for negative tests instead of burying them
  behind defaults
- negative coverage preserves wrong role, wrong producer, missing artifact, duplicate side-effect
  transition, replay-with-live-capability, tampered spec/certificate, and secret-redaction cases
- generated fixtures obey canonical JSON and no-float/no-secret rules

Open questions:

- where should shared scenario crates live without creating runtime/store dev-dependency cycles?
- how many old hand-written tests remain as permanent parity sentinels after scenario conversion?
- should generated compile-fail matrices be checked in or generated during test setup?

### Deep Dive: Duplicate Service Surface Collapse

Target shape:

- make async app services primary
- provide an async in-memory store adapter for tests/local tools
- collapse `RunServices` and `AsyncRunServices` into one service facade once parity is proven
- extract transport-neutral helpers for production service construction, verified run context
  loading, launch-and-drive, stream filtering, replay verification, public-output authority, and
  status rendering

Ownership boundary:

- `mfm-app` assembles certified specs, stores, artifact stores, schedulers, registries, and
  capability bindings
- CLI and REST map inputs/outputs and error classes around the same app helpers
- HTTP status mapping stays outside shared CLI JSON contracts

Migration slice:

1. Add async in-memory store support.
2. Introduce `VerifiedRunContext` that loads stream, verifies certified bundle, constructs runtime
   spec authority, verifies retained artifacts, and returns the shared run view.
3. Port sync and async methods to the helper.
4. Move CLI and REST to the unified service.
5. Remove or deprecate the sync facade after route parity.

Hard invariants:

- full authoritative stream validation precedes stream range filtering
- public output is minted only through public-output read authority
- output schemas and CLI JSON remain stable unless changed deliberately
- async locking and transaction boundaries stay explicit

Open questions:

- should shared response envelope types live in app or stay per transport?
- is a temporary sync compatibility wrapper worth keeping?
- should drive/resume use borrowed store handles or owned async service methods only?

### Deep Dive: Projection Persistence Boundary

Target shape:

Split projections into three categories:

- authoritative folds rebuilt from run streams
- optional persisted read models for status/query speed
- store-owned admission indexes needed for concurrency, such as resource lanes

Ownership boundary:

- `typed_run_events` remains semantic authority
- persisted projections are caches unless verified against stream head, fold version, and spec hash
- resource-lane persistence may remain a store admission structure because it protects cross-run
  exclusivity, but it still derives from committed side-effect evidence

Migration slice:

1. Measure status, resume, and replay cost from stream-only rebuilds.
2. Add differential checks comparing persisted projection tables to stream rebuilds.
3. Prototype either ephemeral run-local projections or one compact persisted snapshot with
   `{run_id, head_seq, fold_version, spec_hash, snapshot_json}`.
4. Keep resource lanes separate until concurrency behavior is proven.

Hard invariants:

- append-only streams and atomic commits remain the source of truth
- projection corruption is repairable by rebuild
- resume, replay, public output, retention, and side-effect status never trust projection rows alone
- Postgres parity remains explicit

Open questions:

- what latency target justifies persisted read models?
- which projection families are queried independently enough to deserve tables?
- can retention and garbage collection use verified stream folds at scale?
- should compact snapshots be canonical JSON with golden hashes or typed row families with schema
  checks?

### Deep-Dive Validation Matrix

| Bet | Baseline before refactor | Replacement gate | Focused checks |
|---|---|---|---|
| Kernel contract protocol | event schema descriptors, enum tags, payload JSON, spec JSON, spec hashes | generated descriptors/codecs/lenses are byte-identical or deliberately versioned | `cargo test -p mfm-events`, `cargo test -p mfm-spec`, `cargo test -p mfm-store` |
| Artifact role table | role tag parser/encoder behavior, schema/semantic policy, producer policy | role matrix table drives event requirements, staging, replay, and artifact-store metadata | artifact tamper tests, replay missing/wrong-role tests, store rebuild tests |
| Side-effect FSM/driver | side-effect transition matrix and runner output goldens | driver emits the same runner payload/artifact sequence and preserves ambiguity/recovery states | `cargo test -p mfm-store --test commit_contract side_effect`, `cargo test -p mfm-runtime side_effect` |
| Shared run view | committed-stream folds, runtime views, replay broker behavior, app route outputs | runtime, replay, and app consume one verified fold without gaining extra authority | corruption fixtures, route parity tests, replay authority tests |
| Certified graph typestate | certified spec hashes, descriptor ids, framework topology | staged graph views replace raw scans without changing hash-defining bytes | `cargo test -p mfm-certify`, runtime/replay parity, authority UI tests |
| Runner kit | proof/portfolio/EVM runner output and artifact goldens | helpers reproduce identical staged artifacts, payloads, retention refs, and executable identities | `cargo test -p mfm-transports-proof`, `cargo test -p mfm-adapters-portfolio`, `cargo test -p mfm-adapters-evm-contracts` |
| Program/package declarations | descriptor maps, registry summaries, certified spec hashes, config artifact sets | package helpers and derives produce identical registries/specs before boilerplate removal | `cargo test -p mfm-program`, `cargo test -p mfm-program-derive`, `cargo test -p mfm-certify` |
| Scenario DSL | existing runtime/store/replay/proof/CLI/REST fixtures | scenario cases drive production APIs and preserve every negative case class | runtime tests, store contract tests, proof conformance, CLI/REST integration tests |
| Service surface collapse | sync/async service parity, CLI/REST route outputs | unified async service preserves status, stream, replay, resume, and output behavior | `cargo test -p mfm-app`, `cargo test -p mfm`, REST integration tests |
| Projection boundary | projection-table rebuild parity and status/resume timings | persisted projections are either removed or verified as caches against stream head/fold version | store contract tests, Postgres parity tests with `DATABASE_URL`, performance checks |

## Incremental Refactors Worth Doing

These do not solve the large LOC problem by themselves, but they are still good engineering moves.

### Move EVM Lifecycle Runners Into `crates/adapters/evm-contracts`

Move runner structs, phase handling, materialized input loading, and side-effect payload
construction out of `mfm-app`. App should keep env-backed provider construction and registry
wiring only.

Impact: removes about 800-1000 LOC from app assembly, with modest net LOC reduction.

Main risk: executable identity is replay authority. Preserve existing identity bytes or version the
change deliberately.

### Centralize Runner Artifact And Output Emission

Create neutral helpers for canonical JSON artifact creation, staged artifacts, retention refs, and
typed cell output payloads.

Impact: about 250-400 LOC removed across portfolio, proof, and EVM runner code.

### Extract Deterministic Config Artifact Compilation For Ops

Portfolio and EVM ops both compile and validate config artifacts from certified specs. Add a
planning-only op support abstraction.

Impact: about 100-150 LOC removed and fewer config drift risks.

### Move Portfolio Read Capability To A Capability Contract Crate

Create `crates/portfolio-capabilities` for the portfolio read capability, request/response types,
backend trait, and typed errors.

Impact: near-term LOC neutral, but improves reusable boundaries.

### Move Portfolio Fanout And Decimal Arithmetic Into The Domain Model

Expose deterministic observation targets from the validated portfolio model. Move pure decimal
arithmetic out of the state crate.

Impact: modest LOC reduction, but better model ownership and reuse.

### Reuse Store Codec For Artifact Roles

Use `mfm_store::v1::codec` from the filesystem artifact store instead of duplicating
`ArtifactRole` string mappings.

Impact: about 45-55 LOC removed and one fewer persisted metadata drift point.

### Centralize Trybuild And CLI Test Harnesses

Add reusable test-only helpers for trybuild setup, CLI command execution, JSON envelope parsing,
and env sanitization.

Impact: about 200-400 LOC removed across tests.

## What Is Not A Good 30-50% LOC Strategy

### Merging Crates

Merging crates may reduce manifests and imports, but it weakens boundary checks and does not remove
much implementation code. The current crate boundaries largely reflect real authority boundaries.

### Replacing Typed Contracts With `serde_json::Value`

This would reduce code, but it directly violates the design contract. It would trade explicit
authority for dynamic shape checks and weaker compile-time guarantees.

### Deleting Negative Tests

The repo has many compile-fail and tamper tests because authority boundaries are security and
correctness boundaries. Removing them would reduce LOC without reducing complexity.

### Making App A Workflow Host

Putting more reusable behavior into app would reduce adapter/runtime code in the short term, but it
would violate the placement contract. App should assemble, not own domain execution semantics.

## Recommended Roadmap

### Phase 0: Golden Baselines Before Refactoring

1. Add or identify descriptor, canonical JSON, fixture hash, event payload, and certified spec
   goldens for the protocol surfaces that will be generated or table-driven.
2. Add focused drift tests for artifact role tags, schema descriptors, and store codec mappings.
3. Add old/new differential harnesses for projection folds and side-effect phase transitions.

Expected result: the later refactors can be reviewed as authority-preserving rewrites instead of
behavior changes.

### Phase 1: Kernel Contract And Artifact Role Sources

1. Prototype the kernel contract declaration for a small event family.
2. Generate or derive schema descriptors, enum tag tables, artifact requirements, and store payload
   codecs for that family.
3. Add `ArtifactRoleContract` behind existing artifact role APIs.
4. Migrate one contained event/artifact family end to end and compare generated behavior against
   existing goldens.

Expected result: this is the first phase that can plausibly unlock 30%+ LOC reduction.

### Phase 2: Side-Effect FSM And Driver

1. Define the typed side-effect FSM inside the kernel/store authority path.
2. Route runtime, replay, and tests through the FSM for phase classification and transition rules.
3. Define the generic side-effect driver trait surface.
4. Port proof first, because it has a smaller side-effect surface.
5. Port EVM lifecycle after identity, replay, and ambiguity behavior are locked.

Expected result: large reuse for future mutation workflows, and a smaller trusted surface for
side-effect protocol correctness.

### Phase 3: Shared Run And Graph Authority Views

1. Introduce `CommittedRunIndex` or `VerifiedRunHistoryView` as the shared stream fold.
2. Route runtime history, replay, and app status/output paths through the shared view.
3. Expand certified graph read APIs into a staged `SpecGraph<Stage>` surface.
4. Replace repeated framework lifecycle matching with `FrameworkNodeContract`.

Expected result: fewer duplicated correctness checks, less runtime/replay divergence, and a more
obvious authority boundary.

### Phase 4: Runner Kit And Workflow Declarations

1. Extract runner registration, input materialization, artifact staging, and output event helpers.
2. Move EVM lifecycle runner logic into `crates/adapters/evm-contracts`.
3. Add `ProgramPackage` or `WorkflowDescriptor` helpers for registry and certification plumbing.
4. Add `StateSpec`/capability derive only after descriptor-id goldens are in place.

Expected result: cleaner boundaries and lower per-workflow implementation cost.

### Phase 5: Scenario-Based Tests

1. Build runtime certified-spec fixture mutation helpers.
2. Build side-effect timeline factories.
3. Build stream corruption cases and backend store contract runners.
4. Convert large tests incrementally while preserving assertion specificity.

Expected result: large test LOC reduction without reducing coverage.

### Phase 6: Service And Projection Simplification

1. Make async app/store services primary if sync compatibility is no longer buying enough value.
2. Route CLI and REST through shared verified run context helpers.
3. Explore whether projections should remain ephemeral or become one compact verified read model.

Expected result: lower app/store duplication after the kernel authority surfaces have stabilized.

## Verification Baseline

Use focused Cargo checks by default:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mfm-app
cargo test -p mfm-runtime
cargo test -p mfm-replay
cargo test -p mfm-store
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

For live parity behavior, start services manually and run focused parity tests with explicit env
vars. Do not use Nixfied wrappers as the default gate unless the change is specifically about
Nixfied behavior.

## Bottom Line

The fundamental abstraction problem is not that MFM has the wrong runtime model. It is that the
typed kernel protocol is implemented as repeated hand-written Rust across too many layers. The
large LOC win is to make the protocol declarative and generated/derived while preserving the same
certified authority boundaries.

The second large win is to make side-effect execution a generic FSM and driver instead of requiring
every adapter to hand-write the saga event sequence.

The third large win is to fold committed run streams and certified graphs into shared authority
views that runtime, replay, app, and tests all consume.

Everything else is worth doing, but it is supporting structure. The 30%+ thesis lives in contract
declaration/codegen, side-effect protocol reuse, shared authority views, and scenario-driven tests.

## Appendix A: Independent Feasibility Review

This appendix records a skeptical review of the RFC's abstraction bets. It treats append-only
streams, content addressing, canonical JSON, replay evidence, no ambient IO in state logic, and no
secret persistence as hard invariants. The conclusion is intentionally stricter than the main RFC
thesis: LOC reduction is useful only when it removes maintained duplication without hiding
correctness checks in opaque macros or generated files.

### Evidence Summary

- The local Rust LOC baseline is about `139852` LOC. A 30% reduction therefore means roughly
  `42k` net maintained LOC, which is larger than the clearly duplicated production surfaces found
  in this pass.
- The design constraints are explicit in `docs/design.md` and `docs/architecture.md`: certified
  specs, append-only streams, canonical JSON, replay from stored evidence, and no ambient IO in
  state logic are not optional simplification targets.
- There is concrete role-contract drift risk already. `ArtifactRole::TypedSpecCertificate` exists
  in `crates/kernel/events/src/lib.rs`, the store parses and emits `typed_spec_certificate`, and
  `RunStarted` can require the certificate artifact, but the event schema role tag list appears not
  to include `typed_spec_certificate`.
- `VerifiedRunHistory` already exists in `crates/kernel/runtime/src/history.rs`, while
  `ReplayBroker::from_validated_parts` in `crates/kernel/replay/src/lib.rs` still verifies, folds,
  indexes, and authorizes a committed stream again. This supports a shared committed-run authority
  view, but not a single untyped "god object".
- Side-effect state is repeated across store ledger projection, runtime historical validation, and
  app-level EVM runner logic. The duplication is real, but this path is correctness-sensitive
  because it controls replay evidence, idempotency, ambiguity, and non-persistence of signed/raw
  payloads.
- Sync and async app service surfaces are materially duplicated in `crates/app/src/lib.rs`. This is
  feasible cleanup, but it is not a large LOC lever by itself.

### Ranked Feasibility Table

LOC estimates are directional and count maintained Rust. If generated files are checked in and
reviewed, they should be counted as maintained LOC.

| Rank | RFC bet | Feasibility | Estimated LOC impact: production / tests / generated / net maintained | Confidence | Main correctness risk |
|---:|---|---|---|---|---|
| 1 | Scenario-based test specs | High | `+0.3k..+0.8k / -4k..-8k / 0..+2k / -3k..-6k` | Medium | A scenario DSL can hide important negative assertions or make corruption tests less specific. |
| 2 | `ArtifactRoleContract` table | High | `-1.5k..-3k / +0.5k..+1.5k / 0 / -0.5k..-2k` | Medium | A centralized table bug can misclassify schema, producer, or same-commit requirements everywhere. |
| 3 | Shared committed-run authority view | High | `-2k..-4k / +1k..+2k / 0 / -1k..-2.5k` | Medium | Runtime, replay, app, and public-output paths must not collapse distinct authority boundaries. |
| 4 | Runtime runner kit | High | `-0.6k..-1.2k / +0.3k..+0.8k / 0 / -0.2k..-0.8k` | Medium-high | Generic builders can make invalid runner payloads easier to construct if they are too permissive. |
| 5 | Collapse duplicate app service surfaces | High | `-0.4k..-0.8k / +0.2k..+0.4k / 0 / -0.2k..-0.5k` | High | Stream range filtering must remain after full stream validation, not before it. |
| 6 | Declarative kernel protocol | Medium | `-4k..-8k / -2k..-5k / +5k..+12k / -3k..-7k` | Medium-low | Byte stability, canonical JSON stability, schema descriptor stability, and hidden generated code drift. |
| 7 | Generic side-effect FSM and driver | Medium | `-1.5k..-3.5k / +1k..+2k / 0 / -0.5k..-2k` | Medium-low | The driver can obscure replay uncertainty, duplicate store authority, or accidentally persist secret-bearing payloads. |
| 8 | `ProgramPackage` and `StateSpec` derive | Medium | `-0.8k..-1.5k / +0.8k..+1.5k / macro expansion / 0..-0.8k` | Medium | Macro indirection can make descriptor identity and certification failures harder to audit. |
| 9 | Certified graph typestate views | Medium | `-1k..-2k / +0.5k..+1k / 0..+0.5k / -0.5k..-1.5k` | Medium | Much of the authority already exists in `CertifiedRuntimeSpec`; over-layering can add API complexity. |
| 10 | Projection persistence boundary | Low | `0..-2k or +1k / +1k..+3k / 0 / unknown` | Low | Store repair, Postgres parity, resource-lane semantics, and read performance may become more complex. |

### Bet-By-Bet Assessment

#### 1. Declarative Kernel Protocol

This is technically feasible, but it is the riskiest production refactor because it touches durable
hashes, schema descriptors, codec mappings, artifact requirements, and replay validation. The RFC's
large production LOC estimate is plausible only if generated code is not counted as maintained LOC.
If generated Rust is checked in and reviewed, the reduction is much smaller.

Required baselines:

- event schema descriptor goldens
- enum tag list goldens, including artifact roles
- canonical JSON and spec hash goldens
- store encode/decode round trips for every event payload
- artifact requirement matrix by event kind
- replay and runtime equivalence from the same committed stream

#### 2. Artifact Role Contract Table

This is the most actionable production cleanup. It addresses real drift, has bounded scope, and is
useful before any larger generator work. It should encode role tag, schema descriptor, producer
binding rules, same-commit constraints, and whether the artifact is required or optional for each
event surface.

Required baselines:

- one table-driven test for every `ArtifactRole`
- one test proving each event's retained artifacts satisfy the role contract
- negative tests for wrong role, wrong schema, wrong producer, wrong node, wrong attempt, and wrong
  commit binding

#### 3. Generic Side-Effect FSM And Driver

The FSM half is feasible and likely valuable. The generic driver half should be delayed until
existing EVM and proof behavior is captured in goldens. The store must remain the authority for
legal committed transitions; the driver should construct intents and evidence, not become a second
source of truth.

Required baselines:

- side-effect phase transition matrix
- resume-after-each-phase tests
- unknown submission and ambiguity recovery tests
- failed receipt and terminal failure tests
- replay tests proving no live network or filesystem dependency
- tests that raw signed transactions or secret-bearing material never appear in artifacts, events,
  public output, or errors

#### 4. Shared Committed-Run Authority View

This is feasible and worth doing, but the abstraction must be a verified read view, not a relaxed
replacement for store/runtime/replay authority. `VerifiedRunHistory` and replay's validated broker
already show the right shape. The cleanup should remove duplicate folding and indexing while
preserving explicit minting boundaries.

Required baselines:

- old/new status output equivalence
- old/new stream output equivalence
- old/new public output equivalence
- old/new replay evidence equivalence
- tampered stream, tampered artifact, tampered spec, and stale projection tests

#### 5. Certified Graph Typestate Views

This mostly improves consistency and API clarity. It is unlikely to remove much net LOC because the
current certification and runtime spec authority already perform much of the staged validation. The
best use is to prevent accidental mixing of uncertified, certified, and runtime-indexed graph
surfaces.

Required baselines:

- certified spec hash goldens
- lifecycle topology corruption tests
- compile-time or API tests showing uncertified graphs cannot enter runtime paths
- framework lifecycle role matrix tests

#### 6. Runtime Runner Kit

This is feasible and low-risk if kept as builders and helpers for known runner output shapes:
input materialization, staged artifacts, fact events, cell events, and runner registration. It
should not become a generic JSON escape hatch.

Required baselines:

- emitted event and artifact goldens for proof, portfolio, and EVM runners
- descriptor identity goldens
- negative tests for missing artifact hash, wrong schema, wrong cell, and wrong fact bindings

#### 7. `ProgramPackage` And `StateSpec` Derive

This is a maintenance improvement, not a major LOC lever. It should come after descriptor and spec
hash goldens exist. The derive is acceptable only if generated identifiers remain obvious in source
or diagnostics.

Required baselines:

- descriptor ID goldens
- config artifact set goldens
- public output draft goldens
- `trybuild` diagnostics for malformed derives

#### 8. Scenario-Based Tests

This is the strongest net maintained LOC bet, mostly in test code. It should be built as a precise
fixture language for certified specs, committed streams, side-effect timelines, and corruption
cases. The goal is not fewer assertions; it is fewer hand-built copies of the same stream and spec
ceremony.

Required baselines:

- equivalence between current hand-written fixtures and scenario-generated streams
- projection equivalence
- public output equivalence
- replay equivalence
- negative fixture snapshots for every corruption family

#### 9. App Service Surface Collapse

This is clearly feasible but small. It should probably be merged with the shared committed-run view
work. The important rule is that CLI and REST range filtering must consume an already validated
stream instead of becoming a partial validation path.

Required baselines:

- sync and async service parity tests
- CLI and REST response parity tests
- validation-before-filtering tests

#### 10. Projection Persistence Boundary

This should be cut from the main roadmap. It may be worthwhile, but it is not yet a refactor. It is
a storage design investigation with hidden costs in Postgres parity, repair, migration, query
performance, and resource-lane semantics.

Required baselines before any implementation:

- memory/Postgres projection parity tests
- rebuild-from-events diff tests
- query performance baselines
- repair and migration tests

### Recommended Roadmap Changes

1. Start with `ArtifactRoleContract` and its goldens. This is the smallest change that directly
   attacks real protocol drift.
2. Build scenario/golden infrastructure next, then convert one narrow family of existing tests.
   This creates the safety net required before deleting handwritten protocol ceremony.
3. Introduce a shared committed-run read authority and merge the duplicated app service read paths
   around it. Keep store, runtime, replay, and public-output authority wrappers explicit.

After those three steps, extract the runner kit. Then consider the side-effect FSM. Only after those
baselines exist should declarative kernel protocol generation move from "check-only" to replacing
hand-written code.

Recommended cuts and merges:

- Treat `ArtifactRoleContract` as the first slice of the declarative kernel protocol, not as a
  later cleanup.
- Merge shared committed-run view work with app service surface collapse.
- Split side-effect work into two phases: kernel/store FSM first, generic adapter driver later.
- Demote certified graph typestate work unless concrete lifecycle drift appears in the new goldens.
- Move projection persistence to a separate measurement RFC.
- Do not lead with macros or derives. They need descriptor, spec-hash, and diagnostic goldens first.

### Blunt Conclusion

The RFC is directionally right that repeated protocol authority is the main abstraction problem.
However, the 30-50% LOC thesis is too aggressive as a maintained-LOC claim.

The realistic near-term target is **10-20% net maintained LOC reduction** if scenario fixtures,
artifact role contracts, shared committed-run views, runner helpers, and selected service cleanup
all land cleanly. A **30% reduction is plausible only if generated code is excluded from maintained
LOC and large test fixtures are aggressively rewritten without losing coverage**. If generated Rust
is checked in and reviewed, or if goldens add back the necessary coverage, the net reduction will be
lower.

The better success metric is not raw LOC. It is fewer independently maintained copies of durable
protocol truth, with no weakening of append-only stream authority, content addressing, canonical
hashing, replay evidence, no-ambient-IO state logic, or secret non-persistence.
