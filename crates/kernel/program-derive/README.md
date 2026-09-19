# mfm-program-derive

Nested `Object` fields use Values' structural Object shape, including when fully qualified.
The derive does not make `Object` an `MfmValue` or certify its native protocol. Typed decoding
retains the checked Object decoder; descriptor admission alone does not prove nested digest equality.

Unit structs use the existing unit schema shape and Serde's `null` representation. Their nominal
identity remains distinct from other unit contracts and from empty named structs (`{}`).

Small consuming-crate derives for `MfmValue` and `PersistedSchema`. Schema expansion
emits checked metadata and delegates canonical qualification to `mfm-values`; it does not create
execution or configuration APIs.

Checked identity fields, including `EffectId`, lower to their owner-specific bounded string grammar.
`CanonicalBytes` fields may declare exact decoded bounds with
`#[mfm(minimum_bytes = N, maximum_bytes = M)]`; the generated shape is `BoundedBytes`.
One-field `#[serde(transparent)]` wrappers inherit the checked shape of `String`,
`CanonicalBytes`, `NonZeroU64`, and supported ordered maps. Wrappers with explicit
`serde(try_from = "String", into = "String")` conversion describe a canonical string wire.

A checked owner can select `#[mfm(decode_native = "Self::decode_checked")]` on `MfmValue`.
The derive emits only an override of the existing `decode_native(&[u8])` method. That function
returns `Result<Self, mfm_values::InvocationDiagnostic>` and shares the owner's checked construction with
ordinary deserialization, preserving typed constructor causes at Runtime entry. This attribute
does not change schema identity and is not supported by `PersistedSchema`.
