# mfm-runtime

Typed kernel crate for the certified serial scheduler.

`RFC_TYPED_CORE_PROPOSAL_1.md` is the authority for this crate during the typed-core rewrite.
This crate executes only certified typed execution specs. It must not depend on old dynamic
machine, SDK, `PlannedOp`, `PortKey`, public `StateGraph`, `DependencyEdge`, `DynContext`, or
generic `IoProvider` surfaces.
