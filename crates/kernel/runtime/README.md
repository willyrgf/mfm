# mfm-runtime

Typed kernel crate for the event-sourced typed state-machine workflow runtime.

`docs/design.md` is the normative typed-core authority contract. This crate executes only certified
typed execution specs.

`CertifiedRuntimeSpec::new` accepts only `mfm_certify::CertifiedTypedSpec`. Runtime callers cannot
construct runtime authority from a parsed `TypedExecutionSpec`, a hash-only
`mfm_spec::v1::HashedSpecEnvelope`, or parsed persisted spec/certificate data. Persisted
spec/certificate bytes must pass through the certifier verifier before they can reach this crate.

`CertifiedRuntimeSpec` indexes certified semantics, including transition contexts, and derives
erased runner plans for execution. The runner plan is not authority by itself. Runtime-only checks
remain runtime-owned: runner availability, capability availability, seed/config evidence, stream
drift, context materialization, and execution contract validation.

External reads use one generic runner contract. Runtime loads the typed config, arbitrary certified
input tree, and certified context; asks the state for its immutable plan; lets the adapter execute
only that plan; invokes the state reducer; and stages exactly one typed primary read-evidence
artifact plus any kernel-owned fact-query evidence and the canonical output. Replay performs the
same materialization and reduction from retained evidence without constructing live capabilities.

The visible runtime model is:

```text
certified spec + verified run history
  -> bound runtime context
  -> deterministic frontier scheduler decision
  -> attempt lifecycle
  -> sealed runner invocation
  -> guarded commit
```

The certified spec is the static certified transition graph. The verified run history is rebuilt
from the append-only run stream authority before the scheduler decides whether the run is blocked,
completed, or ready to execute one certified node. The bound runtime context proves runner binding
availability and executable identity for every certified executable node before run admission or
resume dispatch, and rejects context-bound output nodes whose runners do not expose a context-output
extractor. Runner invocation is sealed by runtime-owned input and certified-context
materialization, runner identity checks, and capability scoping. The commit planner verifies typed
payloads, side-effect protocol rules, staged artifacts, context-bound output evidence, references,
retention bindings, and commit preconditions before building purpose-specific
`PreparedCommit<Purpose>` values and submitting them through `PreparedCommitPlan`.

Launch is a pre-FSM admission lifecycle, not a scheduler-dispatched state attempt.
`RunAdmissionLifecycle` verifies the certified spec, launch artifacts, seeds, configs, executable
availability, capability availability, and binding digest before minting the single-payload
`RunAdmitted` commit. No state attempt, cell production, artifact-reference payload, or retention-ref
payload is appended during admission. Ordinary state attempts, public-output rendering, retention
projection, and terminal framework work then use the shared started-before-run authority path.
`RunCompleted` is derived from the sealed `CompleteRun`
framework state for successful public-output completion, or from `ResolveSagaTerminal` when the
saga terminal path resolves compensation, manual resolution, or failure without an AC/DC claim.

Module roles:

- `spec_authority`: runtime authority wrapper over `CertifiedTypedSpec` and certified contexts
- `binding`: bound runner identity and executable availability for a certified runtime spec
- `admission`: pre-FSM run-start admission lifecycle
- `history`: spec-aware verified history and context-aware input materialization
- `frontier`: pure scheduler decision and attempt planning
- `transition`: closed transition decisions over certified spec and verified history
- `attempt`: ordinary attempt lifecycle from selection through terminal commit
- `framework_lifecycle`: started-before-run framework attempt lifecycle for public-output rendering,
  retention, and terminal states
- `recovery`: open-attempt recovery frontier validation
- `side_effect_lifecycle`: side-effect attempt uncertainty and recovery evidence guards
- `scheduler`: serial orchestration over admission, transition, lifecycle dispatch, and store
- `invocation`: sealed runner context builder, certified context handles, and materialized inputs
- `commit`: launch and runner-output commit planning
- `side_effects`: runtime protocol guards for durable side-effect ledgers
- `framework`: certified framework lifecycle states
- `artifacts`, `runners`, and `error`: artifact evidence, runner registry/types, and runtime errors
