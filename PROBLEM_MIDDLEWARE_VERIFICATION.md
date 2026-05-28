# Middleware And Verification Problem

Date: 2026-05-28
Branch reviewed: `rfc-state-ops`

This note records the current architectural concern around persistence middleware,
framework states, and verification ownership in the typed runtime work.

It should be read together with:

- `RFC_STATE_OPS_PROBLEM.md`
- `RFC_TYPED_CORE_PROPOSAL_1.md`
- `PROBLEM_IMPLEMENTATION_TYPED_CORE.md`
- `DECISION_TYPED_CORE_CORRECTIVE_ARCHITECTURE.md`

## Executive Summary

The branch has enough typed-core structure to show the intended direction: states are
the executable runtime unit, and the runtime already supports framework-owned nodes
such as public-output rendering. However, persistence and verification are still split
across app helpers, scheduler methods, transport runners, and read paths.

The main problem is not that persistence or verification exists. Both are mandatory
for replay, resume, audit, and content-addressed execution. The problem is where those
responsibilities live.

The better target is:

```text
all run-authority mutations are framework/user state executions
all state executions are wrapped by mandatory runtime middleware
framework states are sealed and runtime/lowering-minted
user states use the public typed state authoring API
stores remain append-only commit authority
```

Under that model, manual `persist_*` functions should mostly disappear from app,
transport, and state code. State code should return typed values, staged managed-write
refs, or domain effect results. Runtime middleware should persist staged artifacts,
validate evidence, and atomically bind everything to typed events.

Verification should be pushed as far left as possible:

- type system for authoring-time wiring, handles, typestates, capability access, and
  public-output shape;
- certification for persisted typed-spec authority;
- runtime middleware for state execution pre/post conditions;
- runtime read/query boundaries for public-output reads, replay broker construction,
  and stored authority reconstruction;
- storage/artifact layers for byte/evidence integrity.

App-level helpers that manually remember to verify and persist are the wrong long-term
authority boundary.

## Current Branch Evidence

The branch already has the core concept needed to avoid a new public runtime-transition
API: `NodeSpec` represents both user states and framework nodes.

Important current anchors:

- `crates/kernel/spec/src/lib.rs`
  - `NodeSpec` has `framework: Option<FrameworkNodeSpec>`.
  - `FrameworkNodeSpec` currently includes `Bridge` and `PublicOutputRender`.
- `crates/kernel/runtime/src/lib.rs`
  - `ErasedRunnerRegistry::resolve` handles `PublicOutputRender` as a runtime built-in.
  - `FrameworkPublicOutputRunner` executes the framework public-output node.
  - `SerialTypedScheduler::start_run` manually builds `RunStarted`.
  - `SerialTypedScheduler::append_retention_manifest_projection` manually builds
    retention projection events.
  - `validate_runner_output` currently rejects runner output containing scheduler-owned
    payloads such as `RunStarted`, `RunCompleted`, `RetentionRefsAppended`, and
    `RetentionManifestProjected`.
- `crates/app/src/lib.rs`
  - `persist_certified_spec_artifact`, `persist_certified_spec_certificate_artifact`,
    `persist_seed_inputs_for_spec`, and `persist_framework_public_output_receipts`
    stage artifacts outside a unified state middleware path.
  - `verify_public_output_read_authority`, `load_certified_spec_for_run`, and
    `replay_authority_for_run` rebuild trust at app-level read/resume boundaries.
- `crates/transports/proof/src/lib.rs`, `crates/transports/portfolio/src/lib.rs`,
  and `crates/transports/evm-dcv/src/lib.rs`
  - transport runners call local `persist_artifact` helpers and return
    `ErasedRunnerOutput` with required artifact evidence and payloads.

This proves the branch is halfway between two models:

1. Framework states are real executable nodes.
2. Some framework lifecycle work still bypasses the state execution path.

The second part creates the current arbitrariness.

## Problem 1: Manual Persistence Leaks Across Layers

The current branch has persistence helpers in app, transport, and conformance code.
Some examples:

- app-level spec/certificate/seed persistence before `RunStarted`;
- app-level public-output receipt persistence after runtime stream inspection;
- transport-level artifact persistence from proof, portfolio, and EVM runners;
- conformance helpers that manually write framework config/spec artifacts.

These helpers exist because events reference content-addressed artifacts. Bytes must be
stored before the event can safely bind artifact id, digest, length, media type, role,
schema id, semantic type id, and producer evidence.

That requirement is valid. The leak is making each caller remember how to do it.

The desired rule is:

```text
orphan/staged bytes may exist before commit
only runtime/store-bound typed events make them authority
```

State or domain code may stage managed platform writes through a certified capability.
It should not hand-author durable authority. The runtime middleware should:

1. collect staged artifacts and typed state output;
2. persist or verify artifact bytes through the configured artifact store;
3. derive evidence;
4. validate evidence against the certified node/spec;
5. append the typed commit atomically through the store.

## Problem 2: Lifecycle Hooks And State Execution Are Artificially Split

Earlier framing separated "runtime lifecycle hooks" from "state execution middleware".
That split is probably the wrong abstraction for MFM.

The existing branch already models framework-owned work as states for public-output
rendering. That pattern should generalize.

Potential framework states:

- `BootstrapRun` or `RunStart`
  - persists/binds certified spec, certificate, config evidence, seed cells, executable
    identities, framework/source revision, and emits `RunStarted`;
- `BridgeSameValue`
  - already represented as a framework node in the spec model;
- `RenderPublicOutputs`
  - already represented as a framework node and built-in runtime runner;
- `CompleteRun`
  - emits `RunCompleted` once public-output evidence exists;
- `ProjectRetentionManifest`
  - builds and persists a retention manifest projection, then emits
    `RetentionManifestProjected` and retention refs;
- possible future framework states for diagnostics, migrations, or replay-audit
  materialization.

This gives one conceptual model:

```text
state = executable runtime unit
  user state
  framework state
```

The public API does not need a new "runtime transition" concept if states can express
this. Internally the runtime may still have helper structs/functions, but the semantic
surface should stay "states execute, runtime middleware persists/verifies/commits".

## Problem 3: Scheduler-Owned Payloads Break The Unified State Model

`validate_runner_output` currently rejects scheduler-owned payloads from runners:

- `RunStarted`
- `RunCompleted`
- `RetentionRefsAppended`
- `RetentionManifestProjected`
- `StateAttemptStarted`

This was reasonable as a safety measure while runner output was too broad. Domain
states should not be able to mint these events.

But if framework states are the unified model, the rule needs more nuance:

```text
user runners cannot mint framework/scheduler-owned events
framework runners can request framework-owned effects
runtime middleware/store still builds or validates the final typed commit
```

In other words, the runtime should not let arbitrary runner code append events. But
sealed framework states need a way to participate in producing run-start, completion,
retention, and other framework events.

There are two possible designs:

1. Framework runners return specialized typed framework outputs, and middleware builds
   the corresponding events.
2. Framework runners return event payloads, but only for a certified `FrameworkNodeSpec`
   kind whose validator explicitly allows those payload variants.

The first option is cleaner because it prevents framework runners from becoming a
second store API. The second is closer to the branch's current `ErasedRunnerOutput`
shape and may be easier to stage.

In both cases, domain/user states should not return raw scheduler-owned payloads.

## Problem 4: `RunStarted` Is The Hardest Framework State

`RunStarted` currently has special status:

- it must be the first authoritative event for a typed run;
- it binds the certified spec hash, spec artifact, certificate artifact, seed cells,
  descriptor identities, runner executables, adapter executables, source/framework
  evidence, canonicalizer identity, and public-output schema id;
- resume/replay/public-output reads depend on it as the run authority root.

That does not mean `RunStarted` cannot be represented as a framework state. It means
the model must handle the bootstrap boundary deliberately.

Open design options:

1. **Bootstrap state outside the certified node list**
   - Runtime has a sealed internal bootstrap state that executes before a run stream
     exists.
   - It still uses the same middleware pipeline.
   - This keeps `RunStarted` first, but the bootstrap state is not part of the certified
     state program.
   - Risk: this preserves one special lifecycle path.

2. **Certified bootstrap framework node**
   - Typed lowering inserts a framework `BootstrapRun` node into the certified spec.
   - The first commit has `RunStarted` as ordinal 0 and may include bootstrap terminal
     evidence in the same sequence.
   - This makes bootstrap inspectable and hash-defining.
   - Risk: the spec must include the node that persists the spec itself, so the
     canonicalization and event ordering rules must be precise.

3. **Run-start as store-owned commit derived from a framework state output**
   - The framework bootstrap state returns a typed `RunStartOutput`.
   - Middleware/store builds `RunStarted` as the first event.
   - This keeps state execution as the source of the transition while preserving store
     ownership over event shape.
   - Risk: still needs a clear answer for whether the bootstrap state has an attempt
     envelope.

The investigation should not hand-wave this. If `RunStarted` remains the only
non-state exception, the exception must be narrow, named, and justified. Otherwise the
same special-case pattern will keep growing.

## Problem 5: Retention Projection Should Be A Framework State

Retention projection currently lives outside normal state execution:

- app code decides when to project;
- app code persists public-output receipt artifacts;
- runtime code validates and appends the retention manifest projection.

This is a strong candidate for a sealed framework state:

```text
ProjectRetentionManifest
  input: authoritative run stream/projection evidence
  output: retention manifest artifact evidence
  events: RetentionManifestProjected + RetentionRefsAppended
```

It should not be a user-authored state. It should be inserted or scheduled by the
runtime when the certified retention policy says projection is due.

The middleware should persist the manifest bytes before append, then the store should
atomically bind the manifest event and retention refs.

## Problem 6: Public-Output Rendering Is Correct In Shape But Persistence Is Leaky

Public-output rendering is already a framework state in the current branch. That is
the best current precedent.

The remaining issue is that public-output receipt bytes are rebuilt and persisted by
app helpers after events exist. The render runner returns artifact evidence, but the
actual bytes are persisted elsewhere.

The better flow is:

```text
RenderPublicOutputs framework state
  -> returns typed public-output receipt / rendered output
  -> persistence middleware writes receipt/rendered artifacts
  -> middleware/store append CellProduced + PublicOutputProduced + StateAttemptCompleted
```

After that, public-output read APIs should only read already-authoritative evidence.
They should not have to repair missing framework artifacts in app code.

## Problem 7: Verification Is Real, But Its Ownership Is Mixed

Some verification cannot move to the type system:

- persisted bytes may be corrupt;
- event streams may be tampered with;
- artifact stores may return bytes that do not match evidence;
- resume/replay/public-output reads start from stored data, not compiler-known values;
- external side-effect receipts and replay verifiers depend on recorded runtime facts.

But many current verifiers are compensating for missing type/certification/runtime
boundaries.

Verification should be assigned to the earliest sound owner:

| Verification class | Target owner |
| --- | --- |
| wrong state-to-state wiring | type system and typed handles |
| wrong operation expansion path | sealed framework operation invocation and certification |
| wrong public-output handle/schema | type system and certification |
| descriptor/registry authority | certification |
| untrusted spec/certificate bytes | certifier verifier, returning non-forgeable authority |
| run stream event order and historical integrity | runtime resume/read boundary and store projection rebuild |
| state input readiness and attempt boundaries | state execution middleware |
| state output evidence | state execution middleware |
| artifact bytes vs evidence | artifact store plus runtime persistence middleware |
| public-output read authority | runtime read boundary returning non-forgeable authority |
| replay fact/receipt availability | replay runtime and capability-specific verifiers |

The app layer should assemble dependencies and call runtime APIs. It should not be the
place where trust is reconstructed by ad hoc helper calls.

## Current Function Ownership Sketch

This is a starting point for the audit, not a final implementation plan.

| Current function/surface | Current concern | Target owner |
| --- | --- | --- |
| `persist_certified_spec_artifact` | spec artifact staging | bootstrap framework state + persistence middleware |
| `persist_certified_spec_certificate_artifact` | certificate artifact staging | bootstrap framework state + persistence middleware |
| `persist_seed_inputs_for_spec` | seed artifact staging and seed refs | bootstrap framework state + persistence middleware |
| `persist_config_artifact` | config artifact staging | certification/bootstrap input materialization |
| `validate_launch_artifacts` | start evidence validation | bootstrap state middleware/store preconditions |
| `start_run` | manual `RunStarted` append | framework bootstrap state path |
| `persist_framework_public_output_receipts` | delayed render receipt persistence | render framework state + persistence middleware |
| `append_retention_manifest_projection` | manual retention projection append | retention projection framework state |
| `verify_public_output_read_authority` | read authority reconstruction | runtime public-output read boundary |
| `render_typed_public_output` | public-output read rendering | runtime/app read API after authority minted |
| `load_certified_spec_for_run` | persisted authority reconstruction | runtime/certifier read boundary |
| `replay_authority_for_run` | retained evidence reconstruction | runtime replay boundary |
| `validate_run_stream` | historical stream integrity | runtime resume/read boundary |
| `validate_runner_output` | post-state output validation | mandatory state middleware |
| transport `persist_artifact` helpers | domain artifact writes | managed-write capability + middleware |
| artifact-store evidence checks | bytes/evidence integrity | artifact store |

## Middleware Responsibilities

The persistence middleware should be mandatory and runtime-owned. "Middleware" here
does not imply a user-pluggable optional callback. It means a uniform wrapper around
state execution.

For every executable state, including framework states, the middleware should own:

### Before execution

- verify certified runtime authority;
- rebuild or load the authoritative projection;
- check run/cell/attempt/side-effect preconditions;
- materialize typed input cells from certified evidence;
- recover recorded facts and side-effect phase when resuming;
- append or confirm attempt-start evidence where the state kind uses an attempt
  envelope;
- provide only certified capabilities.

### During execution

- prevent ambient IO by giving states only framework capability tokens;
- provide managed platform write capabilities for artifacts, diagnostics, public
  output, and retention staging;
- provide live or replay capability implementations according to runtime mode;
- collect staged writes, typed output values, facts, side-effect evidence, and
  framework-state outputs.

### After execution

- persist staged artifacts and derive evidence;
- verify artifact evidence against output cell/schema/semantic type/producers;
- build framework-owned event payloads when needed;
- validate the terminal payload batch against the certified spec and current
  projection;
- append the typed commit atomically with store preconditions;
- bind retention refs in the run stream;
- avoid making staged/orphan artifacts resume or replay authority until the commit
  lands.

### On failure

- persist only redacted diagnostic evidence when allowed;
- append typed failure events when the runtime contract requires them;
- never persist secrets in errors, artifacts, events, or public output;
- preserve side-effect uncertainty boundaries.

## Framework State Rules

Framework states should be:

- sealed: external crates cannot define new framework node kinds;
- inserted by lowering/certification/runtime only;
- represented in certified specs when they affect run semantics;
- bound to built-in or registry-known runners by certified descriptor identity;
- validated more strictly than user states when they produce framework-owned events;
- unable to bypass store append preconditions.

User states should:

- implement public typed state authoring traits;
- receive only typed inputs and certified capabilities;
- not hand-author run lifecycle events;
- not directly persist authority;
- not append events directly.

## Read Boundaries Are Not Always Mutating States

Public-output reads and replay verification need the same verification primitives, but
they may not always be run-mutating states.

If a read operation is meant to be audited as part of a run, it can be modeled as a
state. If it is a pure query over existing run authority, it should be a runtime read
boundary that:

1. verifies persisted authority;
2. rebuilds projections from the event stream;
3. validates artifact evidence;
4. returns a non-forgeable authority or rendered response.

The important rule is that read boundaries must not invent authority from rendered
JSON, app-level projection caches, or unverified persisted bytes.

## Desired Acceptance Criteria

The architecture is corrected when these are true:

- There are no domain/transport `persist_artifact` helpers that directly write runtime
  authority artifacts from state execution.
- `crates/app` does not contain mandatory runtime persistence logic such as public
  output receipt persistence or seed/spec/certificate authority staging.
- Run-mutating framework work is represented as sealed framework states or a single
  explicitly justified bootstrap exception.
- Public-output rendering, retention projection, run completion, and bootstrap
  persistence all pass through the same mandatory persistence/verification wrapper as
  user state execution.
- User states cannot emit scheduler/framework-owned payloads.
- Framework states can produce framework-owned effects only through certified
  framework node kinds and runtime validation.
- `RunStarted` authority comes from certifier-backed spec authority and is bound by
  runtime/store logic, not by app helper convention.
- Public-output read authority is minted only by runtime verification over certified
  spec, run stream, projection, and artifact evidence.
- Replay authority is rebuilt only from certified spec/certificate evidence and
  retained artifact evidence.
- Orphaned/staged artifacts never become resume, replay, public-output, or retention
  authority without a committed typed event.

## Open Questions For Implementation

1. Can `RunStarted` be represented as a certified bootstrap framework node while
   preserving the invariant that `RunStarted` is the first authoritative event?
2. Should bootstrap have a normal attempt envelope, or should it be a framework state
   with a specialized first-commit envelope?
3. Should `RunCompleted` become a framework state after public-output evidence, or
   remain store-owned derived commit logic?
4. Should retention projection be part of the certified spec graph, or a runtime-minted
   framework state scheduled by retention policy?
5. Should framework runners return raw event payloads, or typed framework outputs that
   middleware converts into events?
6. How should read-only public-output/replay verification share middleware primitives
   without pretending every query mutates the run stream?
7. Which current `validate_*` checks become unnecessary once framework states and
   middleware own persistence, and which remain mandatory defense against corrupt
   persisted history?

## Investigation Plan

Audit every `persist_*`, `verify_*`, and `validate_*` function with this template:

```text
function:
invariant protected:
current caller:
can Rust types enforce it:
should certification enforce it:
should state middleware enforce it:
should framework state enforce it:
should runtime read/resume/replay boundary enforce it:
should storage enforce it:
move/delete/keep:
tests needed:
```

The expected result is not fewer invariants. The expected result is fewer arbitrary
places where invariants are enforced.
