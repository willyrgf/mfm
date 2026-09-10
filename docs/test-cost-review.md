# Slow test review

This inventory supports a review of test value and necessity. It does not change test bodies,
fixtures, compiler profiles, or test scheduling. The CI change removes only the second invocation
of tests already selected by `cargo test --workspace --all-targets`.

## Timing evidence

Baseline: the successful 2026-09-10 Nixfied run
`run-578226-1789046970663948836`, before CI deduplication. Its `artifacts/run-summary.json`
and `logs/task.ci.*.stdout.log` record 3,519.60 seconds (58m40s) overall. The repeated capacity
stage accounts for 1,080.08 seconds (18m00s, 30.7%). Removing it removes no unique test selection;
actual end-to-end savings remain dependent on the next run's conditions.

The default Rust harness records durations for each test executable, not individual test cases.
A `has been running for over 60 seconds` message establishes a lower bound for the named test.
Do not assign its executable's full duration to that individual test or add overlapping durations.
The managed e2e executables each contain one selected test, so their test durations are individual.
All durations below exclude the Cargo compilation preceding the test executable.

| Test | Timing evidence | What the test establishes | Test-quality review question |
| --- | --- | --- | --- |
| [`two_creations_call_observation_and_reports_fit_the_unchanged_capacity_envelope`](../crates/live/evm/tests/support/context_capacity.rs) | Individual >60s; shares a 14m04s executable with the accumulated fixture below | Real expanded transaction/observation States fit the admitted schema, object, frame and history bounds across large input/evidence sizes; success and failure reports retain prior facts and cold reconstruction agrees | Which fault cases need maximum payloads to expose a size/closure defect, and which only recheck behavior already covered by the accumulated fixture? Preserve distinct prior-creation and complete-closure cases. |
| [`accumulated_fixture_preserves_all_success_failure_and_cold_facts`](../crates/live/evm/tests/support/accumulating_contract.rs) | Individual >60s; same 14m04s executable, not another 14 minutes | Seven success/failure cases preserve caller context and transaction/observation facts through composition and cold reconstruction using small payloads | For each case, identify the unique retained-data assertion beyond the capacity fixture and domain interpreter tests. Does the composition boundary need every failure reason, or only those with different retention behavior? |
| [`the_supported_source_capacity_plans_and_one_more_is_rejected`](../crates/domains/portfolio/tests/planning_contract.rs) | Individual >60s; planning executable 11m58s; repeated executable 12m03s in the old capacity stage | Six 64-source configurations cover native/mixed/token sources split into 1 or 64 collections, worst-case escaped public identities, Program round-trip, finite history admission and rejection of a 65th source | Which configurations establish distinct worst cases? Are the repeated oversized-input rejection and round-trip assertions testing a separate boundary each time? Explain any reduction with the actual bound contract, not a timing target. |
| [`balance_authoring_specializes_checked_native_token_and_mixed_requests`](../crates/domains/evm/src/lib.rs) | Individual >60s; EVM unit executable 2m45s | Three 64-source native/mixed/token requests produce the expected specialized sequence, reject substituted/reordered inputs, and round-trip the Program | Is 64 required to validate specialization and input binding, or is maximum-size admission already owned by the Portfolio capacity test? Retain EVM's direct consuming boundary coverage. |
| [`maximum_token_portfolio_admits_and_retains_the_first_typed_provider_failure`](../crates/app/tests/support/portfolio_contract.rs) | Individual >60s; App Portfolio executable 2m33s | A maximum-width, 64-collection token Program admits through Runtime, invokes only the first failing provider, retains its typed cause/context, fits actual genesis/history bounds, and reconstructs cold | This crosses admission and Runtime boundaries that planning alone cannot establish. Which assertions specifically require the maximum fixture, and which repeat ordinary provider-failure coverage? |
| [`every_transaction_journal_boundary_recovers_after_ambiguous_append`](../crates/live/evm/src/transaction_tests.rs) | Individual >60s; live EVM unit executable 2m21s | Fourteen scenarios inject committed/uncommitted lost acknowledgements at seven Journal boundaries; cold recovery reaches the same terminal history and nonce, using retained prepared authority | Keep the two acknowledgement outcomes and every distinct persistence boundary. Review whether each case asserts the specific invariant that can fail there, beyond eventual completion. |
| [`evm_contract_effect_recovers_cold_and_accepts_external_nonce_advance`](../crates/live/evm/tests/evm_contract_effect_e2e.rs) | Individual 1m56s; managed task including setup 2m03s | Actual PostgreSQL, Reth, signer/authority and compiled contract integration supports cold recovery and external nonce advancement | Keep coverage that requires the real external stack. Separate its evidence from deterministic fixture claims; it does not prove a full process restart. |
| [`generated_rest_run_survives_deletion_and_matches_fresh_cli_execution`](../bin/rest-api/tests/client_execution_e2e.rs) | Individual 1m20s; managed task including setup 1m28s | Real CLI/REST execution preserves exact admitted revisions after config deletion and agrees on results; publication and dependent admission survive recovery | Identify which assertions uniquely test transport/process/config integration versus repeat library behavior. Retain the real cross-transport contract. |

The report-capacity executable is the next notable group at 55.69 seconds for two tests:
[`inline_report_size_preserves_original_values_and_pending_authority`](../crates/kernel/runtime/tests/report_capacity.rs)
and the included
[`mapped_values_and_adapter_contexts_preserve_size_errors_and_acknowledged_heads`](../crates/kernel/runtime/tests/support/qualification_capacity.rs).
These distinguish original-object limits from combined-report limits and preserve acknowledged
heads/Effect authority on overflow. Review the chosen payload sizes against those distinct
boundaries; their large allocations serve a concrete size contract.

The nine pending-failure tests, including the stop-reason validation matrix, took 0.26 seconds
combined in this baseline. They are not a current runtime-priority target.

## Review criteria

For each expensive scenario, state the production defect its assertions catch, the owning boundary,
and why the chosen size or combination is necessary. Compare assertions and affected boundaries,
not just similar test names. A test can share setup with another test and still protect a distinct
contract. Conversely, a large matrix needs evidence that its dimensions affect the guarantee.
No test is approved for deletion merely because it appears here.

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

Individual durations for the shared executables are unmeasured. The working assumption is that
the tests explicitly reported above 60 seconds dominate those executables; if that is wrong, a
strict individual ranking could misdirect review. Validate individual timings before making a
performance-based prioritization within a shared executable. Necessity judgments above are review
questions, not conclusions that coverage is redundant. The repeated Effect failure establishes
capacity exhaustion, but not its underlying operational cause; inspect its retained incidents to
validate that cause before changing the test.
