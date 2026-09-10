# Test cost review

The expensive synthetic capacity scenarios have been deleted after reviewing their assertions.
Production limits and execution behavior are unchanged. This is a test-scope reduction; compiler
profiles, scheduling and implementation performance are unchanged.

## Removed scenarios and retained evidence

| Removed scenario | Retained evidence and scope of deletion |
| --- | --- |
| Two-creation maximum-payload matrix and accumulated contract fixture | Domain [transaction](../crates/domains/evm/tests/transaction_contract.rs) and [anchored-call](../crates/domains/evm/tests/anchored_call_contract.rs) contracts, [generic Runtime](../crates/live/evm/tests/generic_transaction_runtime.rs), and the managed Effect e2e cover facts, composition and recovery. The exhaustive composed fixture failure matrix and maximum-payload admission claim are retired. |
| Six 64-source Portfolio planning configurations | [Planning contracts](../crates/domains/portfolio/tests/planning_contract.rs) retain route selection, invalid routes and canonical Program round-trip. Maximum-size planning is no longer an acceptance scenario. |
| Three 64-source EVM expansion configurations | [App consuming scenarios](../crates/app/tests/support/portfolio_contract.rs) execute native and token plans. A small direct EVM input-validation test retains the unique substituted/reordered-request rejection contract; declaration-count assertions and repeated Program round-trips are deleted. |
| Maximum-width token Portfolio Runtime admission | App consuming scenarios already retain typed provider errors, domain mappings and hot/cold equality. The maximum-width admission fixture is deleted. |
| Report/qualification capacity integration matrices | [Value contracts](../crates/kernel/values/tests/value_contract.rs) and [report unit tests](../crates/kernel/runtime/src/report/tests.rs) retain size enforcement; [Runtime contracts](../crates/kernel/runtime/tests/runtime_contract.rs), callback errors and [pending-failure tests](../crates/kernel/runtime/tests/pending_failure.rs) retain prefix and command authority checks. The combined large-payload error-path matrix is no longer separately exercised. |

Deleting these matrices does not establish that every deleted assertion has an identical surviving
assertion. The retained tests own the underlying behavior; synthetic product maxima and exhaustive
combinations are no longer treated as requirements worth this recurring cost.

## Historical timing evidence

Successful run `run-578226-1789046970663948836` took 58m40s. Duplicate capacity-task
invocations accounted for 18m00s and were previously removed from CI without changing test selection.
The deleted scenarios dominated executables lasting 14m04s (contract fixtures), 11m58s (planning),
2m45s (EVM units), 2m33s (App Portfolio), and 55.69s (report capacity). These are executable
durations, not individually measured test durations; do not sum them as promised savings.

The retained transaction ambiguous-append matrix exceeded 60 seconds in a 2m21s executable.
The managed Effect e2e took 1m56s and the CLI/REST e2e 1m20s individually. The nine
pending-failure tests took 0.26s combined. These retain distinct persistence and external-integration
coverage. In particular, the transaction matrix's fourteen scenarios are seven actual Journal
boundaries times two ambiguous-append outcomes (committed or uncommitted), not a chosen capacity.
The managed e2e's single acknowledgement-loss scenario cannot replace that matrix. No new
full-suite timing was required for deleting test-only scenarios.

## Failure found while validating deduplication

The revised graph passed model admission and eight of its nine CI leaves in run
`run-606178-1789051697354099290` (40m23s overall). The final managed Effect e2e failed;
a focused rerun, `run-628235-1789054215298038690`, reproduced the same failure. Neither test
implementations nor compiler settings changed.

`evm_contract_effect_recovers_cold_and_accepts_external_nonce_advance` fails during its fresh
wallet follow-up with `SizeLimit { resource: PendingFailures, actual: 3, limit: 2 }`.
Its shared transaction fixture admits two pending-failure records, while `drive_to_success`
continues after `RecoveryStopped`. The helper labels those outcomes as dependency unavailability
without reporting their operational causes, so these logs do not establish why the two records
were consumed.

This is a concrete quality-review priority: inspect the retained operational causes and the
fixture's expected failure/resume contract before deciding whether the external setup, expectation,
or admitted bound is wrong. Do not infer that the test is unnecessary or increase its bound merely
to obtain a green run. The deduplicated CI run is not green, and the root cause remains unresolved.

## Material uncertainties

Actual end-to-end savings from the test deletions are unmeasured. Historical executable timings
identify the removed workload but cannot predict an exact new CI duration. The managed Effect
failure described above remains unresolved; deleting other tests provides no evidence of a fix.
