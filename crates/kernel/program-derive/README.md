# mfm-program-derive

Small consuming-crate derives for `MfmValue` and `PersistedSchema`. Expansion emits checked schema
metadata and delegates canonical qualification to `mfm-values`; it does not create execution,
configuration, output-marker, or field-path APIs.

Checked identity fields, including `EffectId`, lower to their owner-specific bounded string grammar.
