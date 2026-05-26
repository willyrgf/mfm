# mfm-runtime

Typed kernel crate for the certified serial scheduler.

`RFC_TYPED_CORE_PROPOSAL_1.md` is the authority for this crate during the typed-core rewrite.
This crate executes only certified typed execution specs. It must not depend on old dynamic
machine, SDK, `PlannedOp`, `PortKey`, public `StateGraph`, `DependencyEdge`, `DynContext`, or
generic `IoProvider` surfaces.

`CertifiedRuntimeSpec::new` accepts only `mfm_certify::CertifiedTypedSpec`. Runtime callers cannot
construct runtime authority from a parsed `TypedExecutionSpec`, a hash-only
`mfm_spec::v1::HashedSpecEnvelope`, or parsed persisted bundle data. Persisted spec/certificate
bytes must pass through the certifier verifier before they can reach this crate.
