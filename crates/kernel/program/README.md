# mfm-program

Typed kernel crate for state-program authoring, handles, builders, and registry evidence.

`RFC_TYPED_CORE_PROPOSAL_1.md` is the authority for this crate during the typed-core rewrite.
This crate is framework-owned and must remain domain-free. It must not depend on old dynamic
machine or SDK crates.

Operations expand through `OperationExpansion`, a framework-minted context created only by
`ScopeBuilder::call` and `ScopeBuilder::call_registered`. Operation implementations can compose
registered states, nested operations, and child scopes through that context, but downstream crates
cannot construct it or call `Operation::expand` directly with a raw `ScopeBuilder`.
