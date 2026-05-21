# RFC: State / Operation Type-Safety Problem Statement

Date: 2026-05-18

Status: problem statement; Proposal 1 split to `RFC_TYPED_CORE_PROPOSAL_1.md`

## Purpose

This document captures a core design problem in the current MFM state and operation model.

This document intentionally does not define the full implementation solution. Its purpose is to
describe the mismatch between the expected model and the model currently implemented in the
repository, so design and implementation work can start from a clear shared understanding of the
problem.

The detailed typed-core design contract draft lives in `RFC_TYPED_CORE_PROPOSAL_1.md`. For the
typed-core implementation effort, Proposal 1 is the source of truth. The repository design
documents, including `docs/design.md` and `docs/architecture.md`, must be updated to match it as
part of the implementation effort.

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
- resume rejects completed or failed historical state ids that are not present in the resolved plan

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
- `ApplySideEffect` states must declare a non-empty idempotency key

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

The portfolio execution workflow is intended to follow a semantic DAG progression. At a high level:

1. prepare execution sources
2. resolve subjects
3. pin execution views
4. resolve valuation inputs
5. observe compiled batches
6. merge observations
7. assemble snapshot
8. project report

The current operation code constructs child operation instances and bindings that express this
progression, including branches after prepared sources and dependency edges induced by bindings.
The SDK planner validates the resulting graph and binding names.

However, the state DAG is not represented as a typed program. The compiler cannot reject a
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

The compiler does not know that configure and validate belong to the same intended
deploy/configure/validate lifecycle. In the current config model, configure may consume the contract
address produced by deploy or use an inline `contract_address`, while ordering is still represented
by planned values and runtime behavior. These relationships are expressed through ports, context
keys, config values, and runtime behavior rather than typed lifecycle values.

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

## Proposal Shape

The preferred solution direction is captured in `RFC_TYPED_CORE_PROPOSAL_1.md`. In compact form,
Proposal 1 replaces the dynamic graph and JSON context model with:

- typed values and typed planning config
- branded typed handles for state-to-state dataflow
- operations as deterministic typed expansion recipes, not runtime units
- a certified typed execution spec as the runtime contract
- effect-specific state traits and capability-mediated framework IO
- managed platform writes distinguished from external domain mutations
- typed public outputs, not terminal string context keys
- append-only typed events, typed cells, typed facts, and explicit replay/resume evidence

For the typed-core rewrite, Proposal 1 is the source of truth. The implementation effort must update
the existing authoritative repository documents to match the proposal, including `docs/design.md`,
`docs/architecture.md`, and the relevant crate READMEs.
