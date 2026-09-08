# mfm-journal

Journal owns the exact `mfm.run.frame.v3` canonical encoder and qualifier. Frames contain admission,
fused Pure/Read conclusions, or separate Effect prepare and adjacent conclusion. Every frame has
an exact sorted local object closure and a recursive head over its exact bytes. Old frame domains
are rejected; histories are never rewritten or migrated in place.

Pure/Read domain conclusions distinguish success from original failure plus retry, restart target,
or stop with mapped root failure. A Read retains its exact intent and either bound evidence/domain
outcome or the original operational error, State-owned context and recovery disposition. There is
no fabricated evidence or independent report frame. Effect prepare retains execution position,
EffectId and complete command. Its conclusion retains evidence and success or terminal failure;
its shape cannot encode retry/restart. A prepare may end a complete prefix.

`EncodedRunFrame` is sealed, `StoredRunBytes` is an opaque complete transfer, and `JournalHistory`
provides qualified borrowed records/objects, exact head, frame lengths and cumulative bytes.
Journal checks canonical bytes, references, closure, capacities and Effect adjacency. Runtime owns
Program association, EffectId derivation, State/visit agreement, policy safety, checkpoint activation,
budgets and barriers. Store owns physical complete-prefix loading and atomic exact-head append.

Format ceilings are 8 MiB per canonical object, 65,536 non-payload envelope bytes, 25,231,360 bytes
per frame, 65,536 frames and 512 MiB cumulative frame bytes. These are fixed format bounds.
Program supplies a conservative declared history budget and Runtime validates it at admission.
