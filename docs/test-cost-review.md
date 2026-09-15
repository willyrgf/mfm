# Test cost review

Workspace tests keep unique facts. They do not repeat a generic protocol, a transport e2e
lifecycle, a max-width fixture, or a 32 MiB payload that only proves a hardcoded ceiling.

## Current cutover

| Change | Unique fact that remains | Deleted overlap |
| --- | --- | --- |
| Live EVM Journal Indeterminate grid | Prepared-bytes identity, rejecting signer, one nonce lookup, secret-free admission/latest frames, and authority persist loss | Runtime already recovers generic Effect Indeterminate at named prepare/settlement/interpretation steps. Store `record()` is the same path for every post-admission frame. |
| Max-width transaction fixture | none | Chain and value constructors already bound inputs. Pending identity stays in `pending_effect_reuses_identical_wire_and_cold_projection_needs_no_signer_call`. |
| Enrichment publication | Incomplete publish, lost publication ack, forged head/value/route, snapshot is not enrichment | CLI/REST e2e already deletes configs, publishes, retries, and starts a dependent snapshot. |
| Stored config lifecycle | Unchanged import, absent digest, idempotent delete, admitted run after delete, one exact start | Transport e2e already uses two revisions and resumes after delete. |
| 32 MiB object/report payloads | `to_json_bounded` and `SizeLimitExceeded::check` with small limits; Runtime size projection | Production constants stay 32 MiB. Tests do not allocate that maximum. |
| Type-restating and parallel-model tests | Application use cases through public methods; one collection-restart policy; frozen public wires; one representative RunRecord roundtrip | Deleted Parent/Child policy scaffolding, constructor field-equals-input asserts, Program declaration-index freezes, RecoveryOutcome cartesian expansion, and same-crate SQL `contains` table-name checks. |

## Retained unique tests

- [`pending_effect_reuses_identical_wire_and_cold_projection_needs_no_signer_call`](../crates/live/evm/src/transaction_tests.rs)
- [`custody_acknowledgement_loss_recovers_each_stage`](../crates/live/evm/src/transaction_tests.rs)
- [`selecting_another_same_typed_source_requires_its_own_assembly_before_io`](../crates/live/evm/tests/generic_transaction_runtime.rs)
- [`one_transaction_state_selects_multiple_exact_generic_codecs_hot_and_cold`](../crates/live/evm/tests/generic_transaction_runtime.rs)
- [`snapshot_token_holdings_and_typed_read_failures`](../crates/app/tests/use_cases.rs)
- [`enrichment_keeps_native_and_nonzero_candidates`](../crates/app/tests/use_cases.rs)
- [`enrichment_publish_rejects_incomplete_forged_and_lost_ack`](../crates/app/tests/use_cases.rs)
- [`config_delete_does_not_revoke_an_admitted_run`](../crates/app/tests/use_cases.rs)
- [`exact_bound_and_first_rejected_write_preserve_sources_without_traversing_suffix`](../crates/kernel/canonical/src/bounded.rs)

## Earlier reduction

An earlier pass deleted synthetic 64-source and maximum-payload matrices. That history is not
current coverage. See Git for the deleted scenarios.

## Material uncertainties

none
