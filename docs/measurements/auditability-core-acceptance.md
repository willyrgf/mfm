# Current-state core acceptance evidence

Current Part 1 correction packet, 2026-09-14. K1 is `8d783557`, K2 is `a1b8d088`; K3 is `5f6f3048`, K4 is `e94e6d4c`. Shipping measurements below use that exact production
candidate. Prior G1/F1 results are recorded below. Final Part 1 sign-off is reopened after architect review
confirmed a source-cycle identity bug and a producing Runtime acknowledgement coverage gap. The
user has approved the interface-pointer contract correction; renewed review and CI are pending. Part 2 enrichment is excluded.

## Current design and removals

K1 deletes Object/Runtime decoder seeds and container-consumption/error-precedence visitors.
Ordinary checked Object Deserialize and derived record decoding retain complete-input checks.
Malformed stored objects follow the explicit parser category/location/reason exception; direct
construction, selected native hooks and postdecode identity/size checks keep their separate facts.

K2 replaces RunCommit/RunState, Phase/OperationFacts and TerminalFailure with one RunRecord and
RecordedOperation. A private borrowed Continuation selector serves validation, dispatch and public
observation. The operation owns its cursor/input; Failure owns original and call facts once.
RecoveryOutcome retains a root only in Stop. Duplicate public cause trees, phase agreement,
yield_after and stopped incident/reason copies are deleted.

K3 deletes mfm-diagnostics, NativeCause/Captured/Owner and stored projectors, source/fact/omission
vocabulary, diagnostic quotas, Runtime/Values serializer mirrors, size-source downcasts,
ReturnedFailure/AdapterReturn/AppendFailure, HttpStatusCode reconstruction and generic reporting
failure owners. Values owns DiagnosticEvidence and InvocationDiagnostic. Existing size data moves
to Values; ObservedSize stays with the EVM owner. Two local source recipes select existing EVM/CLI
facts. Concrete Canonical/Runtime/Store errors retain their typed sources; heterogeneous boundaries
construct immutable invocation data once. Failed initial encoding reports unavailable original
information without retaining or reserializing it. Admitted originals share Object identity/bytes.

App borrows concrete errors; encoding returns one RawValue or JsonError. CLI has finite normal/final
presentation paths and renders text from typed data. REST names the existing first-party Canonical
JsonError through one direct dependency to retain body-channel encoding failure. No external
dependency was added. Necessary complexity is the trusted nested diagnostic profile, the explicit
first-encoding unavailable context and local terminal ownership/delivery paths.

K4 retains the first decoded Read input locally through preparation and moves it into interpretation.
It removes the second input decode without a Clone bound, Driver cache, new carrier or moved
admitted-intent/evidence binding. Existing Read success, operational/internal rejection, evidence
mismatch and cancellation tests exercise the actual runner.

## Cost

Count tracked plus untracked production Rust paths containing `src`, excluding `tests` components,
`tests.rs`, `*_tests.rs` and trailing inline test modules. Count nonblank lines excluding lines that
begin with comment markers after whitespace. Use the same convention for every reference.

| Reference | Production lines | Delta vs implementation | Delta vs original |
| --- | ---: | ---: | ---: |
| Original `7f71beef` | 26,831 | -1,435 | 0 |
| Implementation `5de114d0` | 28,266 | 0 | +1,435 |
| K1 `8d783557` | 27,975 | -291 | +1,144 |
| K2 `a1b8d088` | 27,746 | -520 | +915 |
| K3 `5f6f3048` | 27,018 | -1,248 | +187 |
| K4 `e94e6d4c` | 27,016 | -1,250 | +185 |
| Runtime follow-up `849337c2` | 27,020 | -1,246 | +189 |
| Approved pointer-repetition correction | 27,026 | -1,240 | +195 |

K3 is -728 against K2: diagnostics -778, Values -7, Canonical -42, Runtime +9, App -76,
binaries +59, Live EVM -25, domains +125, remaining callback signature changes +7. The domain
increase is explicit typed-error adaptation plus relocated ObservedSize. The binary increase is
finite delivery handling and direct typed text rendering. G1 accepted the complete cumulative
cost and responsibility removal as recorded below; the numbers alone are not acceptance. Tests/docs/churn are
reported separately on the final candidate.

## Focused verification

All direct commands use `nix develop -c`.

- `cargo check --workspace --all-targets` passes the complete API cutover.
- Canonical, Values, Program, Program derive and Capabilities `--all-targets` pass, including
  consuming compile-fail cases and the 32 MiB boundary.
- EVM, Portfolio and Live EVM `--all-targets` pass, including all 48 Live unit tests, transaction
  ambiguity/recovery and Portfolio checkpoint restart. Managed Effect e2e is reserved for CI.
- Runtime lib/current-state/pending/runtime-contract tests pass. All 19 Runtime contract cases
  pass again on K4, including Read evidence rejection and cancellation. The new first-encoding counter
  initially included a concurrent unrelated fixture; it now counts only the selected originals
  and the focused regression passes. The admitted-Object sharing/report-limit test passes.
- App unit and integration cases pass, including 11 shipping Portfolio Runtime cases. An obsolete
  append-error fixture was updated and its focused contract test passes.
- CLI binary tests (9) and REST binary tests (4) pass. Exact buffer reuse, unavailable normal report,
  failed final delivery, custom IO source cycles, primary status and known insertion are covered.
- Focused Clippy across Runtime, App, Live EVM, CLI and REST, all targets with no dependencies and
  `-D warnings`, passes. Rust is formatted and `git diff --check` passes.

## Shipping frames and loading

Measured 2026-09-14 through production Runtime `start`, `read`, `resume` and MemoryStore, using
Program v8, Journal frame v6 and FailureReport v5. A temporary Store wrapper retained successfully
inserted frames for inspection; it did not provide another execution/persistence model. The harness
and temporary dependency changes were removed. `nix develop -c cargo run -p mfm-app --example
measure_current` completed normally; its separate `limits` case also passed.

The [CSV](auditability-core-frames.csv) records every complete frame. Context bytes are inline
input/initial/output Objects; checkpoint bytes are saved contexts; audit-fact bytes are other inline
Objects, including the admission Program, requests/evidence/originals/root. Counts include each
serialized occurrence. Repeated occurrences are subsequent identical complete value references.
Metadata subtracts all canonical Object payload bytes from the exact complete frame; it includes
identities, record fields and Journal envelope. There is no inline deduplication or size prediction.

| Fixture | Frames | Admission bytes | Largest frame | Largest metadata | History bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| portfolio | 10 | 47,639 | 47,639 | 1,972 | 83,376 |
| portfolio_failure | 5 | 47,639 | 47,639 | 2,364 | 61,191 |
| anchored_success | 2 | 10,224 | 10,224 | 1,754 | 18,095 |
| anchored_failure | 3 | 10,224 | 10,895 | 2,164 | 28,793 |
| transaction | 11 | 26,550 | 26,550 | 1,944 | 75,107 |
| transaction_recovered | 13 | 26,550 | 26,550 | 1,944 | 81,288 |
| checkpoints_identical | 4 | 13,087 | 13,087 | 1,800 | 26,359 |
| checkpoints_distinct | 4 | 13,087 | 13,087 | 1,800 | 26,359 |
| retry_short | 4 | 10,226 | 10,226 | 1,754 | 26,909 |
| retry_long | 12 | 10,226 | 10,226 | 1,755 | 62,160 |

Portfolio uses shipping `plan_snapshot` with one native-balance source, PortfolioContinuation and
controlled adapters; failure returns rejected chain-identity evidence. Anchored Read uses checked
completed-call facts, the public `alpha` endpoint and rejected evidence for its domain failure.
Transaction uses EvmTransaction/CreateAt and a checked three-byte creation plan with real typed
reservation/preparation/settlement/completed contexts. Its recovered case returns AuthorityUnavailable
once, cold-inspects pending Stop without adapter reentry and explicitly resumes the same authority.
These measure existing owner facts, not additional Part 2 producer capture.

The two-active-checkpoint fixtures both retain two 656-byte contexts. The distinct case increments
an unrelated integer before checkpoint two; the control preserves it. At sequence 2 there are four
inline Objects and respectively two or three repeated occurrences. Both frames are 4,424 bytes,
including 1,312 context bytes, 1,312 checkpoint bytes and 1,800 metadata bytes. Equal widths do not
imply deduplication. The shipping Portfolio restart regression separately exercises authorized
restoration/pruning and the unchanged acknowledged prefix.

Terminal derived reports measure 1,462 bytes for Portfolio domain failure and 6,939 bytes for
anchored domain failure. Both are checked before terminal append. The existing real 16 MiB original
plus mapped root regression exceeds the separate 32 MiB report ceiling, leaves AwaitingRecovery
acknowledged and proves that reporting shares the admitted original's canonical allocation.

Normal Portfolio load returns two rows: 50,491 bytes for success and 51,873 for domain failure.
The same anchored Program/final output after one versus five retries has 4 versus 12 frames, but
normal loading still returns two rows: 18,097 versus 18,098 bytes. The one-byte difference is the
sequence width. This proves selected-row IO, not a latency ratio or historical semantic validation.

Single debug-profile native Object decode measurements: Portfolio 516 bytes in 9.1–9.2 ms,
anchored input 2,329 bytes in 105.7–108.6 ms, transaction input 656 bytes in 20.2–20.7 ms. These are
observations, not performance guarantees. K4 removes one native input decode from the Read runner;
no native cache or alternate resume path remains.

Actual cumulative refusal appended 31 real 16 MiB payload frames totaling 520,101,729 bytes.
The next 16,777,478-byte frame attempted 536,879,207 bytes against the unchanged 536,870,912-byte
limit and was rejected. A subsequent load retained the exact sequence, digest and history total.
Runtime's 65,536-frame regressions separately preserve an acknowledged original and an externally
returned but unrecordable settlement without inventing a new command or claiming settlement durability.

## Diff accounting

Physical line additions/deletions are distinct from production LOC; Rust source-file churn includes
inline test modules. Separate test paths, docs and configuration/fixtures are counted separately.
The exact K1–K4 source candidate `e94e6d4c` has no untracked implementation additions:

| Baseline | Rust source files + / - (files) | Separate tests + / - (files) | Docs + / - (files) | Config/fixtures + / - (files) | Total + / - (files) |
| --- | --- | --- | --- | --- | --- |
| `5de114d0` | 2,393 / 3,915 (47) | 1,427 / 1,999 (40) | 1,663 / 829 (21) | 5 / 50 (9) | 5,488 / 6,793 (117) |
| `7f71beef` | 5,842 / 6,365 (65) | 5,218 / 3,107 (49) | 3,314 / 2,309 (30) | 204 / 115 (20) | 14,578 / 11,896 (164) |

Evidence/document consolidation at `b5bde7df` adds 172 and deletes 253 physical lines across
four files, with no production-code change. F1 documentation reconciliation is likewise separate. The original K1–K4 cumulative +185
production lines (0.7%) includes its replacements; the two follow-ups add ten counted lines
(four test-only Runtime wiring lines and six local walker lines), reaching +195; no future Part 2 deletion offsets it. Compared
with the original, Journal loses lifecycle/fold responsibilities, Program loses capacity prediction,
and Runtime gains required continuation/recording/settlement phases while deleting historical
reconstruction. Compared with the implementation baseline, the actual redundant decoder, record,
error and reporting mechanisms above are removed. G1 accepted aggregate complexity improvement, without treating LOC as a quota.

## E1–E6 boundary evidence

| Case | Evidence |
| --- | --- |
| E1 | Runtime callback-error, current-state, pending and contract tests forward diagnostic fields through Pure/Read/Effect, handler and map without internal-fault append or classification. Resume follows committed authority. |
| E2 | State/domain originals commit before policy; EVM RPC tests retain distinct codes, ordered transport/IO/parser fields, RPC raw data spelling and trusted text through whole-owner admission, cold decode and FailureReport. Typed classification is unchanged. |
| E3 | Actual erased callbacks prove non-Error first original encoding runs once, retains known slot/contract/size and explicitly unavailable original. Admitted-original recording failures share Object bytes; Store/NotInserted retain submitted candidates and independent causes; projection acknowledgement is limited to known insertion. |
| E4 | Shipping App stored-data cases reject malformed reference/hash/oversize as restore/decode internal/500 with parser category/location/reason and no callback/append. Direct size remains 422; postdecode slot mismatch retains typed facts. |
| E5 | Shipping balance metadata.correlation empty and excessive-length constructors reach the actual Runtime Read callback and App with case/location/limit/observed length. |
| E6 | Workspace all-target compilation, consuming derives and transport tests cover the changed public APIs. K4's actual Read suite retains success, failure, mismatch and cancellation without a Clone bound or moved evidence binding. |

## Consuming requirements

| RFC | Current evidence and the guarantee it exercises |
| --- | --- |
| C1 | [Runtime contract](../../crates/kernel/runtime/tests/runtime_contract.rs) exercises hot/cold Pure, zero-State, Read and Effect runs. [Current wire](../../crates/kernel/runtime/src/state/tests.rs) exercises every current operation, failure, classification and recovery outcome alternative through canonical JSON and derived decoding. These wire cases make no semantic-history claim. |
| C2 | [Current validation](../../crates/kernel/runtime/tests/current_state/validation.rs) rejects inconsistent admission/latest, wrong Program/slot identity, stale Object digest and an out-of-range completed-call position before callbacks. [Collision](../../crates/kernel/runtime/tests/current_state/collision.rs) repeats exact start without driving and rejects different admission. [Store tests](../../crates/kernel/store/src/tests.rs) and managed PostgreSQL cover physical snapshot obligations; measured short/long loads return two normal rows. |
| C3 | Current validation rejects usage cardinality, local/global bounds, checked-sum overflow, undeclared checkpoints, invalid barriers and operation contracts. It deliberately accepts locally valid counter substitution, proving that restoration does not claim historical counter verification. [Pending validation](../../crates/kernel/runtime/tests/pending_failure/audit.rs) covers command/input/position and recovered Stop relations. |
| C4 | [Checkpoint restart](../../crates/kernel/runtime/tests/current_state.rs) restores full input, advances the visit, prunes later checkpoints and retains usage. [Shipping Portfolio restart](../../crates/domains/portfolio/tests/recovery_policy.rs) refreshes the actual anchored collection and retains its acknowledged prefix. |
| C5–C6 | Current-state tests commit the original before a failing policy, inspect it cold, and resume only unfinished recovery. [Terminal mapping](../../crates/kernel/runtime/tests/current_state/terminal.rs) retains the domain original when root mapping fails. [Pending audit](../../crates/kernel/runtime/tests/pending_failure/audit.rs) proves charged Retry, exhausted/requested Stop, and unchanged unresolved command authority. |
| C7–C8 | Current-state and Runtime contract tests preserve command/EffectId before adapter entry and commit settlement before an injected interpreter failure. Cold resume interprets retained evidence without adapter IO. Pending audit covers cancellation and ambiguous acknowledgement. |
| C9–C10 | [Callback errors](../../crates/kernel/runtime/tests/support/callback_errors.rs), Runtime preparation/evidence tests and [native constructors](../../crates/kernel/runtime/tests/current_state/constructors.rs) retain supplied operation/stage diagnostics with unchanged heads. Local rejection makes no provider call; post-response binding rejection preserves the fact that the provider was entered. |
| C11 | Recording tests retain admitted Failure/Object and exact submitted candidates without a Store-error probe. Size/task tests prove first encoding runs once, reports unavailable original detail/identity and leaves admission unchanged. Report overflow shares the admitted Object with the last observed view and leaves AwaitingRecovery durable. |
| C12 | Collision tests cover exact candidate, later commits, conflicting row, absent candidate, failed reload and a physically present candidate followed by an invalid latest Runtime payload. The physical finding survives the separate restoration failure; no case drives further work. |
| C13 | [Public Store scenarios](../../crates/kernel/store/tests/support/scenarios.rs), MemoryStore and managed PostgreSQL tests retain atomic exact-head append, complete selected snapshots, immutable prior bytes, cumulative bounds and COMMIT ambiguity. SQLx metadata is checked against the managed baseline. Additional SQLx source extraction remains outside Part 1. |
| C14 | [Shipping App constructor](../../crates/app/tests/support/native_constructor.rs) admits descriptor-valid EVM balance input and delivers distinct empty/oversized correlation constructor causes through the actual Read callback and App report. It also delivers a stale Object digest as restore/decode with parser category/location/reason, without provider entry or append. |
| C16 | [Actual capacity](../../crates/kernel/runtime/tests/current_state/capacity.rs) reaches the physical frame-count limit with an acknowledged original or an externally returned settlement and retains the unchanged current state. A separate 16 MiB original plus mapped root exceeds the derived report limit. The frame-count filler is opaque history, not claimed historical Runtime transitions. Measurements record a real Store cumulative-byte refusal. |
| C15 | Values diagnostic tests admit trusted text in whole owners while ordinary text, floats and actual bounds remain checked. EVM provider tests restore exact owner fields and classification; source recipes preserve ordered messages and stop before repeated interface pointers, permitting alias-prefix cause repetitions. No shared capture framework remains. |
| C17 | App, CLI and REST tests retain primary status and exact candidate/acknowledgement/head facts. CLI reuses exact encoded JSON or text after stdout failure; unavailable original reporting and final stderr failure terminate without retry. REST evidence ends at response-body handoff. |

## G1 disposition and final verification

The dedicated architect independently reviewed exact candidate
`b5bde7df74b3301795c393f1c44d011d4d57ddc7` and returned **ACCEPTED** on 2026-09-14. The review
inspected both baseline diffs and actual production paths, reproduced all three production LOC
totals, checked CSV arithmetic, and confirmed the K1–K4 physical removals and retained guarantees.
It accepts cumulative complexity improvement with +185 original-baseline production lines; it
uses no future Part 2 credit. No implementation failure or required deletion blocks acceptance.
The one F1 documentation correction, distinguishing deferred/excluded audit inventory from Part 1,
is applied with this record. The source candidate remains unchanged after G1.

The prior C18 disposition is reopened by the follow-up findings; the following CI result
applies to the earlier candidate. Final
`nix run .#ci` passed on exact candidate `4214dd39` with **9 passed, 0 failed**, in 635.99 seconds:
format, SQLx metadata, Clippy, workspace compilation/tests, rustdoc, managed PostgreSQL, client e2e
and Effect e2e. Run evidence is `run-3250562-1789400329617222583` under the local Nixfied state
root, with `artifacts/run-summary.json` and task logs. Production code is identical to the G1
candidate; subsequent completion-record changes are documentation only, checked by link/command
review and `git diff --check` under the docs-only verification rule.

The first CI attempt passed eight stages but could not compile the Effect test because the
filesystem was full (`No space left on device`). Removing only disposable `target/debug` artifacts
with pinned `cargo clean --target-dir target/debug` freed about 14 GiB. No source change was needed;
the complete CI rerun above passed, including the formerly blocked stage in 129.35 seconds.

## Follow-up correction: verification pending

The external architect reproduced dropped inline child causes in both local walkers: distinct
`Outer(Inner)` errors can share their data address. The new CLI and EVM regressions both failed
against the accepted implementation. Comparing full trait pointers preserves those children, but
an experiment also duplicated the existing CLI self-cycle layer because the same error can have
different vtables. This disproved exact concrete-object identity as the old implementation
implicitly assumed; the accepted correction describes observed interface pointers instead.

The dedicated architect withdrew the earlier walker acceptance: stable Error exposes no public
arbitrary dynamic identity, and Rust does not promise reliable concrete-object identity from trait
pointers. The recommended minimal refinement detects repeated full interface pointers and explicitly
permits alias-prefix repetitions. The user explicitly approved this narrower contract: each local
walker compares complete dyn Error pointers using std::ptr::eq, and source_cycle means precisely
“traversal stopped on a repeated interface pointer.” The correction adds no identity registry,
wrapper, message comparison or restored diagnostic budget. See [Rust pointer equality](https://doc.rust-lang.org/std/ptr/fn.eq.html).

The producing Runtime coverage gap is independently corrected in
[engine tests](../../crates/kernel/runtime/src/engine/tests.rs). One run-scoped, one-shot test-only
fault inside view construction exercises real Runtime start/resume with MemoryStore for admission,
continuation insertion, initial resume projection, Pending projection, recovered Retry and recovered
Stop. It checks exact mechanical acknowledgement only after insertion, the prior observation's
sequence/digest/state, unchanged head for no-insertion cases, cause forwarding and adapter entry.
Temporary false-resume-ack, false-Pending-ack and lost-prior-observation mutations each fail this
regression; the mutations were removed. All 15 Runtime library tests, including the six-case
regression, and focused Runtime Clippy pass.

Runtime release behavior and public APIs are unchanged. The repository LOC convention counts four
new cfg(test) wiring lines in engine.rs; all actual fault injection and scenario code lives in the
separate test module. This adds necessary producing-boundary evidence without a production hook,
callback parameter, cache or alternate projection implementation. Full candidate CI and renewed
contract review remain pending for the corrected candidate.

The corrected source regressions cover distinct and equal-message inline children, complete ordered
40-layer acyclic chains, self-cycles and a multi-node cycle. Cycle checks require termination and the
repetition marker without an exact visit count across compiler configurations. The narrowed contract
and its information limit are documented in RFC section 5.3, design, adapter audit and both owner
READMEs. Focused debug and release source tests both pass (three CLI and four Live EVM cases each), using
`nix develop -c cargo test --target-dir target/verification [--release] -p mfm-evm-live -p mfm
--lib --bins source`. Focused Clippy for both owners, including tests, passes with `--no-deps --
-D warnings`. Renewed review and final CI are pending. The complete-pointer comparisons replace
the old thin-address comparisons locally: +3 non-comment production lines in each walker, no new
public type, dependency, shared mechanism or diagnostic quota. Shipping frame measurements retain
the same schemas/data and are unaffected by this source-pointer comparison correction.

## Material uncertainties

None concerning the user-approved pointer-repetition contract. Concrete-object identity and exact
cyclic visit counts remain outside that guarantee. Renewed architect review and full CI are pending.
