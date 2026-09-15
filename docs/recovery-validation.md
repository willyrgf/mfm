# Recovery validation

[Design](design.md) owns the current Program v8, Journal frame v6 and Runtime contracts.
[The RFC](../RFC_SIMPLIFY_CLASSIFICATION_HANDLER.md) records rationale and checked consuming examples.
Superseded Program/output schemas are rejected; retained histories and revisions are never rewritten.

## Verification selection

Run affected focused tests in the default Nix shell, then one final `nix run .#ci` on the exact
cross-crate candidate. The composed CI covers format, Clippy, compilation, workspace/rustdoc tests,
capacity, PostgreSQL, CLI/REST e2e and managed Effect recovery. A prior candidate's green result
does not establish this cutover's correctness. See [build and verification](build-and-verification.md).

## Coverage

| Boundary | Consuming evidence |
| --- | --- |
| Program identity, ABI/parameter mismatch, old-format rejection and scoped authoring | [Program contracts](../crates/kernel/program/src/tests.rs), [compile-fail tests](../crates/kernel/program/tests/authoring_boundaries.rs) |
| Intrinsic classification, common handler association, root maps, retry/restart, independent budgets and Effect barriers | [Runtime recovery](../crates/kernel/runtime/src/assembly/recovery/tests.rs), [nested recovery](../crates/kernel/runtime/src/assembly/recovery/tests/execution/nested.rs) |
| Pending failure Retry/Stop audit, retained command, cold latest failure and ambiguous acknowledgement | [Pending failures](../crates/kernel/runtime/tests/pending_failure.rs) |
| Cancellation, ambiguous acknowledgement, pending identity and hot/cold equivalence | [Runtime contracts](../crates/kernel/runtime/tests/runtime_contract.rs), [transaction recovery](../crates/live/evm/src/transaction_tests.rs) |
| Exact frame hashes and headers, atomic append and actual limits | [Journal contracts](../crates/kernel/journal/tests/frame_contract.rs), [Store scenarios](../crates/kernel/store/tests/support/scenarios.rs) |
| Exhausted frame count after original failure or external settlement | [Runtime capacity](../crates/kernel/runtime/tests/current_state/capacity.rs) preserves the exact acknowledged head and original/pending authority |
| Object and report size comparison | [Bounded JSON](../crates/kernel/canonical/src/bounded.rs), [Runtime size projection](../crates/kernel/runtime/tests/current_state/sizes.rs) |
| Real policy inheritance and acknowledged-prefix preservation | [Portfolio policies](../crates/domains/portfolio/tests/recovery_policy.rs) |
| Route-aware planning, checked full-width snapshots and compiled product execution | [Planning](../crates/domains/portfolio/tests/planning_contract.rs), [snapshot extremes](../crates/domains/portfolio/src/snapshot_extremes.rs), [App consuming scenarios](../crates/app/tests/support/portfolio_contract.rs) |
| Checked admission names/digests, paired output and publication bounds | [Admission](../crates/domains/portfolio/src/enrichment/tests.rs), [publication bounds](../crates/app/src/config/tests.rs) |
| Incomplete/forged/lost-ack publication; transport delete/resume/dependent admission | [App publication](../crates/app/tests/support/enrichment.rs), managed [CLI/REST e2e](../bin/rest-api/tests/client_execution_e2e.rs) |

## RFC acceptance evidence

| Criteria | Observable evidence |
| --- | --- |
| 1 | [Portfolio policy scenarios](../crates/domains/portfolio/tests/recovery_policy.rs) exercise inherited, replaced and occurrence-specific handlers; [App Portfolio execution](../crates/app/tests/portfolio_runtime.rs) associates the real heterogeneous EVM/Portfolio State sequence with the single framework Stop. [Scoped Program tests](../crates/kernel/program/src/tests.rs) retain parameters, targets and explicit zero independently. |
| 2–3 | [Operational classifications](../crates/domains/evm/src/recovery.rs) distinguish observational timeouts from uncertain submission. [Qualified nonacceptance](../crates/kernel/runtime/tests/pending_failure/protocol.rs) exercises a distinct error contract. [Compile-fail integration](../crates/kernel/runtime/tests/ui/classification_required.rs) rejects missing classification at authoring and registration while declaring the capability successfully. |
| 4–5 | [Portfolio policies](../crates/domains/portfolio/tests/recovery_policy.rs) retain original/root failures; [Runtime contracts](../crates/kernel/runtime/tests/runtime_contract.rs) preserve internal-failure boundaries. [Pending JSON](../crates/app/tests/pending_view.rs) retains exact original cause, complete executed input and decision through the public invocation envelope. |
| 6 | [Recovery execution](../crates/kernel/runtime/src/assembly/recovery/tests/execution.rs) and [nested restart](../crates/kernel/runtime/src/assembly/recovery/tests/execution/nested.rs) cover committed budgets and Effect barriers. [Phase authorization](../crates/kernel/runtime/tests/pending_failure/phase.rs) denies Pure retry, settled Effect retry and ineligible restart. |
| 7–8, 12 | [Pending audit scenarios](../crates/kernel/runtime/tests/pending_failure/audit.rs) cover Retry/Stop, retained authority, explicit resume, cancellation before/after insertion, ambiguous acknowledgements and competing causes. [Protocol reconstruction](../crates/kernel/runtime/tests/pending_failure/protocol.rs) proves cold pending and completed histories do not invoke classification or handlers. |
| 9 | [Exact handler association](../crates/kernel/runtime/src/assembly/recovery/tests.rs) rejects implementation/parameter mismatches and checks parameter-sensitive Program identity. [Program contracts](../crates/kernel/program/src/tests.rs) reject superseded descriptors and distinguish exact operational error contracts. Revised classification semantics require revision of that exact error identity, as specified in [design](design.md). |
| 10 | The prospective capacity contract is superseded: [pending audit](../crates/kernel/runtime/tests/pending_failure/audit.rs) preserves reconciliation after recovery exhaustion without a failure quota. [Journal](../crates/kernel/journal/src/lib.rs) and [Store](../crates/kernel/store/src/lib.rs) retain actual size/count and arithmetic rejection. The 32 MiB object/report ceiling is the production constant; comparison is tested with small limits in [bounded JSON](../crates/kernel/canonical/src/bounded.rs). |
| 11 | [Journal frames](../crates/kernel/journal/tests/frame_contract.rs) check opaque canonical envelopes; [current-state validation](../crates/kernel/runtime/tests/current_state/validation.rs) checks Runtime payload relationships. [Pending audit](../crates/kernel/runtime/tests/pending_failure/audit.rs) rejects position mismatch; [public pending view](../crates/app/tests/pending_view.rs) checks hot/cold latest failure equality. |

Selected row bytes remain untrusted until exact Journal decoding and Runtime current-state validation.
Inline original/mapped report duplication is intentional. A combined report can exceed 32 MiB even
when its causes fit individually; the size error preserves the acknowledged head and pending Effect
authority. Admission makes no future capacity promise. See [design](design.md).

## Material uncertainties

none for this cutover. Production finality and other deliberately deferred capabilities remain
listed in [known gaps](known-gaps.md).
