# mfm-program-derive

Small consuming-crate derives for `MfmValue`, `PersistedSchema`, and `MfmContext`. Schema expansion
emits checked metadata and delegates canonical qualification to `mfm-values`; it does not create
execution or configuration APIs.

Checked identity fields, including `EffectId`, lower to their owner-specific bounded string grammar.
`CanonicalBytes` fields may declare exact decoded bounds with
`#[mfm(minimum_bytes = N, maximum_bytes = M)]`; the generated shape is `BoundedBytes`.
One-field `#[serde(transparent)]` wrappers inherit the checked shape of `String`,
`CanonicalBytes`, `NonZeroU64`, and supported ordered maps. Wrappers with explicit
`serde(try_from = "String", into = "String")` conversion describe a canonical string wire.

`MfmContext` requires `#[context(namespace = "...")]` on a named struct. Each replaceable field
is a distinct bare type parameter used exactly once. Bounds, defaults, where clauses, lifetime and
const parameters, nested parameter occurrences, and opaque type macros are unsupported. It emits
`<Struct><FieldInPascalCase>Slot` markers with the context's visibility, implementing
`mfm_values::ContextSlot` for borrowing and replacing that field. Replacement moves all siblings
without `Clone` and accepts every `MfmValue`. Slot identities are `<namespace>/slot/<field>` and
remain the same across replacement types. Context namespaces are explicit caller-owned identities;
`slot_id()` checks the resulting `StableId` grammar and length. Serde and `MfmValue` continue to
own encoding and exact schemas. This mechanical primitive does not enforce accumulation or history
provenance; domain stage types and checked fact constructors own those constraints.
