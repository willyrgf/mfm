# mfm-runtime

Runtime progresses a complete immutable Program through a mechanical Store. Program owns typed
callbacks, native adapters, exact contract admission and recovery handlers. Runtime holds the
Program directly and owns continuation, acknowledgement, recovery authorization and local safety
checks; it has no assembly builder or registration APIs.

Every opaque Journal payload is a Runtime-owned `RunRecord` with required domain `mfm.runtime-record.v1`: the admitted Program ref, current
`RecordedOperation`, active checkpoint inputs, per-State recovery usage and irreversible Effect
barrier. The operation alone determines continuation. A private borrowed selector serves dispatch,
validation and public projection; no stored phase or input copy accompanies it. Objects retain
exact canonical bytes and refs without native caches or object tables. Program commits the exact
initial value, checked before admission or provider entry.

`program_document` bootstraps cold construction without executable association. It uses one Store
admission/latest snapshot, qualifies both Journal envelopes and their identity/head linkage, parses
the admission through the same private RunRecord decoder as restore, and checks admission variant,
Program reference linkage and metadata bounds. It returns the retained Program Object without
re-encoding. It neither parses the latest Runtime payload nor validates its continuation, resolves
code/resources, executes callbacks or appends. Program's decoder owns canonical Program validation.
Failures retain the requested RunId with no last observation. Extraction grants no append authority;
a later read/resume loads its own fresh snapshot.

`Runtime::new(store)` constructs the driver. `resume(run_id, program)` explicitly progresses an
existing run, and `read(run_id, program)` observes without progression. Both require the complete
Program and reject a reference mismatch with the admitted document. `execute(run_id, program, &input)`
returns a checked terminal ExecutionResult; `start(run_id, program, &input)` returns a RunView at the
next manual progression boundary. Both encode the borrowed input once synchronously with panic
containment, then check its exact contract and commitment before appending. This admission encoding
is the explicit exception to the blocking-work rule. Cold restore loads admission/latest and any requested candidate probe from
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
classification or policy. A separate recovery commit records classification, request
and authorized decision. Accepted retry/restart spends allowance and yields. Restart restores an
active checkpoint and prunes later snapshots without resetting usage. Policy failure
leaves the committed original available to a later invocation.

An Effect validates native command extraction, then commits its complete input, semantic command
and EffectId before adapter entry. Pending returns
the unchanged view. Operational failure first commits its original, then a recovery decision
returns to EffectPending with the same authority. Retry spends allowance and yields; Stop returns
`InvocationFailure::RecoveryStopped`, while explicit resume may reconcile that command. There is no
pending-failure quota. Accepted evidence is bound and committed before interpretation as
`AwaitingInterpretation`; resuming this phase performs no adapter IO. Settled Effects cannot recover,
and Effect barriers prevent restart across their position.

Known insertion adopts the candidate continuation locally. NotInserted performs one exact candidate
probe, adopting a checked observation only when the candidate is present. Manual progression yields
there; automatic `execute` may continue from the checked retained continuation using its exact
command authority. Exclusion or absence remains this attempt's noninsertion. Presence is retained independently if subsequent projection
fails. Store errors return immediately without probing; ambiguous acknowledgement remains
Indeterminate. Recording errors retain the available original, separate recording cause and exact
candidate only after submission. BeforeAppend requires an admitted Failure and retains its concrete
preparation cause without unsent bytes. Failed first-original encoding instead reports known
position/contract, unavailable original contents/identity and the concrete encoding facts. A projection failure after known insertion additionally retains its
acknowledged head, separately from any older last-observed view.

Audit records cover acknowledged originals. Cancellation between provider response and failure
append can leave a physical attempt unrecorded. There is no fallback record through a failed Store,
background completion loop, or prospective capacity promise. Actual object, metadata, frame, run
and derived-report limits apply at their owning boundaries.

RunView distinguishes Runnable, EffectPending, AwaitingRecovery, AwaitingInterpretation, Succeeded
Object and Failed FailureReport. Pending views expose their latest committed original and decision.
FailureReport v6 is a content-addressed canonical projection of RunId, Program ref, State
implementation ref, execution ABI, the complete original Failure, reason and usage, bounded to 32 MiB before terminal append and never separately persisted. Internal
errors remain immutable invocation data with their originating operation/stage and supplied primary
size facts. Application borrows those facts for normal or final transport presentation; no native
projector, independent reporting quota or omission ledger remains.

Typed constructors materialize at selected callbacks. Before reconciling an unresolved Effect,
Runtime re-prepares and compares the exact command and identity. Reading completed work does not
rerun interpretations, classification, policy, provider or signer IO. Heavy validation,
callback work and encoding use immediately awaited pure blocking jobs; Store and adapter IO remain
in the async driver. Program owns typed State/adapter invocation and original classification
callbacks. Runtime runners are nongeneric and carry no State decoding or encoding functions.
Runtime supplies only the execution position to wrappers that capture their declared original
contract; it attaches its operation to Decode/Execute/Encode without reconstructing diagnostics.
Capabilities owns `EffectAdapterOutcome` (Pending/Settled). Read/Effect callbacks receive exact
intent/command instance refs.

`Failure` directly owns domain/Read/pending-Effect facts and original values. Persisted evidence
slots retain native Objects; semantic projection happens at selected callbacks. `RecoveryOutcome::Stop`
contains only its reason. `FailureReport::failure()` borrows the complete retained failure, and its
canonical artifact supplies hashing/output without a second cause tree or root mapping. Pending
views own their EffectCall and optional original/outcome pair. `RecoveryStopped` retains only the
observed view.

Use [build and verification](../../../docs/build-and-verification.md) for current commands and
custody, cancellation, recovery, cold-load and managed acceptance coverage.
