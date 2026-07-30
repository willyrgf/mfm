# mfm-runtime

Audited owner of run admission and one-action execution.

The store owns the verified append-only journal and all persisted structural
validation. `Runtime<B>` consumes the sole non-cloneable `RunHistoryWriter<B>` during process
assembly and becomes the only application-visible run-history mutation path. It is shared as an
`Arc`; callers can neither clone nor recover the writer. The runtime selects the admitted package
from one qualified program registry. That registry owns the exact admitted support graph,
deterministic callbacks, semantic read/effect entries, and process-private
runtime invokers as one closed selection.

`Runtime::admit` consumes an opaque `AuthorizedAdmissionPlan` containing only owned, already
authorized application inputs. It verifies the exact registry, entry point, operation,
invocation, artifacts, configured value, and source seals, injects the registry's admitted
support, prepares the immutable root, and appends it. The application cannot supply support,
prepared append bytes, a store, or a writer through that plan.

`Runtime::drive_once` loads a fresh `VerifiedRunView` under drive-purpose authority and performs at
most one semantic transition or one audited protocol operation.

`Runtime::drive_once` uses the frozen priority:

1. block on invalid evidence anywhere in a complete consumable suffix or
   another integrity finding;
2. return the existing closure when the verified fold is already closed;
3. block on the impossible integrity state in which the fold is open but every
   occurrence is terminal;
4. settle the first callback-accepted committed observation;
5. commit the first ready pure settlement, effect request, or dependency skip;
6. authorize one live read or effect ensure, choosing the fewest prior matching
   authorizations and then certified node order;
7. return an operational block when a required prerequisite is unavailable; or
8. return waiting when no action or block exists.

An unavailable current candidate produces an operational block, while a sealed
candidate identity or callback-integrity failure produces an integrity block.
Candidate execution failure is returned only through its redaction-safe typed
runtime error.

Already-terminal read and effect occurrences are still reproduced against the
complete consumable observation suffix. Runtime remembers the first accepted
observation without replacing it with a later settlement, but any invalid
evidence anywhere in the suffix—including after that remembered
settlement—blocks. Only a complete all-insufficient suffix allows another live
access.

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

The runtime has no caller-supplied append path, retry state, attempt object, runner lifecycle,
replay-specific reducer, or compatibility fallback. Its process-local writer capability is not
persisted semantic state; another independently qualified runtime may continue the same journal
under backend compare-and-swap.
