# mfm-runtime

Stateless audited interpreter for an admitted MFM run.

The store owns the verified append-only journal and all persisted structural
validation. `mfm-runtime` receives a drive-purpose store authority, loads a
fresh `VerifiedRunView`, and selects the admitted package from one qualified
program registry. That registry owns the exact admitted support graph,
deterministic callbacks, semantic read/effect entries, and process-private
runtime invokers as one closed selection. It performs at most one semantic
transition or one audited protocol operation.

`Runtime::drive_once` uses the frozen priority:

1. block on invalid evidence or another integrity finding;
2. settle the first callback-accepted committed observation;
3. commit the first ready pure settlement, effect request, or dependency skip;
4. authorize one live read or effect ensure, choosing the fewest prior matching
   authorizations and then certified node order; or
5. return a closed or waiting outcome.

An unavailable current candidate produces an operational wait, while a sealed
candidate identity or callback-integrity failure produces an integrity wait.
Candidate execution failure is returned only through its redaction-safe typed
runtime error.

Already-terminal read and effect occurrences are still reproduced against the
complete consumable observation suffix. The first accepted observation must be
the transition's recorded consumed observation, and any later invalid evidence
still blocks.

A state callback receives only typed value views. It cannot append, perform
ambient IO, select another implementation, or retain runtime authority.
Read/effect request intent is fixed before authorization. Runtime appends the
authorization before invoking the capability, mints affine live-access
authority only from the directly observed newly-appended witness, and appends
the observation before any semantic reduction.

For reads, the qualified typed adapter resolves the request's exact immutable
routing generation before authorization. The generation must be both a member
of the admitted aggregate routing catalog and present in the adapter's private
route table; otherwise the operation fails closed without boundary entry.

The reserved fact-selection read has no process invoker. Runtime validates its
exact admitted scan contract, appends the barrier authorization, and lets only
the resulting fresh affine permit enter the store-owned authoritative scan.
Before a returned fact-selection observation reaches the state reducer, the
store rechecks complete prefix coverage against the exact verified run view.
A crashed scan is audit-only and a later drive must append a fresh
authorization.

Effect executors own durable keyed convergence and return a verified pending or
terminal evidence bundle. They cannot settle a node. Runtime promotes only the
verified transitive object closure referenced by that result and journals the
observation before calling the state settlement function.

Semantic closure is never a standalone action. The store attaches `RunClosed`
atomically to the transition that terminalizes the final occurrence. After
closure, runtime can append only the single unmatched observation for each
authorization committed before closure.

The runtime has no admission path, retry state, attempt object, runner
lifecycle, raw journal append, replay-specific reducer, or compatibility
fallback.
