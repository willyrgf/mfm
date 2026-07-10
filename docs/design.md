# MFM Design Contract

Status: authoritative typed-core design contract

This document defines the runtime and authoring contract for the current MFM codebase. It is the
normative contributor-facing design reference.

The central rule is:

```text
typed state programs are the only semantic executable surface;
certified typed execution specs are the only runtime contract;
erased runner plans are implementation artifacts.
```

Operations may plan typed state programs, but operations are not runtime execution units after
certification. Binaries and app crates assemble stores, artifacts, registries, and capabilities;
they do not own workflow semantics.

## Non-Negotiable Invariants

- New execution uses certified typed execution specs only.
- Runtime state transitions are committed as typed kernel events through `mfm-store`.
- The typed run stream for `run:{run_id}` is append-only and authoritative.
- Store commits are atomic: a commit either appends every payload and updates derived projections,
  or appends nothing.
- Event envelopes, run-local sequence numbers, store-wide append coordinates, ordinals, event ids,
  logical keys, and projections are store-owned. `StreamSeq` orders one run; the durable
  `StoreCommitOrder` totally orders committed appends across the store and is the only authority
  for cross-run fact ordering and fact-query commit watermarks.
- Manifest, config, seed, fact, artifact, output, and spec identities are content-addressed.
- Hashed structured data uses canonical JSON and must not contain floats.
- Typed values, typed configs, public outputs, and event payloads must not contain secrets.
- Replay and resume are driven by the stored certified spec and the authoritative run stream.
- Replay adapters must answer only from recorded facts, typed artifacts, and side-effect evidence.
- Process identity and process topology are not semantic authority. Workers are interchangeable
  executors over the store, and leases/claims are liveness coordination only.
- Admission, drive, verify, and replay must not consult mutable registries or external policy
  oracles; outcome-affecting policy is resolved once into hash-defining certified spec material.
- Side effects use typed intent, typed idempotency input, durable ledger events, and typed receipt
  or recovery evidence.
- Side-effect verification policy is hash-defining certified config. `RunAdmitted` may record
  launch audit evidence, but it is not independent finality or verification authority.
- Certified saga decisions are derived from the certified spec plus append-only stream facts.
- Manual saga resolution requires certified schema authority and a signed authorization proof.
- Public output JSON is a render surface. Typed terminal cells plus public-output specs and events
  are the authority.
- Source scans, naming conventions, hash-only envelopes, persisted summaries, and CI summary keys
  are not typed-core architectural proof or runtime authority.

## Authority Surfaces

Typed-core code distinguishes data, evidence, authority, and implementation artifacts:

| Surface | Runtime authority? | Contract |
|---|---:|---|
| `mfm_spec::UntrustedTypedSpec` | no | Persisted/user bytes decoded into typed Rust data. Hostile until certification verifies the spec against the registry. |
| `mfm_certify::LoweredTypedSpec` | no | Program-lowered draft data. It is the certifier input for authored programs, not runtime authority. |
| `mfm_certify::ValidatedTypedExecutionSpec` | no, certifier-private | Registry-validated spec authority used inside certification to mint certified objects and certificates. It does not cross into runtime as a mutable execution surface. |
| `mfm_spec::v1::HashedSpecEnvelope` | no | Hash-only envelope for canonical spec bytes and non-semantic audit metadata. It cannot certify a spec. |
| `CertifiedSpecCertificate` bytes/evidence | no | Persisted certificate evidence. Hostile until the certifier verifier checks spec hash, certificate hash, registry digest, descriptor identities/digests, lowering/canonicalizer identity, public-output schema id, and audit metadata. |
| `mfm_certify::CertifiedTypedSpec` | yes | Non-forgeable in-memory authority minted only by registry-backed certification or verified persisted spec/certificate evidence. |
| `CertifiedDescriptorSet` / `CertifiedFrameworkLifecycle` | yes, within certified spec authority | Certified descriptor and framework lifecycle views derived from a validated spec. Runtime consumes these views instead of recertifying raw descriptor tables. |
| `mfm_spec::v1::CertifiedContextSpec` / `ContextRef` | no, spec data until certified | Hash-defining transition-context table entries and content-addressed refs. They become execution authority only through certified spec validation plus node, cell, and input context constraints. |
| `mfm_runtime::CertifiedRuntimeSpec` | yes, runtime-only | Runtime wrapper derived only from `CertifiedTypedSpec`; owns scheduler indexes and erased runner derivation. |
| `PreparedCommit<Purpose>` / `PreparedCommitPlan` | yes, store mutation | Purpose-specific commit authority built by runtime/app authority. The store rejects mismatched payload purpose, missing saga proof, and missing admitted artifact evidence. |
| `CertifiedRunStoreAuthority` | yes, store admission | Policy-bound run-start/certified run authority minted from the certified typed spec and tied to run id, certified spec hash, saga policy, and side-effect terminal policies. Store admission checks the token spec hash against the projected `RunAdmitted.spec_hash`, not only the incoming saga payload. |
| `ManualResolutionProofAuthority` / `VerifiedManualResolutionForPrefix` | yes, manual resolution | Prefix-bound proof authority over a certified manual-blocked stream prefix, retained artifacts, canonical proof bytes, and certified operator policy. |
| `CertifiedSideEffectContract` | yes, side-effect verification | Certified resource-claim and side-effect contract authority shared by live execution, resume, and replay. |
| `SideEffectLedgerState` | yes, store transition | Typed ledger state used by store/runtime to admit only legal side-effect transitions. |
| `SagaTerminalProof` | yes, terminal saga | Store-required proof object for completed, compensated, manually resolved, or failed-without-claim terminal saga outcomes. |
| `CommittedRunStream` / `VerifiedRunArtifactStore` | yes, stream/history evidence | Store-owned committed stream authority plus retained-artifact authority tied to that stream. |
| Erased runner plans | no | Runtime implementation artifacts reproducibly derived from certified authority and runner registry. |
| `PublicOutputReadAuthority` | yes, render-only | App authority minted after certified spec/certificate verification and projection rebuild from the authoritative stream. |
| Rendered public-output JSON/artifacts | no | Output/cache material for users and integrations. They cannot authorize resume, replay, or another render. |

Persisted spec bytes, persisted certificate bytes, rendered JSON, and projection rows must be
validated or rebuilt before they influence semantic execution.

## Crate Layout

Typed kernel crates are framework-owned and domain-free:

| Crate | Responsibility |
|---|---|
| `crates/kernel/ids` | Strong identity types for specs, states, events, values, artifacts, runs, and digests |
| `crates/kernel/canonical` | Canonical JSON bytes and content digests |
| `crates/kernel/values` | Typed value, context-bound output, config, artifact reference, and public-output descriptors |
| `crates/kernel/effects` | Framework-owned effect classes |
| `crates/kernel/capabilities` | Capability descriptors, roles, and effect-checked capability sets |
| `crates/kernel/program` | Typed state-program authoring, handles, scopes, registries, lineage, and lowering evidence |
| `crates/kernel/program-derive` | Derives for typed values, configs, state inputs, operation outputs, and public outputs |
| `crates/kernel/spec` | Versioned typed execution-spec data model and hash-only spec envelopes |
| `crates/kernel/certify` | Certification checks, non-forgeable certified typed-spec authority, and persisted certificate verification |
| `crates/kernel/events` | Versioned typed kernel event schemas |
| `crates/kernel/store` | Typed commit API, side-effect ledger rules, projection contract, and retention refs |
| `crates/kernel/manual-auth` | Canonical manual-resolution authorization claims, proofs, and verifier contracts |
| `crates/kernel/runtime` | Certified typed scheduler and erased runner boundary |
| `crates/kernel/replay` | Replay authority, brokers, and verifier contracts |

Domain and product crates sit outside the kernel:

| Area | Responsibility |
|---|---|
| Domain capability contract crates | Typed capability specs, authority traits, request/response evidence types, and redacted error contracts used by state and adapter crates |
| `crates/states/*` | Typed state contracts and deterministic state-owned behavior |
| `crates/ops/*` | Typed operation planners that assemble state programs |
| `crates/adapters/*` | Runner bindings from state intent to capabilities, evidence phases, and domain replay verifiers |
| `crates/transports/*` | Live and replay capability backend implementations |
| `crates/runtime-config` | Runtime-only config parsing, value-source resolution, validation, and redaction |
| `crates/storages/*` | Implementations of typed store and typed artifact contracts |
| `crates/app` | Assembly of registries, stores, artifacts, start/resume/replay, and public output |
| `bin/cli`, `bin/rest-api` | Transport-only user surfaces |

Dependency direction is strict:

```text
ids -> canonical -> values -> effects/capabilities
  -> program/spec -> certify/events/store -> runtime/replay
```

Kernel crates must not depend on domain crates, binaries, app assembly, storage implementations, or
transport implementations. States must not depend on runtime/store implementations, binaries, live
transport implementations, signer provider implementations, or operation crates. States may depend
on capability contract crates because those crates define typed authority contracts, not live IO.
Ops may depend on typed states and domain config/model crates, but not on runtime scheduling or
storage implementations. Adapters may bind states to capability implementations, but must not
depend on workflow operation crates, app assembly, binaries, or storage implementations. Storages
implement storage contracts and know no domain semantics.
Adapters bind state-owned intent to capabilities and evidence phases. Transports implement
capability backends. Neither adapters nor transports mint production authority outside app/runtime
assembly.

## Typed Values And Configs

Typed configs are deterministic planning inputs. They are decoded at boundaries, validated,
canonicalized, and retained by value or content-addressed reference in certified specs.
Run-start rejects config artifacts that match certified metadata but cannot be decoded and validated
through the trusted certification registry or an exact framework-owned config reference.

Typed runtime values cross state boundaries through typed cells and handles. A runtime value cannot
change the topology of the same certified run. If produced data must select future topology, the
workflow must create a new planning boundary such as a child run or a separately certified
continuation.

Persisted values and configs must implement the typed descriptor contracts through the framework
derive path or framework-owned generic constructors. Manual descriptor implementations outside the
framework boundary are rejected by checks because they bypass schema, no-float, and no-secret policy.

Secrets remain below the typed semantic boundary. Private keys, mnemonics, passwords, decrypted
bytes, raw signing material, and signed raw transactions must not be typed values, configs, facts,
artifacts, events, public outputs, error details, or fixtures. States refer to secret-bearing systems
through non-secret labels, references, and capabilities.

Persisted and public surfaces are inventoried in `docs/persisted-public-surfaces.md`; that inventory
is the review checklist for applying this no-secret invariant to app, CLI, REST, storage, artifact,
and diagnostic boundaries.

## Typed Program Authoring

State outputs are represented by branded typed handles. Handles carry the produced Rust value type,
program brand, scope brand, cell identity, schema identity, semantic type identity, and value
lineage. Handles cannot be forged from raw ids.

Scopes are part of certified semantics. Cross-scope same-value movement requires framework bridge
nodes with persisted bridge evidence. Transforming movement is modeled as an ordinary state.

Optional runtime paths use `MaybeValue<T>` cells, not absent context keys. Artifact references use
`ArtifactRef<T>` and must verify digest, schema id, semantic type id, role, and producer evidence
before materialization.

Operations are typed planners. They receive typed config and typed handles, assemble a typed state
program, bind public outputs, and return a draft for certification. Operation lineage and stable
domain keys are recorded in the certified spec so fanout/fanin ordering and semantic identity are
auditable.

## State And Operation Registration

A state type is executable only after framework registration validates:

- state kind and version
- config, input, output, and public descriptor identities
- effect class
- capability set
- side-effect contract when applicable
- runner kind and executable identity

Planning requires state and operation membership in the builder registry. Runtime requires the
certified descriptor identity and the registered runner identity to match the stored spec.

## Effects And Capabilities

Effect classes are framework-owned:

- pure: no external capabilities
- read: read/support capabilities only
- managed platform write: platform persistence/output capabilities only
- apply side effect: exactly one external mutation authority plus allowed support capabilities

The runtime injects only capabilities certified for the current node. State code must not create its
own live network, filesystem, clock, process, or signer access when that access is part of semantic
execution. Domain helpers may compute deterministic values, parse data, or validate typed inputs,
but side effects and replayable observations must pass through typed capabilities.

## State Capability Boundary

States declare authority. Transports implement authority. Adapters bind the two at runtime.

Capability contract crates are part of the typed state-facing contract. They may define capability
specs, request types, response/evidence types, redacted error contracts, and traits that represent
external authority. They must not perform live IO, route endpoints, resolve signer material, or own
workflow topology.

States may depend on capability contract crates. States must not depend on live transport
implementation crates. For example, a contract validation state may depend on an EVM capability
contract that defines an EVM call-read capability, request, and evidence type. It must not depend on
the live JSON-RPC transport that chooses an endpoint, attaches authorization, retries HTTP calls, or
uses a concrete client library.

Runtime/app assembly supplies concrete capability implementations for live execution. Replay
supplies replay implementations backed only by recorded facts, typed artifacts, and side-effect
evidence. Adapters translate state-owned intent into capability calls and evidence phases without
moving protocol IO or signer material into state code.

App assembly keeps evidence-only services separate from live driver services. Status, stream
inspection, list/watch, replay, and public-output rendering construct only store, artifact, and
certification/replay authority; they do not parse live runtime config, construct live EVM transports,
or construct signer providers. Malformed or missing live capability wiring can block live
start/resume when that run needs it, but it must not affect evidence-only reads.

Runtime admission binds each certified capability descriptor to a registered non-secret
implementation identity before the run can start or resume. Missing or mismatched implementation
bindings are deployment/ingress failures, not semantic attempt outcomes.

Replay and resume semantics follow the effect class:

- Pure states replay by recomputing deterministic state behavior.
- Read states replay from recorded read evidence. Replay must not call live transports.
- Side-effect states resume from durable phase evidence such as intent, idempotency, preparation,
  submission, receipt, confirmation, or recovery evidence. Resume must not duplicate external
  mutations or infer mutation status from unstored state.
- Replay never constructs live transports or signer providers.

Portfolio reporting is fact-backed and report-only after collectors cutover: pure report states
recompute from certified config; SelectHoldings recomputes from recorded Platform fact-query
evidence and retained response artifacts only. Public-facts CLI/REST is not report selection
authority. See `docs/portfolio-collect-then-report.md`.

## Certified Saga Semantics

Certified saga behavior is part of the typed runtime contract. The detailed saga and scoped AC/DC
contract is maintained in `docs/saga.md`; this section summarizes the authority rules that every
runtime, store, replay, CLI, REST, and app change must preserve.

MFM implements certified saga semantics for external side effects. It does not claim full AC/DC
semantics for arbitrary external systems. Stronger AC/DC-style claims require MFM-owned
transactional resources or replay-verifiable domain proof from typed evidence. Core saga outcomes
therefore mean exactly what the certified stream can prove: forward success, certified
compensation, authorized manual decision, or failure without an AC/DC-equivalence claim.

`TypedExecutionSpec` carries hash-defining saga policy. `mfm-store` derives saga engagement,
obligations, run mode, manual-block state, resource lanes, and terminal agreement from the
certified policy plus the append-only stream. Directive selection, obligation open/close,
run-mode changes, and manual-block requests are not separate event families.

Saga handling engages at the first non-retryable failure or forward side-effect ambiguity. After
engagement, no new forward side-effect boundary crossings may be admitted. Runtime drives
past-boundary forward ledgers to quiescence before resolving obligations or terminal outcomes.
Remediation ledgers use the same side-effect protocol as forward ledgers and carry
`SideEffectLedgerPurpose::Remediation { forward_pair_id }`.

Public status reports semantic `RunMode`: `forward`, `remediating`, `manual_blocked`,
`completed`, `compensated`, `manually_resolved`, or `failed_without_acdc_claim`.
Attempt lifecycle is reported separately as committed attempt dispositions: `started`, `completed`,
`failed`, or `interrupted`. Interruption is retryable attempt bookkeeping, not a run mode, saga
engagement, compensated outcome, AC/DC claim, or manual-resolution authority.
`Compensated` is never a vacuous outcome; it requires owed obligations closed by certified remedial
evidence. `FailedWithoutAcdcClaim` is the honest terminal result when policy permits failure
without a compensation or AC/DC-equivalence proof.

Manual resolution is a signed authorization protocol. Certification uses the registry as live
authority for manual evidence schema roles, manual authorization verifier identities, operator
authority snapshots, supported signing scheme, and quorum. The certified spec and certificate then
carry replay authority. Runtime and replay verify manual resolution from certified
spec/certificate, stream prefix, retained artifacts, and canonical proof bytes only; they must not
call live signer, registry, keystore, environment, or runtime signer sources.

`ManuallyResolved` means an authorized manual decision was recorded. It does not mean MFM
independently proved external domain truth.

## Certified Spec

`mfm-spec::v1::TypedExecutionSpec` is the persisted execution contract. It includes:

- spec version and canonicalization identity
- certified transition context table entries, context refs, and node/cell/input context constraints
- certified descriptors and executable identity requirements
- scopes, seeds, configs, nodes, cells, bridge nodes, and public outputs
- input binding trees and value lineage
- effect and capability evidence
- side-effect contracts
- retained config and seed artifact refs
- public-output render nodes and output evidence

The spec hash is computed from canonical bytes. The erased runner plan must be reproducibly derived
from the certified spec and runner registry. It must not carry semantics missing from the certified
spec.

Certification validates transition-context authority before a spec can become runtime authority:
context refs must be content-derived from the certified context table, no-context descriptors cannot
run under a semantic context, context-required nodes must match the registered context descriptor,
context-bound user outputs and inputs must match their descriptor resource kind, stage, and approved
producer contract. The producer contract may list multiple approved state descriptors for a stage,
and raw seeds cannot produce context-bound resources unless the certified producer constraint
explicitly permits seed producers. Framework same-value bridges and
side-effect verify nodes may only preserve an existing context binding; framework receipt nodes must
remain no-context.

Runtime preserves that authority after certification. `CertifiedRuntimeSpec` indexes certified
context table entries by `ContextRef`, sealed runner invocations materialize typed
`CertifiedContext<C>` values only from the certified node context, and no-context states receive only
explicit no-context authority. Input materialization rechecks that each input binding context matches
the certified source cell and that required context-bound inputs are consumed under the same node
context ref. Context-bound state-output cells also require a registered runner extractor; terminal
output admission checks the `CellProduced` context against the certified cell and validates the
staged state-output artifact bytes against the certified context ref, resource kind, and stage.

`mfm_spec::v1::HashedSpecEnvelope` is not certification authority. Persisted spec bytes, hash-only
envelopes, and persisted certificate bytes are hostile data until `mfm-certify` verifies them
against a registry and returns `CertifiedTypedSpec`.

## Store And Events

`mfm-store` is the only semantic commit contract for certified typed runs. Production execution
callers submit purpose-specific `PreparedCommit<Purpose>` authority through `PreparedCommitPlan`.
Each plan carries typed event payloads, commit preconditions, and the artifact evidence that becomes
run authority in the same atomic append. The store constructs envelopes and maintains projections.
Synthetic direct mutation is confined to explicitly named non-execution test, migration, repair,
corruption, or low-level storage contract fixtures.

The authoritative event stream contains:

- run start and attempt lifecycle events, including interrupted-attempt terminal bookkeeping
- seed/config/fact/artifact evidence
- cell terminal events
- side-effect ledger events
- public-output render and produced events
- retention refs and manifests
- run completion or terminal failure

Projection corruption is repairable by rebuilding from the run stream. Projection data must never
be the sole authority for resume, replay, public output, retention, or side-effect status.

The first certified persistent storage path is:

```text
crates/storages/stream-store-postgres
```

Postgres is the only production persistence backend. It stores append-only `commits`, canonical
`run_events`, artifact blobs/evidence, resource-lane claim/release/transition rows,
commit cursor authority, store metadata, and mutable operational admission-lane coordination rows.
`store_metadata.store_scope_id` is store-owned, non-secret identity material for the deployment's
trust boundary; callers cannot supply or update it. Observation list/watch rows are derived from
strict authority at read time. Artifact bytes live in Postgres; production app, CLI, and REST paths
do not stage, read, or migrate workflow artifacts through filesystem artifact roots. The schema and
migrations are owned by `crates/storages/stream-store-postgres`; runtime callers validate schema
contract shape and must not run startup auto-DDL. Because MFM is pre-production, replacing a
persisted contract shape is a destructive schema change that updates the baseline directly.
Existing-run detection folds authoritative `run_events`; there is no separate run-admission index.
Admission-lane rows are operational coordination only. They may select who tries next or who is the
current active driver, but they never grant replay, resume, public-output, side-effect, resource
ownership, or terminal-state authority. Observation rows and list/watch cursors have the same limit:
they are read models, not authority.

Production deployments must give the Postgres run store a dedicated MFM database tenancy. List/watch
cursors order committed `commits` rows by the durable store-owned `store_commit_order` coordinate;
the coordinate is assigned within the append transaction and advances only with a successful commit.
Run-local validation, artifact verification, and projection rebuild complete before the process
acquires the global `store_commit_order` row lock (`SELECT … FOR UPDATE`). That lock is then held
until commit so order assignment and durable append materialization stay atomic. Cross-run contention
on the remainder of the transaction is accepted for correctness; multi-transaction order allocation
is not used. Cursor epochs and artifact cleanup have no public v1 maintenance entry points; any
future maintenance role must first specify Postgres roles, ownership, credentials, and restore/clone
runbooks.

v1 has two operational lane uses:

- execution lanes: `nowait_skip` leases keyed by base work identity
  (`certified_spec_hash` + `store_scope_id`) with the concrete holder `run_id` stored separately;
- resource-admission waiters: FIFO waiters for one certified exclusive side-effect resource claim.

Execution-lane acquire, renew, release, and reap require the holder `run_id` and token to match the
current row. Acquire does not auto-reap expired holders. Resource-admission checks run under the
store transaction and may use Postgres advisory transaction locks as an implementation detail. A
resource claim commits only when the certified lane has no active authoritative holder and no earlier
live waiter. `AdmissionBlocked` persists no run event, commit, resource-lane claim/release,
lane-transition, or other domain authority row; it may insert or refresh one mutable waiter row.
Expired or admitted waiters no longer block later attempts, and retries after expiry receive a fresh
lane-local ticket. Release notifications are wake hints only.

## Process-Fungible Execution

MFM runs are durable certified work, not process-owned work. A process can start, drive, stop, crash,
or resume, but the run's meaning comes only from the certified spec, `RunAdmitted`, the append-only
stream, and store validation.

The model has three identities:

- run identity: `certified_spec_hash` + store-owned `store_scope_id` +
  `invocation_key_digest`; this derives `run_id` and is recorded in `RunAdmitted`;
- execution lane: `certified_spec_hash` + `store_scope_id`; this allows at most one live driver for
  the same base work in one store scope;
- resource lane: the certified side-effect resource key resolved by state preflight; this protects
  external mutation.

A worker may drive a run only after all execution validations pass:

- the stored stream validates against the certified runtime spec and recorded run identity;
- the worker's executable and capability bindings match the stored admission evidence;
- the worker holds the live execution-claim token for the run's execution lane and holder `run_id`.

Public start uses the same rules. If the requested `run_id` already exists, compatible callers attach
or observe `already_active`. If a different invocation of the same base work is already driving, the
execution lane returns `already_active` with the holder `run_id` and no contender `RunAdmitted` event
is appended.

Execution lanes reduce duplicate live work; resource lanes protect side effects. Neither replaces the
other. Execution-claim expiry is not permission to repeat an external mutation. It is permission to
reload the stream and either continue from recorded side-effect evidence, record defended terminal
evidence, or block for certified manual/resolution policy.

v1 is invoker-driven and manual-resumable. Automatic dead-driver takeover, background worker-pool
dispatch, feed-driven dispatch, `due_at` re-wake, long-wait tenure release, pipelined nonces, and
multi-lane admission remain deferred unless this document and `docs/saga.md` define a new certified
contract.

## Runtime

The runtime is the authority boundary for an event-sourced typed state-machine workflow. Its input
model is deliberately small:

```text
static certified transition graph + verified run history
  -> deterministic frontier scheduler
  -> sealed runner invocation
  -> guarded commit
```

The runtime authority contract starts from `CertifiedTypedSpec`, not from parsed spec JSON or a
hash-only envelope. Authoring and persistence move through explicit stages:
`UntrustedTypedSpec`, `LoweredTypedSpec`, certifier-private `ValidatedTypedExecutionSpec`, and then
non-forgeable `CertifiedTypedSpec`. Persisted nodes and renderers refer to descriptor authority by
`DescriptorRef`; `CertifiedDescriptorSet` and `CertifiedFrameworkLifecycle` are the certified views
runtime consumes. `CertifiedRuntimeSpec` is the runtime view of the static certified transition
graph. Before `RunAdmitted`, the assembly/runtime boundary verifies:

- spec hash and schema/version fields
- staged spec/certificate/config/seed artifact bytes and typed evidence
- descriptor identities and executable identity requirements
- runner registry availability
- capability registry availability
- context-bound output extractor availability

Context-bound output value traits live in `mfm-values`. Runtime owns extractor registration,
artifact decoding, and admission checks against certified context-bound cell constraints.

After launch, runtime advances only from the append-only run stream authority. It loads the stream,
delegates spec-independent ordering and projection checks to `mfm-store`, then performs
runtime-owned spec-aware validation of seeds, configs, artifacts, completed cells, side-effect
ledger evidence, public-output events, retention events, and terminal run state. The rebuilt
projection is derived from the stream; it is not independent semantic authority. Store-owned stream
validation requires attempt-bound payloads to be preceded by a separate `StateAttemptStarted`
commit before runtime, replay, or Postgres-backed loads trust them.

Read, resume, replay, and status paths must construct a `VerifiedRunHistoryView` from a
`CommittedRunStream` plus `VerifiedRunArtifactStore` before trusting history. Replay authority is
minted from the verified view and certified runtime authority; raw event vectors or retained
artifact bytes without committed evidence do not cross the runtime/replay boundary. Scheduler drive
paths construct `VerifiedRunContext` through
`VerifiedRunContextLoader`, which combines the same committed stream/fold authority with
`BoundRuntimeContext` runner, capability, and framework-handler authority before any transition is
selected.

The deterministic frontier scheduler is pure. Given the static certified transition graph and
verified run history, it returns one closed transition decision: start a node, continue an open
attempt, start remediation, wait for manual resolution, resolve saga terminal state, or report
blocked. Public-output, retention, and completion work are ordinary certified
framework node selections; an already projected public output is a scheduler facade status, not a
frontier transition decision. Open-attempt recovery, including legal interruption, is handled by the
attempt recovery lifecycle when the continued attempt is dispatched. The frontier decision does not
write the store, stage artifacts, construct live capabilities, or call runners. Open-attempt recovery
classifies verified open attempts into continue, retry terminalization, interrupt, side-effect
recovery, or operational block dispositions. Operational blocks are reserved runtime recovery states
for malformed evidence such as terminal side-effect ledger evidence without matching
attempt-terminal evidence; normal store/history validation rejects those malformed streams before
scheduler recovery, and public status collapses any surviving operational block to blocked without
minting semantic terminal events.

Sync and async drive paths may remain separate IO wrappers. Shared lifecycle authority belongs in
pure helpers for transition/recovery classification, attempt planning, invocation build, output
validation, and commit planning. Full sync/async driver collapse is deferred to a later async-primary
cleanup and must not change lifecycle semantics.

For a new ordinary runnable node attempt, runtime first appends `StateAttemptStarted` from
certified attempt authority. It then materializes state inputs from certified binding trees and
prior typed cell evidence, materializes the certified node context, checks runner identity and
capability availability, and constructs a sealed runner invocation. If post-start materialization,
runner-output validation, or runtime validation fails inside a valid started attempt and no
side-effect authority has been acquired, runtime stages a redacted diagnostic artifact and records a
failure-safe `StateAttemptFailed` plus runtime-evidence retention from minimal trusted attempt
authority. Corrupt history before a valid
attempt context, missing deployment bindings, storage/artifact outages before terminal evidence
commits, and side-effect attempts with acquired ledger authority remain non-semantic runtime or
recovery concerns. Runners receive only scoped typed inputs, allowed capabilities, and erased
context surfaces. They return typed payload intent, staged artifacts, side-effect evidence, or
sealed handles but cannot append to the run stream. Context-bound output artifacts are accepted only
when the runner's registered extractor can recover the certified context metadata from the staged
bytes and it matches the output cell's certified context binding.

The commit planner owns all production execution appends. `RunAdmissionLifecycle` verifies and
stages launch material, then commits exactly one `RunAdmitted` root event with certified spec,
certificate, config, seed, executable, binding-digest, framework, source, and caller launch-time
evidence. Root artifact retention is projection-derived from `RunAdmitted`; launch does not append
attempt, cell, artifact-reference, or retention-ref payloads. Ordinary states, `PublicOutputRender`,
`ProjectRetentionManifest`, `CompleteRun`, and `ResolveSagaTerminal` append
`StateAttemptStarted` before sealed invocation construction, then use the same guarded terminal
commit path: output/reference bindings are checked against the certified graph, side-effect
protocol rules are enforced, commit preconditions are built, and artifact bytes/evidence are
admitted only by the commit that first references them. Production callers submit
`PreparedCommitBundle` values built from purpose-specific `PreparedCommit<Purpose>` authority;
stores do not expose or accept a raw typed-batch or plan-only append escape hatch. Purpose
constructors reject purpose mismatches, missing `CertifiedRunStoreAuthority`, missing `SagaTerminalProof`, or
artifact evidence that was not admitted in the same commit. Artifact blobs are admitted inside the
append transaction; failed appends leave no authoritative run-store evidence.

Normal launch identity is content-addressed from `RunIdentityMaterialV1` using canonical JSON:
`certified_spec_hash`, store-owned `store_scope_id`, and required `invocation_key_digest`. Public
entry-point starts accept an optional raw `invocation_key`; when omitted, the app mints a fresh opaque
key before deriving identity. The raw key is not persisted. The run id must equal the digest of the
recorded identity material, and `RunAdmitted` records the material so attach, resume, replay, status,
and public-output authority can fail closed on identity mismatch. Raw caller-supplied run ids are not
a normal launch surface.

Run admission and first execution-claim acquire are one store operation. The app validates the
store-owned scope, the certified spec hash, and the derived `run_id`; the store validates that
the prepared execution claim matches the `RunAdmitted` identity material before admission. If the
execution lane is already held, the store returns `ExecutionClaimBusy`, appends no contender run
event, and public start reports `already_active` with the holder `run_id`.

Resume reads the recorded identity from `RunAdmitted`, rebuilds the execution lane from that identity,
validates stored launch evidence and executable/capability bindings, then acquires or renews the
claim. Runtime drive entry points require the current token for the execution lane and holder
`run_id`; callers without it cannot drive through the public scheduler API. The v1 claim lease has a
60 second TTL and a 20 second heartbeat interval. While short receipt-level waits are active, the
invoker loop keeps heartbeating instead of releasing tenure.

Framework lifecycle work is represented by certified graph nodes, not ad hoc runtime side effects.
Run admission is the sole pre-attempt root authority and is not represented by a certified graph
node. `PublicOutputRender`, `ProjectRetentionManifest`, `CompleteRun`, and
`ResolveSagaTerminal` are sealed framework runners with the same append-only stream, rebuilt
projection, deterministic scheduler, started-before-run attempt lifecycle, and guarded commit rules
as domain states.

Resume loads the stored certified spec, rebuilds the verified history and projection from the run
stream, verifies completed cell and side-effect evidence against the spec, then advances only from a
type-valid frontier.

Replay loads the stored certified spec and certificate artifacts, verifies them against the compiled
certification registry, compares the hashes to `RunAdmitted`, rebuilds stream evidence, and uses
replay adapters only. Live capability construction during replay is a contract violation.
Replay service construction itself is evidence-only app assembly: it must not construct the live
runner registry, live transports, signer providers, keystores, or live capability runtime config.

Live provider identity is enforced by bound provider implementations. Runners derive a certified
semantic binding, such as EVM `network_id` plus expected chain id or Bitcoin `network_id`,
`source_identity`, and expected network tag, before issuing operation-only capability requests. A
route that resolves but observes incompatible source evidence fails after `RunAdmitted` as an
attempt/capability failure with a closed redacted provider diagnostic. Provider diagnostics carry a
provider family, stable diagnostic code, optional redaction-safe operation id, and closed
boolean/integer/id fields only. Examples include HTTP status, JSON-RPC numeric code,
response-shape failure, unsupported operation, operation incomplete, and source mismatch. They must
not carry RPC URLs, authorization headers, file paths, provider messages, request/response bodies,
signer material, or signed transactions. Replay providers rebuild the certified provider binding
and verify recorded evidence against that binding without resolving source refs or policy ids
through current runtime config.

Manual-resolution replay additionally verifies that the stream prefix derives `ManualBlocked`, the
event matches certified policy, evidence and authorization artifacts match certified roles and
digests, the canonical proof claim matches the event and prefix exactly, signatures verify, signers
belong to the certified authority snapshot, and quorum is satisfied. Runtime and replay build this
through `ManualResolutionPrefixAuthority` and `ManualResolutionProofAuthority`; only a
`VerifiedManualResolutionForPrefix` can authorize the corresponding manual-resolution commit or
terminal saga proof. Terminal saga proof construction re-verifies that authority from the current
verified prefix and retained artifacts; scheduler-local proof caches are not authority.

## Side Effects

Side-effect states are multi-commit protocols. The scheduler/store own:

- logical ledger key derivation
- idempotency key stability
- claim generation and fencing
- invocation epoch
- durable transition ordering
- ambiguous recovery blocking
- terminal output binding

The durable uncertainty boundary is the invocation-started event. After that boundary, resume must
recover or block using typed evidence; it must not duplicate an external mutation or guess from
unstored state.

`CertifiedSideEffectContract` is the single side-effect resource-claim and verification authority
shared by live execution, resume, and replay. Verification is a hash-defining contract value:
`Receipt` terminalizes from receipt evidence, while `Finalized(depth)` terminalizes only from
confirmation evidence at the certified depth. The depth is lowered into the certified spec, never
resolved from a registry at admission or from worker-local policy. `RunAdmitted` may echo launch
diagnostics, but it cannot override or duplicate this verification authority. Receipt-level
terminalization is final-at-risk by explicit operation design: a later reorg can invalidate the
published output or wedge the next nonce, and that recovery belongs to the deferred stuck-transaction
or nonce-reclaim workflow.

`CertifiedSideEffectContract` exposes domain output construction for the configured terminal
evidence level. Runtime invokes the domain-owned receipt or confirmation output contract and never
constructs domain output values itself. `NotSubmittedProven` is a defended investigation outcome,
not a terminal result from one transient RPC miss.

The store exposes legal phase information through
`SideEffectLedgerState`, so transition admission is a typed state-machine check instead of an
optional-field projection heuristic. Forward side-effect ambiguity is admissible only when paired in
the same commit with the non-retryable attempt failure that engages saga handling.
The store is the source of truth for forward-fence admission after saga engagement; runtime
early-rejects are scheduling convenience and cannot substitute for store validation.
Resource-lane scheduling remains conservative. A lane-blocked attempt is parked before invocation
and may be retried after bounded backoff or a lane-release wakeup; ordinary contention is not
terminal evidence. Runtime resolves concrete exclusive lane keys in pure preflight, then asks the
store to materialize `ResourceLaneClaimed` from `ResourceLaneClaimIntent`. The store assigns the
lane-local fencing token and transition sequence and records the lane mirror/transition rows in the
same append transaction. `ResourceLaneClaimed`, not the waiter row, is held-lane authority. Recovery
must either reuse that committed held lane for the same invocation or release it through certified
cleanup authority before interruption. `SideEffectInvocationPrepared` and every later phase are
owned by `SideEffectLifecycle`; recovery either resumes from the concrete ledger phase, records
evidence-backed terminal side-effect outcome, or reports an operational block.

Prepared-invocation artifacts may retain unsigned mutation plans, expected hashes, and non-secret
signer references. Signed raw transactions are bearer mutation material and remain transient
submit-time bytes inside the mutation adapter.

Submission observed, submission unknown, and not-submitted-proven evidence share one logical
submission-result slot for an invocation epoch. Unknown submission can be superseded only by the
legal recovery transitions enforced by the store.

## Public Outputs

Public-output specs are part of the certified spec. Runtime public output is produced by typed
render states and events. Rendered JSON artifacts are cache material and must be checked against
typed output evidence before use.

Rendering helpers require `PublicOutputReadAuthority`, which `mfm-app` mints only after verifying
stored certified authority and rebuilding the projection from the authoritative run stream.
Rendered JSON cannot be used as resume, replay, certification, or render authority.

CLI and REST outputs are public API surfaces, but they are not semantic execution authority.

## CLI And REST Boundaries

`bin/cli` and `bin/rest-api` may:

- decode JSON/TOML/user input
- build typed configs and certified specs through operation crates
- construct the production Postgres run store and app services
- start, resume, replay, inspect, and render typed runs through app services
- read observation-only run list/watch pages through the shared app API
- preserve stable response envelopes

They must not:

- plan workflow semantics directly
- execute state behavior directly
- bypass typed certification
- infer public outputs from untyped snapshots
- migrate uncertified historical runs into certified typed runs
- depend directly on SQLx, filesystem artifact stores, or alternate production storage selectors

## Architecture Placement

This design contract defines runtime authority. Crate placement, taxonomy, operation/state/adapter/
transport/signer/config boundaries, public naming rules, and reviewer checks are maintained in
`docs/architecture.md`.

## Documentation Update Rules

Update this document when a change alters:

- certified spec semantics
- typed event schemas
- store commit or projection authority
- resume or replay semantics
- certified saga or manual-resolution authority
- effect/capability rules
- public-output authority
- crate ownership boundaries
- CLI or REST runtime contracts

Use `docs/architecture.md` for the short contributor map.

The former typed-core source-scan gates and summary-key CI scripts have been deleted. Real
guarantees now live in typed APIs, private constructors, crate dependency boundaries, Rust tests,
trybuild fixtures, cargo-metadata checks, and production-path integration tests.
