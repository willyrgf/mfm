# Capacity envelope and reservation proof

This is the checked-in U1/U7/U9 artifact. Values are fixed contract bounds, not latency targets.
Exact-bound and independent bound-plus-one tests are required at the owner named in each row.

## Fixed outer limits

| Resource | Exact bound | Owner and evidence |
| --- | ---: | --- |
| Canonical frame | 33,554,432 bytes | Journal `MAX_FRAME_BYTES`, `RunFrame::validate` |
| Run frames | 65,536 | Journal `MAX_RUN_FRAMES`, `QualifiedRun::new` |
| Reachable objects | 1,048,576 | Journal `MAX_RUN_OBJECTS`, Store qualification |
| Canonical frame bytes per run | 536,870,912 bytes | Journal `MAX_RUN_FRAME_BYTES`, Store reservation |
| Portfolio collections | 64 | `PORTFOLIO_COLLECTION_LIMIT`, domain constructor |
| Portfolio total EVM sources | 64 | `EVM_BALANCE_SOURCE_LIMIT`, Portfolio constructor |
| Configuration revisions | 1,024 | Journal limit, Memory/PostgreSQL writer |
| Configuration stream bytes | 67,108,864 bytes | Journal limit, Memory/PostgreSQL writer |
| One configuration revision | 16,777,216 bytes | Configuration revision constructor |
| Read attempts | 3 total | `ExecutionMode::Read`, `PreparationMode::Read` |
| Absorbing Effect attempts | 3 total | Sealed Effect mode; unproven Effects are `EntryOnce` |
| EntryOnce attempts | 1 total | `PreparationMode::Effect { absorbing: false }` |

## Reservation law

`StatePrepared::maximum_conclusion_bytes` is fixed by the State declaration and must be no larger
than one frame. `QualifiedRun::reserved_conclusion_bytes` sums only unresolved selected
preparations; a replacement transfers the one liability and a conclusion removes it. Store checks
the candidate frame, complete object closure, facts, publication coordinate, and the retained
liability before direct-new preparation or conclusion ownership is returned. The Memory and
PostgreSQL paths both reject a candidate that exceeds the same outer run ceiling.

No deduplication, checkpoint, structural sharing, suffix cache, parallel lane, or collection
optimization is part of this envelope. Complete cumulative contexts may therefore retain `O(n^2)`
canonical bytes, bounded by the run ceiling.

## Maximum fixtures and exact/+1 checks

The current production fixture families are the two builders in `crates/app/src/lib.rs`: one EVM
submission with four sequential State declarations and one Portfolio plan that expands each
declared collection and source into explicit State/Match declarations. The Store tests cover
selected-preparation reservations, direct-new preparation identity, conclusion capacity, dense
fact coordinates, and the 64-source Portfolio bound. The Journal and domain constructors enforce
the exact fixed bounds before allocation of a retained owner.

The final Runtime/Store conformance harness must add the measured hot advancement and complete
cold-prefix resume high-water for the maximum EVM and Portfolio fixtures. These measurements have
no pass/fail latency threshold; they only characterize the admitted envelope. Until that harness
lands, no cold-resume performance claim is made.
