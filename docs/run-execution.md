# Run execution

One run is identified by an explicit caller-supplied RunId and one checked Program whose initial
value ref commits to the exact C0 supplied at admission.

```text
start(RunId, Program, C0)
  -> append RunAdmitted(Program, C0)
  -> fold exact qualified prefix
  -> Pure: evaluate input -> append domain conclusion and recovery decision
  -> Read: prepare intent -> await typed evidence/error -> append fused conclusion and decision
  -> Effect: append exact command/value ref/EffectId -> await Pending or Settled evidence
       -> Pending: append nothing -> return EffectPending
       -> Operational error: append original/context/Retry-or-Stop -> yield or stop invocation
       -> Settled: bind evidence -> interpret -> append adjacent conclusion and decision
  -> advance, yield committed Retry/Restart, or return terminal success/FailureReport
```

Program contains a linear State sequence. Every selected occurrence has a checked State position
and monotonic visit identity. Success advances; a zero-State Program succeeds at genesis with C0.
The exact original error implements intrinsic ClassifyError semantics. One selected handler
consumes its common summary and requests Stop, RetryState, or a typed checkpoint restart. Runtime checks finite
State/run allowances, active checkpoints, and retained Effect barriers before committing a decision.
Only Stop applies the declared root failure maps. Accepted recovery yields to the caller.

Runtime associates the entire Program with one immutable RuntimeAssembly before admission or fold.
Association checks exact codecs, State implementations, mode-specific capability contracts, policy
parameters, and maps. Runtime owns the only semantic fold; Application and Store do not inspect
frames to derive state.

Read callbacks and hot/cold evidence binders receive the exact qualified intent value ref. Effect
callbacks receive the exact qualified command value ref used in EffectId derivation. A value ref
identifies one canonical instance; a codec contract ref identifies its schema.

After an inserted frame, Runtime extends its private hot accumulator without loading. NotInserted
triggers one complete reload and returns the winning view without executing another visit. Cold
resume/read load once, ask Journal to qualify the complete prefix, and use the same fold. Read
performs no adapter IO and appends nothing. Cold qualification of retained Effect prepare repeats
its deterministic State preparation to compare the exact command and derived identity.

Pure evaluation, qualification, and encoding use immediately awaited blocking jobs. Store and adapter
IO stay on the async driver. Pending Effect settlement never appends a second prepare. An operational
pending error appends its original cause/context and Retry/Stop before acknowledgment, preserving
command authority and visit. Every failure consumes admitted record capacity, including Stop.
Exhaustion prevents further provider entry. An invariant violation is Internal and appends nothing.
Dropping at any await is safe: either no candidate was submitted or one in-flight Store append may
commit atomically, and a later complete reload determines the result.

Store Unavailable is definitely noncommitted; append Indeterminate means COMMIT acknowledgement was
ambiguous. InvocationFailure preserves the mechanical source and last observed view when available.
Runtime retains no background finalizer, semaphore, timeout policy, or cancellation token.
