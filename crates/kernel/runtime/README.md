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
