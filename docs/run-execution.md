# Run execution

One run has an explicit caller-supplied RunId and one checked Program committing to its exact
initial value. Each frame carries the complete current continuation and that operation's facts.

```text
start(RunId, Program, C0)
  -> append admission with exact Program, C0 and current state
  -> Pure: evaluate -> append success or original failure
  -> Read: prepare -> observe -> bind/interpret -> append success or original failure
  -> Effect: append exact input/command/EffectId -> reconcile
       -> Pending: append nothing -> return EffectPending
       -> Operational failure: append original -> AwaitingRecovery
       -> Settled: bind -> append settlement -> AwaitingInterpretation -> interpret
  -> AwaitingRecovery: classify -> handle -> authorize/map -> append recovery decision
  -> advance, yield committed retry/restart, or return terminal success/FailureReport
```

Success advances linearly; a zero-State Program succeeds in admission. The exact original error
supplies intrinsic Classification. The selected handler requests Stop, RetryState or checkpoint
restart. Runtime checks finite State/run allowances, active checkpoints and Effect barriers before
committing the decision. Root maps apply only to terminal domain failure. Accepted retry/restart
yields; restart prunes later checkpoint inputs without resetting recovery usage.

Runtime associates the Program with one immutable assembly before admission or restore. It owns the
current continuation, transition rules and local validation. Journal owns only the exact opaque
canonical envelope; Store supplies admission/latest and an optional candidate row in one mechanical
snapshot. Neither Application nor Store reconstructs execution from frames. There is no semantic
history fold or parallel native-value cache.

Read callbacks and evidence binders receive the exact intent value ref. Effect callbacks receive
the exact command value ref used for EffectId derivation. Before unresolved Effect reconciliation,
Runtime re-prepares and compares the retained command. Accepted settlement is authoritative before
interpretation; a cold AwaitingInterpretation performs no adapter IO. Reading completed work invokes
no classifier, handler, root map or State interpretation.

After known insertion, Runtime adopts its candidate locally. NotInserted performs one bound
candidate probe. Presence can adopt the checked current observation; exclusion or absence remains
noninsertion. A failed load or projection is secondary, and an already established presence finding
survives it. These paths yield without executing another visit. Store errors return immediately
without probing: Unavailable is definitely noncommitted; Indeterminate is ambiguous acknowledgement.
The invocation retains the original when available, exact candidate once sealed, separate recording
cause and last observed view. Known insertion followed by projection failure retains the newer
acknowledged head independently.

Original failures precede policy commits. A handler, root-map or later encoding failure therefore
leaves AwaitingRecovery intact. Pending-Effect Stop ends the invocation and retains command authority;
explicit progress can reconcile it. Actual size/count limits may prevent recording an IO result.
They do not reserve future capacity or prevent all oversized results after provider entry.

Pure validation, callbacks and encoding use immediately awaited blocking work. Store and adapter IO
stay async. Cancellation can leave an in-flight physical append whose outcome was not acknowledged
to this caller, or a provider attempt whose result was not recorded. No internal fault or fallback
record is appended. A later selected-row snapshot observes retained state; it does not retroactively
prove delivery. Runtime has no background finalizer or implicit recovery loop.
