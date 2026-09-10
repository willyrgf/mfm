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

## Measured rerun after deletion — 2026-09-10

Full CI passed **9/9 stages in 530.41 seconds (8m50s)** in
`run-658256-1789058404792819642`, on the Rust tree committed as `b8247953`.
The workspace test stage took **292.94 seconds (4m53s)**, versus 2,185.41 seconds
(36m25s) in the prior deduplicated run. These are observed runs, including compilation;
they are not a controlled benchmark. The initial attempt stopped before tests because Clippy
caught an unused import left by the deletion; that import was removed before the successful run.

For individual timing, the nine workspace executables exceeding five seconds were rerun in the
default Nix shell, using the exact compiled CI binaries and normal test concurrency. All 73
selected tests passed. Runtime-only `RUSTC_BOOTSTRAP=1` enabled libtest's
`-Z unstable-options --format=json --report-time`; no compiler invocation or optimization change
was made in this timing pass. Other workspace executables each took at most 4.90 seconds in CI.
The two managed e2e timings below come directly from their one-test CI executions. Durations
exclude compilation/service setup and can overlap within an executable; do not sum them.

| Rank | Individual test | Seconds | Behavior and retention rationale |
| --- | --- | ---: | --- |
| 1 | [`every_transaction_journal_boundary_recovers_after_ambiguous_append`](../crates/live/evm/src/transaction_tests.rs) | 138.79 | Fourteen cases lose acknowledgement at seven Journal boundaries, both before and after commit, then recover cold with a rejecting signer once preparation exists. Protects recovery from unknown persistence outcomes without replacing retained transaction authority. |
| 2 | [`evm_contract_effect_recovers_cold_and_accepts_external_nonce_advance`](../crates/live/evm/tests/evm_contract_effect_e2e.rs) | 114.51 | Runs PostgreSQL, Reth, signer/authority and a compiled contract: acknowledgement loss, cold recovery, deployment/configuration, and a later external nonce advance. Protects real integration that scripted adapters cannot establish. Passed this run; the earlier failure cause remains unknown. |
| 3 | [`generated_rest_run_survives_deletion_and_matches_fresh_cli_execution`](../bin/rest-api/tests/client_execution_e2e.rs) | 79.07 | Runs actual CLI/REST binaries, deletes configurations, resumes admitted runs, compares independent results, and publishes enrichment for dependent execution. Protects transport wiring and retained configuration identity across process boundaries. |
| 4 | [`enrichment_publication_and_matching_admission_survive_configuration_deletion`](../crates/app/tests/support/enrichment.rs) | 31.96 | Exercises incomplete enrichment, ambiguous publication, idempotent retry, deleted configurations and forged head/value/route provenance. Protects publication of the exact acknowledged result and rejection of unrelated inputs. Some lifecycle steps overlap the transport e2e; hostile provenance and publication acknowledgement are the stronger reasons to retain coverage. |
| 5 | [`stored_config_lifecycle_uses_exact_revisions_and_preserves_admitted_runs`](../crates/app/tests/portfolio_runtime.rs) | 27.76 | Imports two revisions under one name, runs each exact revision, rejects absent revisions, lists/deletes configurations and reads admitted history after deletion. Protects revision selection and immutable admitted runs. Configuration-deletion coverage overlaps the transport e2e; this is a candidate for assertion-level consolidation review. |
| 6 | [`planned_native_and_token_collections_execute_and_reconstruct_typed_failures`](../crates/app/tests/support/portfolio_contract.rs) | 24.83 | Executes native/token plans, verifies amounts, domain-to-root failure mapping and typed provider causes/context, then compares cold reports. Protects actual planner/State/map composition rather than declaration counts. The success and two failure kinds are materially different outcomes. |
| 7 | [`enrichment_retains_native_and_nonzero_candidates_and_never_filters_provider_failure`](../crates/app/tests/support/portfolio_contract.rs) | 24.40 | Keeps native and funded token sources, removes an observed zero-balance token, and separately injects a timeout. Protects against silently treating failed observation as zero balance and publishing incomplete discovery as success. |
| 8 | [`maximum_transaction_closures_fit_and_pending_operational_failure_preserves_authority`](../crates/live/evm/tests/support/recovery_transaction.rs) | 20.05 | Uses maximum input/identity widths and a large caller context across success, revert and timeout/resume, checking history bounds and retained Effect identity. The pending-authority behavior matters, but the maximal fixture has the same justification weakness as the removed matrices. Candidate for removal/consolidation review; its full cost is not justified merely by that behavior. |
| 9 | [`pending_effect_reuses_identical_wire_and_cold_projection_needs_no_signer_call`](../crates/live/evm/src/transaction_tests.rs) | 17.44 | Resumes with a rejecting signer, asserts repeated submissions use identical prepared bytes and only one nonce lookup, then settles success/revert and reads cold without provider calls. Directly protects transaction identity and completed-read behavior; the Journal matrix does not assert all these external operations. |
| 10 | [`selecting_another_same_typed_source_requires_its_own_assembly_before_io`](../crates/live/evm/tests/generic_transaction_runtime.rs) | 17.23 | Builds two same-typed creation facts pointing to different addresses, selects each as a call target, and rejects an assembly registered for the other recipe before IO. Protects semantic target selection where Rust types alone cannot distinguish the sources. |
| 11 | [`one_transaction_state_selects_multiple_exact_generic_codecs_hot_and_cold`](../crates/live/evm/tests/generic_transaction_runtime.rs) | 15.95 | Executes the same transaction family with two caller context shapes and reconstructs both cold, checking retained caller/transaction facts and exact value contracts. Protects generic codec association and durable decoding across consuming types. |
| 12 | [`custody_acknowledgement_loss_recovers_each_stage`](../crates/live/evm/src/transaction_tests.rs) | 15.59 | Loses transaction-authority acknowledgement during reservation and preparation, then resumes using retained authority and no second nonce lookup. Protects a persistence boundary separate from the Journal; a Journal-only fault cannot establish it. |

Next measured costs include App start/progress recovery envelopes (14.66s), exact report-limit
rejection (12.92s), RPC timeout/rate-limit mapping (10.08s), and Journal maximum-size closure
validation (8.00s). The remaining capacity tests should be judged against concrete requirements,
not treated as automatically necessary because their boundaries exist in production.

Raw timing evidence is in `/tmp/mfm-individual-test-timings/results.json` and the adjacent
per-executable JSONL files for this session; full CI evidence is under the run ID above in
Nixfied's state directory. No tests were deleted or optimized during this measurement pass.

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
The managed e2e's single acknowledgement-loss scenario cannot replace that matrix. The subsequent measured rerun is recorded above.

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
to obtain a green run. Those two historical runs failed. The new post-deletion run passed without an e2e fix; the original root cause remains unresolved.

## Material uncertainties

The measured ordering is from single runs with normal within-executable concurrency, not isolated
CPU costs or a statistical benchmark. Close timings may change order; repeated isolated
measurements would validate fine-grained comparisons if needed. The prior managed Effect failure
did not recur, but its cause is unknown, so the passing rerun does not establish a fix. Retention
rationales identify protected behavior, not proof that every assertion and fixture is minimal.
