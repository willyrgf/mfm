# Current-state core acceptance evidence

Current Part 1 correction packet, 2026-09-14. K1 is committed at `8d783557`; the K2 candidate builds
on that commit. This is not G1 acceptance or F1 final CI. The RFC's K3/K4 requirements remain open;
upstream producer enrichment assigned to Part 2 does not block or supply credit for Part 1.

## Current design and deletion evidence

K1 deletes Object/Runtime decoder seeds, drain/error-precedence visitors and their nested result.
Object Deserialize calls the checked constructor; Runtime derives decoding with complete input
consumption. Malformed stored Objects retain parser category/location/reason under the RFC's
explicit exception. Direct admission and selected native hooks retain their separate fields.

K2 replaces `RunCommit`, `RunState`, `Phase` and `OperationFacts` with `RunRecord` and
`RecordedOperation`. The operation is the only stored cursor/input/phase authority. One private
borrowed `Continuation` selector serves dispatch, validation and public projection; it is neither
stored nor cached. The dedicated architect's target review selected this representation and
confirmed validation order, checkpoint entry, stop/root relations and the one-commit cutover.

K2 also deletes `TerminalFailure`, `DomainFailure`, `ReadFailure`, `PendingFailureView`, owned
`AdapterIncidentView`/`FailureCauseView`, phase agreement checks, phase payload accounting,
`yield_after`, and duplicate `RecoveryStopped` incident/reason fields. `FailureReport` owns Failure
and optional root, with borrowing accessors. Pending views own EffectCall and optional original/
outcome. A small App borrowing decision serializer preserves the existing public wire while
`RecoveryOutcome` retains a domain root only in the stored Stop. Native original custody and its
recording argument remain until K3's inseparable replacement.

## Consuming correction proofs

- [Values](../../crates/kernel/values/tests/value_contract.rs) rejects malformed references, stale
  hashes, noncanonical/oversized Objects, duplicate/unknown fields and trailing input.
- [Current wire](../../crates/kernel/runtime/src/state/tests.rs) round-trips all operation/failure/
  outcome alternatives, accepts ordinary Serde sequence forms, rejects obsolete state/facts wire,
  and compares metadata payload accounting with actual serialized inline Object occurrences.
- [Current validation](../../crates/kernel/runtime/tests/current_state/validation.rs) retains
  structured postdecode identities, parser diagnostics and local usage/checkpoint/barrier checks.
  Locally valid substituted outputs are accepted, without claiming historical verification.
- [Terminal regression](../../crates/kernel/runtime/tests/current_state/terminal.rs) preserves
  typed original/root inspection and rejects absent or wrongly typed stored domain roots before
  callbacks or append; pending Stop rejects any root. Report overflow leaves the acknowledged original awaiting recovery.
- [Pending recovery](../../crates/kernel/runtime/tests/pending_failure/audit.rs) preserves fresh
  Read visits, unchanged pending command authority, grant accounting, stopped recovery and exact
  candidate reconciliation. A schema-valid input substitution reaches typed preparation, where
  mismatch with the retained command rejects it without provider entry.
- [Shipping Application](../../crates/app/tests/support/native_constructor.rs) exercises malformed
  references, bad hashes and oversized nested Objects through real Runtime/App loading, with
  internal category and no provider call/append. Postdecode slot errors and both metadata native
  constructor cases retain their separate fields. [Pending JSON](../../crates/app/tests/pending_view.rs)
  preserves original/input/command, decision wire, RecoveryStopped and its observed head.
- [REST](../../bin/rest-api/src/server/reporting_tests.rs) distinguishes nested stored-size 500
  from direct Object construction-size 422 with measured fields.

A temporary numeric-only measurement in the real failed-Read/handler-error/resume test recorded:

| Sequence / operation | Inline Objects | Repeated occurrences | Canonical payload bytes | Metadata bytes | Complete frame bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 / admitted | 2 | 0 | 9,758 | 1,076 | 10,834 |
| 3 / failed Read | 3 | 0 | 89 | 1,464 | 1,553 |
| 4 / recovered Retry | 3 | 0 | 89 | 1,550 | 1,639 |

These are actual Journal frames from Runtime and MemoryStore, not estimated successor records.
The failure appears once with input/intent; recovery retains those same three Objects. The temporary
instrumentation was removed. Full shipping portfolio/anchored/transaction/checkpoint/load measurements
must be refreshed on the complete K3/K4 candidate; the earlier CSV is historical evidence only.

## Cost convention and current delta

Count tracked plus untracked `*.rs` paths containing `src`, excluding `tests` path components,
`tests.rs`, `*_tests.rs`, and trailing `#[cfg(test)] mod ... { ... }`. Count nonblank lines excluding
lines beginning with `//`, `/*`, `*` or `*/` after whitespace. Apply that same convention to all
three references; earlier manually excluded measurement totals are not mixed into this comparison.

| Reference | Production Rust lines | Delta against implementation baseline |
| --- | ---: | ---: |
| Original `7f71beef` | 26,831 | -1,435 |
| Implementation `5de114d0` | 28,266 | 0 |
| K1 `8d783557` | 27,975 | -291 |
| Current K2 candidate | 27,746 | -520 |

K2 is **-229** against K1: engine -139, state -40, report -48, public Runtime payloads -6, App
borrowing serializers +4. Current cumulative change against the original is **+915**; this remains
an unfinished Part 1, not an accepted cumulative simplification. Necessary added complexity is the
shared borrowed selector and root/outcome relationship, replacing duplicate owned continuation and
failure models. Tests/docs/churn are separate from production counts.

## Focused verification

All commands below use `nix develop -c`. K1's Values contract evidence remains applicable; K2
changes Runtime and its consumers, not Values construction.

- `cargo test -p mfm-runtime --all-targets`: library/current-state passed; the competing-pending
  test exposed an obsolete duplicate-custody assertion. After correction, the complete integration
  targets passed with `cargo test -p mfm-runtime --test pending_failure --test runtime_contract
  --test private_boundaries --test current_state`.
- `cargo test -p mfm-runtime --lib state::tests`: all current wire/accounting cases pass.
- `cargo test -p mfm-app --all-targets`: passes, including shipping constructor and pending JSON.
- `cargo test -p mfm -p mfm-rest-api --all-targets`: passes; managed client e2e remains ignored
  outside its Nixfied fixture.

- `cargo test -p mfm-portfolio -p mfm-evm-live --all-targets`: passes, including the complete
  transaction append-ambiguity matrix, transaction recovery and shipping Portfolio policies.
  Managed Effect e2e remains ignored outside its Nixfied fixture.
- `cargo test -p mfm-runtime --test pending_failure current_record_validates`: the added
  pending-Stop root rejection passes without provider entry.
- `cargo clippy -p mfm-runtime -p mfm-app --all-targets --no-deps -- -D warnings`: passes.
  The initial dependency-inclusive attempt exposed an existing `InvalidBase` enum-name lint in
  live/evm/error.rs, outside K2; that source adapter is part of K3's existing conversion cutover.
- `cargo check --workspace --all-targets`: passes. Changed Rust files are formatted in the pinned
  shell and `git diff --check` passes.

The public owned RunViewState and temporary borrowing Cause serializer keep local large-enum lint
allowances rather than adding allocations or restoring deleted boxed cause trees. Final CI remains
reserved for the complete Part 1 candidate after K3, K4 and G1. The retained evidence below records earlier
behavior and is not independent completion evidence for those open requirements.

## Consuming requirements

| RFC | Current evidence and the guarantee it exercises |
| --- | --- |
| C1 | [Runtime contract](../../crates/kernel/runtime/tests/runtime_contract.rs) exercises hot/cold Pure, zero-State, Read and Effect runs. [Current wire](../../crates/kernel/runtime/src/state/tests.rs) exercises every current operation, failure, classification and recovery outcome alternative through canonical JSON and derived decoding. These wire cases make no semantic-history claim. |
| C2 | [Current validation](../../crates/kernel/runtime/tests/current_state/validation.rs) rejects inconsistent admission/latest, wrong Program/slot identity, stale Object digest and an out-of-range completed-call position before callbacks. [Collision](../../crates/kernel/runtime/tests/current_state/collision.rs) repeats exact start without driving and rejects different admission. [Store tests](../../crates/kernel/store/src/tests.rs) and managed PostgreSQL cover physical snapshot obligations; measured short/long loads return two normal rows. |
| C3 | Current validation rejects usage cardinality, local/global bounds, checked-sum overflow, undeclared checkpoints, invalid barriers and operation contracts. It deliberately accepts locally valid counter substitution, proving that restoration does not claim historical counter verification. [Pending validation](../../crates/kernel/runtime/tests/pending_failure/audit.rs) covers command/input/position and recovered Stop relations. |
| C4 | [Checkpoint restart](../../crates/kernel/runtime/tests/current_state.rs) restores full input, advances the visit, prunes later checkpoints and retains usage. [Shipping Portfolio restart](../../crates/domains/portfolio/tests/recovery_policy.rs) refreshes the actual anchored collection and retains its acknowledged prefix. |
| C5–C6 | Current-state tests commit the original before a failing policy, inspect it cold, and resume only unfinished recovery. [Terminal mapping](../../crates/kernel/runtime/tests/current_state/terminal.rs) retains the domain original when root mapping fails. [Pending audit](../../crates/kernel/runtime/tests/pending_failure/audit.rs) proves charged Retry, exhausted/requested Stop, and unchanged unresolved command authority. |
| C7–C8 | Current-state and Runtime contract tests preserve command/EffectId before adapter entry and commit settlement before an injected interpreter failure. Cold resume interprets retained evidence without adapter IO. Pending audit covers cancellation and ambiguous acknowledgement. |
| C9–C10 | [Callback errors](../../crates/kernel/runtime/tests/support/callback_errors.rs), Runtime preparation/evidence tests and [native constructors](../../crates/kernel/runtime/tests/current_state/constructors.rs) retain native operation/stage causes with unchanged heads. Local rejection makes no provider call; post-response binding rejection preserves the fact that the provider was entered. |
| C11 | Current-state recording tests retain the native original and sealed candidate on Store failure without probing. [Size/task custody](../../crates/kernel/runtime/tests/current_state/sizes.rs) drives an oversized original and a panic in failure encoding through actual erased adapter callbacks: no candidate, no classification, unchanged admission and original retained in native custody. [App reporting](../../crates/app/src/reporting_tests.rs) preserves this custody through AppendIndeterminate and secondary projection failure. An unwinding native projector returns a separate reviewed failure with explicit payload withholding; incomplete reporting retains original/head custody without retrying it. |
| C12 | Collision tests cover exact candidate, later commits, conflicting row, absent candidate, failed reload and a physically present candidate followed by an invalid latest Runtime payload. The physical finding survives the separate restoration failure; no case drives further work. |
| C13 | [Public Store scenarios](../../crates/kernel/store/tests/support/scenarios.rs), MemoryStore and managed PostgreSQL tests retain atomic exact-head append, complete selected snapshots, immutable prior bytes, cumulative bounds and COMMIT ambiguity. SQLx metadata is checked against the managed baseline. Native SQLx source extraction remains step 4. |
| C14 | [Shipping App constructor](../../crates/app/tests/support/native_constructor.rs) admits descriptor-valid EVM balance input and delivers distinct empty/oversized correlation constructor causes through the actual Read callback and App report. It also delivers a stale Object digest as restore/decode with parser category/location/reason, without provider entry or append. |
| C16 | [Actual capacity](../../crates/kernel/runtime/tests/current_state/capacity.rs) reaches the physical frame-count limit with an acknowledged original or an externally returned settlement and retains the unchanged current state. A separate 16 MiB original plus mapped root exceeds the derived report limit. The frame-count filler is opaque history, not claimed historical Runtime transitions. Measurements record a real Store cumulative-byte refusal. |
| C17 | App, [CLI](../../bin/cli/src/reporting_tests.rs) and [REST](../../bin/rest-api/src/server/reporting_tests.rs) exercise retained current results, exact primary status, original/candidate custody, explicit acknowledgement, bounded omissions and failed preparation/delivery. Failed incomplete reporting retains both errors without recursively retrying projection. REST evidence ends at response-body handoff, not socket delivery. The managed client scenario exercises the shipping REST/PostgreSQL/CLI continuation. |

## Verification scope

All direct Cargo commands use `nix develop -c`. The core has focused evidence from:

- Runtime `--all-targets`, including private-API compile failures, current-state, pending and contract
  tests; added position-order, wire and encoding-task cases have their focused regressions.
- Values Object admission and original-projection tests, kernel owner tests, and public Store tests.
- App `--all-targets`, CLI and REST reporting tests; the shipping native-constructor and Portfolio
  recovery consumers were repeated after replacing Object's derived decoding route.
- `cargo check --workspace --all-targets` and focused Clippy after the native seed cutover.
- Managed `sqlx-check`, `postgres-test`, `client-e2e` and `effect-e2e` from this worktree. Client e2e
  completed in 141.78 seconds; Effect e2e completed in 152.27 seconds. Their fixtures exercise the
  shipping transports, PostgreSQL continuation, transaction authority and live Reth interaction.

Final `nix run .#ci` is reserved for the complete implementation after K3/K4 and G1 acceptance.

## Material uncertainties

None within the core boundary. Projection-panic handling covers unwinding callbacks, not aborts
or disclosure by arbitrary process panic hooks. K3/K4, refreshed shipping measurements, independent G1 acceptance and final F1 CI remain required
and are not claimed by this evidence.
