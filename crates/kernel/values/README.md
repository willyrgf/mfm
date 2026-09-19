# mfm-values

`MfmValue` defines strict Serde/schema identity for persisted typed values.
`canonicalize_mfm_value` is the sole qualification path and returns exact canonical bytes plus the
content ref. Values reject floats, malformed schema descriptors, known secret markers, and payloads
above `MAX_RUN_OBJECT_CANONICAL_BYTES` (32 MiB).

`Object` stores checked canonical bytes with their content ref. Its ordinary `Deserialize`
implementation calls the same checked constructor before returning a value. Stored-wire rejection
uses Serde's category, location and message; nested canonical/hash/size constructor fields are not
separately retained. Direct typed construction still returns concrete `ValueError` fields.
Consumers admit the decoded Object against its selected slot descriptor before typed use.

`DiagnosticEvidence` is nested immutable JSON data with the `diagnostic_float_free` persisted
profile. It admits dependency-supplied text under ordinary canonical/numeric/structural limits;
ordinary strings retain the secret-marker rule. It has no standalone executable identity or quota.
`InvocationDiagnostic::from_fields` converts selected owner fields once and preserves the supplied
code, operation and primary size even if conversion fails or unwinds. Receivers borrow or move its
data without recovering a native owner or retrying a serializer. This does not intercept aborts or
suppress an installed panic hook.

Persisted string shapes name checked owner grammars, including the exact `EffectId` grammar; schema
validation delegates to the corresponding checked identity type.

The only derives are `MfmValue` and `PersistedSchema`. The reserved `Never` root-schema descriptor is
the one narrow empty-enum exception; no JSON value can inhabit it.

`Unsigned256` owns canonical unsigned decimal construction, the full 256-bit range, and checked
addition. Its serde decoder shares construction checks. `checked_add` returns `None` only for
arithmetic overflow; constructor failures distinguish noncanonical text from out-of-range integers
without retaining rejected text. Native domains can reuse its mechanics while retaining their own
nominal schema identities.
