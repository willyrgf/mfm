# mfm-runtime

Runtime associates an immutable linear Program with typed States, codecs, root maps, intrinsic error projections,
handlers and adapters. `RuntimeAssemblyBuilder::new` installs framework units; explicit registrations
select exact contracts and parameters. A failed registration does not poison the builder. `finish`
freezes the assembly. Generic States may share an implementation ID with different exact ABIs;
value semantic IDs never substitute for exact schema/Rust-type/descriptor agreement.

The sole semantic fold drives admission, hot pre-append validation, local advancement and cold
reconstruction. It derives position/visit, active checkpoint input, recovery usage and Effect
barriers from retained history. Program commits the exact initial value. Admission checks that
commitment and the complete bounded history before genesis or provider entry.

`start` admits and progresses, `resume` explicitly progresses an existing run, and `read`
reconstructs without progression. Pure/Read outcomes and recovery decisions append atomically.
Operational Read errors retain their original capability error and State-owned context without
fabricating evidence or domain failure. Accepted retry/restart spends the committed allowance and
yields at a fresh visit. Handling, root mapping or validation failure before append
leaves the previous head unchanged.

An Effect appends its complete command before adapter entry. A pending response yields the same
EffectId and visit without a record. Operational failure appends original error, State context and
Retry/Stop before acknowledgement. Retry spends allowance and yields with the same command; Stop
returns `InvocationFailure::RecoveryStopped`. Both consume failure capacity. Before adapter entry,
Runtime checks another admitted failure slot; exhaustion returns `pending_failures` without IO.
Cold `EffectPending` views expose `latest_failure`, including its committed decision. Explicit
resume can settle while capacity remains. Settlement ends the prepare/failure*/conclusion lifecycle;
settled Effects cannot recover and retained prepares prevent restart across their position.

Audit records cover acknowledged qualified outcomes. Cancellation between provider response and
failure append can leave a physical attempt unrecorded; no outcome is acknowledged from that gap.

Store insertion extends the local fold. A losing append reloads and returns the winner without
executing its newly selected visit. Ambiguous acknowledgement preserves the Store error source and
last observed qualified head; that observation does not assert the current state. Cancellation
before append spends no recovery allowance and there is no background completion or retry loop.

Cold reconstruction never reruns completed State interpretations, classifiers, handlers, context
builders or maps, and performs no provider/signer IO. It re-prepares only the final unresolved
Effect to qualify its exact retained command before reconciliation. Completed outcomes and policy
decisions are authoritative retained facts, subject to structural and Runtime safety qualification.

RunView distinguishes runnable (position and advance/retry/restart reason), Effect-pending,
succeeded ValueView and failed FailureReport. Reports are content-addressed canonical values derived
from retained original/root causes, reason, position and usage; they are not independent Journal
frames. Typed decoding rechecks the exact value contract. InvocationFailure separately represents
an interrupted call, possibly with unknown durable state. Application owns transport rendering.

Heavy qualification, callback evaluation and report construction run in immediately awaited pure
blocking work. Store and adapter IO stay in the async driver. Public callbacks receive exact intent
or command instance references, never codec contract references in their place.
