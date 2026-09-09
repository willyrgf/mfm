# mfm-values

`MfmValue` defines strict Serde/schema identity for persisted typed values.
`canonicalize_mfm_value` is the sole qualification path and returns exact canonical bytes plus the
content ref. Values reject floats, malformed schema descriptors, known secret markers, and payloads
above `MAX_RUN_OBJECT_CANONICAL_BYTES` (32 MiB).

Persisted string shapes name checked owner grammars, including the exact `EffectId` grammar; schema
validation delegates to the corresponding checked identity type.

The only derives are `MfmValue` and `PersistedSchema`. The reserved `Never` root-schema descriptor is
the one narrow empty-enum exception; no JSON value can inhabit it.
