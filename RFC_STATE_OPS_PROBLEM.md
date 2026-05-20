# RFC: State / Operation Type-Safety Problem Statement

Date: 2026-05-18

Status: problem statement with proposal appendix

## Purpose

This document captures a core design problem in the current MFM state and operation model.

The main body intentionally does not propose a solution. Its purpose is to describe the mismatch
between the expected model and the model currently implemented in the repository, so future design
work can start from a clear shared understanding of the problem. Proposal appendices are separated
from the problem statement so they can be compared, replaced, or refined independently.

Correctness, determinism, reproducibility, and traceability are core platform features for MFM, not
secondary implementation qualities. MFM is intended to produce executions that can be trusted,
replayed, audited, and explained. For that reason, structural graph validity is not enough. The
state and operation model must preserve domain intent strongly enough that the compiler can reject
invalid executable programs before they become run plans.

This is also one of the reasons Rust is a deliberate fit for the platform. The goal is not merely to
write the runtime in a memory-safe systems language. The goal is to use Rust's type system to encode
as much of the execution contract as practical, so correctness properties that are knowable before
execution do not depend on strings, JSON shape, naming conventions, and review discipline.

## Non-Goal

This document does not claim that every semantic property of an MFM execution can or should be
proven at compile time.

Some errors are inherently runtime concerns: IO failures, external system failures, chain state,
clock-dependent observations, replay data availability, storage failures, concurrency, and domain
facts that cannot be known before execution.

The problem is narrower and more actionable: many producer/consumer, transition, lifecycle-stage,
and terminal-result guarantees that should be represented at the type level are currently
represented through string identifiers, string ports, JSON context values, planner conventions, and
runtime validation.

## Expected Model

The expected model is that MFM states and operations should form a strongly typed executable
program.

An operation should be able to expand into a sequence or graph of states in a way that is checked by
the Rust type system. The typechecker should reject invalid state sequences, invalid transitions,
invalid input/output wiring, and invalid terminal shapes before the program can run.

In that expected model:

- states are not just dynamically scheduled handlers
- state transitions are part of the typed program structure
- operations are planning constructs over states, not runtime execution units
- operations plus typed configuration can be stacked recursively, but must ultimately expand into a
  typed state program before execution
- users can compose a workflow from larger operations, individual states, or a mix of both, with
  typed configuration injected at the operation or state boundary
- operation expansion produces a well-typed state program, not merely a value-level graph
- a state that requires a prior value can only be placed after a state that produces that value
- a state that consumes a typed artifact, typed context value, or typed domain output can only be
  wired to a producer of the same expected type
- an operation's declared result shape is guaranteed by its state program
- the state machine executes states only; after the planning/expansion phase there are no
  operations left to execute
- states are pure by default unless explicitly tagged as impure
- impure states declare the external capabilities/adapters they need, such as EVM, Bitcoin, other
  blockchain families, HTTP, databases, object storage, clocks, or other external systems
- invalid sequences are unrepresentable, not merely rejected later by planner/runtime validation
- runtime errors are reserved primarily for real runtime concerns: IO failures, external system
  failures, replay data availability, storage failures, concurrency, and domain errors that cannot
  be known statically

The expectation is not merely that the graph is acyclic or that named edges point to existing nodes.
The expectation is that the state program is semantically well-typed.

For example, a portfolio execution flow should not merely be a graph containing nodes named
`prepare_execution_sources`, `resolve_subjects`, `pin_execution_views`,
`resolve_valuation_inputs`, `observe`, `assemble_snapshot`, and `project_report`. It should be a
typed execution program where each step's input requirements are satisfied by prior typed outputs,
and where impossible arrangements cannot compile.

The expected developer model follows from this. Once the platform core is established, developers
and users should mostly implement new states and operations while reusing MFM's framework harness:
typed expansion, deterministic lowering, adapters, effect discipline, artifact/fact handling,
replay, resume, tracing, and tests. The safety of composed workflows should come primarily from the
typed framework, not from each operation author manually reconstructing the same conventions.

## Current Model

The current system is closer to a dynamic workflow DAG executor than a strongly typed state program.

The high-level flow is:

1. An operation receives JSON configuration.
2. The operation expands into `PlannedOp`.
3. The SDK recursively flattens planned operations into one `ExecutionPlan`.
4. The execution plan contains a flat `StateGraph`.
5. The state graph contains `StateNode` values and `DependencyEdge` values.
6. Each `StateNode` holds a dynamically erased `Arc<dyn State>`.
7. Each `DependencyEdge` connects two states by `StateId`.
8. The runtime topologically orders the graph and executes states.
9. States communicate through context keys and JSON values.

This superficially matches the desired rule that the state machine executes states only. The
problem is that the operation expansion phase produces a value-level state graph instead of a typed
state program. Operations can be stacked and flattened, but their composition is validated through
names, JSON config, child bindings, and runtime state behavior rather than through the Rust type
system.

Important current representation boundaries:

- `Operation::expand(...)` receives `op_config: &serde_json::Value`.
- `PlannedOp` records leaf or composite shape at value level.
- `StateNode` stores the concrete state implementation behind `DynState`.
- `DependencyEdge` is an edge between string-like state identifiers.
- `OpInterface` uses string-like `PortKey` values for imports and exports.
- `DynContext` reads and writes `serde_json::Value` by `ContextKey`.
- `StateOutcome` reports execution metadata such as snapshot policy, not a typed output or typed
  next-state value.

This representation erases most semantic type information before the runtime executes the plan.

## Current Guarantees

The current implementation does enforce a number of important value-level guarantees.

The runtime validates basic graph shape:

- empty plans are rejected
- duplicate state identifiers are rejected
- dependency edges that reference missing states are rejected
- cycles are rejected
- resume rejects run history that does not match the resolved plan

The SDK planner validates additional planning rules:

- operation interfaces cannot contain duplicate imports or duplicate exports
- leaf plans cannot be empty
- leaf state lineage must match deterministic lowering rules
- leaf dependency edges must reference existing leaf states
- composite child operation identifiers must be unique
- child operation order must be acyclic
- child import bindings must reference declared imports
- child export bindings must reference declared exports
- every declared import must be bound
- every declared export must be re-exported where required
- duplicate lowered state identifiers are rejected
- duplicate pipeline export ports are rejected
- side-effecting states must declare idempotency metadata

These checks are valuable, but they are not the same as type-level correctness.

They validate values produced by planners. They do not make invalid state programs impossible to
construct.

They also add implementation weight. Because the current model loses semantic type information
early, the repository needs more defensive validation, naming rules, context qualification,
lineage tracking, deserialization checks, metadata checks, regression tests, and review conventions
to make the platform safe enough in practice. Much of that code exists to compensate for facts the
compiler cannot currently see.

## Missing Guarantees

The current system does not let the compiler prove that a state graph is semantically valid.

The compiler cannot prove that:

- a state only runs after all of its required typed inputs have been produced
- a state consumes the same type that a producer exports
- an operation stack fully expands into a typed state program whose state-level inputs and outputs
  are correct
- configuration injected at an operation boundary and configuration injected directly at a state
  boundary have the same typed meaning after expansion
- a context key points to the expected logical value
- an operation's imports and exports have stable typed meaning beyond their string names
- a state's declared metadata is consistent with its actual behavior
- a state is pure unless it explicitly declares an impure effect
- an impure state can access only the external adapters/capabilities it declared
- a side-effecting state is placed only after every required precondition state
- a report state can only run after a snapshot state that produced the correct snapshot type
- a validation state can only run after the deployment/configuration state it is meant to validate
- every non-terminal state has a valid continuation
- the terminal state of an operation produces the operation's declared result shape
- skipped states cannot invalidate later typed assumptions
- replay/resume will re-enter a state program at a type-valid boundary
- third-party states and operations can be safely composed by users without re-implementing the same
  validation, naming, context, effect, adapter, and replay conventions

These properties may be true for a particular operation because the code was written carefully and
tested. They are not generally enforced by the type system.

## Structural Validity vs Semantic Validity

The current implementation can reject many structurally invalid plans.

Examples of structural validity:

- every edge endpoint exists
- the graph has no cycle
- imports and exports use known names
- child operation IDs are unique
- all declared imports are bound

Structural validity is necessary, but it is weaker than semantic validity.

Examples of semantic validity:

- `ResolveSubjectsState` receives prepared source payloads with the expected type and meaning
- `ObserveCompiledBatchState` receives resolved subjects, pinned views, and resolved valuations
  produced by the appropriate prior semantic states
- `AssembleSnapshotState` receives observations that correspond to the same execution plan,
  portfolio config, subjects, and pinned views
- `ProjectReportState` receives a portfolio snapshot, not some other JSON value bound to the same
  string key
- an EVM validation state validates the results of the deployment/configuration states that are part
  of the same typed workflow

The current planner can often enforce the existence of named bindings. It cannot generally prove
the semantic type and lineage of the value behind those bindings.

## Type Erasure Points

The current design contains several type erasure points that prevent the Rust compiler from seeing
the full state program.

### Dynamic State Objects

States are stored as `Arc<dyn State>`.

This allows heterogeneous state implementations to live in one graph, but it hides each state's
concrete input/output type from the compiler. Once a state is in a `StateNode`, the runtime only
knows that it implements `State`; it does not know the state-specific semantic transition it
represents.

### String State Identifiers

Edges are represented as `from: StateId` and `to: StateId`.

This proves that two named nodes are ordered. It does not prove that the downstream state is allowed
to follow the upstream state, or that the upstream state produced what the downstream state needs.

### String Ports

Imports and exports are represented as `PortKey`.

This proves that two operations agree on a string. It does not prove that they agree on the type,
schema, domain meaning, provenance, or lifecycle stage of the value behind that string.

### JSON Context

State data flow goes through `DynContext` as `serde_json::Value`.

Typed reads exist as convenience helpers, but the type is recovered by deserialization at runtime.
That means a wrong producer, wrong port, wrong context key, or wrong JSON shape becomes a runtime
state error instead of a compile-time error.

### Untyped State Outcomes

`StateOutcome` does not encode a typed output or typed continuation.

The runtime can know that a state succeeded and whether snapshotting was requested. It cannot infer
from the outcome that the state produced a particular typed value required by a later state.

## Problem Taxonomy

### 1. Invalid Topology

Examples:

- graph is empty
- graph contains a cycle
- edge references a missing state
- state IDs collide

This class is mostly handled by current planner/runtime validation.

### 2. Invalid Interface Wiring

Examples:

- a child import is not bound
- a binding references an unknown child
- a binding references an unknown export
- a pipeline contains duplicate exported port names

This class is partially handled by current SDK validation, but only at the level of names and graph
shape.

### 3. Invalid Semantic Transition

Examples:

- a report state is placed before a snapshot state
- a validation state is wired to the wrong deployment result
- a state that depends on resolved subjects is wired after a state that produces unrelated JSON
- a side-effecting state appears before the state that establishes its idempotency input

This class is not generally rejected by the compiler.

### 4. Invalid Data Shape

Examples:

- a context key exists but contains the wrong JSON shape
- a producer writes a value that the consumer cannot deserialize
- a producer and consumer agree on a port name but disagree on its schema
- a dynamically generated set of child operations creates a shape the downstream state does not
  actually support

This class is usually discovered inside state handlers at runtime.

### 5. Invalid Data Meaning

Examples:

- the value has the correct JSON shape but belongs to the wrong logical run segment
- the value was produced from a different portfolio config than the downstream state assumes
- a validation result corresponds to a different network, contract, or control scope than expected
- two values share a structural type but represent different semantic lifecycle stages

This class is especially difficult for the current model because JSON shape and port names do not
fully capture semantic provenance.

### 6. Invalid Terminal Shape

Examples:

- an operation declares a report export but the state graph does not guarantee report production
- a workflow completes without producing the expected final typed output
- the final snapshot contains a value under the expected key, but not the expected typed result

This class is currently governed by planner conventions, runtime context state, and tests rather
than a type-level operation result.

## Concrete Repository Symptoms

### Portfolio Execution

The portfolio execution workflow is intended to follow a semantic progression:

1. prepare execution sources
2. resolve subjects
3. pin execution views
4. resolve valuation inputs
5. observe compiled batches
6. merge observations
7. assemble snapshot
8. project report

The current operation code constructs child operation instances and bindings that express this
progression. The SDK planner validates the resulting graph and binding names.

However, the state sequence is not represented as a typed program. The compiler cannot reject a
wrong semantic arrangement by itself. The validity of the workflow depends on the planner emitting
the right children, ports, bindings, and configs, plus runtime states reading and deserializing the
expected context values.

### Proof Workflow

The proof operation manually wires:

1. read facts
2. apply side effect
3. assemble output
4. publish output

The code constructs explicit dependency edges between these states. The runtime can validate the
edges structurally.

The compiler does not know that `apply_side_effect` requires the read fact output, or that
`assemble_output` requires both the read fact and side-effect result. Those requirements are encoded
by context keys and runtime reads.

### EVM Deploy / Configure / Validate

The deploy/configure/validate workflow has strong semantic ordering requirements:

1. build or obtain deploy/configure/validate config
2. deploy
3. configure
4. validate

The current code constructs child operations and ordering/binding relationships. Those
relationships are validated as planned values.

The compiler does not know that configure must consume the contract address produced by deploy, or
that validate must validate the result of the same deployment/configuration sequence. Those
relationships are expressed through ports, context keys, config values, and runtime behavior.

## Consequences

The current model pushes semantic correctness out of the compiler and into:

- planner implementation discipline
- runtime validation
- state-local deserialization checks
- tests
- documentation
- naming conventions
- review discipline

This creates several risks:

- a graph can be structurally valid but semantically wrong
- two states can agree on a port name but disagree on the value's meaning
- JSON shape errors are found during execution instead of during compilation
- operation result correctness is not guaranteed by the operation type
- operation stacking can look valid as child bindings while still failing to express a well-typed
  state-level program
- state ordering mistakes can compile if they still produce valid `StateId` edges
- purity and adapter access are governed by metadata and implementation discipline rather than by a
  state capability type
- future dynamic or plugin operations can bypass intended semantic relationships while satisfying
  current structural checks
- future developers must learn and repeat framework conventions instead of receiving them as typed
  guarantees from the platform
- replay/resume correctness relies on the resolved value-level plan matching past events rather than
  on a typed execution program
- important invariants live in conventions spread across ops, states, tests, and docs

The system can be robust in practice, but its robustness is not rooted in a type-level proof of the
state program.

It also makes the implementation larger and harder to reason about than it should be. The dynamic
representation requires code to repeatedly recover, validate, qualify, and audit relationships that
could otherwise be carried by types. A stronger typed state-program model should let the compiler
handle more of this work directly, reducing the amount of hand-written safety scaffolding and
lowering the number of places where semantic invariants must be restated.

## Why This Is A Core Design Problem

MFM is intended to be a reproducible, auditable execution engine. For that kind of system, the
state/operation boundary is not just an implementation detail. It is the place where domain intent
becomes executable behavior.

If that boundary is mostly dynamic, then the most important correctness properties are enforced
after the fact:

- after an operation expands
- after the planner validates names and graph shape
- after runtime starts
- after a state reads JSON from context
- after a deserialization attempt succeeds or fails

The expected model is stronger: the state/operation boundary should carry enough type information
that invalid state programs are rejected before they can exist as executable plans.

The current design does not meet that expectation.

## Problem Statement

The core problem is that MFM currently represents state programs as dynamically assembled,
value-level graphs of erased state handlers connected by string identifiers, string ports, and JSON
context values.

This allows flexible recursive planning and runtime execution, but it prevents Rust's type system
from proving that operation expansion produces semantically valid state programs.

Operation expansion should be the planning phase for states: operations plus typed configuration
should recursively reduce to configured states, and the state machine should then execute states
only. The current model follows that shape operationally, but not semantically: the final plan is
not a type-checked state program whose inputs, outputs, effects, adapter access, provenance, and
terminal shape are guaranteed by Rust.

As a result, invalid sequences and invalid transitions are representable. Some are rejected by
planner or runtime validation. Some are discovered only when a state handler reads or deserializes
context. Some can only be prevented by careful tests and code review.

This falls short of the desired model: operations should expand into state programs whose valid
transitions, typed inputs, typed outputs, and terminal results are enforced at the type level, with
runtime errors reserved for conditions that genuinely cannot be known before execution.

## Appendix: Proposal 1 - Typed Expansion Core With Certified Lowering

This proposal is a candidate architecture for replacing the current dynamic state/operation core.
It is intentionally breaking. It assumes the platform is still on a development branch and does not
need to preserve the existing `PlannedOp`, `PortKey`, `DynContext`, or dynamic DAG authoring APIs as
public framework surfaces.

The proposal is centered on one stricter principle:

```text
typed state programs are the only semantic executable surface;
certified typed execution specs are the only runtime contract;
erased execution plans are implementation artifacts.
```

In this proposal, "state program" means a sequence or graph of states. It does not mean operations.
Operations may author, plan, and explain a state program, but they are not part of the executable
program after expansion.

This is not an incremental validation layer over the current dynamic DAG model. The dynamic model is
the architectural defect this proposal replaces. A compatibility lowering path may exist during
migration, but it must not remain the semantic contract.

### Architecture Summary

MFM should move from:

```text
JSON config
  -> Operation::expand
  -> PlannedOp
  -> flat StateGraph<dyn State + StateId edges + PortKey bindings + JSON context>
  -> runtime validation and execution
```

to:

```text
typed planning config
  -> deterministic typed operation/state expansion
  -> typed state program IR
  -> certified typed execution spec
  -> erased runner plan derived from the certified spec
  -> state machine executes states only
```

The certified typed execution spec is the authoritative representation. It captures state inputs,
outputs, effects, capabilities, authoring provenance, planning lineage, value provenance, stable node
identity, public terminal output shape, replay boundaries, and resume rules.

The erased runner plan exists only so the scheduler can execute work. It must be reproducibly
derivable from the certified spec. It must not carry extra semantics that are absent from the
certified spec.

This proposal is intentionally scoped to a versioned, Rust-authored typed core first. Dynamic
third-party plugins are required as a future platform capability, but they are not part of the first
implementation slice and must not weaken the typed core. Until a future plugin certification design
exists, certified execution should assume states, operations, adapters, and connectors are compiled
Rust components registered with explicit versions.

### Design Principles

- Operations are deterministic expansion recipes over typed planning config and typed input
  handles.
- Operations are not runtime execution units. After expansion, the state machine executes states
  only.
- A single state and a larger operation should use the same composition model, so users can build a
  workflow by choosing states directly, operations directly, or a mix of both.
- States are pure by construction unless they implement an explicit effect-specific execution
  trait.
- IO is never ambient. EVM, Bitcoin, other blockchain families, HTTP, databases, object storage,
  clocks, and other external systems are accessed through typed adapters exposed only to states whose
  effect/capability declarations allow them.
- Values crossing state boundaries are typed MFM values stored in typed cells, not anonymous JSON
  blobs.
- JSON context is not a semantic dataflow mechanism. JSON may appear at API, artifact, snapshot,
  public-output, and audit boundaries, but state-to-state wiring must use typed cells and handles.
- Dependency edges are derived from typed handles. Workflow authors do not manually write state
  edges.
- Operation outputs are Rust types containing typed handles. An operation cannot declare a terminal
  result without returning a handle to a value produced by its expanded state program.
- Runtime-produced values cannot change the topology of the already-certified same-run program.
- Side effects require typed intent, typed idempotency material, typed receipt, and typed replay
  verification.
- States, adapters, connectors, operation descriptors, value schemas, public output schemas, and the
  lowering algorithm are versioned contracts. Breaking semantic changes create new versions; old
  versions remain valid for replay/resume.
- Resume and replay are allowed only against the exact certified typed execution spec that produced
  the run history.
- Runtime errors should represent facts that cannot be known before execution. Invalid wiring,
  invalid lifecycle ordering, illegal adapter access, and invalid terminal shapes should be rejected
  by the compiler where practical.

### MFM Values

Every value that can cross a state boundary must implement a core value trait:

```rust
pub trait MfmValue:
    serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static
{
    const SEMANTIC_ID: SemanticTypeId;
    const SCHEMA_ID: SchemaId;
}
```

The semantic type id names the meaning of the value. The schema id names its stable serialized
shape. Two values with the same JSON representation but different domain meaning must be different
Rust types.

For the first implementation slice, schema ids may be explicit constants rather than automatically
derived. That is enough to make schema identity visible in specs and events while leaving the full
schema derivation and migration policy for later design. The minimum rule is:

- every persisted `MfmValue`, `MfmConfig`, and public output type declares a stable schema id
- schema ids are manually versioned at first
- breaking serialized-shape or semantic changes require a new schema id
- hashed value/config/public-output structures must use canonical serialization and must not contain
  floats
- no secret-bearing type may implement `MfmValue`, `MfmConfig`, or public output traits

Examples:

```rust
pub struct PreparedSources { /* ... */ }
pub struct ResolvedSubjects { /* ... */ }
pub struct PinnedViews { /* ... */ }
pub struct ResolvedValuations { /* ... */ }
pub struct ObservationBatchOutput { /* ... */ }
pub struct MergedObservations { /* ... */ }
pub struct PortfolioSnapshot { /* ... */ }
pub struct PortfolioReport { /* ... */ }

pub struct ProofFact { /* ... */ }
pub struct ProofSideEffectIntent { /* ... */ }
pub struct ProofSideEffectReceipt { /* ... */ }
pub struct ProofSideEffectResult { /* ... */ }
pub struct ProofOutput { /* ... */ }

pub struct DcvBuiltConfig { /* ... */ }
pub struct DeployConfig { /* ... */ }
pub struct ConfigureConfig { /* ... */ }
pub struct ValidateConfig { /* ... */ }
pub struct DeployedContract { /* ... */ }
pub struct ConfiguredContract { /* ... */ }
pub struct ValidationReport { /* ... */ }
```

Typed artifact references should also carry the value type:

```rust
pub struct ArtifactRef<T: MfmValue> {
    pub id: ArtifactId,
    pub digest: ContentDigest,
    _value: PhantomData<T>,
}
```

JSON still exists at boundaries: CLI/API input, manifests, artifacts, facts, snapshots, and audit
records. But JSON is the serialization format, not the semantic wiring contract.

### Secret Boundary

Secrets must remain below the state-machine boundary. Keystore secrets, private keys, mnemonics,
password material, raw signing keys, decrypted bytes, and similar secret-bearing values must not be
MFM values, typed cells, artifacts, facts, events, public outputs, or error details.

State programs may reference secret-bearing systems only through non-secret typed references and
capabilities, such as:

```rust
pub struct WalletRef { /* non-secret wallet label or account id */ }
pub struct KeyLabel { /* non-secret keystore entry label */ }
pub struct SignerRef { /* non-secret signing authority reference */ }
```

The keystore layer should be modeled as a typed adapter/capability boundary. States may ask a
keystore capability to sign, decrypt, or unlock by using non-secret references and non-secret
configuration. They must never receive the secret material itself. This makes secret leakage a type
and boundary violation, not a convention that state authors must remember.

### Typed Planning Config

The architecture must distinguish planning config from runtime values.

```rust
pub trait MfmConfig:
    serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static
{
    const SCHEMA_ID: SchemaId;

    fn validate(&self) -> Result<(), ConfigError>;
}
```

Typed configs are deterministic planning inputs. They are decoded before expansion, validated before
lowering, canonicalized for hashing, and safe to persist in manifests and certified specs. Hashed
config structures must use canonical serialization and must not contain floats. Typed configs may
decide which states are generated.

Certified specs must retain enough config material to recreate every state runner. A node-level
config hash alone is not sufficient. Each node must carry either canonical config bytes or a
content-addressed reference to those bytes, plus the config schema id used to decode them.

Runtime values are produced by states and referenced through typed handles. A runtime value cannot
change the already-certified topology of the same run. If a later state needs data produced by an
earlier state, that data must be part of the later state's typed input, not the later state's
planning config.

If a runtime-produced value is needed to choose future topology, MFM must create an explicit
planning boundary. Acceptable boundaries include:

- compile a new certified child spec
- start a child run
- resume into a newly certified continuation
- split the workflow into two user-visible planning phases

Using a runtime handle to mutate planner config after lowering is forbidden.

### Typed Handles

State outputs are referenced through typed handles:

```rust
#[derive(Clone, Copy)]
pub struct Handle<'program, 'scope, T: MfmValue> {
    cell: OutputCellId,
    _program: PhantomData<&'program ()>,
    _scope: PhantomData<&'scope ()>,
    _value: PhantomData<T>,
}
```

The `T` parameter enforces producer/consumer type compatibility. A state that requires
`Handle<ResolvedSubjects>` cannot receive `Handle<PinnedViews>`.

Handles are small immutable references to planned output cells. They can be copied or cloned freely
inside expansion code without copying the produced runtime value.

The `'program` brand prevents mixing handles from unrelated typed programs. The `'scope` brand
prevents accidentally mixing values from different semantic workflow instances inside the same
program. This matters when two values have the same Rust type but belong to different execution
segments.

For example, snapshot assembly can require all inputs to belong to the same portfolio execution
scope:

```rust
fn assemble_snapshot<'p, 's>(
    subjects: Handle<'p, 's, ResolvedSubjects>,
    views: Handle<'p, 's, PinnedViews>,
    observations: Handle<'p, 's, MergedObservations>,
) -> Handle<'p, 's, PortfolioSnapshot>;
```

Passing values from another portfolio execution scope must require an explicit bridge state or
operation. The bridge creates a new value with new provenance, making the semantic transition
visible in the program.

Scope ids must be persisted in the certified spec and value provenance records. This matters for
workflows with the same Rust type in multiple semantic scopes, such as two portfolio executions in
one program, two chains producing the same `DeployedContract` type, or staging and production
deployments with identical schemas.

The precise bridge API is intentionally left for a later design step. The first typed core should
still brand scopes and reject accidental cross-scope wiring. Explicit cross-scope transfer should be
added only when real workflows force the exact bridge semantics.

### Typed Optionality And Skip Cells

Optional runtime paths must be represented explicitly in the typed program. A plain missing context
key or absent JSON value is not enough for audit or replay.

The preferred model is a typed optional/skip cell rather than an untracked absence:

```rust
pub enum MaybeValue<T: MfmValue> {
    Produced(T),
    Skipped(SkipReason),
}

pub struct SkipReason {
    pub code: SkipCode,
    pub explanation: String,
}
```

The exact API may use `MaybeValue<T>`, `OptionalCell<T>`, `Skipped<T>`, or a wrapper around
`Option<T>`, but the certified spec and event stream must preserve skip provenance. A skipped value
must have a typed cell identity, semantic type id, schema id, producer node, and reason. Downstream
states must declare whether they accept a produced value only or a typed maybe/skip value.

### States

A state is the only executable unit. Each state declares its configuration, input, output, effect,
capability set, kind, and version:

```rust
pub trait StateSpec {
    type Config: MfmConfig;
    type Input;
    type Output: MfmValue;
    type Effect: EffectSpec;
    type Caps: CapabilitySet;

    const KIND: StateKind;
    const VERSION: StateVersion;

    fn new(config: Self::Config) -> Result<Self, PlanError>
    where
        Self: Sized;
}
```

`StateSpec` describes the state. Execution must be split by effect class. There must not be one
universal execution trait that hands every state a context object, IO provider, clock, artifact
store, and event recorder.

State input types may be a single value, tuples of values, typed collections, or domain-specific
input structs:

```rust
pub struct ObserveBatchInput {
    pub subjects: ResolvedSubjects,
    pub views: PinnedViews,
    pub valuations: ResolvedValuations,
}
```

During typed expansion, inputs are handles. During execution, the runtime materializes those handles
into typed values from prior output cells.

### Versioned State Descriptors And Runner Rehydration

A certified spec cannot persist Rust trait objects. It must persist enough information for the
runtime to rehydrate the exact versioned runner that was certified.

Every executable state kind must therefore have a registered descriptor:

```rust
pub struct StateDescriptor {
    pub kind: StateKind,
    pub version: StateVersion,
    pub config_schema: SchemaId,
    pub input_types: InputTypeSpec,
    pub output_type: ValueTypeSpec,
    pub effect: EffectKind,
    pub capabilities: CapabilitySpec,
}
```

The runtime must resolve `(StateKind, StateVersion)` through a state registry before lowering to an
erased runner. State descriptors are immutable semantic contracts. If a state's config shape, input
contract, output contract, effect behavior, capability needs, idempotency behavior, or replay
semantics changes incompatibly, the state version must change.

Certified node specs must reference canonical state config bytes or a content-addressed config
artifact, not only a config hash. Hashes prove identity; bytes are required to reconstruct the
runner, audit the planned behavior, and verify replay. Old state versions must remain resolvable as
long as runs certified against them may need replay/resume.

### Effects, Purity, And Capabilities

States are pure unless they explicitly opt into an effect-specific execution trait.

```rust
pub enum Pure {}
pub enum ReadExternal {}
pub enum WriteInternal {}
pub enum ApplySideEffect {}
```

Pure states receive no external capability:

```rust
pub trait PureState: StateSpec<Effect = Pure> {
    fn run(&self, input: Self::Input) -> Result<Self::Output, StateError>;
}
```

Read states may observe external systems through declared read capabilities. Reads produce recorded
facts and may be replayed from those facts:

```rust
#[async_trait]
pub trait ReadState: StateSpec<Effect = ReadExternal> {
    async fn run(
        &self,
        input: Self::Input,
        caps: &Self::Caps,
    ) -> Result<Self::Output, StateError>;
}
```

Internal write states may write runtime-managed artifacts, outputs, facts derived from deterministic
inputs, or audit records without mutating external systems. They are not pure, because they need
runtime-managed persistence capabilities, but they are also not external side effects. This category
prevents artifact publication and output rendering from being mislabeled as either pure computation
or external mutation.

Effect and capability traits should be sealed by the framework. A pure state must not be able to
obtain IO accidentally. A read state must not be able to submit transactions. A state that writes
internal artifacts must receive only the scoped artifact/output capabilities declared in its
descriptor. A side-effect state must receive only the specific external mutation capabilities
declared in its descriptor.

Side-effect states are split into typed intent, application, and confirmation:

```rust
#[async_trait]
pub trait SideEffectState: StateSpec<Effect = ApplySideEffect> {
    type Intent: MfmValue;
    type IdempotencyInput: MfmValue;
    type Receipt: MfmValue;
    type Confirmation: MfmValue;

    fn prepare_intent(&self, input: Self::Input) -> Result<Self::Intent, StateError>;

    fn idempotency_key(
        &self,
        input: &Self::Input,
        intent: &Self::Intent,
    ) -> Result<IdempotencyKey, StateError>;

    async fn apply(
        &self,
        intent: Self::Intent,
        caps: &Self::Caps,
    ) -> Result<Self::Receipt, StateError>;

    async fn confirm(
        &self,
        input: Self::Input,
        intent: Self::Intent,
        receipt: Self::Receipt,
        caps: &Self::Caps,
    ) -> Result<(Self::Confirmation, Self::Output), StateError>;
}
```

This separation forces side-effect state authors to model external mutation explicitly. It also
gives replay a durable boundary: replay verifies intent, idempotency key, receipt, and confirmation
instead of reapplying the mutation.

### Adapters

There must be no typed-state equivalent of:

```text
IoProvider(namespace: String, request: serde_json::Value) -> serde_json::Value
```

External systems are represented by typed capabilities. Capabilities define typed requests, typed
responses, canonical request hashing, fact keys, redaction behavior, and replay behavior.
Capabilities are the state-facing contract; adapters/connectors are versioned implementations of
those contracts.

Examples:

```rust
#[async_trait]
pub trait EvmReadCap {
    async fn call_contract(
        &self,
        request: EvmCallRequest,
    ) -> Result<RecordedFact<EvmCallResult>, AdapterError>;
}

#[async_trait]
pub trait EvmTxCap {
    async fn submit_transaction(
        &self,
        intent: EvmTransactionIntent,
        idempotency: IdempotencyKey,
    ) -> Result<EvmTransactionReceipt, AdapterError>;
}
```

Examples:

- EVM read adapter
- EVM transaction adapter
- Bitcoin read adapter
- Bitcoin transaction adapter
- HTTP adapter
- SQL database adapter
- object storage adapter
- artifact store adapter
- clock adapter

States must reuse these adapters rather than reaching into ambient IO. The platform must provide
live and replay implementations for each capability. Replay implementations must answer only from
recorded facts or receipts. They must not silently fall back to live IO.

Adapter and connector versions are part of the certified execution contract when they affect
canonical request formation, fact-key derivation, receipt interpretation, replay verification,
redaction, or externally visible behavior. Breaking changes create new adapter/connector versions;
old versions must remain available for replay/resume of specs that reference them.

This creates a stable extension surface: future developers implement new states and operations while
reusing the MFM harness for adapters, replay, facts, artifacts, idempotency, tracing, and testing.

Adapters own:

- canonical request serialization
- request and response schema ids
- fact key derivation
- idempotency ledger interaction
- secret redaction
- no-secret persisted payload validation
- live versus replay behavior
- adapter-specific trace records

States own domain transformation logic. They must not implement bespoke fact lookup, replay, or
idempotency plumbing.

### Idempotency And Side-Effect Replay

Side effects require a durable idempotency model. Runtime metadata is insufficient.

For every side-effect node, the certified spec and event stream must record:

- state kind and version
- scope id
- input cell ids
- input semantic type ids and schema ids
- intent semantic type id and schema id
- canonical intent hash
- idempotency input semantic type id and schema id
- idempotency key
- adapter capability
- receipt semantic type id and schema id
- confirmation semantic type id and schema id
- side-effect attempt record id

The idempotency ledger must be keyed by certified state identity plus canonical intent hash and
idempotency key. Re-running the same certified side-effect state must either:

- observe that the effect already succeeded and return the recorded receipt
- observe that the effect is pending and follow the adapter's confirmation protocol
- fail without reapplying if the previous state is ambiguous

Replay must verify the recorded receipt and confirmation against the certified spec. It must not
apply the side effect.

### Operations

Operations are typed expansion recipes over states and other operations. They never execute at
runtime.

```rust
pub trait Operation {
    type Config: MfmConfig;
    type Input;
    type Output<'p, 's>;

    const KIND: OperationKind;
    const VERSION: OperationVersion;

    fn expand<'p, 's>(
        &self,
        config: Self::Config,
        input: Self::Input,
        builder: &mut ScopeBuilder<'p, 's>,
    ) -> Result<Self::Output<'p, 's>, PlanError>;
}
```

An operation's output is usually a struct of typed handles:

```rust
pub struct PortfolioOutputs<'p, 's> {
    pub snapshot: Handle<'p, 's, PortfolioSnapshot>,
    pub snapshot_artifact: Handle<'p, 's, ArtifactRef<PortfolioSnapshot>>,
    pub report: Handle<'p, 's, PortfolioReport>,
}
```

An operation cannot claim to export a report unless its expansion returns a
`Handle<PortfolioReport>`.

States and operations must share a common expansion interface:

```rust
pub trait Expandable {
    type Config;
    type Input;
    type Output<'p, 's>;

    fn expand<'p, 's>(
        self,
        config: Self::Config,
        input: Self::Input,
        builder: &mut ScopeBuilder<'p, 's>,
    ) -> Result<Self::Output<'p, 's>, PlanError>;
}
```

A single state is the smallest expandable unit. An operation is a larger expandable unit. This lets
users compose workflows by stacking operations, selecting individual states, or mixing both with
typed configuration injection.

Recursive operation expansion is valid only if every child operation reduces to the same typed
program IR and certified spec. No child operation may smuggle a dynamic graph around the typed
builder.

### Typed Program Builder

The typed builder is the only way to create executable program structure.

```rust
pub struct ScopeBuilder<'program, 'scope> {
    // internal typed program representation
}

impl<'p, 's> ScopeBuilder<'p, 's> {
    pub fn state<S, I>(
        &mut self,
        key: StableNodeKey,
        config: S::Config,
        input: I,
    ) -> Result<Handle<'p, 's, S::Output>, PlanError>
    where
        S: StateSpec,
        I: IntoStateInput<S::Input, 'p, 's>;

    pub fn call<O, I>(
        &mut self,
        key: StableOperationKey,
        operation: O,
        config: O::Config,
        input: I,
    ) -> Result<O::Output<'p, 's>, PlanError>
    where
        O: Operation;
}
```

The builder derives dependencies from handles. Workflow authors do not manually write
`DependencyEdge` values. The builder records:

- which typed input cells each state consumes
- which typed output cell each state produces
- state kind and version
- operation kind and version
- canonical state config hash and config artifact/reference
- planning lineage
- effect and capability declarations
- idempotency policy
- scope identity
- stable node key
- deterministic ordering metadata

The builder must reject:

- duplicate stable node keys in the same scope
- cross-program handles
- cross-scope handles without an explicit bridge
- undeclared capability use
- missing terminal output handles
- dynamic graph fragments that bypass typed handles

### Dynamic Deterministic Expansion

Dynamic fanout is still supported, but only when expansion remains deterministic. The number of
generated states may depend on typed planning config, as long as expansion is deterministic and
produces typed handles.

Required:

- stable domain keys for generated children
- canonical sorting before emission
- duplicate-key rejection
- deterministic node and cell id derivation
- explicit `NonEmptyHandles<T>` or equivalent when a domain rule requires at least one child
- no dependence on wall-clock time, random values, live IO, database reads, or same-run state output
  during expansion

If expansion must depend on observed runtime data, that is not same-run expansion. It is a new
planning boundary and must produce a new certified spec.

### Stable Node And Cell Identity

Stable ids are part of reproducibility. They must be derived, not accidentally produced.

Node and cell ids should be derived from:

- authoring provenance descriptor and version
- planning lineage
- scope id
- stable local node key
- state kind and version
- canonical config hash
- deterministic lowering algorithm version

They must not depend on:

- `HashMap` iteration order
- allocation order
- pointer identity
- thread scheduling
- non-canonical JSON
- runtime wall-clock time
- process-local counters unless the counter input is itself derived from canonical ordering

Changing the lowering algorithm version must change the certified spec hash.

### Certified Typed Execution Spec

After typed expansion, MFM produces a certified typed execution spec:

```rust
pub struct TypedExecutionSpec {
    pub spec_version: SpecVersion,
    pub lowering_version: LoweringVersion,
    pub authoring: AuthoringProvenance,
    pub state_program: StateProgramSpec,
    pub outputs: PublicOutputSpec,
}
```

Authoring provenance names what authored the state program. It is non-executable audit metadata:
the state machine executes the `StateProgramSpec`, not the operations or composition helpers that
authored it. MFM supports three authoring modes: operation expansion, direct state composition, and
a mix of operations and directly declared states:

```rust
pub enum AuthoringProvenance {
    OperationExpansion {
        operation: OperationDescriptor,
        config_hash: ContentDigest,
    },
    StateComposition {
        descriptor: StateCompositionDescriptor,
        config_hash: ContentDigest,
    },
    MixedComposition {
        descriptor: MixedCompositionDescriptor,
        config_hash: ContentDigest,
    },
}

pub struct StateProgramSpec {
    pub nodes: Vec<NodeSpec>,
    pub cells: Vec<CellSpec>,
}
```

This keeps the state machine model explicit. A workflow built from a single state, a sequence or DAG
of states, an operation, or a mix of states and operations all reduce to a certified
`StateProgramSpec`. The runtime executes only the states in that spec.

Each node spec must include:

- node id
- state kind and version
- state descriptor id or `(state kind, state version)`
- state config hash
- state config artifact/reference
- planning lineage
- scope id
- input cells
- input semantic type ids
- input schema ids
- output cell
- output semantic type id
- output schema id
- effect kind
- capability set
- adapter/connector version references where behaviorally relevant
- idempotency policy
- stable node key
- deterministic predecessor ids

Each cell spec must include:

- cell id
- semantic type id
- schema id
- producer node id
- scope id
- content-addressing policy
- redaction policy
- artifact/fact policy where applicable

The spec is certified only after a lowerer verifies that:

- every consumed cell has exactly one producer
- every handle type matches the consumer input type
- every `(state kind, state version)` resolves to a registered immutable state descriptor
- every state config reference resolves to canonical bytes with the expected schema id and hash
- every state has a deterministic id
- every dependency is derived from typed handles
- every effect matches the state's execution trait
- every capability is allowed by the state effect
- every side-effect state has typed intent, idempotency, receipt, and confirmation
- every public output is reachable from produced typed cells
- every dynamic collection is canonically ordered
- every persisted value has semantic and schema ids

### Certified Lowering

Certified lowering turns the typed execution spec into an erased runner plan:

```rust
pub struct CertifiedPlan<O> {
    pub spec: TypedExecutionSpec,
    pub spec_hash: ContentDigest,
    erased_plan: ErasedExecutionPlan,
    _output: PhantomData<O>,
}
```

The lowered plan may use erased runners internally:

```rust
pub trait ErasedNodeRunner {
    async fn run_erased(
        &self,
        store: &mut dyn TypedValueStore,
        caps: &mut dyn CertifiedRuntimeCaps,
        events: &mut dyn EventSink,
    ) -> Result<(), StateError>;
}
```

Trait object erasure is acceptable only after certification. Before certification, the authoring API
must remain typed and generic. After certification, object safety and scheduler practicality become
implementation concerns, not semantic compromises.

The erased plan must not be persisted as the authoritative contract. The persisted contract is the
certified typed execution spec plus its canonical hash.

Lowering must use registered state descriptors and runner factories. It must not deserialize a
spec into arbitrary executable code, and it must not allow unregistered state kinds to run. The
erased runner is an implementation detail derived from a certified, versioned spec.

### Runtime Execution

The runtime executes states only.

For each node, runtime must:

1. verify that the run uses the expected certified spec hash
2. check that predecessor cells are complete
3. load typed input cells by cell id
4. verify stored semantic type ids and schema ids against the certified spec
5. resolve the node's registered state descriptor and canonical config reference
6. deserialize input cells into the state input type
7. provide only the capabilities allowed by the certified spec
8. execute the state through the effect-specific runner
9. serialize and persist the typed output cell
10. content-address the output
11. append provenance-bearing events

Deserialization may still fail if persisted data is corrupt or unavailable. It must not fail because
the planner wired the wrong producer to the wrong consumer.

The runtime may maintain context snapshots for observability, but states must not read their
semantic inputs from those snapshots.

### Traceability And Provenance

Every produced value must have a typed value reference:

```rust
pub struct TypedValueRef {
    pub cell: OutputCellId,
    pub semantic_type: SemanticTypeId,
    pub schema_id: SchemaId,
    pub producer_node: NodeId,
    pub producer_state_kind: StateKind,
    pub producer_state_version: StateVersion,
    pub planning_lineage: PlanningLineage,
    pub scope_id: ScopeId,
    pub config_hash: ContentDigest,
    pub content_digest: ContentDigest,
    pub artifact_id: Option<ArtifactId>,
    pub fact_id: Option<FactId>,
}
```

The certified spec plus typed value refs must make it possible to audit:

- which authoring source created a state
- which state produced a value
- which typed inputs a state consumed
- which artifacts, facts, and outputs belong to a value
- which semantic workflow scope a value belongs to
- which adapter capability was used
- which side-effect intent and receipt were recorded
- why replay or resume re-enters at a type-valid boundary

### Replay And Resume

Replay and resume must be driven by certified specs, not by best-effort graph reconstruction.

On run start, MFM must persist:

- canonical manifest input
- certified typed execution spec artifact
- certified spec hash
- authoring provenance descriptor
- lowering version
- framework version
- public output schema id

The run event stream must bind events to the certified spec hash. A `RunStarted` event without a
spec hash is insufficient for the new architecture.

Replay must:

1. load the stored certified spec
2. rebuild the typed program from the stored manifest input when the original operation code is
   available
3. lower it again
4. compare canonical spec bytes or spec hash
5. reject if the rebuilt spec differs
6. execute using replay adapters only
7. verify every fact, receipt, typed cell, and terminal output against the spec

Resume must:

1. load the stored certified spec
2. verify the requested code path still produces the same certified spec, or explicitly run against
   the stored spec artifact
3. inspect event history at certified node boundaries
4. resume only from nodes whose predecessors have complete typed cells
5. reject if any completed node has an output semantic id, schema id, content digest, effect record,
   receipt, or provenance record inconsistent with the spec

Matching state ids is not enough. Resume must reject if state kind, state version, state descriptor,
config hash, config bytes/artifact reference, input cells, output type, effect, capability set,
adapter/connector version, idempotency policy, or terminal output shape changed.

### Terminal Outputs And Public API

Operation outputs must be typed structs of handles. String exports are not sufficient.

The public output contract must be derived from a typed output spec:

```rust
pub trait PublicOutputs<'p, 's> {
    const PUBLIC_SCHEMA_ID: SchemaId;

    fn output_cells(&self) -> Vec<PublicOutputCell>;
}
```

CLI and API rendering must load typed terminal cells and render stable JSON from the public output
schema. The renderer may emit JSON, but it must not discover final results by looking up arbitrary
context keys.

Artifact-bearing outputs must use typed artifact refs. Public output schemas are part of the
user-facing API. They require rustdoc and versioning.

### Portfolio Example

The portfolio workflow becomes typed staged expansion:

```rust
b.scope::<PortfolioExecution>(|b| {
    let prepared = b.state::<PrepareSources, _>(
        StableNodeKey::new("prepare_sources"),
        cfg.sources,
        (),
    )?;

    let subjects = b.state::<ResolveSubjects, _>(
        StableNodeKey::new("resolve_subjects"),
        cfg.subjects,
        prepared,
    )?;

    let views = b.state::<PinViews, _>(
        StableNodeKey::new("pin_views"),
        cfg.views,
        prepared,
    )?;

    let valuations = b.state::<ResolveValuations, _>(
        StableNodeKey::new("resolve_valuations"),
        cfg.valuations,
        views,
    )?;

    let batches: Vec<Handle<'_, '_, ObservationBatchOutput>> = cfg
        .batches
        .canonical_sorted()
        .map(|batch| {
            b.state::<ObserveBatch, _>(
                StableNodeKey::from_domain_key(batch.key()),
                batch.config,
                (subjects, views, valuations),
            )
        })
        .collect::<Result<_, _>>()?;

    let observations = b.state::<MergeObservations, _>(
        StableNodeKey::new("merge_observations"),
        cfg.merge,
        batches,
    )?;

    let snapshot = b.state::<AssembleSnapshot, _>(
        StableNodeKey::new("assemble_snapshot"),
        cfg.snapshot,
        (subjects, views, observations),
    )?;

    let snapshot_artifact = b.state::<PublishSnapshot, _>(
        StableNodeKey::new("publish_snapshot"),
        cfg.snapshot_artifact,
        snapshot,
    )?;

    let report = b.state::<ProjectReport, _>(
        StableNodeKey::new("project_report"),
        cfg.report,
        snapshot,
    )?;

    Ok(PortfolioOutputs {
        snapshot,
        snapshot_artifact,
        report,
    })
})
```

The compiler prevents wiring `PinnedViews` where `ResolvedSubjects` is required. The shared scope
prevents assembling a snapshot from values belonging to different portfolio execution instances
unless an explicit bridge is introduced.

Zero observation batches must be modeled explicitly. If empty observation sets are valid,
`MergeObservations` should accept `Vec<Handle<ObservationBatchOutput>>`. If not, it should require
`NonEmptyHandles<ObservationBatchOutput>`.

### Proof Example

The proof workflow becomes:

```rust
let fact = b.state::<ReadProofFact, _>(
    StableNodeKey::new("read_fact"),
    cfg.read,
    (),
)?;

let side_effect = b.state::<ApplyProofSideEffect, _>(
    StableNodeKey::new("apply_side_effect"),
    cfg.apply,
    fact,
)?;

let output = b.state::<AssembleProofOutput, _>(
    StableNodeKey::new("assemble_output"),
    cfg.assemble,
    (fact, side_effect),
)?;

let artifact = b.state::<PublishOutput, _>(
    StableNodeKey::new("publish_output"),
    cfg.publish,
    output,
)?;
```

`ApplyProofSideEffect` declares `Effect = ApplySideEffect`, typed intent, typed idempotency input,
typed receipt, and typed confirmation. `PublishOutput` requires a `ProofOutput`, so it cannot run
before `AssembleProofOutput` has produced one.

### EVM Deploy / Configure / Validate Example

The deploy/configure/validate workflow should encode lifecycle stages:

```rust
b.scope::<DcvExecution>(|b| {
    let built = b.state::<BuildDcvConfig, _>(
        StableNodeKey::new("build_config"),
        cfg.build,
        (),
    )?;

    let deploy_cfg = b.state::<SelectDeployConfig, _>(
        StableNodeKey::new("select_deploy_config"),
        cfg.deploy,
        built,
    )?;

    let configure_cfg = b.state::<SelectConfigureConfig, _>(
        StableNodeKey::new("select_configure_config"),
        cfg.configure,
        built,
    )?;

    let validate_cfg = b.state::<SelectValidateConfig, _>(
        StableNodeKey::new("select_validate_config"),
        cfg.validate,
        built,
    )?;

    let deployed = b.state::<DeployContract, _>(
        StableNodeKey::new("deploy_contract"),
        (),
        deploy_cfg,
    )?;

    let configured = b.state::<ConfigureContract, _>(
        StableNodeKey::new("configure_contract"),
        (),
        (configure_cfg, deployed),
    )?;

    let validated = b.state::<ValidateContract, _>(
        StableNodeKey::new("validate_contract"),
        (),
        (validate_cfg, configured),
    )?;

    Ok(validated)
})
```

`ValidateContract` expects a `ConfiguredContract`, not a `DeployedContract` or raw contract address.
Validation before configuration is therefore unrepresentable in Rust-authored workflows.

Deploy and configure are side-effecting EVM states. They must use typed EVM transaction intents,
idempotency keys, receipts, and confirmations. Validate is normally a read state using typed EVM
read capabilities.

### Plugin And Third-Party Workflows

Rust-authored third-party states and operations can compile against the typed framework and receive
the same compile-time guarantees.

Dynamic third-party plugins are required as a future capability, but the first architecture slice
does not need to solve them. The typed core must be designed so dynamic plugins can be added later
without reopening the semantic contract.

Dynamic plugins cannot be allowed to bypass the typed core by submitting arbitrary erased graphs. A
future dynamic plugin design must either:

- expose Rust types and compile as a typed extension, or
- submit a declarative typed spec that passes runtime certification against trusted registered
  state descriptors, semantic type ids, schema ids, state kinds and versions, effect declarations,
  capabilities, idempotency contracts, adapter/connector versions, and public outputs.

If a plugin cannot provide that evidence, it cannot participate in certified MFM execution.

Until that future design exists, dynamic plugin execution should remain out of scope for certified
MFM runs. The typed Rust-authored core should not keep compatibility hooks that allow arbitrary
dynamic graphs to enter the runner.

### Compile-Time Guarantees

This architecture should make the following classes of mistakes unrepresentable in Rust-authored
programs:

- consuming `PortfolioSnapshot` before it exists
- passing `PinnedViews` where `ResolvedSubjects` is required
- projecting a portfolio report without a `PortfolioSnapshot`
- declaring an operation result that no state produced
- validating an EVM deployment before configuration
- applying a side effect without typed idempotency material
- applying a side effect without a typed intent and receipt
- using external adapters from a pure state
- using a transaction adapter from a read-only state
- hand-authoring dependency edges that lie about data flow
- confusing same-shape values with different semantic types
- mixing values from separate workflow scopes without an explicit bridge
- treating a skipped value as a produced value without accepting typed optionality
- exporting terminal results through string context keys
- using a runtime-produced value to change same-run topology

These guarantees should be locked with compile-fail tests using `trybuild`.

Required compile-fail cases:

- wrong producer type passed to a consumer
- same Rust type from wrong scope passed without a bridge
- pure state attempts to access IO
- read state attempts to submit a transaction
- side-effect state lacks idempotency input
- side-effect state lacks typed receipt
- operation output references an unavailable handle
- terminal output lacks a public schema id
- dynamic state graph bypasses the typed builder
- runtime value is used where planning config is required
- secret-bearing type attempts to cross the state-machine boundary as an `MfmValue`
- produced-only consumer is passed a `MaybeValue<T>` or skipped cell without explicit handling

### Runtime Responsibilities

Some checks remain runtime concerns:

- malformed user input before it is decoded into typed config
- external IO failures
- chain state and external system behavior
- database, HTTP, and storage availability
- replay fact availability
- corrupted persisted artifacts
- clock-dependent observations
- concurrency and scheduler failures
- dynamic fanout cardinality when domain rules require at least one generated child
- domain facts observed from external systems

This is the intended boundary. Runtime should handle facts unknowable before execution, not recover
from invalid semantic wiring.

### Crate Boundary Proposal

A breaking rewrite should split the core around typed programs:

```text
crates/program
  typed handles, scopes, typed builders, operation expansion, typed IR,
  certified specs, lowering verification

crates/values
  MfmValue, MfmConfig, semantic ids, schema ids, typed artifact refs,
  canonical serialization helpers

crates/effects
  effect specs, sealed capability traits, idempotency contracts,
  side-effect intent/receipt/confirmation traits

crates/capabilities
  state-facing capability traits and typed request/response contracts

crates/collectors/*
  typed domain clients and request/response models for external systems

crates/transports/*
  live and replay connector implementations for capabilities/collectors

crates/machine
  certified spec executor, scheduler, typed value store, event streams,
  replay, resume

crates/states/*
  reusable executable states implemented against typed state/effect traits

crates/ops/*
  typed operation expansion recipes

bin/cli
  input decoding, launch/resume commands, output rendering

bin/rest-api
  transport only
```

This split should avoid creating a new monolithic adapter crate. Capability traits are the
state-facing contract; collectors and transports can continue to own concrete domain clients and
live/replay connector behavior where that matches the existing repository shape.

The thin-layer principle remains:

- reusable executable behavior belongs in states and typed capabilities/connectors
- ops assemble typed state programs
- binaries parse input, start/resume runs, and render typed public outputs only

`crates/sdk` should not remain the owner of semantic planning. It can either become a thin
compatibility facade over `crates/program` during migration or be replaced by the typed program API.

### Developer Extension Model

Once the platform is established, external developers should usually implement only values, states,
and operations:

```rust
#[derive(MfmValue)]
pub struct MyOutput {
    // stable public fields
}

#[derive(MfmConfig)]
pub struct MyConfig {
    // canonical planning fields
}

#[mfm_state(effect = Pure)]
impl MyState {
    // typed config, typed input, typed output
}

#[mfm_operation]
impl MyOperation {
    // deterministic expansion over states/operations
}
```

The framework should provide:

- typed builder APIs
- canonical config hashing
- stable id derivation
- adapter mocks
- replay harnesses
- idempotency helpers
- certified lowering
- trace rendering
- public output rendering
- compile-fail test helpers
- integration-test helpers for live and replay modes

Future developers should not need to understand the erased runner plan, event encoding details, or
manual graph validation to create correct states and operations.

### Rust Type System Fit

The proposal should use Rust's type system aggressively but not encode the entire DAG as nested
types.

Good fits:

- associated types for state config, input, output, effect, and capabilities
- generic associated types for operation outputs branded by program and scope lifetimes
- phantom/lifetime branding for program and scope isolation
- sealed traits for framework-owned effect markers and capability set internals
- typestate values for lifecycle transitions such as deployed -> configured -> validated
- trait objects only after certification
- `trybuild` tests for API misuse

Risks:

- overly nested generics can make error messages unusable
- operation output GATs may require careful API design
- object safety before certification can distort the authoring model
- monomorphization cost may increase with many states
- macros can hide important type errors if they are too magical

The right compromise is typed handles plus a typed IR. Handles give compile-time producer/consumer
and scope guarantees without requiring the whole workflow graph to exist as one enormous generic
type.

### Complexity Reduction

The typed expansion core should reduce or eliminate several categories of defensive code:

- manual semantic port compatibility checks
- repeated context key shape checks
- producer/consumer schema checks for known typed edges
- terminal output existence checks
- hand-authored dependency edges
- review-only semantic ordering conventions
- deserialization checks caused by wrong wiring
- repeated lineage recovery checks
- duplicated side-effect precondition checks
- ad hoc replay fact lookup inside states
- adapter access checks performed after state construction

Some validation remains, but it moves to the correct layers: canonicality, storage integrity,
external IO behavior, replay availability, idempotency ambiguity, and domain facts.

### Migration Plan

Backward compatibility is not required, but the migration must avoid cementing the old model as the
new foundation.

Recommended order:

1. Add the new typed program core behind `crates/program`.
2. Define `MfmValue`, `MfmConfig`, typed handles, scopes, stable ids, typed IR, and certified spec
   structs.
3. Define the minimal schema id policy: explicit manually versioned schema ids first, with canonical
   no-float serialization for hashed values/configs/outputs.
4. Define the secret boundary: secret-bearing types cannot implement value/config/output traits and
   keystore access crosses only through non-secret typed references and capabilities.
5. Define immutable state descriptors and a state registry for `(StateKind, StateVersion)` runner
   rehydration.
6. Define effect-specific execution traits before porting states.
7. Define sealed typed capability traits and live/replay connector interfaces.
8. Define certified spec hashing, canonical serialization, config artifact/reference handling, and
   provenance records.
9. Add `trybuild` compile-fail tests for the core invalid programs before porting workflows.
10. Implement certified lowering into a temporary compatibility runner plan.
11. Port the proof workflow first. Success requires no semantic JSON context dataflow.
12. Port proof side effects with typed intent, idempotency, receipt, and replay verification.
13. Port portfolio execution next. Success requires deterministic fanout/fanin, stable dynamic ids,
    scope branding, and typed terminal outputs.
14. Port EVM deploy/configure/validate. Success requires lifecycle typestate and typed EVM
    side-effect contracts.
15. Rewrite `mfm-machine` around certified typed specs, typed cells, and typed provenance.
16. Replace `mfm-sdk` dynamic planning APIs with typed expansion APIs or reduce it to a facade.
17. Delete `PortKey`-driven semantic data flow, dynamic context wiring, and hand-authored state
    dependency edges from public authoring APIs.
18. Update `docs/design.md`, `docs/architecture.md`, and `docs/ops-and-states.md`.
19. Expand compile-fail tests and replay/resume integration tests for every migrated workflow.

Temporary compatibility lowering is acceptable only if:

- the certified typed spec remains the authoritative persisted contract
- no new workflow is authored directly against dynamic `PlannedOp`
- semantic JSON context is not used by migrated typed states
- deletion of the compatibility layer is an explicit migration milestone

### Deferred Design Decisions

The following decisions are intentionally not required for the first implementation slice, but they
must remain explicit open design items:

- full schema id derivation and migration policy beyond explicit manually versioned schema ids
- dynamic third-party plugin certification and loading
- exact cross-scope bridge API and bridge provenance model
- final typed optionality API shape (`MaybeValue<T>`, `OptionalCell<T>`, `Skipped<T>`, or another
  equivalent)
- long-term policy for keeping old state/adapter/connector versions available for replay, including
  whether replay may bind to reproducible build artifacts or only in-process registered runners
- macro ergonomics for deriving values, configs, states, operations, and descriptors
- schema evolution policy for public terminal outputs

These are deferred because the proof slice can validate the core without them. They must not be
resolved by reintroducing dynamic erased graphs, string ports, or JSON context dataflow as semantic
surfaces.

### Risks And Tradeoffs

This design has real costs:

- generic APIs can become complex
- Rust error messages may be harder for workflow authors
- compile times may increase
- object safety is harder before certified lowering
- dynamic plugin support must be deferred until it can be constrained by certification
- schema evolution must be designed deliberately
- old state/adapter/connector versions must remain replayable, which creates version-retention costs
- effect-specific traits require more up-front framework design
- stable id derivation becomes part of the public correctness contract

These costs are acceptable because the alternative is worse: a reproducible execution platform whose
core semantic contract is enforced mostly by string names, JSON shape checks, runtime validation,
and code review.

The proposal should avoid encoding an entire DAG as nested generic types. That would be too rigid
for MFM's dynamic workflows. Typed handles are the better fit: they preserve practical compile-time
producer/consumer, lifecycle, effect, and scope guarantees while allowing deterministic dynamic
expansion and runtime scheduling.

### Minimal First Slice

The smallest proof of the architecture should include:

1. `MfmValue`
2. `MfmConfig`
3. explicit manually versioned schema ids
4. secret-boundary marker/enforcement sufficient to prevent secret-bearing types from implementing
   value/config/output traits
5. `Handle<'program, 'scope, T>`
6. typed scopes and stable node keys
7. typed optional/skip cell prototype
8. `StateSpec`
9. immutable `StateDescriptor` and state registry for runner rehydration
10. `PureState`, `ReadState`, `WriteInternalState`, and `SideEffectState`
11. one typed read capability with live and replay implementations
12. one typed internal artifact/output capability
13. one typed side-effect capability with receipt verification
14. `Operation`
15. `ScopeBuilder`
16. `TypedExecutionSpec`
17. certified spec hashing and `RunStarted`/event binding to the certified spec hash
18. certified lowering into a temporary erased runner plan
19. a typed proof workflow
20. proof side-effect intent/idempotency/receipt modeling
21. typed terminal outputs
22. replay of the same certified typed spec
23. resume rejection when the rebuilt certified spec differs
24. compile-fail tests showing invalid proof wiring does not compile

If that slice works without semantic JSON context dataflow, the architecture is viable enough to
expand to portfolio and EVM workflows. If it still depends on context keys, generic IO, or
hand-authored edges for semantic correctness, it has not solved the core problem.
