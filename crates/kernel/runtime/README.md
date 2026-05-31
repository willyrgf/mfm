# mfm-runtime

Typed kernel crate for the certified serial scheduler.

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

Runtime mutation middleware is the only production execution writer. Launch, ordinary state
attempts, public-output rendering, retention projection, and completion all produce staged artifacts
or sealed handles plus typed payload intent; middleware verifies bindings, stages bytes, builds
`PreparedTypedCommit`, and calls `append_prepared_typed_commit`. `RunStarted` is emitted only by the
sealed `BootstrapRun` genesis batch, and `RunCompleted` is derived only from the sealed
`CompleteRun` framework state.
