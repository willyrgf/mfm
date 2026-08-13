# mfm-runtime

Runtime owns immutable live assembly, affine sessions, direct-new `CommittedCall`, typed handoff,
the Runtime-owned affine `PendingConclusion`, and the exhaustive `SuspendedRun` owner-fate
coordinator around Store. Provider entry requires a consuming call token. Access registration
retains the immutable binding descriptor, and conclusion recovery retains no provider or callback
authority. Integrity-blocked Access outcomes use the declared static failure contract and do not
invoke State interpretation.

Runtime receives only one exact non-Clone `QualifiedHistoryPort` and never returns it or an opened
Store. `RunSession` retains exactly one affine `SelectedRun` plus the latest catalog-qualified typed
value. Cold resume selects the full prefix once; direct admission, preparation, and conclusion
consume and advance that owner without a backend reload. Callback-free `QualifiedRun` evidence is
available only after giving up selected mutation authority.

Assembly finalization checks every State input/output/failure and Access intent/evidence type
against Program's exact finalized catalog associations before any session or provider authority
exists. Runtime's private `TypeId` values correlate live implementations only; cold reification
uses catalog qualification and cannot accept a schema-only match.

Access preparation projects only the canonical fact request. Store fixes any prior-fact selection
from the admitted source manifest and a bounded publication frontier before minting the direct-new
typed continuation; Runtime and State code cannot submit or replace that selection. State success
outcomes carry coordinate-free fact proposals, and pending conclusion recovery can rebind only the
Store-assigned publication coordinate when the independent fact frontier moves; Store rotates the
physical append identity for that rebind. Same-run conclusion races return qualified history,
`NoLongerSelected`, conflict, or invalid-history classifications without re-entering State or an
adapter. Permanent Store rejection remains an owner-bearing `ConclusionRejected` result until an
explicit owner boundary discards it.

Each Runtime opening also owns bounded active-session, deterministic CPU, planning, and provider
ingress permits. Pure evaluation, access preparation, retained-prefix qualification, and typed
reification run as bounded blocking jobs; provider futures remain attached to their affine owner.
These permits provide backpressure without a scheduler, per-run lock, or process-wide writer lease.
