# Current-state core acceptance evidence

This records RFC section 12's step-3 boundary. It is not C15 source-owner closure or C18 final CI.
The retained upstream gaps remain in [the owner inventory](../adapter-error-audit.md). Frame/load
measurements, physical deletions and production LOC are in [the measurements](auditability-core.md).

## Consuming requirements

| RFC | Current evidence and the guarantee it exercises |
| --- | --- |
| C1 | [Runtime contract](../../crates/kernel/runtime/tests/runtime_contract.rs) exercises hot/cold Pure, zero-State, Read and Effect runs. [Current wire](../../crates/kernel/runtime/src/state/decode/tests.rs) exercises every current Phase, operation-facts, failure, classification and recovery alternative through canonical JSON and the actual seeds. These wire cases make no semantic-history claim. |
| C2 | [Current validation](../../crates/kernel/runtime/tests/current_state/validation.rs) rejects inconsistent admission/latest, wrong Program/slot identity, stale Object digest and an out-of-range completed-call position before callbacks. [Collision](../../crates/kernel/runtime/tests/current_state/collision.rs) repeats exact start without driving and rejects different admission. [Store tests](../../crates/kernel/store/src/tests.rs) and managed PostgreSQL cover physical snapshot obligations; measured short/long loads return two normal rows. |
| C3 | Current validation rejects usage cardinality, local/global bounds, checked-sum overflow, undeclared checkpoints, invalid barriers and mismatched completed facts. It deliberately accepts locally valid counter substitution, proving that restoration does not claim historical counter verification. [Pending validation](../../crates/kernel/runtime/tests/pending_failure/audit.rs) covers command/input/position and recovered Stop relations. |
| C4 | [Checkpoint restart](../../crates/kernel/runtime/tests/current_state.rs) restores full input, advances the visit, prunes later checkpoints and retains usage. [Shipping Portfolio restart](../../crates/domains/portfolio/tests/recovery_policy.rs) refreshes the actual anchored collection and retains its acknowledged prefix. |
| C5–C6 | Current-state tests commit the original before a failing policy, inspect it cold, and resume only unfinished recovery. [Terminal mapping](../../crates/kernel/runtime/tests/current_state/terminal.rs) retains the domain original when root mapping fails. [Pending audit](../../crates/kernel/runtime/tests/pending_failure/audit.rs) proves charged Retry, exhausted/requested Stop, and unchanged unresolved command authority. |
| C7–C8 | Current-state and Runtime contract tests preserve command/EffectId before adapter entry and commit settlement before an injected interpreter failure. Cold resume interprets retained evidence without adapter IO. Pending audit covers cancellation and ambiguous acknowledgement. |
| C9–C10 | [Callback errors](../../crates/kernel/runtime/tests/support/callback_errors.rs), Runtime preparation/evidence tests and [native constructors](../../crates/kernel/runtime/tests/current_state/constructors.rs) retain native operation/stage causes with unchanged heads. Local rejection makes no provider call; post-response binding rejection preserves the fact that the provider was entered. |
| C11 | Current-state recording tests retain the native original and sealed candidate on Store failure without probing. [Size/task custody](../../crates/kernel/runtime/tests/current_state/sizes.rs) drives an oversized original and a panic in failure encoding through actual erased adapter callbacks: no candidate, no classification, unchanged admission and original retained in native custody. [App reporting](../../crates/app/src/reporting_tests.rs) preserves this custody through AppendIndeterminate and secondary projection failure. An unwinding native projector returns a separate reviewed failure with explicit payload withholding; incomplete reporting retains original/head custody without retrying it. |
| C12 | Collision tests cover exact candidate, later commits, conflicting row, absent candidate, failed reload and a physically present candidate followed by an invalid latest Runtime payload. The physical finding survives the separate restoration failure; no case drives further work. |
| C13 | [Public Store scenarios](../../crates/kernel/store/tests/support/scenarios.rs), MemoryStore and managed PostgreSQL tests retain atomic exact-head append, complete selected snapshots, immutable prior bytes, cumulative bounds and COMMIT ambiguity. SQLx metadata is checked against the managed baseline. Native SQLx source extraction remains step 4. |
| C14 | [Shipping App constructor](../../crates/app/tests/support/native_constructor.rs) admits descriptor-valid EVM balance input and delivers distinct empty/oversized correlation constructor causes through the actual Read callback and App report. It also delivers a stale Object digest as restore/decode with reviewed expected/actual fields, without provider entry or append. |
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

Final `nix run .#ci` is reserved for the complete implementation after source-owner closure.

## Material uncertainties

None within the core boundary. Projection-panic handling covers unwinding callbacks, not aborts
or disclosure by arbitrary process panic hooks. Remaining source-owner closure and final CI are
required and are not claimed by this evidence.
