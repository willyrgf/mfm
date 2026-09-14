# Current-state core acceptance evidence

Current Part 1 correction packet, 2026-09-14. K1 is `8d783557`, K2 is `a1b8d088`; K3 is the
current candidate. K4, refreshed shipping measurements, G1 acceptance and F1 final CI remain due.
This packet does not claim Part 1 acceptance. Part 2 enrichment is excluded.

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
| K3 candidate | 27,018 | -1,248 | +187 |

K3 is -728 against K2: diagnostics -778, Values -7, Canonical -42, Runtime +9, App -76,
binaries +59, Live EVM -25, domains +125, remaining callback signature changes +7. The domain
increase is explicit typed-error adaptation plus relocated ObservedSize. The binary increase is
finite delivery handling and direct typed text rendering. G1 must assess the complete cumulative
cost and responsibility removal; these numbers alone are not acceptance. Tests/docs/churn are
reported separately on the final candidate.

## Focused verification

All direct commands use `nix develop -c`.

- `cargo check --workspace --all-targets` passes the complete API cutover.
- Canonical, Values, Program, Program derive and Capabilities `--all-targets` pass, including
  consuming compile-fail cases and the 32 MiB boundary.
- EVM, Portfolio and Live EVM `--all-targets` pass, including all 48 Live unit tests, transaction
  ambiguity/recovery and Portfolio checkpoint restart. Managed Effect e2e is reserved for CI.
- Runtime lib/current-state/pending/runtime-contract tests pass. The new first-encoding counter
  initially included a concurrent unrelated fixture; it now counts only the selected originals
  and the focused regression passes. The admitted-Object sharing/report-limit test passes.
- App unit and integration cases pass, including 11 shipping Portfolio Runtime cases. An obsolete
  append-error fixture was updated and its focused contract test passes.
- CLI binary tests (9) and REST binary tests (4) pass. Exact buffer reuse, unavailable normal report,
  failed final delivery, custom IO source cycles, primary status and known insertion are covered.
- Focused Clippy across Runtime, App, Live EVM, CLI and REST, all targets with no dependencies and
  `-D warnings`, passes. Rust is formatted and `git diff --check` passes.

## Measurements still required

The historical shipping CSV predates K1–K3 and is not current acceptance evidence. Refresh actual
Portfolio/anchored/transaction/checkpoint frames, report/input bytes, short/long load rows/bytes and
cumulative-limit refusal after K4, using the existing temporary consuming harness. No estimate or
future Part 2 deletion counts toward acceptance.

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
| C15 | Values diagnostic tests admit trusted text in whole owners while ordinary text, floats and actual bounds remain checked. EVM provider tests restore exact owner fields and classification; source recipes preserve ordered messages and stop before cyclic duplicates. No shared capture framework remains. |
| C17 | App, CLI and REST tests retain primary status and exact candidate/acknowledgement/head facts. CLI reuses exact encoded JSON or text after stdout failure; unavailable original reporting and final stderr failure terminate without retry. REST evidence ends at response-body handoff. |

## Material uncertainties

No ownership/design question remains open within the specified cutover. Aggregate simplification
and the final shipping cost still require measurements and independent G1 judgement. Assuming the
current replacement cost is acceptable without that evidence could incorrectly close Part 1;
validate the exact K1–K4 candidate before F1 and final CI.
