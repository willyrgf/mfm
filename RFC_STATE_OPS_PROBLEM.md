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
need to preserve the existing `PlannedOp`, `PortKey`, `DynContext`, or dynamic DAG authoring APIs.

The proposal is centered on one principle:

```text
operations deterministically expand into typed state programs;
the state machine executes only states;
runtime execution plans are lowered artifacts, not the source of truth.
```

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
typed config
  -> typed operation/state expansion
  -> typed state program
  -> certified lowered execution plan
  -> state machine executes states only
```

The typed state program is the authoritative representation. It captures state inputs, outputs,
effects, capabilities, lineage, and terminal result shape in Rust types. The lowered execution plan
exists so the scheduler can run states, persist artifacts, and replay/resume deterministically. Type
erasure is allowed only after the typed program has been built and certified.

### Design Principles

- Operations are deterministic expansion recipes: a typed configuration plus typed inputs expands
  into states and/or other operations.
- Operations are not runtime execution units. After expansion, the state machine executes states
  only.
- A single state and a larger operation should use the same composition model, so users can build a
  workflow by choosing states directly, operations directly, or a mix of both.
- States are pure by default. A state becomes impure only by declaring an effect and the
  capabilities/adapters it needs.
- IO is never ambient. EVM, Bitcoin, other blockchain families, HTTP, databases, object storage,
  clocks, and other external systems are accessed through typed adapters exposed only to states whose
  effect/capability declarations allow them.
- Values crossing state boundaries are typed MFM values, not anonymous JSON blobs.
- Dependency edges are derived from typed handles. Workflow authors do not manually write state
  edges.
- Operation outputs are Rust types containing typed handles. An operation cannot declare a terminal
  result without returning a handle to a value produced by its expanded state program.
- Runtime errors should represent facts that cannot be known before execution. Invalid wiring,
  invalid lifecycle ordering, and invalid terminal shapes should be rejected by the compiler where
  practical.

### MFM Values

Every value that can cross a state boundary should implement a core value trait:

```rust
pub trait MfmValue:
    serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static
{
    const SEMANTIC_ID: SemanticTypeId;
    const SCHEMA_ID: SchemaId;
}
```

The semantic type id names the meaning of the value. The schema id names its stable serialized
shape. Two values with the same JSON representation but different domain meaning should be different
Rust types.

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
pub struct ProofSideEffectResult { /* ... */ }
pub struct ProofOutput { /* ... */ }

pub struct DcvBuiltConfig { /* ... */ }
pub struct DeployedContract { /* ... */ }
pub struct ConfiguredContract { /* ... */ }
pub struct ValidationReport { /* ... */ }
```

JSON still exists at boundaries: CLI/API input, manifests, artifacts, facts, snapshots, and audit
records. But JSON is the serialization format, not the semantic wiring contract.

### Typed Config Versus Runtime Values

The architecture must distinguish plan-time configuration from runtime values.

Typed configs are deterministic planning inputs. They are decoded before expansion, validated before
lowering, canonicalized for hashing, and safe to persist in manifests or compiled execution specs.
They may decide which states are generated.

Runtime values are produced by states and referenced through typed handles. A runtime value cannot
change the already-certified topology. If a later state needs data produced by an earlier state, that
data must be part of the later state's typed input, not the later state's plan-time config.

For example, if deploy/configure/validate built configuration is produced by a runtime build state,
then deploy, configure, and validate states consume typed config values as inputs. They do not use a
runtime handle to mutate their own planner config after lowering.

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

Passing values from another portfolio execution scope should require an explicit bridge state or
operation, making cross-scope semantics visible in the program.

### States

A state is the only executable unit. Each state declares its configuration, input, output, effect,
and required capabilities:

```rust
pub trait State {
    type Config: MfmConfig;
    type Input;
    type Output: MfmValue;
    type Effect: EffectSpec;
    type Caps: CapabilitySet;

    const KIND: StateKind;

    fn new(config: Self::Config) -> Result<Self, PlanError>
    where
        Self: Sized;
}
```

Execution receives typed input values and returns a typed output:

```rust
#[async_trait]
pub trait StateExec: State {
    async fn run(
        &self,
        input: Self::Input,
        caps: &mut Caps<Self::Caps, Self::Effect>,
        rec: &mut dyn EventRecorder,
    ) -> Result<Self::Output, StateError>;
}
```

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
into typed values from prior outputs.

### Effects, Purity, And Capabilities

States are pure unless they explicitly declare otherwise.

```rust
pub enum Pure {}
pub enum ReadExternal {}
pub enum ApplySideEffect {}
```

A pure state receives no external IO capability. An impure state must declare both its effect and
the capabilities it needs:

```rust
pub struct EvmRead;
pub struct EvmTx;
pub struct BitcoinRead;
pub struct BitcoinTx;
pub struct HttpClient;
pub struct ObjectStore;
pub struct SqlDatabase;
pub struct ArtifactStore;
pub struct Clock;
```

Example:

```rust
impl State for ObserveBatch {
    type Config = ObserveBatchConfig;
    type Input = ObserveBatchInput;
    type Output = ObservationBatchOutput;
    type Effect = ReadExternal;
    type Caps = (EvmRead, BitcoinRead);

    const KIND: StateKind = StateKind::new("portfolio.observe_batch");
}
```

Side-effecting states must declare idempotency at the type level, not only as runtime metadata:

```rust
pub trait SideEffectState: State<Effect = ApplySideEffect> {
    type IdempotencyInput: MfmValue;

    fn idempotency_key(input: &Self::Input) -> Result<IdempotencyKey, StateError>;
}
```

This preserves MFM's strict side-effect model while making side-effect preconditions part of the
state contract.

### Adapters

Adapters are reusable framework capabilities. They encapsulate deterministic request hashing, fact
lookup, replay behavior, external calls, and idempotency handling for external systems.

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

States should reuse these adapters rather than reaching into ambient IO. The platform can provide
live and replay implementations for each capability.

This creates a stable extension surface: future developers implement new states and operations while
reusing the MFM harness for adapters, replay, facts, artifacts, idempotency, tracing, and testing.

### Operations

Operations are typed expansion recipes over states and other operations. They never execute at
runtime.

```rust
pub trait Operation {
    type Config: MfmConfig;
    type Input;
    type Output<'p, 's>;

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

States and operations should share a common expansion interface:

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

### Typed Program Builder

The typed builder is the only way to create state graph structure.

```rust
pub struct ScopeBuilder<'program, 'scope> {
    // internal typed program representation
}

impl<'p, 's> ScopeBuilder<'p, 's> {
    pub fn state<S, I>(
        &mut self,
        state: S,
        config: S::Config,
        input: I,
    ) -> Result<Handle<'p, 's, S::Output>, PlanError>
    where
        S: State + StateExec,
        I: IntoStateInput<S::Input, 'p, 's>;

    pub fn call<O, I>(
        &mut self,
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

- which typed inputs each state consumes
- which typed output each state produces
- state kind and config hash
- operation lineage
- effect and capability declarations
- scope identity
- deterministic ordering metadata

Dynamic fanout is still supported. The number of generated states may depend on typed config, as
long as expansion is deterministic and produces typed handles.

### Certified Lowering

After typed expansion, the program lowers to a certified executable plan:

```rust
pub struct CertifiedPlan<O> {
    typed_spec: TypedExecutionSpec,
    erased_plan: ExecutionPlan,
    _output: PhantomData<O>,
}
```

The lowered plan may use erased runners internally:

```rust
pub trait ErasedNodeRunner {
    async fn run_erased(
        &self,
        store: &mut dyn TypedValueStore,
        caps: &mut RuntimeCaps,
        rec: &mut dyn EventRecorder,
    ) -> Result<(), StateError>;
}
```

This erasure is acceptable only because the typed builder has already certified the program. The
typed execution spec must preserve the evidence needed for replay, audit, and resume:

- node id
- state kind
- state config hash
- operation lineage
- input cells
- input semantic type ids
- input schema ids
- output cell
- output semantic type id
- output schema id
- scope id
- effect kind
- capability set
- idempotency policy
- deterministic lowering version

The runtime scheduler consumes the lowered plan. The audit and replay layers consume the certified
typed execution spec.

### Runtime Execution

The runtime executes states only.

For each node, runtime should:

1. check that dependencies are complete
2. load prior typed output cells
3. deserialize them into the state input type
4. provide only the capabilities allowed by the state's effect/capability declaration
5. execute the state
6. serialize and persist the typed output
7. record event stream entries with typed provenance

Deserialization can still fail if persisted data is corrupt or unavailable. It should not fail
because the planner wired the wrong producer to the wrong consumer.

### Traceability And Reproducibility

Every produced value should have a typed value reference:

```rust
pub struct TypedValueRef {
    pub cell: OutputCellId,
    pub semantic_type: SemanticTypeId,
    pub schema_id: SchemaId,
    pub producer_node: NodeId,
    pub producer_state_kind: StateKind,
    pub operation_lineage: OperationLineage,
    pub scope_id: ScopeId,
    pub config_hash: ArtifactId,
    pub artifact_id: ArtifactId,
}
```

The certified spec plus typed value refs must make it possible to audit:

- which operation expansion created a state
- which state produced a value
- which typed inputs a state consumed
- which artifacts/facts/outputs belong to a value
- which semantic workflow scope a value belongs to
- why replay/resume re-enters at a type-valid boundary

Replay should rebuild the typed program from the stored manifest input, lower it again, and compare
the rebuilt certified spec against the stored certified spec. Resume should continue only at
certified node boundaries.

### Portfolio Example

The portfolio workflow becomes typed staged expansion:

```rust
b.scope::<PortfolioExecution>(|b| {
    let prepared =
        b.state(PrepareSources, cfg.sources, ())?;

    let subjects =
        b.state(ResolveSubjects, cfg.subjects, prepared)?;

    let views =
        b.state(PinViews, cfg.views, prepared)?;

    let valuations =
        b.state(ResolveValuations, cfg.valuations, views)?;

    let batches: Vec<Handle<'_, '_, ObservationBatchOutput>> = cfg
        .batches
        .iter()
        .map(|batch| {
            b.state(
                ObserveBatch,
                batch.clone(),
                (subjects, views, valuations),
            )
        })
        .collect::<Result<_, _>>()?;

    let observations =
        b.state(MergeObservations, (), batches)?;

    let snapshot =
        b.state(AssembleSnapshot, cfg.snapshot, (subjects, views, observations))?;

    let snapshot_artifact =
        b.state(PublishSnapshot, cfg.snapshot_artifact, snapshot)?;

    let report =
        b.state(ProjectReport, (), snapshot)?;

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

### Proof Example

The proof workflow becomes:

```rust
let fact =
    b.state(ReadProofFact, (), ())?;

let side_effect =
    b.state(ApplyProofSideEffect, (), fact)?;

let output =
    b.state(AssembleProofOutput, (), (fact, side_effect))?;

let artifact =
    b.state(PublishOutput, (), output)?;
```

`ApplyProofSideEffect` declares `Effect = ApplySideEffect` and its idempotency input. `PublishOutput`
requires a `ProofOutput`, so it cannot run before `AssembleProofOutput` has produced one.

### EVM Deploy / Configure / Validate Example

The deploy/configure/validate workflow should encode lifecycle stages:

```rust
b.scope::<DcvExecution>(|b| {
    let built =
        b.state(BuildDcvConfig, canonical, ())?;

    let deploy_cfg =
        b.state(SelectDeployConfig, (), built)?;

    let configure_cfg =
        b.state(SelectConfigureConfig, (), built)?;

    let validate_cfg =
        b.state(SelectValidateConfig, (), built)?;

    let deployed =
        b.state(DeployContract, (), deploy_cfg)?;

    let configured =
        b.state(ConfigureContract, (), (configure_cfg, deployed))?;

    let validated =
        b.state(ValidateContract, (), (validate_cfg, configured))?;

    Ok(validated)
})
```

`ValidateContract` expects a `ConfiguredContract`, not a `DeployedContract` or raw contract address.
Validation before configuration is therefore unrepresentable in Rust-authored workflows.

### Compile-Time Guarantees

This architecture should make the following classes of mistakes unrepresentable in Rust-authored
programs:

- consuming `PortfolioSnapshot` before it exists
- passing `PinnedViews` where `ResolvedSubjects` is required
- projecting a portfolio report without a `PortfolioSnapshot`
- declaring an operation result that no state produced
- validating an EVM deployment before configuration
- applying a side effect without its typed idempotency input
- using external adapters from a pure state
- hand-authoring dependency edges that lie about data flow
- confusing same-shape values with different semantic types
- mixing values from separate workflow scopes without an explicit bridge

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
  typed handles, scopes, typed builders, operation expansion, certification

crates/machine
  certified plan executor, scheduler, event streams, replay/resume

crates/effects
  effect specs, capability traits, idempotency contracts

crates/adapters
  EVM, Bitcoin, HTTP, database, storage, and clock adapter implementations

crates/values
  common MFM value traits, semantic ids, schema ids, typed artifact refs

crates/states/*
  reusable executable states

crates/ops/*
  typed operation expansion recipes

bin/cli
  input decoding, launch/resume commands, output rendering

bin/rest-api
  transport only
```

The thin-layer principle remains:

- reusable executable behavior belongs in states and adapters
- ops assemble typed state programs
- binaries parse input, start/resume runs, and render outputs only

### Developer Extension Model

Once the platform is established, external developers should usually implement only values, states,
and operations:

```rust
#[derive(MfmValue)]
pub struct MyOutput {
    // stable public fields
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
- adapter mocks
- replay harnesses
- canonical config hashing
- idempotency helpers
- certified lowering
- trace rendering
- compile-fail tests for invalid wiring
- integration-test helpers for live and replay modes

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

Some validation remains, but it moves to the correct layers: canonicality, storage integrity,
external IO behavior, replay availability, and domain facts.

### Migration Plan

Because backward compatibility is not required, the migration should be a clean break:

1. Add the new typed program core behind a new crate such as `crates/program`.
2. Define `MfmValue`, typed handles, scopes, state traits, operation traits, effects, and
   capabilities.
3. Implement certified lowering into a temporary compatibility execution plan.
4. Port the proof workflow first because it is small and side-effect sensitive.
5. Port portfolio execution next because it proves deterministic dynamic fanout/fanin.
6. Port EVM deploy/configure/validate because it proves lifecycle typestate.
7. Rewrite `mfm-machine` around certified typed plans and remove compatibility lowering.
8. Replace `mfm-sdk` dynamic planning APIs with typed expansion APIs.
9. Delete `PortKey`-driven semantic data flow, dynamic context wiring, and hand-authored state
   dependency edges.
10. Update `docs/design.md`, `docs/architecture.md`, and `docs/ops-and-states.md`.
11. Add compile-fail tests for invalid producer/consumer wiring, wrong lifecycle ordering, missing
    terminal outputs, and illegal adapter use from pure states.

### Risks And Tradeoffs

This design has real costs:

- generic APIs can become complex
- Rust error messages may be harder for workflow authors
- compile times may increase
- object safety is harder before certified lowering
- dynamic plugin support must be layered on top of the typed core
- too much type-level cleverness could harm ergonomics
- schema evolution must be designed deliberately

The proposal should avoid encoding an entire DAG as nested generic types. That would be too rigid
for MFM's dynamic workflows. Typed handles are the better fit: they preserve practical compile-time
producer/consumer guarantees while allowing deterministic dynamic expansion and runtime scheduling.

### Minimal First Slice

The smallest proof of the architecture should include:

1. `MfmValue`
2. `Handle<'program, 'scope, T>`
3. `State` and `StateExec`
4. `Operation`
5. `ScopeBuilder`
6. `CertifiedPlan`
7. a typed proof workflow
8. compile-fail tests showing invalid proof wiring does not compile
9. runtime execution through a lowered plan
10. replay of the same certified typed spec

If that slice works, the architecture is viable enough to expand to portfolio and EVM workflows.
