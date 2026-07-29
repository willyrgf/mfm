# mfm-program

Typed state callbacks and deterministic operation-authoring contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

Every `State` selects exactly one closed `StateExecution`: pure, audited read, or recoverable
effect. Callbacks receive borrowed verified value views; read reducers additionally receive an
opaque value-only `ObservationView` containing a typed returned outcome or classifier-approved
safe-failure metadata with an optional typed diagnostic, not the observation's journal
`ValueRef`. Append and live-access authority remain private to runtime. Successful `Settlement`
values carry an ordered `FactSet`.

`AuthoredProgramBuilder` records canonical state occurrences, nested-child composition, typed
edges, bridges, required-success nodes, and public outputs. `connect_many` authors conjunctive
`Vec<T>` fan-in; certification derives canonical producer order from final node ids. Framework and
executor expansion belongs exclusively to `mfm-certify`.

`QualifiedProgramRegistryBuilder` is the sole program-definition assembly boundary. It creates one
immutable shared definition, invokes the registered certification factory exactly once over that
definition, and retains process callbacks separately from semantic values. The builder is generic:
application qualification owns the exact production component inventory, while the kernel enforces
common descriptor identity and exact state-to-operation-to-manifest closure. Current-candidate
selection returns sealed, non-serializable identities and replay-safe deterministic callbacks;
cross-version comparison must pass the pure plan-compatibility predicate before invoking them.
