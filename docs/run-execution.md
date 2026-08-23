# Run execution

One run is identified by an explicit caller-supplied RunId and one checked Program.

```text
start(RunId, Program, C0)
  -> append RunAdmitted(Program, C0)
  -> fold exact qualified prefix
  -> Pure: evaluate input -> append outcome
  -> Read: prepare intent -> await typed adapter evidence -> interpret -> append fused conclusion
  -> Effect: append exact command/EffectId -> await Pending or Settled evidence
       -> Pending: append nothing -> return Runnable
       -> Settled: bind evidence -> interpret -> append adjacent conclusion
  -> Match: project closed-sum tag/payload without an append
  -> exact root success or exact root failure
```

Program index zero is the root. Only forward successor indices are valid. Each conclusion determines
the next declaration from its branch. A missing successor terminates at that branch's declared root
contract; `Never` cannot terminate or have a failure successor. A zero-State Program succeeds at
genesis with C0.

Runtime associates the entire Program with one immutable RuntimeAssembly before admission or fold.
Association checks exact codecs, State implementations, Read capability/binding callbacks, and
closed Match projection contracts. Runtime then owns the only semantic fold. Neither Application nor
Store inspects frames to derive state.

After an inserted frame, Runtime extends its private hot accumulator without loading. A
`NotInserted` result triggers one complete reload because another writer may have advanced the run.
Cold resume/read always load once, ask Journal to qualify the complete prefix, and use the same fold.
`read` performs no adapter IO and appends nothing. Cold qualification of a retained Effect prepare
deterministically invokes its `EffectState::prepare` implementation to compare the exact command and
derived identity.

Pure evaluation and typed encoding are pure blocking jobs that are immediately awaited. A Read is
entered at most once in each start/resume call. An Effect adapter is also entered at most once per
invocation: `Pending` returns a Runnable view at the existing prepare, while `Unavailable` or
`Internal` returns the corresponding error. All three append no Effect conclusion.
Dropping at any await is safe: either no candidate was submitted, or the one in-flight Store append
may commit atomically and a later complete reload determines the result.

Store `Unavailable` is definitely noncommitted; append `Indeterminate` means COMMIT acknowledgement
was ambiguous. Runtime exposes that ambiguity and retains no pending owner, background finalizer,
semaphore, timeout policy, or cancellation token.
