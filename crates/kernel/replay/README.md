# mfm-replay

Typed kernel crate for certified replay evidence brokers and verifier contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate answers replay requests only from certified typed run streams, store projections, and
recorded artifact evidence. It must not construct live capabilities, live transports, old dynamic
machine, SDK, `PlannedOp`, `PortKey`, public `StateGraph`, `DependencyEdge`, `DynContext`, or
generic `IoProvider` surfaces.
