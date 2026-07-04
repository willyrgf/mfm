# mfm-program

Typed kernel crate for state-program authoring, handles, builders, and registry evidence.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

Operations expand through `OperationExpansion`, a framework-minted context created only by
`ScopeBuilder::call` and `ScopeBuilder::call_registered`. Operation implementations can compose
registered states, nested operations, and child scopes through that context, but downstream crates
cannot construct it or call `Operation::expand` directly with a raw `ScopeBuilder`.

State transition contexts are explicit typed values. Domain crates opt in with `MfmContext`, set
`StateSpec::Context`, declare values through `ScopeBuilder::declare_context`, and plan
context-required states with `ScopeBuilder::state_in_context`. Declared contexts lower into the
certified context table and context-bound cell metadata; ordinary state authoring remains
`NoContext` by default.
