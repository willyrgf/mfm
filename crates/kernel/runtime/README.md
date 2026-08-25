# mfm-runtime

`RuntimeAssemblyBuilder::new` fallibly installs the framework codec. The builder registers typed
values, Pure/Read/Effect States, Match descriptors, and distinct Read and Effect callbacks through
one private capability registry; later registration errors never poison it. Infallible `finish`
freezes one immutable assembly. Program association pre-resolves each State by its exact
implementation/input/output/failure ABI into one closed mode-specific executable with only its
valid functions, codecs, validators, and exact callback.
Multiple exact value schemas may belong to one semantic family; exact `ContentRef`, Rust `TypeId`,
and descriptor equality own codec registration and lookup. Semantic identity is never a fallback
lookup key. Fold state keeps only declaration identity and qualified values; execution never
performs a registry lookup or carries a second driver object.

Runtime owns `start`, `resume`, `read`, and the sole hot/cold semantic fold. It appends genesis and
fused Pure/Read conclusions through Store. An Effect first appends its exact command and derived
`EffectId`, calls its adapter only after known insertion, then appends evidence and outcome as the
adjacent conclusion. Runtime extends locally after `Inserted` and completely reloads after
`NotInserted` or on cold entry. Match is a no-frame structural projection. `read` never progresses
or calls an adapter; it re-prepares only to validate a retained Effect command and identity.
The Match projection selects and qualifies the exact nested canonical payload bytes through the
registered payload codec; it never serializes a typed selector or payload.

Every Read callback receives `QualifiedValue.value_ref` for its exact intent, and hot and cold
evidence binding receives that same reference. Every Effect callback receives the exact command
value ref used in `EffectId` derivation. Neither callback receives the shared codec contract ref in
its place.

An Effect adapter returns `Pending` or `Settled(evidence)`. Pending appends no conclusion, restores
the identical prepared fold state, and returns a publicly Runnable view without another adapter
entry in that invocation. Adapter errors remain distinct redaction-safe Runtime errors and also
append no conclusion. Dropping at any await is safety-neutral; Runtime has no background finalizer,
semaphore, timeout, internal retry loop, or public State-by-State lifecycle.
