# Recovery validation

[Design](design.md) owns the current Program v6, Journal frame v4 and Runtime contracts.
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
| Pending failure Retry/Stop audit, capacity, retained command, cold latest failure and ambiguous acknowledgement | [Pending failures](../crates/kernel/runtime/tests/pending_failure.rs) |
| Cancellation, ambiguous acknowledgement, pending identity and hot/cold equivalence | [Runtime contracts](../crates/kernel/runtime/tests/runtime_contract.rs), [transaction recovery](../crates/live/evm/tests/support/recovery_transaction.rs) |
| Qualified hashes, previous-head linkage, object closure, adjacency and fixed capacities | [Journal contracts](../crates/kernel/journal/tests/frame_contract.rs), [Store scenarios](../crates/kernel/store/tests/support/scenarios.rs) |
| Exact object/report limit enforcement | [Values](../crates/kernel/values/tests/value_contract.rs), [reports](../crates/kernel/runtime/src/report/tests.rs) |
| Real policy inheritance and acknowledged-prefix preservation | [Portfolio policies](../crates/domains/portfolio/tests/recovery_policy.rs) |
| Route-aware planning, history-bound arithmetic and compiled product execution | [Planning](../crates/domains/portfolio/tests/planning_contract.rs), [bounds](../crates/domains/portfolio/src/bounds/tests.rs), [App consuming scenarios](../crates/app/tests/support/portfolio_contract.rs) |
| Checked admission names/digests, paired output and publication bounds | [Admission](../crates/domains/portfolio/src/enrichment/tests.rs), [publication bounds](../crates/app/src/config/tests.rs) |
| Deleted configs, provenance rejection, idempotent/lost-ack publication and dependent admission | [App publication](../crates/app/tests/support/enrichment.rs), managed [CLI/REST e2e](../bin/rest-api/tests/client_execution_e2e.rs) |

## RFC acceptance evidence

| Criteria | Observable evidence |
| --- | --- |
| 1 | [Portfolio policy scenarios](../crates/domains/portfolio/tests/recovery_policy.rs) exercise inherited, replaced and occurrence-specific handlers; [App Portfolio execution](../crates/app/tests/portfolio_runtime.rs) associates the real heterogeneous EVM/Portfolio State sequence with the single framework Stop. [Scoped Program tests](../crates/kernel/program/src/tests.rs) retain parameters, targets and explicit zero independently. |
| 2–3 | [Operational classifications](../crates/domains/evm/src/recovery.rs) distinguish observational timeouts from uncertain submission. [Qualified nonacceptance](../crates/kernel/runtime/tests/pending_failure/protocol.rs) exercises a distinct error contract. [Compile-fail integration](../crates/kernel/runtime/tests/ui/classification_required.rs) rejects missing classification at authoring and registration while declaring the capability successfully. |
| 4–5 | [Portfolio policies](../crates/domains/portfolio/tests/recovery_policy.rs) retain original/root failures; [Runtime contracts](../crates/kernel/runtime/tests/runtime_contract.rs) preserve internal-failure boundaries. [Pending JSON](../crates/app/tests/pending_view.rs) retains exact original cause, State context and decision through the public invocation envelope. |
| 6 | [Recovery execution](../crates/kernel/runtime/src/assembly/recovery/tests/execution.rs) and [nested restart](../crates/kernel/runtime/src/assembly/recovery/tests/execution/nested.rs) cover committed budgets and Effect barriers. [Phase authorization](../crates/kernel/runtime/tests/pending_failure/phase.rs) denies Pure retry, settled Effect retry and ineligible restart. |
| 7–8, 12 | [Pending audit scenarios](../crates/kernel/runtime/tests/pending_failure/audit.rs) cover Retry/Stop, retained authority, explicit resume, cancellation before/after insertion, ambiguous acknowledgements and competing causes. [Protocol reconstruction](../crates/kernel/runtime/tests/pending_failure/protocol.rs) proves cold pending and completed histories do not invoke classification or handlers. |
| 9 | [Exact handler association](../crates/kernel/runtime/src/assembly/recovery/tests.rs) rejects implementation/parameter mismatches and checks parameter-sensitive Program identity. [Program contracts](../crates/kernel/program/src/tests.rs) reject superseded descriptors and distinguish exact operational error contracts. Revised classification semantics require revision of that exact error identity, as specified in [design](design.md). |
| 10 | [Pending audit](../crates/kernel/runtime/tests/pending_failure/audit.rs) counts both decisions, rejects provider entry at capacity, and preserves settlement after an oversized rejected record. [Effect bounds](../crates/kernel/program/src/recovery/bounds.rs) reject zero/overflow and include the complete lifecycle. Existing report/capacity and shipping Portfolio tests remain in the composed gate. |
| 11 | [Journal frames](../crates/kernel/journal/tests/frame_contract.rs) qualify prepare/failure*/settlement, complete prefixes and distinct original/context closure. [Pending audit](../crates/kernel/runtime/tests/pending_failure/audit.rs) rejects position mismatch; [public pending view](../crates/app/tests/pending_view.rs) checks hot/cold latest failure equality. |

Raw transfer bytes remain untrusted until Journal qualification and Runtime semantic validation.
Inline original/mapped report duplication is intentional. A combined report can exceed 32 MiB even
when its causes fit individually; the size error preserves the acknowledged head and pending Effect
authority. Admission bounds Journal lifecycles, not every derived report. See [design](design.md).

## Material uncertainties

none for this cutover. Production finality and other deliberately deferred capabilities remain
listed in [known gaps](known-gaps.md).
