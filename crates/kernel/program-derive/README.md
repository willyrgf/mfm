# mfm-program-derive

Small consuming-crate derives for `MfmValue` and `PersistedSchema`. Expansion emits checked schema
metadata and delegates canonical qualification to `mfm-values`; it does not create execution,
configuration, output-marker, or field-path APIs.

Checked identity fields, including `EffectId`, lower to their owner-specific bounded string grammar.
String fields carrying canonical base64url public bytes may declare exact decoded bounds with
`#[mfm(minimum_bytes = N, maximum_bytes = M)]`; the generated shape is `BoundedBytes`.
Checked transparent public-byte newtypes use `#[mfm(transparent_bytes)]` with the same field bounds.
Bounded `#[mfm(transparent_string)]` newtypes use those bounds as exact UTF-8 byte limits.
Unsigned fields may declare exact inclusive bounds with
`#[mfm(unsigned_minimum = N, unsigned_maximum = M)]`.
