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

`NativeCause::project` also catches unwinding projection panics while retaining the borrowed owner.
The separate failure identifies native projection and explicitly withholds the panic payload.
Callers must not retry a failed projector. This does not intercept aborts or suppress a process's
installed panic hook.

Persisted string shapes name checked owner grammars, including the exact `EffectId` grammar; schema
validation delegates to the corresponding checked identity type.

The only derives are `MfmValue` and `PersistedSchema`. The reserved `Never` root-schema descriptor is
the one narrow empty-enum exception; no JSON value can inhabit it.
