# mfm-journal

Journal owns the exact `mfm.run.frame.v4` canonical encoder and qualifier. Frames contain admission,
fused Pure/Read conclusions, or an Effect prepare, zero or more operational failure records, and
settlement conclusion. Every frame has
an exact sorted local object closure and a recursive head over its exact bytes. Old frame domains
are rejected; histories are never rewritten or migrated in place.

Pure/Read domain conclusions distinguish success from original failure plus retry, restart target,
or stop with mapped root failure. A Read retains its exact intent and either bound evidence/domain
outcome or the original operational error, State-owned context and recovery disposition. There is
no fabricated evidence or independent report frame. Effect prepare retains execution position,
EffectId and complete command. Its conclusion retains evidence and success or terminal failure;
its shape cannot encode retry/restart. EffectAdapterFailed retains execution position, original
cause, State context and PendingDecision (Retry or Stop; never Restart). Prepare and each failure
may end a complete prefix. No unrelated record intervenes and failures cannot follow settlement.

`EncodedRunFrame` is sealed, `StoredRunBytes` is an opaque complete transfer, and `JournalHistory`
provides qualified borrowed records/objects, exact head, frame lengths and cumulative bytes.
Journal checks canonical bytes, references, closure, capacities and Effect adjacency. Runtime owns
Program association, EffectId derivation, State/visit agreement, policy safety, checkpoint activation,
budgets and barriers. Store owns physical complete-prefix loading and atomic exact-head append.

Format ceilings are 32 MiB per canonical object, 65,536 non-payload envelope bytes, 134,283,264 bytes
per frame, 65,536 frames and 512 MiB cumulative frame bytes. These are fixed format bounds.
Program supplies a conservative declared history budget and Runtime validates it at admission.

The frame ceiling covers four maximum-sized objects (Read intent, evidence, original failure and
mapped failure) plus metadata. All payloads belong to the same atomic frame; they are not separate
appends. The generic canonical parser has a larger syntax ceiling. PostgreSQL provisioning uses
run-history baseline v2 for this capacity contract and rejects the old baseline.
