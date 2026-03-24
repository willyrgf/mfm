# RFC: From Scattered States To Semantic Execution And Recursive Operation Planning

Status: implemented

Last updated: 2026-03-24

## Summary

This RFC proposes restructuring MFM execution away from protocol-specific state proliferation and
toward a semantics-first runtime plus a single recursive planning model for the whole project.

This design is implemented on the current `portfolio_tracker v2` runtime path. The built-in
portfolio flow now runs through the fixed semantic state family, recursive op flattening, and the
semantic adapter catalogs described below.

`mfm::portfolio::snapshot` is the first proving ground, not the only target.

The portfolio runtime should be rebuilt from a small fixed set of semantic execution states:

- `PrepareExecutionSources`
- `PinExecutionViews`
- `ResolveSubjects`
- `ResolveValuationInputs`
- `ObserveCompiledBatch`
- `MergeObservations`
- `AssembleSnapshot`
- `ProjectReport`

The key change is to move protocol and network specialization out of the state graph shape and into
adapter registries plus a pure semantic compiler, centered on the following primary semantics:

- `Subject`
- `NetworkView`
- `Instrument`
- `Position`
- `Venue`
- `Valuation`
- `Observation`

This RFC also proposes a project-wide planning model:

- a root operation may expand into sub-operations
- sub-operations may expand into more sub-operations or directly into states
- the SDK planner must recursively flatten the result into one final `StateGraph` /
  `ExecutionPlan` before runtime starts
- the runtime executes states only

The right slogan is:

- ops compile ops
- runtime executes states

This RFC treats:

- `Swap`, `Borrow`, and `Lend` as `ActionSemantics`
- `ContractRef` as an adapter-owned implementation detail carried only inside compiled bindings
- `Snapshot` and `Report` as projection and read-model semantics

The resulting design preserves the current execution model and invariants:

- binaries remain transport-only
- ops remain thin, deterministic planners
- reusable executable logic lives in shared state/runtime crates
- planning still happens entirely before runtime starts
- replayability stays centered on append-only events plus content-addressed artifacts
- state logic continues to use `IoProvider` only
- secrets remain forbidden from persisted surfaces

The intended outcome is not "fewer adapters". It is:

- fewer state types
- one project-wide planning abstraction
- a more stable semantic execution topology

Protocol-specific growth should happen in adapters and typed config decoders, not in the op DAG or
in runtime graph-shaping logic.

## Goals

- Replace protocol-first state expansion with semantic planning over canonical config.
- Unify MFM around one planning abstraction: recursive operation planning that flattens into one
  final `ExecutionPlan` before runtime starts.
- Make `Subject`, `NetworkView`, `Instrument`, `Position`, `Venue`, `Valuation`, and
  `Observation` the central vocabulary of the refactor.
- Use `mfm::portfolio::snapshot` as the first proving ground for rules that are intended to apply
  to the whole project.
- Keep the op planner deterministic and free of ambient IO.
- Keep all domain planning in planner layers and out of states.
- Preserve the current canonical `Observation`, snapshot, and report flow where possible.
- Make adapter selection explicit, testable, and deterministic.
- Make planner-time batch partitioning explicit, testable, and deterministic.
- Allow EVM, Aave, Bitcoin, zkEVM, and future protocol families to plug into the same semantic
  execution model.
- Force at least one non-EVM portfolio source through the new semantic model as part of the
  refactor so generic semantics are validated immediately.
- Keep protocol-specific validation and acquisition logic out of generic semantic states.

## Non-Goals

- This RFC does not propose changing append-only run semantics, artifact addressing, or checkpoint
  advancement rules.
- This RFC does not propose moving domain logic into binaries or widening `crates/machine` with
  portfolio-specific behavior.
- This RFC does not require dynamic runtime plugin loading in v1. A deterministic in-process
  registry compiled into the app bundle is sufficient.
- This RFC does not require changing the public CLI entrypoint or shell wrapper contract.
- This RFC does not introduce nested runtime planning, runtime-dispatched sub-ops, or state-driven
  graph reshaping.
- This RFC does not introduce child runs, nested manifests, or engine-visible op hierarchy for
  recursive planning.
- This RFC does not change the current executor contract that execution remains sequential in v1;
  graph independence may express future parallelism, but fan-out/join remains deferred.
- This RFC does not preserve backward compatibility for the internal portfolio execution model,
  internal planner surfaces, or the current protocol-first semantic vocabulary. The implementation
  is a deliberate hard cutover.

## Central Semantic Model

The refactor should be organized around seven primary semantics.

### 1. `Subject`

The acting or observed subject.

Today this is often expressed as wallet config and resolved wallet identity. Over time this must
broaden to other subject forms, but the semantic role stays the same: "who owns or acts".

### 2. `NetworkView`

The pinned execution view.

This is not just "network". It is the concrete replayable view of a network or ledger for one run:
family plus pinned anchor.

Naming note:

- `NetworkView` is the semantic noun
- `ExecutionView`, `PinExecutionViews`, and `ExecutionViewPin` below are operational planner/runtime
  forms of the same semantic concept
- they should not be treated as a second primary semantic center

### 3. `Instrument`

The unit being accounted for.

Examples:

- native coin
- fungible token
- quote unit
- BTC UTXO-denominated asset

### 4. `Position`

The holding or liability semantics attached to an instrument.

Examples:

- spot balance
- lending deposit
- lending debt
- liquidity share
- staked claim
- UTXO set

### 5. `Venue`

The semantic venue in which a position exists.

Examples:

- Aave market
- reserve inside an Aave market
- Uniswap pool
- vault

This is deliberately distinct from `ContractRef`. Venue is semantic and should use stable
user-authored ids. Contract references are implementation detail.

### 6. `Valuation`

The logic that turns quantities into priced values.

Examples:

- fixed unit price
- direct unit price
- derived unit price

### 7. `Observation`

The canonical read-model record produced by execution.

An observation binds together:

- subject
- pinned network view
- instrument
- position semantics
- optional venue
- quantity
- valuation outputs

These seven semantics are the primary actors of the refactor.

### Observation identity must be explicit

Observation merge, deterministic ordering, and duplicate detection must use an explicit semantic
identity, not ad hoc tuples chosen independently by states.

The minimum observation identity should be:

```rust
struct ObservationKey {
    subject_id: String,
    network_view_id: String,
    instrument_id: String,
    position_kind: String,
    venue_id: Option<String>,
    discriminator: Option<String>,
}
```

Rules:

- `ObservationKey` is planner-owned
- merge and canonical ordering should be expressed in terms of `ObservationKey`
- if two compiled bindings in one execution spec would produce the same `ObservationKey`, that is a
  planner error unless an explicit aggregation boundary is declared
- v1 portfolio read models may continue to project this identity through familiar surfaces such as
  wallet and symbol ids, but those projection aliases are not the primary semantic identity

### Action semantics are a separate layer

`Swap`, `Borrow`, and `Lend` are important, but they should not be modeled as peers of
`Subject`, `NetworkView`, or `Observation` in this RFC.

They belong to a future `ActionSemantics` layer:

- `Swap`
- `Borrow`
- `Lend`
- `Repay`
- `Deposit`
- `Withdraw`
- `Stake`
- `Unstake`

Those semantics matter most for intent, execution, and transaction-producing workflows.

### Projection semantics are also a separate layer

`Snapshot` and `Report` are not primary domain actors. They are projections and read models built
from observations.

That separation is important because:

- observations are the canonical execution product
- snapshots are persisted projections over observations
- reports are user-facing projections over snapshots

### Contract references stay adapter-owned

Contract addresses, RPC method names, ABI selectors, token addresses, pool addresses, and other
implementation locators should remain adapter-owned details.

They must not become top-level semantic categories in the planner.

The planner may still freeze them into compiled adapter payloads chosen deterministically at plan
time. They remain adapter-owned implementation details, not primary semantic nouns.

## Project-Wide Planning Model

This refactor is not only about `mfm::portfolio::snapshot`. It also defines the planning model MFM
should use across the project.

### One planning abstraction: recursive operation planning

MFM should have one planning abstraction, not two domain-level planning mechanisms.

That abstraction should be recursive operation planning:

1. A root operation may expand into sub-operations.
2. Sub-operations may expand into more sub-operations or directly into states.
3. The SDK planner must recursively flatten the result into one final `StateGraph` /
   `ExecutionPlan` before runtime starts.
4. The runtime executes states only.
5. States never invoke ops, choose sub-ops, or reshape the graph.

The key architectural distinction is:

1. Planning composition.
   This is recursive and may involve `op -> ops -> ops -> states`.
2. Execution.
   This is always `states` only.

The right slogan is not "ops execute ops."

It is:

- ops compile ops
- runtime executes states

### Converging the current two planning layers

Today MFM has a historical split between:

- pipeline-level planning in the SDK
- op-level planning in `Operation::expand()`

The long-term target of this RFC is to collapse that conceptual split into one recursive planning
model.

Whether the caller enters through a pipeline or a single root op, planning should converge on the
same semantics:

- recursive op composition
- deterministic flattening
- one final execution plan
- one run
- one append-only `run:*` stream family for that run

This RFC does not require the user-facing pipeline surface to disappear immediately. It does require
planning semantics to converge.

### Planner-time versus runtime-time dispatch

Planner-time responsibilities:

- semantic validation
- stable venue-id resolution
- adapter-family selection
- child-op identity assignment or validation
- parent-owned port binding validation
- explicit dataflow versus ordering shaping
- batch partitioning
- dependency and ordering shaping
- compilation of opaque adapter payloads
- lowering planner-visible state identity into flat runtime-visible `StateId`s
- optional compiled execution spec artifact emission

Runtime-time responsibilities:

- execute only compiled states
- execute only compiled homogeneous batches
- use planned adapter ids and compiled payloads only
- resolve imported slots through planner-supplied bindings only
- use `IoProvider` only for side effects and data acquisition
- never rediscover graph topology, choose new adapter families, or arbitrate exports

### Resume and auditability

For v1, `manifest.input_params` remains the durable source of truth for resume.

That means:

- resume may intentionally recompile from raw config against current planner code
- exact cross-build or cross-cutover planner equivalence is not guaranteed
- callers must not assume that changing planner code preserves resume equivalence across versions

For auditability and debugging, the planner should also persist the compiled execution spec as a
content-addressed artifact.

Rules:

- the compiled execution spec artifact is optional but recommended in v1
- the compiled execution spec should capture:
  - the resolved child-op tree and full hierarchical `OpPath`s
  - explicit import/export bindings and re-export aliases
  - the root export resolution table
  - planner-visible state lineage plus the lowered runtime `StateId` mapping
  - compiled semantic payloads such as observation batches and valuation tasks
- if persisted or hashed, it must obey canonical JSON and secret-safety invariants
- the compiled execution spec artifact is not the authoritative runtime resume input in v1
- if persisted, the artifact kind should be `ArtifactKind::Other("compiled_execution_spec")`

Minimum compiled execution spec schema for v1:

```rust
struct CompiledExecutionSpec {
    schema_version: String, // "compiled_execution_spec/v1"
    root_op_id: String,
    root_op_version: String,
    root_op_path: String,
    ops: Vec<CompiledOpRecord>,
    root_exports: Vec<CompiledRootExport>,
    state_lineage: Vec<CompiledStateLineage>,
    planner_payloads: serde_json::Value,
}

struct CompiledOpRecord {
    op_path: String,
    op_id: String,
    op_version: String,
    imports: Vec<String>,
    exports: Vec<String>,
    import_bindings: Vec<CompiledImportBinding>,
    after: Vec<CompiledAfterEdge>,
    re_exports: Vec<CompiledReExport>,
}

struct CompiledRootExport {
    export: String,
    slot: QualifiedSlotRef,
}

struct CompiledStateLineage {
    state_id: String,
    op_path: String,
    state_local_id: String,
}
```

Rules:

- `schema_version` MUST equal `compiled_execution_spec/v1` in the first cut
- `planner_payloads` is where planner-owned semantic payloads such as compiled observation batches
  and valuation tasks live
- emitted arrays MUST already be in deterministic planner order; consumers must not rely on
  hash-iteration order
- v1 does not require a generic engine/runtime consumer for this artifact; it is audit/debug data

### Batches are planner products, not runtime discoveries

A state should remain unitary in responsibility, not necessarily single-item.

That means one state may execute one compiled homogeneous batch.

Examples:

- one batch of subject-resolution work
- one batch of valuation-source reads
- one batch of position observations for one adapter family on one pinned view

What matters is that the batch boundary is decided by the planner, not by runtime graph shaping.

Graph independence may express future parallelism, but executor behavior remains sequential in v1.

## Initial Decisions Locked In By This RFC

- `Subject` replaces `Wallet` in the semantic core.
- Portfolio observation remains fail-fast in v1 for easier debugging and a clearer cutover.
- Venue identifiers are user-authored stable semantic ids.
- `ContractRef` remains adapter-owned and must not become a first-class planner semantic.
- At least one non-EVM portfolio source must be implemented as part of this refactor so the new
  semantic core is validated beyond the EVM slice.
- The implementation strategy is a hard cutover, not a compatibility-preserving migration.
- Work should be decomposed into commit-scoped logical changes on the current branch, not into
  backward-compatibility phases.

## Target Architecture

### Project-wide planning target

The target architecture is:

- a root op may expand into sub-ops
- sub-ops may expand recursively or directly into states
- the planner recursively flattens the result into one final `StateGraph` before runtime starts
- runtime executes states only

This means batch partitioning, ordering, and semantic adapter selection are planning concerns.
States execute compiled units only.

The recursive structure may be:

- `op -> states`
- `op -> ops -> states`
- `op -> ops -> ops -> states`

The runtime model remains:

- one final execution plan
- one run
- one append-only `run:*` stream family for that run

No state may:

- invoke an op
- choose a sub-op
- reshape the graph
- rediscover batch topology dynamically

### Portfolio snapshot as the first consumer

`mfm::portfolio::snapshot` should be the first consumer of this project-wide planning model.

The user-facing entrypoint may remain the same while the internal planning model changes.

A first-cut root op shape should be:

- `portfolio_snapshot`
  - `prepare_execution_sources`
  - `pin_execution_views`
  - `resolve_subjects`
  - `resolve_valuation_inputs`
  - `observe_account_holdings`
  - `observe_venue_positions`
  - `assemble_snapshot`
  - `project_report`

These names describe semantic planner boundaries, not mandatory public op ids. They may begin as
internal composite-op boundaries while the transport layer still points at the existing public
feature surface.

### Portfolio semantic input model

The portfolio refactor should stop treating `wallet`, `symbol`, and `protocol_position` as the
primary semantic center.

Those are current config and implementation surfaces, not the right long-term semantic model.

The target semantic input model should separate:

- subjects
- execution views
- instruments
- venues
- positions
- valuations
- observation targets

In pseudocode:

```rust
struct PortfolioSemanticConfig {
    portfolio_id: String,
    quote_codes: Vec<String>,
    execution_views: Vec<ExecutionViewConfig>,
    subjects: Vec<SubjectConfig>,
    instruments: Vec<InstrumentConfig>,
    venues: Vec<VenueConfig>,
    positions: Vec<PositionConfig>,
    valuations: Vec<ValuationConfig>,
    observation_targets: Vec<ObservationTargetConfig>,
}

struct SubjectConfig {
    subject_id: String,
    subject_kind: String,
    locator: serde_json::Value,
}

struct ExecutionViewConfig {
    execution_view_id: String,
    network_id: String,
    family: String,
    route_policy: serde_json::Value,
}

struct InstrumentConfig {
    instrument_id: String,
    display_symbol: Option<String>,
    quantity_schema: serde_json::Value,
}

struct PositionConfig {
    position_id: String,
    instrument_id: String,
    position_kind: String,
    venue_id: Option<String>,
    reader_hint: Option<String>,
}

struct ValuationConfig {
    valuation_id: String,
    instrument_id: String,
    quote: String,
    strategy: serde_json::Value,
}

struct ObservationTargetConfig {
    target_id: String,
    subject_id: String,
    execution_view_id: String,
    position_id: String,
    valuation_ids: Vec<String>,
}
```

This does not require the first implementation to land exactly with these type names. It does
require the planner to think in these terms.

The current config should be treated as a migration surface into this semantic model:

- `wallet` becomes `subject`
- `network` becomes `execution view input`
- `symbol` splits into instrument, position, and valuation concerns
- `protocol_position` becomes a position plus venue plus adapter payload concern
- `valuation_source_registry` remains part of valuation planning

### Portfolio observation sub-op expansion

The two observation sub-ops are where most protocol and ledger variation should be compiled away.

Examples:

- `observe_account_holdings`
  - native account balances
  - fungible token balances
  - non-EVM account or UTXO-backed holdings
- `observe_venue_positions`
  - Aave reserve positions
  - Aave debt positions
  - future pool, vault, or staking venue positions

Each of those planner boundaries may expand into multiple compiled observation batches.

The batch partitioning key should be semantic and planner-owned. It will typically include:

- adapter family
- pinned execution view
- compatible venue or position reader shape
- any additional deterministic batch-compatibility dimensions

### Composite operation shape

This RFC now recommends a scoped composite-op intermediate representation.

The key rule is:

- op instance identity is hierarchical and planner-owned
- interface wiring is explicit and parent-owned
- runtime-visible execution units are still states only

Conceptually, every operation expands to a planned op that is either:

- a leaf op with executable states
- a composite op with child ops plus explicit interface bindings

In pseudocode:

```rust
struct PlannedOp {
    interface: OpInterface,
    kind: PlannedOpKind,
}

struct OpInterface {
    imports: BTreeSet<PortName>,
    exports: BTreeSet<PortName>,
}

enum PlannedOpKind {
    Leaf(LeafOpSpec),
    Composite(CompositeOpSpec),
}

struct LeafOpSpec {
    states: Vec<LeafStateNode>,
    edges: Vec<LeafStateEdge>,
}

struct CompositeOpSpec {
    children: BTreeMap<ChildOpLocalId, ChildOpInstance>,
    bindings: Vec<ImportBinding>,
    order: Vec<AfterEdge>,
    re_exports: BTreeMap<PortName, PortSource>,
}

struct ChildOpInstance {
    op_id: String,
    op_version: String,
    op_config: serde_json::Value,
}

struct ImportBinding {
    to_child: ChildOpLocalId,
    import: PortName,
    source: PortSource,
}

enum PortSource {
    ParentImport(PortName),
    ChildExport {
        child: ChildOpLocalId,
        export: PortName,
    },
}
```

This representation keeps the planner semantics explicit:

- children are instances with parent-local identity
- bindings express data flow
- `AfterEdge` expresses pure ordering
- parents re-export explicitly instead of inheriting child exports implicitly

### Planning API direction

The most direct code-facing evolution is to make recursive expansion explicit in the operation API
and to return interface plus shape together.

Conceptually:

```rust
trait Operation {
    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError>;
}
```

This RFC no longer recommends a separate planner surface that discovers imports/exports independently
from `expand()`. Because internal backward compatibility is not required, the cleaner target is:

- one expansion result
- one interface contract
- one recursive flattening algorithm

The SDK flattening algorithm should work like this:

1. expand the root op
2. if the result is `Leaf`, validate leaf-state structure and keep it
3. if the result is `Composite`, validate child identities, interface bindings, and re-exports
4. assign deterministic child `OpPath`s
5. recursively expand every child op
6. build the child dependency graph from:
   - explicit `AfterEdge`s
   - dataflow implied by `ImportBinding`s
7. topologically order children with lexical `child_op_local_id` tie-breaks
8. lower every leaf state from planner-visible lineage into one flat runtime `StateGraph`
9. emit one final `StateGraph` plus an optional serializable compiled execution spec

That is the correct place to solve the current two-planner split.

The exact Rust surface may differ, but the invariant must hold:

- recursive op composition is a planner concern
- the final planner output before runtime is one flat `StateGraph`

### Child-op identity, namespacing, and flattening contract

Recursive planning needs an explicit identity and wiring model, otherwise flattening becomes
ambiguous.

Rules:

- every child op must declare a parent-local `child_op_local_id`
- sibling `child_op_local_id`s must be unique within one parent op
- the planner assigns the full child op path as:
  - `<parent_op_path>.<child_op_local_id>`
- full child op paths must be unique within the flattened planning tree
- context namespacing must use the full hierarchical child op path, not only the root step path
- authored import/export wiring should be port-based, not path-based
- resolved compiled plans may record path-based slot references after flattening
- there is no implicit "last writer wins" export policy for recursive child-op composition
- there is no implicit promotion of child exports into the parent interface
- every child import must be bound exactly once
- pure ordering and dataflow must remain distinct planner concepts even if they lower to similar
  runtime edges in v1

Export collision rules must be scoped correctly.

- duplicate import names are invalid within one op interface
- duplicate export names are invalid within one op interface
- duplicate bindings to the same child import are invalid
- duplicate parent re-export names are invalid
- siblings may each expose the same export name because those exports are child-scoped until the
  parent binds or re-exports them

Deterministic flattening rules:

- child dependencies are the union of:
  - explicit `AfterEdge`s
  - implied dependencies from `ImportBinding` sources to bound children
- topologically order child dependencies
- use lexical `child_op_local_id` as the stable tie-breaker
- recursively apply the same rule before emitting final flattened state nodes
- within a leaf state graph, use topological order with lexical `state_local_id` tie-breaks for
  deterministic compiled-spec emission
- persisted or serialized planner structures should use deterministic ordered collections, not
  hash-iteration order

### Context namespacing after flattening

Only leaf ops materialize context at runtime.

Recommended slot convention:

- imported ports are read locally as `in.<port>`
- op-private working data is read and written under `work.*`
- exported ports are written locally as `out.<port>`

After flattening:

- a leaf state's runtime context keys are qualified as `<full_op_path>.<slot>`
- imported slots resolve to planner-supplied source slots
- private `work.*` data never crosses op boundaries
- composite op re-exports are aliases in the compiled execution spec, not hidden runtime copies

This means namespacing survives flattening through full leaf `OpPath`s even though runtime executes
one flat state graph.

### State identity in the first cut

The first cut should keep `crates/machine` small while still making planner identity explicit.

The recommended contract is:

- full hierarchical child-op identity lives in `OpPath`
- planner-visible state identity lives in a first-class `StateAddr`
- flat runtime-visible `StateId` remains a three-segment id in v1
- `StateId` must be treated as a lowered execution handle, not as the semantic lineage carrier

In pseudocode:

```rust
struct StateAddr {
    op_path: OpPath,
    state_local_id: String,
}

struct QualifiedSlotRef {
    op_path: OpPath,
    slot: String,
}

struct CompiledExecutionSpec {
    root_exports: BTreeMap<PortName, QualifiedSlotRef>,
    state_lineage: BTreeMap<StateId, StateAddr>,
}
```

This is the important distinction:

- hierarchical `OpPath` plus flat runtime `StateId` is sufficient only if `StateAddr` is the
  planner source of truth
- attempting to encode nested lineage only inside `StateId` is an unstable compromise because it
  overloads a runtime handle with planner semantics
- compiled execution specs should preserve the exact lineage map for audit and debugging

Recommended lowering rule:

- derive runtime `StateId`s deterministically from canonical `StateAddr`
- use:
  - `<machine_id>.<step_id>.<flattened_state_local_id>`
  - where `flattened_state_local_id` is the `__`-joined sequence of:
    - `op_path` segments after the root `<machine_id>.<step_id>`
    - followed by `state_local_id`
  - if the leaf op is the root step, `flattened_state_local_id = state_local_id`
- do not escape underscores during lowering; any resulting collision is a planner error
- collision-check the fully flattened plan after lowering
- do not require `StateId` to be reversible into lineage

### Compiled observation binding shape

Observation planning should compile raw semantic config into stable executable bindings before
runtime starts.

The minimum compiled binding should carry:

```rust
struct CompiledObservationBinding {
    binding_id: String,
    observation_key: ObservationKey,
    valuation_ids: Vec<String>,
    adapter: AdapterId,
    adapter_payload: serde_json::Value,
}
```

Rules:

- `adapter` is planner-selected and deterministic
- `adapter_payload` is opaque to generic states
- `adapter_payload` may contain contract refs or ledger-specific execution details
- if a compiled binding is persisted or hashed, it must obey the repo-wide canonical JSON and
  secret-safety invariants
- generic semantic states must not reinterpret `adapter_payload` beyond routing to the selected
  adapter

### Compiled observation batch shape

The runtime-facing batch unit should be explicit and planner-owned.

```rust
struct CompiledObservationBatch {
    batch_id: String,
    adapter: AdapterId,
    network_view_id: String,
    bindings: Vec<CompiledObservationBinding>,
}
```

Recommended invariants:

- one batch uses exactly one exact executable adapter id
- one batch targets exactly one pinned `network_view_id`
- every binding in the batch is executable by the same runtime state implementation
- batch ordering is deterministic

This is the unit consumed by `ObserveCompiledBatchState`.

### Stable venue identity shape

Venue identity should be semantic, stable, and user-authored.

It should not be derived from opaque contract refs at runtime.

The minimum venue identity should support:

```rust
struct VenueId {
    venue_id: String,
    venue_kind: String,
    network_id: Option<String>,
    parent_venue_id: Option<String>,
}
```

Examples:

- `venue_id = "aave-v3-mainnet"`
- `venue_kind = "lending_market"`
- `parent_venue_id = None`

- `venue_id = "aave-v3-mainnet/usdc"`
- `venue_kind = "reserve"`
- `parent_venue_id = Some("aave-v3-mainnet")`

Contract addresses, selectors, and token refs may still live inside adapter payloads, but they do
not define venue identity.

Additional contract:

- `venue_id` must be unique within one semantic config
- `venue_id` is intended to remain stable across manifests and runs for the same logical venue
- changing adapter payload details must not require changing `venue_id`

### State topology for the first cut

The first cut should flatten to a final state DAG that is still small and semantically stable:

1. `PrepareExecutionSources`
2. `PinExecutionViews`
3. `ResolveSubjects`
4. `ResolveValuationInputs`
5. `ObserveCompiledBatchState` repeated `N` times
6. `MergeObservations`
7. `AssembleSnapshot`
8. `ProjectReport`

The crucial rule is that `ObserveCompiledBatchState` is not allowed to inspect raw user config and
decide what it is. The planner already decided that.

### Observation batch contract

Each compiled observation batch should be homogeneous from the runtime's point of view.

That means a batch state should execute one already-compiled family of work such as:

- one EVM native/erc20 holdings batch for one pinned view
- one Aave reserve/debt batch for one pinned view
- one non-EVM holdings batch for one pinned view

All observation paths must converge on the same canonical `Observation` output shape so that:

- merge stays generic
- snapshot assembly stays generic
- report projection stays generic

### Fail-fast behavior

The first cut remains fail-fast.

If any required source preparation, execution-view pinning, valuation input resolution, or
observation batch fails, the run fails immediately. Partial portfolio snapshots are deferred.

## Implementation Plan

This RFC assumes a full break-change implementation.

There is no requirement to preserve:

- old internal semantic type names
- old op-local planner structure
- compatibility aliases
- dual old/new planner paths
- compatibility re-exports
- temporary adapter layers whose only purpose is to preserve the old architecture

The implementation should be a sequence of hard cutovers, each kept to one logical commit on the
current branch.

### Commit decomposition policy

Each commit should satisfy all of the following:

- one architectural outcome
- no compatibility shim added only to ease transition
- in-repo consumers updated in the same commit
- docs updated in the same commit when contract language changes

This RFC is intentionally commit-oriented, not PR-oriented.

The goal is a clean final architecture, not a long-lived migration surface.

### Implemented commit sequence on this branch

#### Commit 1: planner contract groundwork

Goal:

- establish recursive op planning as the project-wide target model in code-facing planner surfaces

Scope:

- introduce the planner-facing representation for child-op expansion
- add recursive flattening support in the SDK planner layer
- preserve final runtime output as one flat `ExecutionPlan`
- keep runtime behavior state-only

Must be true at the end of the commit:

- root ops can compile child ops
- planner flattening happens before runtime
- states still execute exactly as runtime units

#### Commit 2: semantic portfolio model cutover

Goal:

- replace the current portfolio-centered internal semantic model with
  subject/view/instrument/position/venue/valuation/observation terms

Scope:

- introduce new semantic config/model types
- remove the current overloaded internal center around wallet/symbol/protocol_position
- re-express portfolio validation in terms of the new semantic model

Must be true at the end of the commit:

- planner code thinks in semantic-first terms
- old internal vocabulary is no longer the architectural source of truth

#### Commit 3: compiled binding and batch model

Goal:

- make compiled observation bindings and compiled homogeneous batches explicit

Scope:

- introduce compiled binding types
- introduce compiled batch types
- make adapter-family selection planner-owned
- make batch partitioning planner-owned

Must be true at the end of the commit:

- runtime batch states consume compiled units, not raw user config

#### Commit 4: portfolio op refactor into composite planning

Goal:

- replace the current protocol-shaped `portfolio_tracker` expansion with semantic recursive op
  planning

Scope:

- remove planner branching shaped like base-vs-Aave special cases
- introduce semantic child-op or internal composite-op boundaries
- flatten to the new state topology:
  - `PrepareExecutionSources`
  - `PinExecutionViews`
  - `ResolveSubjects`
  - `ResolveValuationInputs`
  - `ObserveCompiledBatchState x N`
  - `MergeObservations`
  - `AssembleSnapshot`
  - `ProjectReport`

Must be true at the end of the commit:

- portfolio planning is semantic-first
- runtime graph shape is not protocol-specific

#### Commit 5: generic observation runtime cutover

Goal:

- remove protocol-specific runtime graph shaping from observation execution

Scope:

- replace `CollectObservationsState` / `CollectAaveObservationsState` style architectural splits
  with compiled-batch execution
- keep protocol-specific acquisition logic behind adapter-family execution code
- preserve one canonical `Observation` output shape

Must be true at the end of the commit:

- observation merge, snapshot assembly, and report projection are generic
- protocol-specific growth happens in adapter execution paths, not in graph topology

#### Commit 6: non-EVM source landing

Goal:

- prove the semantic model is not just renamed EVM architecture

Scope:

- add at least one real non-EVM portfolio source
- thread it through subject resolution, execution-view pinning, compiled binding, batch execution,
  valuation, and observation assembly

Recommended first source:

- Bitcoin spot / UTXO-backed holdings

Must be true at the end of the commit:

- the semantic architecture is forced through at least one non-EVM path

#### Commit 7: naming and dead-code cleanup

Goal:

- remove obsolete protocol-first names and old architecture leftovers

Scope:

- delete superseded model types and states
- rename root ops, states, and modules where needed so the codebase matches the RFC vocabulary
- update docs and inventories to reflect the new steady state

Must be true at the end of the commit:

- the repo no longer presents the old architecture as current

### Explicit hard-cutover rules

The implementation should follow these rules:

- do not keep both old and new planner models alive longer than necessary
- do not add deprecated aliases for internal crate paths or internal semantic types
- do not add compatibility re-exports for superseded internal modules
- do not preserve protocol-first planner branching just to ease review
- do not preserve the old semantic vocabulary as a parallel internal API

All in-repo consumers should be updated in the same commit that introduces the new internal source
of truth.

### What should remain stable during the cutover

Even though the implementation is a hard cutover, these invariants remain non-negotiable:

- append-only run semantics
- per-append atomicity
- content-addressed artifacts
- canonical JSON hashing rules
- no ambient IO in state logic
- no secrets in persisted surfaces
- transport-only binaries
- runtime executes states only

## The Problem

This section preserves the earlier current-state trace with only light editorial changes. It
describes the existing flow accurately and serves as the problem statement that motivates the
redesign below.

### Current summary

This note captures the current code path for:

```bash
nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>
```

The most important finding is that the current live code path is:

1. flake app launcher
2. `task.mfm.portfolio.snapshot` shell wrapper
3. packaged `mfm_cli --output-format json portfolio snapshot --request-file ...`
4. `AppServices::start_portfolio_snapshot`
5. single-op run for `portfolio_tracker v2`
6. namespaced state graph
7. final context snapshot extraction
8. raw JSON envelope emitted unchanged to stdout

It is not currently:

1. app
2. `workflow.mfm.portfolio.snapshot`
3. hidden `services-start` / `exec` / `services-stop` tasks

That workflow/task split still appears in docs, but not in the current Nix code or generated
launcher.

### Verified Entrypoint Chain

#### Flake and app resolution

- `flake.nix` builds outputs through `nixfiedLib.mkFlakeOutputs` with `nixfied/project/module.nix`.
- `nixfied/project/module.nix` defines `mkTaskApp`.
- `apps."mfm::portfolio::snapshot"` is a `taskRef` pointing at `task.mfm.portfolio.snapshot`.
- The generated launcher for this build resolves the app name `mfm::portfolio::snapshot`,
  selects the concrete launcher via `run-selected-app.nix`, and `exec`s the selected binary.

Relevant files:

- `flake.nix`
- `nixfied/project/module.nix`
- generated launcher path returned by:
  - `nix eval --json '.#apps.<system>."mfm::portfolio::snapshot"'`

#### Important correction: no current workflow hop

What the current code shows:

- the public app points directly to `task.mfm.portfolio.snapshot`
- the generated launcher records no workflow id for `task.mfm.portfolio.snapshot`

What stale docs still say:

- `README.md` says the app delegates sequencing to `workflow.mfm.portfolio.snapshot`
- `docs/helios.md` says the public task delegates to:
  - `workflow.mfm.portfolio.snapshot`
  - `task.mfm.portfolio.snapshot.services-start`
  - `task.mfm.portfolio.snapshot.exec`
  - `task.mfm.portfolio.snapshot.services-stop`

That stale split is important because any redesign work should start from the current shell task,
not from the older workflow description.

### End-To-End Call Path

#### 1. `nix run` resolves the app

- `nix run .#mfm::portfolio::snapshot` evaluates the flake.
- The app surface comes from `frameworkOutputs.apps`.
- The selected app is the generated launcher for `mfm::portfolio::snapshot`.

#### 2. The app resolves to `task.mfm.portfolio.snapshot`

- `apps."mfm::portfolio::snapshot"` is a `taskRef`.
- The referenced task is `task.mfm.portfolio.snapshot`.
- This is the real public wrapper for the portfolio snapshot flow.

#### 3. The task enforces the shell contract

The shell wrapper:

- accepts exactly one positional request file
- rejects unknown arguments
- normalizes relative paths to absolute paths
- checks that the file exists and is readable
- requires `POSTGRES_PORT`
- rejects `MFM_KEEP_SERVICES`
- requires Postgres lifecycle hooks to be present

This wrapper owns stdout for the public app.

#### 4. The task derives service lifecycle policy

The wrapper infers service policy from:

- `SERVICE_REUSE_POLICY`
- `SERVICE_OWNER_SCOPE`
- `SERVICE_DISCOVERY_SCOPE`

It then:

- sets cleanup traps
- decides whether owned services should be torn down on exit

#### 5. The task bootstraps Postgres

The wrapper:

- checks `SVC_POSTGRES_READY`
- reuses Postgres if already ready
- otherwise starts Postgres via `SVC_POSTGRES_FULL_START`
- then re-checks readiness

#### 6. The task optionally bootstraps Helios

Helios is currently opportunistic, not strictly required by the task model.

Behavior:

- if Helios hooks exist and Helios is not skipped, the wrapper uses Helios
- it requires `HELIOSRPC_PORT`
- it defaults `HELIOS_NETWORK` to `mainnet`
- it rejects any non-mainnet `HELIOS_NETWORK`
- it reuses or starts Helios
- if the caller did not provide managed RPC bootstrap env, it seeds:
  - `MFM_EVM_RPC_SOURCES_JSON`
  - `MFM_EVM_RPC_PREFERRED_ORDER`

If Helios is unavailable or skipped:

- the wrapper requires the caller to already provide `MFM_EVM_RPC_SOURCES_JSON`

This is a useful semantic distinction:

- Postgres is model-required
- Helios is convenience-managed when hooks exist
- direct managed RPC input is the actual runtime requirement

#### 7. The task launches packaged `mfm_cli`

After service bootstrap, the wrapper:

- exports `DATABASE_URL`
- resolves the packaged `mfm_cli` binary
- runs:

```bash
mfm_cli --output-format json portfolio snapshot --request-file "$request_file"
```

This path uses the packaged CLI binary, not `cargo run`.

#### 8. The CLI parses the request and builds stores

`bin/cli/src/commands/portfolio/snapshot.rs` does the following:

- parses request input via `mfm_app::parse_portfolio_snapshot_request_input`
- builds stores from `RunStoresArgs`
- creates `AppServices`
- calls `services.start_portfolio_snapshot(request)`

Store semantics:

- stream store:
  - Postgres
  - from `--database-url` or `DATABASE_URL`
- artifact store:
  - filesystem
  - from `--artifact-root`, `MFM_ARTIFACT_ROOT`, or `~/.mfm/run_artifacts`

#### 9. `AppServices` starts a single-op run

`AppServices::start_portfolio_snapshot`:

- validates the portfolio bundle again
- serializes the full request as op config
- starts `RunsStartRequest::Single`
- uses:
  - `op_id = "portfolio_tracker"`
  - `op_version = "v2"`

Default run config semantics:

- live I/O
- sequential execution
- no retry backoff
- context checkpointing after every state

#### 10. The app bundle provides runtime wiring

The app bundle registers:

- `PortfolioTrackerOp`
- transport factories including:
  - local fs
  - local evm
  - `rpc.control`
  - exec transport
  - nix flake transport

So the portfolio snapshot flow is not a CLI-local special case. It is a normal op execution path
through the engine.

#### 11. `PortfolioTrackerOp` expands into the state graph

The op planner:

- validates and decodes `portfolio` plus `valuation_source_registry`
- splits symbols into:
  - base symbols
  - Aave protocol-position symbols
- filters wallet symbol assignments accordingly
- builds this DAG:

1. `PrepareSourcesState`
2. `PinPortfolioNetworksState`
3. `ResolveWalletsState`
4. `ReadDirectPricesState`
5. `CollectObservationsState`
6. `CollectAaveObservationsState`
7. `MergeObservationsState`
8. `WritePortfolioSnapshotState`
9. `WritePortfolioReportState`

### State Semantics Map

| State | Semantic role | Main inputs | Main outputs | Notes |
|---|---|---|---|---|
| `PrepareSourcesState` | Preflight managed RPC source pools | routed networks and `control_scope` | none in context | Fails fast if a network/scope has no healthy `rpc.control` sources |
| `PinPortfolioNetworksState` | Freeze the execution view to concrete chain/block pins | portfolio config, valuation source registry, live RPC | `network_pins` | Validates runtime inputs again, checks `eth_chainId`, reads `eth_blockNumber` |
| `ResolveWalletsState` | Convert configured wallets into runtime wallet identities and capabilities | wallet configs | `resolved_wallets` | Base runtime currently supports only `address_only` wallets |
| `ReadDirectPricesState` | Read all unique direct valuation sources once | symbol configs, valuation source registry, pinned networks | `direct_prices` | Reads oracle-backed direct prices at pinned blocks; derived prices are not computed here |
| `CollectObservationsState` | Build canonical observations for base symbols | base wallets, base symbols, resolved wallets, direct prices, network pins | `base_observations` | Reads native and ERC-20 balances, then computes values from fixed/direct/derived prices |
| `CollectAaveObservationsState` | Build canonical observations for Aave protocol positions | Aave wallets, Aave symbols, resolved wallets, direct prices, network pins | `aave_observations` | Reads reserve/debt positions and normalizes them into the same observation shape |
| `MergeObservationsState` | Canonical merge and ordering boundary | `base_observations`, `aave_observations` | `observations` | Concatenates and sorts deterministically |
| `WritePortfolioSnapshotState` | Assemble the canonical snapshot and persist the snapshot artifact | resolved wallets, network pins, merged observations, portfolio config | `snapshot`, `snapshot_artifact_id` plus output artifact | Uses `io.now_millis()` for `generated_at_ms` |
| `WritePortfolioReportState` | Derive user-facing totals/report from the snapshot | `snapshot` | `report` | Produces wallet totals and portfolio totals by quote and emits a completion event |

### More Detailed State Notes

#### `PrepareSourcesState`

Semantic meaning:

- "Before any reads happen, prove that each required routed network has at least one responsive
  managed source."

Operational detail:

- deduplicates `(network_id, control_scope)`
- calls `prepare_sources_in_scope`
- fails if every returned source is unhealthy

#### `PinPortfolioNetworksState`

Semantic meaning:

- "Turn an open-ended live portfolio query into a replayable, block-pinned view."

Operational detail:

- computes required networks from:
  - wallet network assignments
  - valuation readers
- validates configured `chain_id` against live RPC `eth_chainId`
- reads `eth_blockNumber`
- writes deterministic sorted `network_pins`

This is the main replay boundary for the rest of the portfolio flow.

#### `ResolveWalletsState`

Semantic meaning:

- "Resolve the portfolio's wallet declarations into concrete runtime wallet identities."

Current limitation:

- only `address_only` is supported in this runtime slice
- `keystore_entry`
- `node_managed_account`
- `external_signer`

all currently fail here

#### `ReadDirectPricesState`

Semantic meaning:

- "Resolve every direct pricing source needed by this portfolio at the already pinned view."

Operational detail:

- scans symbol valuation configs
- collects unique source ids
- resolves valuation sources from the registry
- reads each direct source once
- stores direct price values plus source refs pinned to a block

Derived prices are not materialized here. They are computed later from the cached direct prices.

#### `CollectObservationsState`

Semantic meaning:

- "For each base symbol assigned to each wallet, read the quantity and compute its values."

Operational detail:

- native balances use `eth_getBalance`
- ERC-20 balances use `balanceOf`
- decimals come from config or on-chain `decimals()`
- valuation values are built from:
  - fixed unit price
  - direct price lookup
  - derived unit price from numerator / denominator direct prices

This is the base-token semantic layer.

#### `CollectAaveObservationsState`

Semantic meaning:

- "For each Aave protocol position, read the protocol-specific quantity and normalize it into the
  same canonical observation model."

Operational detail:

- reads reserve and debt token positions
- handles collateral-enabled checks where configured
- emits standard `Observation` records so downstream states do not care whether the source was a
  base token or an Aave position

#### `MergeObservationsState`

Semantic meaning:

- "Collapse multiple observation producers into one canonical observation set."

It is a pure ordering and merge boundary.

#### `WritePortfolioSnapshotState`

Semantic meaning:

- "Produce the replayable canonical portfolio snapshot artifact."

Operational detail:

- groups observations by wallet
- copies and normalizes symbol configs
- constructs `PortfolioSnapshot`
- writes snapshot JSON into context
- writes snapshot JSON as an output artifact
- records the snapshot artifact id in context

#### `WritePortfolioReportState`

Semantic meaning:

- "Turn the canonical snapshot into the user-facing totals/report projection."

Operational detail:

- reads the full snapshot
- computes wallet totals by quote
- merges into portfolio totals by quote
- writes report into context
- emits `portfolio_tracker.completed`

### Context Namespacing

One subtle but important runtime detail:

- the op planner uses local keys like:
  - `resolved_wallets`
  - `network_pins`
  - `report`
- but `AppServices` later extracts:
  - `portfolio_tracker.main.snapshot_artifact_id`
  - `portfolio_tracker.main.report`

The reason is `NamespacedContext` in `crates/sdk/src/unstable.rs`.

What it does:

- local writes are qualified as `<op_path>.<key>`
- local reads default to that same qualified namespace
- imported keys can read from another op path if wired through exports/imports

So the semantic model is:

- planner code talks in local, op-relative keys
- the runtime persists those keys under a fully qualified op path

For this op, the effective path is `portfolio_tracker.main`.

### Persistence And Returned Data

#### Stream store vs artifact store

There are two persisted surfaces involved:

- stream store:
  - Postgres
  - run stream and kernel/domain event history
- artifact store:
  - filesystem by default
  - context snapshots and output artifacts

#### `final_snapshot_id` vs `snapshot_artifact_id`

These are different things:

- `final_snapshot_id`
  - the engine's final context checkpoint artifact id
  - used by `AppServices` to load the final context JSON
- `snapshot_artifact_id`
  - the canonical portfolio snapshot artifact written by `WritePortfolioSnapshotState`

The app response exposes both when available.

#### Where the final report comes from

`AppServices::start_portfolio_snapshot` does not rebuild the report itself.

It:

- loads the final context snapshot artifact
- decodes the snapshot JSON
- extracts the namespaced keys for:
  - `snapshot_artifact_id`
  - `report`

If the run failed:

- it scans the run stream backwards
- finds the last `StateFailed`
- maps that error into the app error contract

### Public Output Contract

The public app stdout contract is owned in two layers:

#### CLI layer

The CLI wraps the portfolio response as:

- `FeatureExecutionResult`
- `feature_id = "portfolio.snapshot"`
- `result = <PortfolioSnapshotResponse>`

Then the JSON output renderer wraps that as:

```json
{
  "status": "success",
  "data": {
    "feature_id": "portfolio.snapshot",
    "result": { "...": "..." }
  }
}
```

#### Shell wrapper layer

The shell task then validates with `jq` that:

- `.status == "success"`
- `.data.feature_id == "portfolio.snapshot"`
- `.data.result` exists

Only after that does it replay the raw CLI JSON to stdout unchanged.

So the public app is intentionally preserving `mfm_cli` as the final output-contract authority.

### Mismatches And Gotchas Discovered While Tracing

#### 1. Docs are stale about workflow composition

Current code path:

- app -> task -> packaged CLI -> app services -> op

Stale docs still describe:

- app -> workflow -> hidden lifecycle tasks

Files to update eventually:

- `README.md`
- `docs/helios.md`

#### 2. The task model explicitly requires Postgres, not Helios

The task's `requirements.services` includes:

- `postgres`

The shell code then opportunistically uses Helios when hooks are available.

That means there is a semantic mismatch between:

- model-declared service requirements
- shell-level optional Helios bootstrap behavior

#### 3. Wallet support is narrower than the config surface

The wallet config model can express multiple implementations, but the current portfolio runtime
slice only supports:

- `address_only`

Any semantics work should call this out clearly.

#### 4. Base and Aave flows deliberately converge on one observation model

This is a useful design invariant:

- base balances and Aave positions are different acquisition paths
- both are normalized into the same canonical `Observation`
- downstream snapshot/report code stays generic

#### 5. The replay boundary is the pinned network view plus canonical observations

This is the important semantic center of the portfolio flow:

- freeze network/block pins
- read direct prices at that pinned view
- produce canonical observations
- persist canonical snapshot artifact
- derive report projection from snapshot

### File Anchors

Primary files touched during tracing:

- `flake.nix`
- `nixfied/project/module.nix`
- `bin/cli/src/commands/portfolio/snapshot.rs`
- `bin/cli/src/support/run_stores.rs`
- `bin/cli/src/support/app_services.rs`
- `bin/cli/src/presentation/output.rs`
- `crates/app/src/lib.rs`
- `crates/ops/portfolio-tracker-op/src/lib.rs`
- `crates/evm-runtime/src/states/rpc_control.rs`
- `crates/states/wallet/src/states.rs`
- `crates/states/symbol/src/states.rs`
- `crates/states/aave-v3/src/portfolio/states.rs`
- `crates/states/portfolio/src/states.rs`
- `crates/sdk/src/unstable.rs`
- `docs/ops-and-states.md`
- `README.md`
- `docs/helios.md`

### High-Value Code Anchors

These were the fastest jump points during tracing. The line numbers are from the current checkout
and will drift over time.

- flake output wiring:
  - `flake.nix:9-37`
- app/task wiring:
  - `nixfied/project/module.nix:543-562`
  - `nixfied/project/module.nix:829-1072`
  - `nixfied/project/module.nix:1971-1979`
- CLI entrypoint:
  - `bin/cli/src/commands/portfolio/snapshot.rs:37-88`
  - `bin/cli/src/support/run_stores.rs:26-67`
  - `bin/cli/src/support/app_services.rs:13-33`
  - `bin/cli/src/presentation/output.rs:72-107`
  - `bin/cli/src/presentation/output.rs:224-247`
- app-layer run and extraction:
  - `crates/app/src/lib.rs:321-336`
  - `crates/app/src/lib.rs:456-580`
  - `crates/app/src/lib.rs:624-710`
  - `crates/app/src/lib.rs:913-1000`
- op planner:
  - `crates/ops/portfolio-tracker-op/src/lib.rs:71-83`
  - `crates/ops/portfolio-tracker-op/src/lib.rs:206-390`
- state implementations:
  - `crates/evm-runtime/src/states/rpc_control.rs:40-110`
  - `crates/states/wallet/src/states.rs:15-79`
  - `crates/states/symbol/src/states.rs:73-207`
  - `crates/states/symbol/src/states.rs:225-405`
  - `crates/states/symbol/src/states.rs:428-605`
  - `crates/states/aave-v3/src/portfolio/states.rs:33-160`
  - `crates/states/portfolio/src/states.rs:29-140`
  - `crates/states/portfolio/src/states.rs:149-315`
- context namespacing:
  - `crates/sdk/src/unstable.rs:467-500`
- inventory docs:
  - `docs/ops-and-states.md:46-77`
- stale docs to reconcile later:
  - `README.md:107-121`
  - `docs/helios.md:16-34`

### Commands Used To Trace This

Repo-local code search:

```bash
rg -n "mfm::portfolio::snapshot|portfolio.snapshot" .
```

Key file reads:

```bash
sed -n '780,1105p' nixfied/project/module.nix
sed -n '1,220p' bin/cli/src/commands/portfolio/snapshot.rs
sed -n '913,1008p' crates/app/src/lib.rs
sed -n '206,390p' crates/ops/portfolio-tracker-op/src/lib.rs
sed -n '68,605p' crates/states/symbol/src/states.rs
sed -n '29,315p' crates/states/portfolio/src/states.rs
sed -n '462,515p' crates/sdk/src/unstable.rs
```

App-surface verification:

```bash
nix eval --json '.#apps.<system>."mfm::portfolio::snapshot"'
```

Then inspect the launcher path returned by that command.

### Open Questions From The Current Trace

1. Should the public app go back to an explicit workflow model, or should the direct-task shape be
   treated as canonical?
2. Should Helios become an explicit model requirement, or should the app remain "Postgres required,
   Helios opportunistic, direct RPC bootstrap authoritative"?
3. Should the state semantics be documented as:
   - a portfolio-specific RFC
   - an expansion of `docs/ops-and-states.md`
   - or both?
4. Should the replay boundary be described primarily around:
   - pinned networks
   - canonical observations
   - canonical snapshot artifact
   rather than around the task wrapper?
5. Do we want a more explicit semantic distinction between:
   - snapshot production
   - report projection
   - stdout envelope ownership

### Working Conclusion

If this RFC gets turned into a more formal design document, the cleanest story appears to be:

- public app shell owns process/bootstrap/stdout semantics
- `mfm_cli` owns the public JSON payload contract
- `AppServices` owns run-launch and final extraction semantics
- `portfolio_tracker` owns domain planning semantics
- the shared states own the real runtime semantics

That decomposition matches the current code better than the stale workflow-centric docs.

## Precise Critique Of The Current Architecture

The current portfolio runtime already has the right high-level semantic phases, but the wrong
specialization boundary.

### 1. The op graph shape is protocol-driven

Today the op decides which states exist by splitting symbols into protocol buckets and wiring one
state per bucket. That makes the DAG change every time a new protocol family is introduced.

This is the wrong place for specialization. Protocol growth should change adapter coverage, not the
semantic graph topology.

### 2. Semantic seams are buried inside protocol-specific states

The following semantic responsibilities are currently mixed into concrete protocol states:

- quantity acquisition
- protocol-specific validation
- protocol-to-observation normalization
- valuation dependency discovery
- network family routing assumptions
- source metadata shaping

As a result, generic semantics such as "observe position", "resolve valuation", and "pin execution
view" are harder to see, reuse, and extend.

### 3. There is duplicated orchestration around one observation model

`CollectObservationsState` and `CollectAaveObservationsState` both:

- read resolved wallets, pinned networks, and direct prices
- iterate wallet assignments
- sort by wallet and symbol
- emit the same canonical `Observation` type

They differ mainly in how quantity is acquired. That is strong evidence that quantity acquisition
belongs behind an adapter seam, not in separate state types.

### 4. Generic surfaces are still EVM-contaminated

The current "canonical" portfolio runtime still bakes EVM assumptions into generic surfaces:

- `NetworkConfig` requires `chain_id`
- `NetworkPin` means `chain_id + block_number`
- wallet validation assumes normalized EVM addresses
- source preparation assumes `rpc.control` is the universal ingress

That contamination makes non-EVM support look like a fork instead of a natural extension.

### 5. Planner-time resolved reader concepts exist, but are unused

The current model already hints at the correct seam with:

- `ResolvedSymbolBalanceReader`
- `ResolvedSymbolValuationReader`

Those types are not used by the planner today. The RFC takes that dormant idea seriously and
elevates it into the core design.

## Semantic Seams That Exist Today But Are Hidden

The current implementation is already organized around a semantic pipeline. The proposal is to make
those seams first-class:

- execution source preparation
- execution view pinning
- wallet subject resolution
- valuation source resolution
- position observation
- observation merge and canonical ordering
- snapshot assembly
- report projection

Within `CollectObservationsState` and `CollectAaveObservationsState`, the hidden seam is even more
specific:

- subject iteration: wallet x symbol assignment
- quantity acquisition: native balance, ERC-20 balance, reserve position, debt position
- quantity normalization: decimals and amount rendering
- value projection: fixed, direct, derived valuation
- observation assembly: canonical record + metadata

The architecture should make those responsibilities composable without making them state-shaped.

## Proposed Target Architecture

The target architecture has four layers:

1. Canonical config normalization
2. Pure semantic compilation
3. Fixed semantic state graph
4. Adapter-backed execution inside semantic states

### High-Level Model

The semantic op should compile the canonical request into a `PortfolioExecutionSpec` and then wire
a fixed semantic DAG. The number of states may vary by batching strategy, but the state types should
stay stable.

Target state family:

1. `PrepareExecutionSourcesState`
2. `PinExecutionViewState`
3. `ResolveSubjectsState`
4. `ResolveUnitPricesState`
5. `ObservePortfolioBatchState`
6. `MergeObservationsState`
7. `AssembleSnapshotState`
8. `ProjectReportState`

The important change is this:

- protocol/network growth changes planned tasks and adapter selection
- protocol/network growth does not create new semantic state types

## Proposed Semantic Vocabulary

This section translates the central semantic model above into execution vocabulary.

### Core semantic phases

- `PrepareExecutionSources`
  - ensure required live source families are available before reads begin
- `PinExecutionView`
  - freeze each required network family to a replayable anchor
- `ResolveSubjects`
  - resolve configured subject declarations into runtime subjects and capabilities
- `ResolveUnitPrices`
  - resolve all externally-read pricing inputs needed by downstream observations
- `ObservePortfolioBatch`
  - acquire quantities for a batch of positions and normalize them into canonical observations
- `MergeObservations`
  - merge multiple producers into a single deterministic observation set
- `AssembleSnapshot`
  - construct and persist the canonical snapshot artifact
- `ProjectReport`
  - derive user-facing totals and summaries from the snapshot

### Primary execution nouns

- `ResolvedSubject`
  - the resolved runtime subject corresponding to the configured `Subject`
- `ExecutionView`
  - the replayable pinned view corresponding to `NetworkView`
- `ExecutionAnchor`
  - a family-specific anchor such as EVM block or Bitcoin block hash
- `InstrumentSemantics`
  - the planned instrument class
- `PositionSemantics`
  - the planned holding or liability class
- `VenueId`
  - semantic venue metadata used by planner and runtime
- `ValuationTask`
  - one planned unit-price acquisition or derivation task
- `CompiledObservationBinding`
  - one planner-compiled quantity acquisition plus normalization unit

### Instrument semantics

```rust
pub enum InstrumentSemantics {
    NativeAsset,
    FungibleToken,
    QuoteUnit,
}
```

### Position semantics

```rust
pub enum PositionSemantics {
    SpotBalance,
    LendingDeposit,
    LendingDebt,
    LiquidityShare,
    StakedClaim,
    UtxoSet,
}
```

### Venue semantics

```rust
pub struct VenueId {
    pub venue_id: String,
    pub venue_kind: String,
    pub network_id: Option<String>,
    pub parent_venue_id: Option<String>,
}
```

These semantics are intentionally broader than protocol names. Aave, Compound, Morpho, and future
lending systems can all map into the same `PositionSemantics` while keeping protocol-specific logic
inside adapters.

### Action semantics

The semantic layer should reserve a separate namespace for future transaction and intent workflows:

```rust
pub enum ActionSemantics {
    Swap,
    Borrow,
    Lend,
    Repay,
    Deposit,
    Withdraw,
    Stake,
    Unstake,
}
```

These are not the primary planning nouns for the current portfolio read-model refactor.

## Execution Model

### 1. Config normalization phase

The incoming request remains canonical JSON decoded into typed config structs. The normalization
phase should:

- validate canonical JSON constraints
- stamp defaults such as control scopes
- classify each network by family
- classify each wallet declaration by subject family
- classify each symbol into instrument semantics, position semantics, protocol venue semantics, and
  valuation intent
- reject ambiguous or inconsistent configurations before planning

This phase stays pure and deterministic.

### 2. Semantic compilation phase

The op should call a pure compiler:

```rust
pub trait PortfolioSemanticCompiler {
    fn compile(
        &self,
        request: &PortfolioRequest,
        catalog: &SemanticCatalog,
    ) -> Result<PortfolioExecutionSpec, PlanningError>;
}
```

`compile()` must:

- discover required semantic tasks from canonical config
- select exactly one adapter per task
- fail on zero matches
- fail on ambiguous matches
- sort deterministically
- produce a serializable execution spec

### 3. Semantic execution phase

Semantic states execute the planned tasks. They do not rediscover protocol behavior from raw
config. They operate from the precompiled `PortfolioExecutionSpec`.

For example:

- `PinExecutionViewState` executes `ViewPinTask`s
- `ResolveUnitPricesState` executes `ValuationTask`s
- `ObservePortfolioBatchState` executes `CompiledObservationBatch`s

### 4. Persistence and replay model

Nothing in the persistence model changes:

- state attempts still emit the same kernel event envelope
- `StateCompleted` remains the only checkpoint advancement boundary
- facts remain recorded through `IoProvider`
- snapshots and outputs remain content-addressed artifacts

The semantic compiler must therefore remain pure, and runtime adapters must remain explicit about
all external reads.

## Where Dispatch Should Happen

Dispatch should happen at four distinct boundaries.

### 1. Config normalization phase

Purpose:

- discover semantic intent from canonical config
- reject obviously invalid combinations early

Examples:

- `protocol = aave_v3` plus `reader = reserve_position` becomes:
  - `InstrumentSemantics::FungibleToken`
  - `PositionSemantics::LendingDeposit`
  - a concrete semantic `VenueId` for the market and reserve
- a wallet declaration becomes an address-backed subject in the currently supported semantic
  families (`EvmAddressSubject` or `BitcoinAddressSubject`)

This phase should not choose transports and should not perform IO.

### 2. Op expansion time

Purpose:

- perform deterministic adapter selection
- compile semantic tasks
- define batching boundaries
- build the semantic state graph

This is the main dispatch point for "which implementation handles this config".

The op should not split the DAG by protocol. It should compile protocol-specific concerns into
task payloads and `AdapterId`s.

### 3. State execution time

Purpose:

- execute already planned tasks
- resolve the exact adapter by its planned `AdapterId`
- perform IO through typed domain clients over `IoProvider`

State execution should not rerun matching or capability discovery logic. The selected adapter must
already be explicit in the planned task.

### 4. Adapter registry / capability registry

Purpose:

- provide the planner with a deterministic catalog of supported capabilities
- provide runtime states with the matching execution adapter implementations

This is not the same as the machine transport registry. The machine transport registry routes IO
namespaces. The semantic adapter registry routes domain capabilities.

## Proposed Core Rust Abstractions

### Family and capability identifiers

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkFamily {
    Evm,
    Bitcoin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityKind {
    PrepareSources,
    PinView,
    ResolveSubject,
    ResolveValue,
    ObservePosition,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AdapterId {
    pub capability: CapabilityKind,
    pub family: NetworkFamily,
    pub implementation: String,
}
```

`AdapterId` is the stable bridge between planner and runtime. It is serializable, deterministic,
and does not rely on trait-object identity.

### Execution view and anchors

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionViewPin {
    pub network_id: String,
    pub anchor: ExecutionAnchor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum ExecutionAnchor {
    Evm { chain_id: u64, block_number: u64 },
    Bitcoin { height: u64, block_hash: String },
}
```

This is the semantic generalization of today's EVM-only `NetworkPin`.

### Planned execution spec

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortfolioExecutionSpec {
    pub source_tasks: Vec<SourcePreparationTask>,
    pub subject_tasks: Vec<SubjectResolutionTask>,
    pub view_tasks: Vec<ViewPinTask>,
    pub valuation_tasks: Vec<ValuationTask>,
    pub observation_batches: Vec<CompiledObservationBatch>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledObservationBatch {
    pub batch_id: String,
    pub adapter: AdapterId,
    pub network_view_id: String,
    pub bindings: Vec<CompiledObservationBinding>,
}
```

### Planner-facing adapter traits

```rust
pub trait PlannerAdapter: Send + Sync {
    fn id(&self) -> AdapterId;
}

pub trait ObservationPlannerAdapter: PlannerAdapter {
    fn supports(&self, req: &ObservationPlanRequest<'_>) -> bool;

    fn plan(
        &self,
        req: ObservationPlanRequest<'_>,
    ) -> Result<CompiledObservationBinding, PlanningError>;
}

pub trait ValuationPlannerAdapter: PlannerAdapter {
    fn supports(&self, req: &ValuationPlanRequest<'_>) -> bool;

    fn plan(
        &self,
        req: ValuationPlanRequest<'_>,
    ) -> Result<ValuationTask, PlanningError>;
}
```

These planner adapters are pure. They decode protocol-specific config, validate invariants, and
emit serializable planned tasks.

### Runtime-facing adapter traits

```rust
#[async_trait]
pub trait ObservationRuntimeAdapter: Send + Sync {
    fn id(&self) -> &AdapterId;

    async fn observe(
        &self,
        binding: &CompiledObservationBinding,
        input: ObservationRuntimeInput<'_>,
    ) -> Result<Observation, StateError>;
}

#[async_trait]
pub trait ValuationRuntimeAdapter: Send + Sync {
    fn id(&self) -> &AdapterId;

    async fn resolve(
        &self,
        task: &ValuationTask,
        input: ValuationRuntimeInput<'_>,
    ) -> Result<ResolvedUnitPrice, StateError>;
}
```

### Compiled binding payload model

To avoid a giant cross-workspace enum that grows with every protocol, planned tasks should have a
small generic envelope plus an adapter-owned canonical payload:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledObservationBinding {
    pub binding_id: String,
    pub observation_key: ObservationKey,
    pub adapter: AdapterId,
    pub valuation_ids: Vec<String>,
    pub payload: BTreeMap<String, serde_json::Value>,
}
```

Rules for `payload`:

- must be canonical JSON
- must be secret-free
- is owned by the selected adapter
- may contain protocol-specific query details

This keeps the semantic API stable while allowing protocol-specific payloads to grow.

The semantic layer should use one compiled observation model only. `CompiledObservationBinding` and
`CompiledObservationBatch` are the canonical planner/runtime bridge; parallel
`ObservationTask` / `ObservationBatch` models should not coexist as separate architectural centers.

### Semantic catalog

```rust
pub struct SemanticCatalog {
    pub observation_planners: Vec<Arc<dyn ObservationPlannerAdapter>>,
    pub observation_runtimes: HashMap<AdapterId, Arc<dyn ObservationRuntimeAdapter>>,
    pub valuation_planners: Vec<Arc<dyn ValuationPlannerAdapter>>,
    pub valuation_runtimes: HashMap<AdapterId, Arc<dyn ValuationRuntimeAdapter>>,
    pub subject_resolvers: HashMap<AdapterId, Arc<dyn SubjectRuntimeAdapter>>,
    pub view_pinners: HashMap<AdapterId, Arc<dyn ViewRuntimeAdapter>>,
}
```

In v1 this catalog can be constructed deterministically inside the linked workspace. No dynamic
plugin system is required.

## Adapter Discovery Model

### Deterministic selection rules

Adapter discovery must be explicit and deterministic:

1. build an intent record from canonical config
2. ask the matching planner adapter set for `supports(intent)`
3. require exactly one match
4. call `plan()`
5. persist the resulting `AdapterId` and task payload

The compiler must reject:

- zero matching adapters
- multiple matching adapters
- adapters that require missing family metadata

There must be no "first registered adapter wins" behavior.

### Separation from transport routing

The semantic adapter registry is above the transport registry:

- semantic adapters choose domain behavior
- transport registry chooses concrete IO namespace handler

An observation adapter may use:

- `rpc.control`
- a future Bitcoin RPC typed client
- local/offline transports

But the semantic state should only know that it is executing an observation adapter.

## How `CollectAaveObservationsState` Disappears

Today Aave needs:

- separate symbol splitting
- separate wallet filtering
- separate state node
- separate merge edge

In the target design:

1. config normalization classifies the symbol as:
   - `NetworkFamily::Evm`
   - `InstrumentSemantics::FungibleToken`
   - `PositionSemantics::LendingDeposit` or `PositionSemantics::LendingDebt`
   - an `Aave` `VenueId`
2. the compiler selects one adapter:
   - `observe_position/evm/aave_v3.reserve_position`
   - or `observe_position/evm/aave_v3.debt_position`
3. the compiler emits a normal `CompiledObservationBinding`
4. `ObservePortfolioBatchState` executes that binding inside its compiled batch just like any other
5. the adapter returns the same canonical `Observation` shape as every other position observation

That means:

- `CollectAaveObservationsState` goes away
- `split_symbols()` goes away
- protocol-specific branching moves into Aave planner/runtime adapters

What remains Aave-specific:

- decoding typed Aave config
- market and reserve validation
- collateral-enabled checks
- debt token selection
- metadata decoration for Aave observations

What becomes generic:

- subject iteration
- batch execution
- observation output shape
- merge behavior
- snapshot assembly
- report projection

## How The Same Model Extends To Bitcoin

Bitcoin support should not require contaminating the semantic layer with EVM concepts.

### Network and view model

Bitcoin would plug into:

- `NetworkFamily::Bitcoin`
- `ExecutionAnchor::Bitcoin { height, block_hash }`

### Wallet subject model

A future subject normalization phase could produce:

- descriptor-backed subject
- script pubkey subject
- address subject for simpler read-only cases

### Observation model

A Bitcoin adapter might plan:

- `InstrumentSemantics::NativeAsset`
- `PositionSemantics::UtxoSet`
- adapter `observe_position/bitcoin/utxo_set`
- payload containing script or descriptor selection plus spendability policy

The generic observation state would still:

- load the pinned execution view
- resolve the selected adapter
- execute the task
- receive canonical `Observation`

The semantic layer does not need to know about:

- UTXO scanning strategy
- RPC method names
- Electrum-style calls
- descriptor expansion

Those belong to Bitcoin adapters and typed Bitcoin client crates.

## Recommended Crate Boundaries

### Keep

- `crates/machine`
  - unchanged ownership of runtime semantics
- `crates/sdk`
  - unchanged ownership of launch/planning glue
- `crates/ops/portfolio-tracker-op`
  - thin planner only
- `bin/cli` and `bin/rest-api`
  - transport-only

### Refactor

- `crates/states/portfolio`
  - own semantic portfolio states:
    - `PinExecutionViewState`
    - `ResolveUnitPricesState`
    - `ObservePortfolioBatchState`
    - `AssembleSnapshotState`
    - `ProjectReportState`
- `crates/states/wallet`
  - own generic wallet subject resolution state plus wallet resolver adapter interfaces

### Add

- `crates/portfolio-semantics`
  - pure semantic compiler
  - semantic vocabulary
  - planned task types
  - adapter IDs and catalogs
  - planner-time matching and ambiguity errors

If adding a new crate is too large for the first step, this module can start under
`crates/states/portfolio/src/semantic/` and later be extracted.

### Keep protocol-specific crates protocol-specific

- `crates/evm-runtime`
  - EVM family adapters and typed helpers
- `crates/states/aave-v3`
  - Aave-specific planner/runtime adapters and typed config decoders
- future `crates/states/bitcoin`
  - Bitcoin-specific planner/runtime adapters

## Migration Plan

This phased sketch is retained as explanatory decomposition only.

If it conflicts with the hard-cutover commit sequence above, the hard-cutover commit sequence
governs and no compatibility-preserving phase should be inferred.

### Phase 1: Introduce semantic compilation without changing public behavior

- add `PortfolioExecutionSpec`
- add semantic catalog interfaces
- keep `portfolio_tracker v1`
- compile current EVM plus Aave config into planned tasks
- do not change snapshot/report output contract yet

Exit criteria:

- pure compiler tests cover deterministic selection and ambiguity failures
- no public CLI change

### Phase 2: Collapse observation collection into generic semantic states

- replace `CollectObservationsState` and `CollectAaveObservationsState` with
  `ObservePortfolioBatchState`
- keep current `Observation` output unchanged
- move Aave-specific logic into adapters

Exit criteria:

- no protocol-specific observation states remain in the portfolio flow
- merge state becomes batch-oriented rather than protocol-oriented

### Phase 3: Replace direct-price special casing with semantic valuation resolution

- replace `ReadDirectPricesState` with `ResolveUnitPricesState`
- let valuation adapters plan and execute direct or derived inputs
- use resolved valuation tasks instead of scanning symbols directly inside the state

Exit criteria:

- valuation discovery is compiler-owned
- valuation execution is adapter-owned

### Phase 4: Generalize network and wallet surfaces

- widen network config beyond EVM-only `chain_id`
- widen execution pins beyond `block_number`
- widen wallet subject surfaces beyond normalized EVM address only

If this requires a breaking change to any persisted or public portfolio JSON surface
(`PortfolioSnapshotRequest`, `PortfolioSnapshotResponse`, `PortfolioSnapshot`, or
`PortfolioReport`), the same commit must:

- cut the op version to `portfolio_tracker v2`
- add a top-level `schema_version` field to each changed JSON object and set it to `2`
- update CLI/REST/docs in the same change

Purely additive changes that preserve existing field meaning do not require the version cut above.

### Phase 5: Add non-EVM support

- implement Bitcoin family adapters
- keep the semantic state graph unchanged
- verify replay and artifact behavior across mixed-family portfolios

## Tradeoffs And Failure Modes

### Tradeoff: more adapters, fewer states

This design intentionally allows many adapters. The win is that adapters are the right place for
variation. State types and planner topology remain stable.

### Tradeoff: planner complexity moves into the compiler

The semantic compiler becomes the central decision point. That is acceptable if:

- it stays pure
- it stays deterministic
- it fails on ambiguity
- it is heavily unit tested

### Failure mode: ambiguous adapter matches

If two adapters claim the same intent, the compiler must fail. Silent precedence rules will make
behavior non-obvious and brittle.

### Failure mode: raw payload maps become untyped dumping grounds

If adapter payloads become arbitrary JSON blobs with no owner or validation rules, the design will
degrade. The rule must be:

- generic envelope owned by the semantic layer
- payload schema owned and validated by the adapter
- payload must remain canonical JSON and secret-free

### Failure mode: semantic states regain protocol branching

If `ObservePortfolioBatchState` starts `match`ing on protocol names internally, the redesign has
failed. Protocol branching belongs in adapters only.

### Failure mode: family-specific assumptions leak into generic models

If generic types continue to assume `chain_id`, EVM addresses, or EVM block numbers, non-EVM
support will remain second-class. The family split must live in enums such as `ExecutionAnchor`,
not in comments.

## What Should Stay Generic vs Protocol-Specific

### Generic

- semantic state types
- semantic compiler
- planned task envelopes
- adapter ID and registry rules
- merge behavior
- snapshot assembly
- report projection
- context namespacing behavior
- replay and persistence contracts

### Protocol-specific

- typed config decoders
- protocol invariants
- quantity acquisition logic
- ABI and RPC details
- protocol metadata shaping
- source-specific valuation reads
- family-specific execution anchors

## Testing Strategy

The redesign should be validated at four layers:

1. compiler tests
   - deterministic plan ordering
   - zero-match failures
   - ambiguous-match failures
2. adapter tests
   - protocol-specific config validation
   - runtime IO behavior with mocked `IoProvider`
3. semantic state tests
   - batch execution
   - merge and ordering
   - snapshot/report correctness
4. replay tests
   - pinned view and fact recording
   - resume from checkpoints
   - mixed-family coverage once non-EVM support lands

## Recommendation

The recommended redesign is:

- keep the current engine, event model, artifact model, and op/state boundary intact
- introduce a pure semantic compiler plus deterministic adapter catalogs
- replace protocol-specific portfolio states with a fixed semantic state family
- move protocol-specific logic into planner/runtime adapters selected from canonical config

The core architectural rule should become:

- semantic states own execution phases
- adapters own protocol and network specialization
- ops own deterministic semantic compilation

That is the cleanest path from the current `portfolio_tracker` flow to a reusable semantics-first
execution architecture without violating the repo's existing invariants.
