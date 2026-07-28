# Architecture

Status: contributor architecture guide

`docs/design.md` is the normative runtime, journal, store, replay, and authority contract. This
document owns responsibility taxonomy, placement, dependency direction, and contributor checks.

## One Sentence

MFM operations deterministically plan certified typed graphs; states own closed pure/read/effect
semantics; adapters bind state requests to explicit capabilities; transports and signers remain
reusable platform primitives; the executor owns durable mutation convergence; runtime performs one
stateless action; the journal records five complete record families; store verifies and appends
them atomically; replay uses the same verified view without live semantic IO; and app, CLI, and
REST are authorization and presentation boundaries only.

## Core Runtime Shape

```text
deployment-provisioned current configuration
  -> app validates and canonicalizes one entry-point input
  -> exact entry-point registration selects one PlanningProfile
  -> operation authoring plus deterministic framework/executor expansion
  -> certification of graph, manifests, dependencies, terminal contract, and public output
  -> app authorizes Admit and appends RunAdmitted without semantic live IO

later drive_once call
  -> app authorizes Drive for one tenant/run
  -> store loads one native committed journal and its exact objects
  -> callback-free verification mints one borrowed VerifiedRunView
  -> runtime derives one closed action
  -> runtime either commits one local transition, performs one audited call, or reports waiting
  -> store appends one legal whole batch through exact-head compare-and-swap
```

The only run-journal records are:

```text
RunAdmitted
StateTransitionCommitted
ExternalAccessAuthorized
ExternalAccessObserved
RunClosed
```

The executor is a separate authority flow:

```text
EffectRequested
  -> immutable executor binding and request identity
  -> append-only keyed delivery/resource ledger
  -> committed affine target-entry authority
  -> one target receipt
  -> exact observation, bounded frontier, and terminal tombstone
  -> terminal evidence returned through one audited ensure call
  -> EffectSettled
```

The executor cannot mutate the MFM journal or settle a state. The runtime cannot choose executor
delivery or resource policy.

## Semantic Package Metadata

Each workspace package declares one closed `package.metadata.mfm.layer`:

| Layer | Responsibility |
| --- | --- |
| `kernel` | Domain-free contracts and runtime infrastructure. |
| `domain` | Pure domain model, capability, state, and operation semantics. |
| `live` | Domain adapter and reusable transport implementations. |
| `signing` | Generic signing contracts. |
| `secret-provider` | Secret-bearing keystore and signer implementations. |
| `storage` | Concrete journal or executor-ledger persistence. |
| `assembly` | Process configuration, implementation construction, authorization, and application services. |
| `binary` | CLI or HTTP transport surfaces. |
| `test` | Cross-boundary fixtures and qualification harnesses. |

Domain packages declare a validated domain and `source` or `aggregate` role. Every live package has
a matching pure-domain owner. Kernel packages declare whether domain and binary packages may
consume them. Cargo target kind, metadata, and dependency direction are checked together; package
names and directory counts are not architecture.

## Responsibility Taxonomy

| Category | Owns | Does not own |
| --- | --- | --- |
| Canonical/identity primitive | Frozen encodings, domain-separated identities, raw-byte content references | Workflow or domain behavior |
| Value contract | Typed value descriptors and the one producer-independent retained-value contract | Producer authority, retained-byte identity, or workflow behavior |
| Journal contract | Five record schemas, producer-bound `ValueRef`, heads, references, batch algebra, persisted fact-slot and actual-emission coordinates | Persistence, scheduling, state callbacks |
| Program | Typed graphs, `StateExecution`, `StateFrame`, value views, settlement values, process-only fact proposals | Store or live-access authority |
| Certifier | Planning-profile verification, deterministic expansion verification, manifests, certified dependency, terminal, and bounded homogeneous fact-slot contracts | Runtime scheduling or live IO |
| State | Pure request authorship, observation interpretation, output/fact/failure construction | Ambient IO, persistence, scheduler policy |
| Capability contract | One typed application-protocol request/return and closed safe-failure contract | State reduction, hidden retry, workflow topology |
| Adapter | Private binding from runtime-authorized state request to reusable transport/executor surface | A second lifecycle or replay reducer |
| Transport | Reusable protocol encoding, IO, checked decoding, and safe error classification | Journal access, state settlement, workflow topology |
| Executor | Keyed delivery convergence, target-entry authority, terminal evidence, typed resource policy | Run scheduling, state settlement, journal mutation |
| Store | Atomic append, object binding, hashes, CAS, structural fold, fact-group validation and actual-ordinal assignment, verified views/readers | State execution, domain outcomes, destination IO |
| Runtime | Deterministic readiness, exact materialization, audited call orchestration, one-action drive | Business policy, protocol phases, resource policy, persisted status |
| Replay | Recorded verification, exact reproduction, candidate comparison | Live scheduler, live capability, append |
| App assembly | Authentication, grants, entry-point catalog, store/catalog wiring, DTO services | Planning logic or state behavior |
| Binary/API | Input decoding, route/command dispatch, response rendering | Store/runtime/live implementation construction |

Before adding a module, crate, schema, capability, public type, route, or command, identify one row
that owns it. If it spans rows, split the responsibility or make the lower authority explicit.

The program registry builder is the sole semantic program-definition assembly boundary. It creates
one immutable shared definition for entry points, states, capabilities, planner identity, policies,
executors, and current manifests; live entries retain only their process callbacks/invokers plus
shared definition references. The registered zero-state certification factory receives that same
definition exactly once and returns the sole private certifier callback. Certifier-local catalogs
are thin borrowed projections, not a second construction or ownership surface. This kernel boundary
is inventory-generic: it enforces common qualification, unambiguous descriptors, and exact
state-operation-manifest closure, while app qualification and deployment assembly own the current
production component inventory.

## Dependency Direction

Kernel dependencies point toward lower contracts:

```text
ids + canonical
  -> values + capabilities
  -> journal
  -> program + spec
  -> certify
  -> store
  -> runtime + replay

ids + canonical + values + capabilities
  -> executor
```

Concrete storage depends on kernel store or executor traits; kernel never depends on a concrete
backend. Pure domains consume only domain-facing kernel and generic signing contracts. Source
domains never depend on aggregate domains. Live packages consume their pure domains and
runtime-facing kernel contracts, but not app, binaries, concrete stores, or secret providers.
Aggregate live packages do not import source-live implementations. Assembly wires lower layers.
Binaries consume only binary-facing assembly or kernel contracts.

The removal of a package must also remove its workspace member, normal/build/dev dependency edges,
feature references, test fixtures, metadata expectations, and lockfile package entry in the same
atomic change.

## Operation Boundary

Operations are deterministic planning only.

Operations may:

- validate canonical typed planning input;
- call child operation builders;
- author typed seeds, nodes, bindings, dependencies, and public-output roots;
- attach stable keys and source roles;
- select exact framework/executor policies through the registered planning profile; and
- return an authored graph for certification.

Operations must not:

- execute state callbacks;
- access network, filesystem, clock, process, signer, or provider for semantic work;
- append records or read current run state;
- select observations or settle effects;
- construct runtime capabilities; or
- hide topology in app or binary glue.

Framework pre/post behavior is an ordinary planner-injected typed node with visible bindings and
rewiring. Runtime has no framework-origin branch.

## State Boundary

States own reusable outcome-affecting semantics.

Every state selects exactly one closed execution case:

- pure: one `StateFrame -> Settlement` callback;
- read: total request authorship and one observation callback; or
- effect: total request authorship and terminal-evidence settlement.

States may define typed config, context, input, output, facts, failure, request, response, and
evidence contracts. They may validate domain rules and construct deterministic results.

States must not:

- instantiate transports or clients;
- inspect a journal, store, scheduler, app, or binary;
- resolve routing or signer material;
- retry a capability invisibly;
- persist values or facts directly;
- add another phase or runner kind; or
- use replay-specific reducers.

`StateFrame`, `RequestView`, `ObservationView`, and `Settlement` are value-only program contracts.
Committed proof and live-access authority stay private to runtime.

## Capability And Adapter Boundary

A capability represents exactly one application-protocol operation for one immutable typed request.
It owns bounded encoding/decoding, source validation, cancellation behavior, and a closed
`SafeFailure` classifier. One runtime authorization permits at most one such operation.

A capability cannot:

- choose graph behavior;
- reduce a state;
- construct state output, facts, or domain failure;
- retry or fail over invisibly;
- append journal records; or
- define replay behavior.

The adapter is private live-crate glue. It consumes runtime's affine `AuthorizedAccess`, invokes a
lower runtime-agnostic transport or executor, and returns one typed wrapper result. It owns no
persisted lifecycle. Reusable transports do not expose or depend on MFM runtime authority.

Every required live bootstrap, including source and chain validation, is its own audited state
after `RunAdmitted`. App admission may bind one immutable non-secret routing generation but cannot
probe or silently replace it.

## Transport Boundary

Transports may:

- resolve the exact admitted routing generation;
- construct checked source-bound sessions;
- execute one bounded protocol request;
- validate protocol identity and response shape;
- return a typed result or reviewed safe failure; and
- redact endpoint, credential, provider text, body, and path details.

Transports must not:

- use workflow recipe names;
- know graph topology or state phase;
- open keystores unless they are a dedicated secret-provider implementation;
- persist journal/executor authority;
- perform hidden retries, source rotation, or fallback; or
- expose unchecked clients through an authority-bearing production path.

Generic protocol code belongs in a reusable public transport module. The sibling MFM adapter stays
private.

## Executor And Mutation Boundary

`mfm-executor` owns domain-free keyed convergence. It retains immutable request identity, bounded
delivery history, affine target-entry/receipt authority, terminal proof, and typed resource-policy
state. A concrete executor backend is a `storage` package.

`KeyedExecutorLedger<Store>` is the sole high-level executor implementation. It owns strict folding,
typed resource-policy validation, deterministic attempt derivation, bounded CAS retry, observation,
and terminal semantics. Its asynchronous `ExecutorLedgerStore` boundary owns only one exact fenced
store identity, complete immutable effect/resource and exact-content reads, and atomic
compare-and-append. Memory, file, and PostgreSQL stores implement that same raw contract; no backend
duplicates the engine.

One compare-and-append may atomically bind an effect and allocate a resource, or upgrade a
previously bound effect that has no target attempt. `Applied` is the only store result that can mint
affine target-entry authority. An already-applied proposal or head conflict is reloaded; an
acknowledgement-ambiguous append returns no authority. Sequence policies cannot advance a shared
sender until the prior allocated effect has immutable terminal evidence.

Each authorization atomically retains the complete schema-qualified target-entry descriptor whose
reference derives the attempt identity. The shared engine can resolve that descriptor by effect and
attempt during recovery. This keeps candidate-specific public target input durable without
retaining signatures, raw signed envelopes, credentials, or other bearer material.

Every reopen strictly refolds the complete immutable effect/resource graph and exact executor-owned
content inventory under the selected binding. Missing, extra, duplicate, mismatched, forked, or
partially linked content fails closed. Backend transactions end before target IO.

Production registration additionally requires deployment-specific proof of:

- non-rollback ledger generation;
- stale and sibling writer exclusion;
- destination convergence and resource ownership;
- exact tenant/deployment binding;
- restart, backup, restore, and promotion behavior; and
- no-secret retained evidence.

The journal's qualified PostgreSQL writer does not satisfy executor or destination fencing.
Production mutation requires its separately qualified executor store and authoritative destination
fence even when both use PostgreSQL.

`mfm-storage-executor-postgres` implements only the raw asynchronous executor-store boundary. It
uses a dedicated schema and executor-only owner/application roles; immutable binding, frontier,
resource, allocation-link, and content rows are authority, while heads are derived views. Opening
requires a deployment-supplied executor writer-generation fence independent of the journal fence,
then performs a strict shared-engine refold before returning the store. Database transactions take
generation, effect, and optional resource locks in that order and end before any destination,
signer, wallet, or RPC IO.

## Journal And Store Boundary

`mfm-values` owns the one producer-independent `RetainedValueContract`. `mfm-journal` owns the
frozen five-record schemas, producer-bound `ValueRef`, and journal references. `mfm-store` owns
legal append and verified read authority.

Stores own:

- native per-run commits and records;
- per-run predecessor sequence/digest;
- canonical record, candidate, and commit hashes;
- exact logical-key and batch rules;
- object admission and `commit_artifact_bindings`;
- tenant fact publication and selection-barrier coordinates;
- append idempotency and exact-head CAS;
- closure and post-closure audit-tail validation;
- one private structural/semantic fold;
- `CommittedRunJournal` and `VerifiedRunView`; and
- purpose-specific readers and direct-new-authorization permits.

A store does not own state callbacks, domain interpretation, executor delivery, routing, or
process scheduling.

The first production backend is PostgreSQL. Every authority-bearing read and write uses one fenced
authoritative writer. The application role cannot update/delete/truncate immutable authority or
manipulate store identity and tenant fact heads directly. HA/WAL promotion must fence old writers
and prove a complete non-rollback lineage. Replica reads cannot mint v1 store authority.

Physical tables and indexes may normalize the journal, objects, bindings, and fact routing fields.
They must not create a separately writable semantic model. Current configuration is a distinct
pre-admission concern and becomes immutable root material when selected for a run.

## Runtime Boundary

Runtime is a stateless interpreter over one certified graph and one borrowed `VerifiedRunView`.
Its private action algebra is:

```text
CommitPure
CallRead
SettleRead
CommitEffectRequest
CallEnsure
SettleEffect
CommitDependencySkip
Closed
Blocked
```

Runtime owns:

- deterministic readiness and dependency-skip derivation;
- exact typed input/context materialization;
- sole qualified-program-registry selection and callback/live-invoker dispatch;
- committed request/observation proof selection;
- complete physical observation-suffix validation before action ranking;
- authorization append followed by one affine call;
- canonical settlement validation;
- transition candidate construction; and
- exact-head retry after reload.

Runtime owns no process-persistent semantic state. Another process with the same exact qualified
program registry and authorized writer can continue the next `drive_once`.

## Replay Boundary

Recorded-history verification is callback-free and produces `VerifiedRunView`. Exact reproduction
reruns the admitted pure planner/state computations. Candidate comparison runs only the explicitly
identified current candidate code.

Replay may read the exact journal, immutable object closure, certification proof closure, and
authorized cross-run source bundle. It may self-attest its executable where the mode requires it.
It does not invoke the live scheduler, authorize access, append, resolve routing, call a provider or
executor, read domain files, or construct a signer.

Purpose-specific trace, audit, fact-completeness, reproduction, and export readers borrow the same
verified authority. A cursor or reference cannot mint reader authority.

## Facts And Cross-Run Data

Facts are transition emissions, not an independent write protocol. A same-run consumer uses an
explicit graph edge. A prior-run consumer authors `FactSelectionRequest` and invokes the reserved
`mfm.journal.fact-selection.v1` read at a tenant fact barrier.

The reserved store capability has no process invoker or independently selected route. Its admitted
read binding's capability contract is strictly validated as the reserved scan contract, while its
retained `routing_catalog_ref` is the singleton internal routing generation passed exactly to
authorization.

The spec and certifier own dense homogeneous fact-slot declarations, their descriptor and
subject/response contracts, their minimum/maximum multiplicity, the per-slot and per-settlement
bounds, and proof that every same-run actual-ordinal dependency lies in exactly one producer
slot's invariant ordinal core with matching source and destination contracts. `mfm-facts` and
`mfm-program` own process-only slot-indexed proposals, preserve order within nondecreasing slot
groups, and reject exact duplicates. They do not mint journal coordinates.

The store validates every callback group against its certified slot, assigns dense actual
`emission_ordinal` values across the accepted sequence, constructs producer-bound fact authority,
and publishes it atomically with the transition. The journal persists both the actual ordinal and
the authorizing `fact_slot_ordinal`; it does not own proposal, certification, or publication
policy. Replay rechecks the persisted grouping and certified multiplicity contract.

The store privately scans the authoritative writer's dense publication prefix through that
barrier. Only complete coverage mints `FactSelectionResponse`; private scan continuation cannot be
persisted or resumed as public authority.

The fresh authorization result owns the sole affine live-scan permit. Scan pages include every
dense publication through the barrier, including unselectable publications from the consuming run.
For each final selected source, the scan builder reduces the full verified producer run to the
exact publication-prefix typed closure before minting `CompletedFactScan`; no broader producer
lookup authority crosses that boundary.

Store-owned closure traversal distinguishes semantic references from transport authority.
`ContentRef` contributes a dependency and resolves exact-prefix bytes through the canonical-lowest
matching `ValueRef`, whose metadata is not recursively interpreted. `ValueRef` contributes a
semantic object plus its evidence-contract dependency. Strict annex projection drives recursive
payload traversal, except that a `ContentRef` target with schema exactly `mfm.value-ref.v1` remains
a transport-only wrapper and stops traversal because the incoming content reference is already the
semantic edge. Bounded insertion and cycle rejection apply to the traversed semantic closure.
Transport-only objects accompany the generic observation append but never enter ordered semantic
object references.

The completed affine scan derives its sealed authorization while consuming itself into generic
observation material. The reserved observation and its private attestation-routing row are assigned
and persisted in the same backend transaction. Live reducer entry, after runtime chooses compatible
returned observations, and replay completeness both load that row against a caller-supplied exact
`VerifiedRunView` and invoke the same completeness verifier; neither runtime nor replay can mint
completeness structurally.

Cross-run correction input uses a separate effective/evidence-only source role and a semantically
closed source. It cannot substitute for ordinary graph data. Corrections are separately planned,
certified, admitted, driven, and closed runs under the normal journal/store contract.

## Domain Placement

Within a pure domain crate, private roles point downward:

```text
model <- capability <- signing <- state <- operation
```

An aggregate domain may consume narrow pure source-domain contracts. A source domain never depends
on an aggregate or its operation topology.

Current domain product placement:

- `mfm-evm` owns typed EVM model, capability, state, and operation contracts.
- `mfm-evm-live` owns reusable JSON-RPC transport plus private audited adapters.
- EVM portfolio reads use decomposed bootstrap, anchor, per-call fan-out, confirmation, and pure
  aggregation nodes.
- EVM transaction submission uses the same stateless transport behind a durable wallet executor;
  the domain owns immutable request/evidence semantics, while guarded signing, sender/nonce
  allocation, target attempts, and terminal convergence remain executor responsibilities.
- `mfm-bitcoin` and `mfm-bitcoin-live` may retain reusable pure/transport foundations, but Bitcoin
  collection is not in the production catalog.
- `mfm-portfolio` owns aggregate configuration, graph planning, same-run typed dataflow, snapshot,
  and report projection.
- The product catalog exposes `mfm.portfolio/snapshot@1` and
  `mfm.evm/submit-transaction@1`.

## App And Binary Boundary

`crates/app` owns:

- the opaque process-facing application facade;
- authentication and exact purpose grants;
- tenant derivation and sealed `RunAccessAuthority<G>`;
- the entry-point/planning-profile catalog;
- sole `QualifiedProgramRegistry` assembly, including admitted support, deterministic callbacks,
  semantic read/effect entries, and process-private live invokers;
- the one content-scoped `61 + N` portfolio/EVM support graph, its single store admission, and
  transfer of that non-cloneable admitted graph into the sole registry;
- the qualified PostgreSQL run store and separately fenced PostgreSQL executor store;
- exact wallet executor, resource-owner, signer-generation, and target binding composition;
- current-configuration resolution for admission;
- exact run services and reviewed DTOs; and
- public-output, trace, audit, replay, and export reader composition.

For the current product, `N` is the exact configured EVM routing-generation count in
`1..=4096`. The support scope hashes the complete field-path-ordered member identities and retained
contracts. Private endpoints, credentials, transports, caller configuration, and per-run
artifacts never enter that graph. Runtime and the private application backend borrow one shared
registry `Arc`; they do not assemble parallel catalogs or clone admitted support authority.

The complete facade is:

```text
entry_points
admit_run
drive_once
read_public_run
replay_run
read_transition_trace
read_access_audit
export_run
```

Entry-point discovery is non-disclosing. Every run operation authorizes one of:
`Admit`, `Drive`, `Replay`, `ReadPublic`, `InspectTrace`, `InspectAudit`, or `Export`.

CLI and REST decode connection/authentication and command/request input, invoke one facade method,
and render the corresponding DTO. They cannot construct stores, catalogs, transports, executors,
keystores, or runtime services.

The exact public surface is:

```text
GET  /v1/entry-points                 mfm ops list
POST /v1/runs                         mfm run admit
POST /v1/runs/{run_id}/drive          mfm run drive
GET  /v1/runs/{run_id}                mfm run show
POST /v1/runs/{run_id}/replay         mfm run replay
GET  /v1/runs/{run_id}/trace          mfm run trace
GET  /v1/runs/{run_id}/audit          mfm run audit
POST /v1/runs/{run_id}/exports        mfm run export
```

There is no arbitrary object read or tenant-wide run/fact discovery contract.

## Public Naming Rules

Public command, route, schema, capability, executable, and output identities use durable domain
language. They must not expose crate names, adapter names, test fixtures, temporary acronyms, or
implementation topology.

Changing a frozen schema or semantic identity requires a reviewed current-design cutover and
regenerated complete golden corpus. Compatibility aliases and dual current paths are forbidden.

## Review Checklist

Before merging, verify:

- one responsibility owns every new type, module, table, command, or route;
- the five-record algebra and exhaustive batch contract remain closed;
- state request authorship is pure and total;
- every semantic external operation receives exactly one preceding authorization;
- only a directly observed new authorization mints live authority;
- runtime performs one action and retains no process semantic state;
- effects use the executor boundary and do not add journal phases;
- same-run data uses graph edges and prior-run selection is audited;
- the store owns one fold/view and no consumer rebuilds another authority;
- PostgreSQL authority uses the fenced writer and immutable-root locking;
- replay constructs no live semantic capability;
- typed/persisted/public data contains no secrets or floats;
- Bitcoin remains absent unless its qualification passes;
- every registered product mutation has its exact qualified executor, resource owner, and
  independent deployment fence;
- app and binaries expose only the exact granted surface; and
- tests enforce negative boundaries as well as successful behavior.

## Companion Documents

- `docs/design.md`: normative semantic and authority contract
- `docs/persisted-public-surfaces.md`: persisted/public no-secret inventory
- `docs/recoverability-app-surface-v1.md`: exact app, CLI, REST, DTO, and disclosure contract
- `docs/portfolio-snapshot.md`: published product objective
- `docs/evm-rpc-routing.md`: EVM routing generation and audited read graph
- `docs/btc-rpc-routing.md`: unregistered Bitcoin qualification target
- `docs/evm-transactions.md`: qualified EVM wallet effect and deployment contract
- `docs/code-quality.md`: mandatory contribution policy
- `docs/build-and-verification.md`: scope-driven verification workflow
