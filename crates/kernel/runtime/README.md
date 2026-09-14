# mfm-runtime

Runtime associates an immutable linear Program with typed States, value codecs, root maps,
classifiers, handlers and adapters. `RuntimeAssemblyBuilder::new` installs framework units;
explicit registrations select exact contracts and parameters. A failed registration does not
poison the builder. `finish` freezes the assembly. Generic States may share an implementation ID
with different exact ABIs; semantic IDs never substitute for exact schema/Rust-type agreement.

Every opaque Journal payload is a Runtime-owned `RunRecord`: the admitted Program ref, current
`RecordedOperation`, active checkpoint inputs, per-State recovery usage and irreversible Effect
barrier. The operation alone determines continuation. A private borrowed selector serves dispatch,
validation and public projection; no stored phase or input copy accompanies it. Objects retain
exact canonical bytes and refs without native caches or object tables. Program commits the exact
initial value, checked before admission or provider entry.

`start` admits and progresses, `resume` explicitly progresses an existing run, and `read` observes
without progression. Cold restore loads admission/latest and any requested candidate probe from
one Store snapshot. Runtime checks selected headers, current operation contracts,
positions, allowances, checkpoints and Effect authority. It does not fold old frames or reconstruct
historical counter increments. Immutable acknowledged history remains a required Store contract.
Runtime derives ordinary Serde decoding on its current payload types; Values' checked Object
constructor runs during Deserialize. Invalid stored Objects report `restore`/`decode` with parser
category, available location and reason, including nested size rejection. No structured nested
constructor or size fields are promised on that path. Direct typed construction and postdecode
slot admission retain their structured errors. Readers accept supported Serde sequence forms and
reject unknown/duplicate fields, invalid checked values and trailing input.

A declared failure commits its original and complete executed facts as `AwaitingRecovery` before
classification, policy or root mapping. A separate recovery commit records classification, request
and authorized decision. Accepted retry/restart spends allowance and yields. Restart restores an
active checkpoint and prunes later snapshots without resetting usage. Policy or mapping failure
leaves the committed original available to a later invocation.

An Effect commits its complete input, command and EffectId before adapter entry. Pending returns
the unchanged view. Operational failure first commits its original, then a recovery decision
returns to EffectPending with the same authority. Retry spends allowance and yields; Stop returns
`InvocationFailure::RecoveryStopped`, while explicit resume may reconcile that command. There is no
pending-failure quota. Accepted evidence is bound and committed before interpretation as
`AwaitingInterpretation`; resuming this phase performs no adapter IO. Settled Effects cannot recover,
and Effect barriers prevent restart across their position.

Known insertion adopts the candidate continuation locally. NotInserted performs one exact candidate
probe, yielding a checked observation only when the candidate is present. Exclusion or absence
remains this attempt's noninsertion. Presence is retained independently if subsequent projection
fails. Store errors return immediately without probing; ambiguous acknowledgement remains
Indeterminate. Recording errors retain the available original, separate recording cause and exact
candidate once sealed. A projection failure after known insertion additionally retains its
acknowledged head, separately from any older last-observed view.

Audit records cover acknowledged originals. Cancellation between provider response and failure
append can leave a physical attempt unrecorded. There is no fallback record through a failed Store,
background completion loop, or prospective capacity promise. Actual object, metadata, frame, run
and derived-report limits apply at their owning boundaries.

RunView distinguishes Runnable, EffectPending, AwaitingRecovery, AwaitingInterpretation, Succeeded
Object and Failed FailureReport. Pending views expose their latest committed original and decision.
FailureReport is a content-addressed canonical projection of original/root causes, reason, position
and usage, bounded to 32 MiB before terminal append and never separately persisted. Internal native
errors remain invocation data, with their originating operation/stage and fallible bounded
projection. Application owns the prepared transport models and incomplete-report omissions.

Typed constructors materialize at selected callbacks. Before reconciling an unresolved Effect,
Runtime re-prepares and compares the exact command and identity. Reading completed work does not
rerun interpretations, classification, policy, mapping, provider or signer IO. Heavy validation,
callback work and encoding use immediately awaited pure blocking jobs; Store and adapter IO remain
in the async driver. Read/Effect callbacks receive exact intent/command instance refs.

`Failure` directly owns domain/Read/pending-Effect facts and original values. `RecoveryOutcome::Stop`
contains a root only for domain failures. `FailureReport::failure()` and `root()` borrow those typed
owners; its canonical artifact supplies hashing/output. Pending views own their EffectCall and
optional original/outcome pair. `RecoveryStopped` retains only the observed view. Public wire tags,
terminal report shape and stop status remain unchanged through borrowing serializers.
