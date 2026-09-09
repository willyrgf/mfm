# Recovery validation

[Design](design.md) owns the current Program v5, Journal frame v3 and Runtime contracts.
[The RFC](../RFC_REFACT_RUNTIME_TO_RECOV_SM.md) records rationale and checked consuming examples.
Superseded Program/output schemas are rejected; retained histories and revisions are never rewritten.

## Candidate evidence

The simplification replaces duplicate state/recovery descriptors, uses checked admission identities,
pairs enriched collections with their publication bindings, shares invocation errors and test
boundaries, and removes the unchecked API sketch. Focused verification uses the pinned Nix shell.

<!-- CURRENT_CI_EVIDENCE -->
The composed CI result for this simplification has not yet been recorded. Earlier branch CI results
are not evidence for these changes. The final run must identify its exact candidate and Nixfied
`artifacts/run-summary.json`; this section is updated only after its result is known.
<!-- END_CI_EVIDENCE -->

## Coverage

| Boundary | Consuming evidence |
| --- | --- |
| Program identity, ABI/parameter mismatch, old-format rejection and scoped authoring | [Program contracts](../crates/kernel/program/src/tests.rs), [compile-fail tests](../crates/kernel/program/tests/authoring_boundaries.rs) |
| Typed mapping, exact association, retry/restart, independent budgets and Effect barriers | [Runtime recovery](../crates/kernel/runtime/src/assembly/recovery/tests.rs), [nested recovery](../crates/kernel/runtime/src/assembly/recovery/tests/execution/nested.rs) |
| Cancellation, ambiguous acknowledgement, pending identity and hot/cold equivalence | [Runtime contracts](../crates/kernel/runtime/tests/runtime_contract.rs), [transaction recovery](../crates/live/evm/tests/support/recovery_transaction.rs) |
| Qualified hashes, previous-head linkage, object closure, adjacency and fixed capacities | [Journal contracts](../crates/kernel/journal/tests/frame_contract.rs), [Store scenarios](../crates/kernel/store/tests/support/scenarios.rs) |
| Exact 32 MiB object/report limits and precise mapped-value/context errors | [Values](../crates/kernel/values/tests/value_contract.rs), [reports](../crates/kernel/runtime/tests/report_capacity.rs), [qualification](../crates/kernel/runtime/tests/support/qualification_capacity.rs) |
| Real policy inheritance and acknowledged-prefix preservation | [Portfolio policies](../crates/domains/portfolio/tests/recovery_policy.rs) |
| 64/65-source planning, complete closure bounds and compiled product execution | [Planning](../crates/domains/portfolio/tests/planning_contract.rs), [bounds](../crates/domains/portfolio/src/bounds/tests.rs), [App consuming scenarios](../crates/app/tests/support/portfolio_contract.rs) |
| Checked admission names/digests, paired output and publication bounds | [Admission](../crates/domains/portfolio/src/enrichment/tests.rs), [publication bounds](../crates/app/src/config/tests.rs) |
| Deleted configs, provenance rejection, idempotent/lost-ack publication and dependent admission | [App publication](../crates/app/tests/support/enrichment.rs), managed [CLI/REST e2e](../bin/rest-api/tests/client_execution_e2e.rs) |

Raw transfer bytes remain untrusted until Journal qualification and Runtime semantic validation.
Inline original/mapped report duplication is intentional. A combined report can exceed 32 MiB even
when its causes fit individually; the size error preserves the acknowledged head and pending Effect
authority. Admission bounds Journal lifecycles, not every derived report. See [design](design.md).

## Material uncertainties

No unresolved architecture choices. Composed verification status is stated above; focused tests alone
do not establish that gate. Production finality and other deliberately deferred capabilities remain
listed in [known gaps](known-gaps.md).
