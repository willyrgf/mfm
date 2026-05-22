# mfm-replay

Typed kernel crate for certified replay evidence brokers and verifier contracts.

`RFC_TYPED_CORE_PROPOSAL_1.md` is the authority for this crate during the typed-core rewrite.
This crate answers replay requests only from certified typed run streams, store projections, and
recorded artifact evidence. It must not construct live capabilities, live transports, old dynamic
machine, SDK, `PlannedOp`, `PortKey`, public `StateGraph`, `DependencyEdge`, `DynContext`, or
generic `IoProvider` surfaces.
