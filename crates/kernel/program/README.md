# mfm-program

Typed kernel crate for state-program authoring, handles, builders, and registry evidence.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

Operations expand through `OperationExpansion`, a framework-minted context created only by
`ScopeBuilder::call`. Operation implementations compose registered states, nested operations, and
child scopes through the builder registry only. Registration is a property of the builder registry,
not a transferable planning token; downstream crates cannot construct `OperationExpansion` or call
`Operation::expand` directly with a raw `ScopeBuilder`.

State transition contexts are explicit typed values. Domain crates opt in with `MfmContext`, set
`StateSpec::Context`, declare values through `ScopeBuilder::declare_context`, and pass either
`NoContext` or `&DeclaredContext<C>` to every state-planning API. Declared contexts lower into the
certified context table and context-bound cell metadata; ordinary state authoring must pass
`NoContext` explicitly.
