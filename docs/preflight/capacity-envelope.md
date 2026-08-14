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
| One configuration revision | 16,777,216 bytes | typed Store configuration ingress |
| Read attempts | 3 total | `ExecutionMode::Read`, `PreparationMode::Read` |
| Effect entries | 1 total | `ExecutionMode::Effect`, `PreparationMode::Effect` |

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

The current production fixture families use the domain-owned EVM and Portfolio planners through
`crates/app/tests/capacity_envelope.rs`: one EVM submission with its receipt/finality suffix and
one Portfolio plan that expands each declared collection and source into explicit State/Match
declarations. The executable task `capacity-envelope` runs the maximum App, Runtime, and
configuration fixtures with `--nocapture`.

| Measurement | EVM submission | Portfolio snapshot |
| --- | ---: | ---: |
| Program declarations | 7 | 770 |
| canonical Program bytes | 17,173 | 1,597,453 |
| maximum `C0` canonical bytes | 262,594 | 26,744 |
| maximum recorded `Cn` canonical bytes | 262,787 | 44,168 |
| entry fixture expansion | seven sequential States | 64 collections / 64 total sources |

The EVM fixture uses the exact 128 KiB transaction-data bound, performs the durable
nonce-to-candidate handoff, finishes both Programs, and decodes each finished canonical document
through `ProgramIngress`. The Portfolio fixture constructs all 64 completed collection results
before constructing its final continuation, so the `Cn` measurement covers retained cumulative
results rather than only the empty continuation.

The Runtime fixture `pure_session_advances_through_runtime_and_store` advances the first repeated
semantic State in a retained session to head 2, drops that owner, then cold-resumes the persisted
Program and drives the second occurrence to head 3. It counts exactly two State entries, proving
that cold resume neither rebuilds the Program nor re-executes the concluded occurrence. No
executor, cache, checkpoint, suffix protocol, or structural sharing is retained.

Exact-bound and independent bound-plus-one evidence is executable at each owner:

| Contract | Exact/+1 test |
| --- | --- |
| frame, run-frame count, reachable-object count, cumulative run-frame bytes | `mfm_store::backend::tests::outer_capacity_accepts_exact_ceiling_and_rejects_each_plus_one` |
| Read attempts, one-entry Effect, State conclusion reservation | `mfm_program::single_trust::tests::attempt_and_conclusion_capacity_bounds_accept_exact_and_reject_plus_one` and `mfm_capabilities::single_trust::tests::access_modes_enforce_their_exact_attempt_bounds` |
| EVM transaction data | `mfm_evm::tests::transaction_data_capacity_accepts_exact_and_rejects_plus_one` |
| 64-source Portfolio ceiling and independent +1 | `mfm_portfolio::tests::portfolio_total_source_bound_is_exact` |
| configuration revisions, one revision, and cumulative stream | `mfm_store::backend::tests::configuration_capacity_accepts_each_exact_bound_and_rejects_plus_one` |
| selected-preparation liability transfer and conclusion discharge | `mfm_store::backend::tests::conclusion_head_race_classifies_same_and_conflicting_semantics` and the Store reduction/capacity checks in `single_trust.rs` |

The configuration fixture accepts exactly 1,024 small revisions, exactly one 16 MiB canonical
revision, and exactly four such revisions totaling 64 MiB. It independently rejects the 1,025th
revision, a 16 MiB-plus-one revision, and any successor after the 64 MiB stream is full. The
conclusion reservation is carried by the selected preparation, transferred by replacement, and
removed only by its matching conclusion; a superseded conclusion never releases or duplicates the
liability.

The shared Memory/PostgreSQL backend conformance covers local exact-head races, and the managed
PostgreSQL lane starts two independent child processes behind an explicit filesystem rendezvous
before each admission, preparation, replacement, and conclusion append. It requires exactly one
`NewlyCommitted` and one `StaleHead` result per stage, then validates the complete seeded prefix and
winning record. This is a mechanical CAS proof; Runtime provider-entry counts remain in Runtime
tests, where only a direct-new preparation winner can reach a provider.
