# mfm-program-derive

Small consuming-crate derives for `MfmValue` and `PersistedSchema`. Expansion emits checked schema
metadata and delegates canonical qualification to `mfm-values`; it does not create execution,
configuration, output-marker, or field-path APIs.

Checked identity fields, including `EffectId`, lower to their owner-specific bounded string grammar.
`CanonicalBytes` fields may declare exact decoded bounds with
`#[mfm(minimum_bytes = N, maximum_bytes = M)]`; the generated shape is `BoundedBytes`.
One-field `#[serde(transparent)]` wrappers inherit the checked shape of `String`,
`CanonicalBytes`, `NonZeroU64`, and supported ordered maps. Wrappers with explicit
`serde(try_from = "String", into = "String")` conversion describe a canonical string wire.
