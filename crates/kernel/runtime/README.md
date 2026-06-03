# mfm-runtime

Typed kernel crate for the event-sourced typed state-machine workflow runtime.

`docs/design.md` is the normative typed-core authority contract. This crate executes only certified
typed execution specs. It must not depend on old dynamic machine, SDK, `PlannedOp`, `PortKey`,
public `StateGraph`, `DependencyEdge`, `DynContext`, or generic `IoProvider` surfaces.

`CertifiedRuntimeSpec::new` accepts only `mfm_certify::CertifiedTypedSpec`. Runtime callers cannot
construct runtime authority from a parsed `TypedExecutionSpec`, a hash-only
`mfm_spec::v1::HashedSpecEnvelope`, or parsed persisted bundle data. Persisted spec/certificate
bytes must pass through the certifier verifier before they can reach this crate.

`CertifiedRuntimeSpec` indexes certified semantics and derives erased runner plans for execution.
The runner plan is not authority by itself. Runtime-only checks remain runtime-owned: runner
availability, capability availability, seed/config evidence, stream drift, and execution contract
validation.

The visible runtime model is:

```text
certified spec + verified run history
  -> deterministic frontier scheduler decision
  -> sealed runner invocation
  -> guarded commit
```

The certified spec is the static certified transition graph. The verified run history is rebuilt
from the append-only run stream authority before the scheduler decides whether the run is blocked,
completed, or ready to execute one certified node. Runner invocation is sealed by runtime-owned
materialization, runner identity checks, and capability scoping. The commit planner verifies typed
payloads, side-effect protocol rules, staged artifacts, references, retention bindings, and commit
preconditions before building `PreparedTypedCommit`.

Launch, ordinary state attempts, public-output rendering, retention projection, and completion all
use the same authority path. `RunStarted` is emitted only by the sealed `BootstrapRun` genesis batch,
and `RunCompleted` is derived only from the sealed `CompleteRun` framework state.

Module roles:

- `spec_authority`: runtime authority wrapper over `CertifiedTypedSpec`
- `history`: spec-aware verified history and input materialization
- `frontier`: pure scheduler decision and attempt planning
- `scheduler`: serial orchestration over history, frontier, invocation, runners, commits, and store
- `invocation`: sealed runner context and materialized input surfaces
- `commit`: launch and runner-output commit planning
- `side_effects`: runtime protocol guards for durable side-effect ledgers
- `framework`: certified framework lifecycle states
- `artifacts`, `runners`, and `error`: artifact evidence, runner registry/types, and runtime errors
