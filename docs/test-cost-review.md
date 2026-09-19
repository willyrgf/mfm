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
| Runtime contract fixtures | One synthetic program in `crates/kernel/runtime/tests/support/program.rs`; unique Effect/Read/construction facts stay in `runtime_contract.rs` | The scenario file no longer rebuilds the program types inline. Hostile Store helpers live with `ScriptedStore`. |
| App scenario setup | Distinct run IDs, native/token outcomes, later-candidate failure and exact Start/Progress recovery identity | Four repeated Application compositions and three repeated imports inside scenarios. Each test still owns its own provider, Store and repository. |

## Retained unique tests

- [`pending_effect_reuses_identical_wire_and_cold_projection_needs_no_signer_call`](../crates/live/evm/src/transaction_tests.rs)
- [`custody_acknowledgement_loss_recovers_each_stage`](../crates/live/evm/src/transaction_tests.rs)
- [Native request, binding and implementation qualification](../crates/domains/evm/tests/native_preparation.rs)
- [Maintained deployment and existing-address request specializations](../crates/live/evm/tests/evm_contract_effect_e2e.rs)
- [Exact generic codec ownership](../crates/kernel/runtime/tests/exact_contracts.rs)
- [`snapshot_token_holdings_and_typed_read_failures`](../crates/app/tests/use_cases.rs)
- [`enrichment_keeps_native_and_nonzero_candidates`](../crates/app/tests/use_cases.rs)
- [`enrichment_publish_rejects_incomplete_forged_and_lost_ack`](../crates/app/tests/use_cases.rs)
- [`config_delete_does_not_revoke_an_admitted_run`](../crates/app/tests/use_cases.rs)
- [`config_rejects_malformed_unbound_and_forged_rows_before_admission`](../crates/app/tests/use_cases.rs) start-time revalidation of a correctly hashed invalid retained row
- Effect e2e `SerializableClientError::for_run` on `RecoveryStopped` in [`evm_contract_effect_e2e.rs`](../crates/live/evm/tests/evm_contract_effect_e2e.rs)
- [`exact_bound_and_first_rejected_write_preserve_sources_without_traversing_suffix`](../crates/kernel/canonical/src/bounded.rs)

## Earlier reduction

An earlier pass deleted synthetic 64-source and maximum-payload matrices. That history is not
current coverage. See Git for the deleted scenarios.

## Coverage ownership

Keep engine mechanics separate from their consuming boundary assertions. A retained scenario
replaces another only for the guarantees it actually checks.

| Guarantee | Owner and evidence | Boundary assertion still required |
| --- | --- | --- |
| Effect preparation, settlement, cancellation and ambiguous appends | [Runtime contracts](../crates/kernel/runtime/tests/runtime_contract.rs) | [Live transaction tests](../crates/live/evm/src/transaction_tests.rs) retain prepared wire, signer and nonce behavior. |
| Pending failure recovery, decisions and command authority | [Runtime pending audit](../crates/kernel/runtime/tests/pending_failure/audit.rs) | [Effect e2e](../crates/live/evm/tests/evm_contract_effect_e2e.rs) checks App pending views and the `RecoveryStopped` client envelope. |
| Stored configuration qualification and admission | [App use cases](../crates/app/tests/use_cases.rs) | Digest qualification and correctly hashed semantic invalidity need distinct hostile rows; import validation does not exercise repository reads. |
| Snapshot results and original/root failure mapping | [App use cases](../crates/app/tests/use_cases.rs) | Assert product output and cold public failure meaning; generic Runtime recovery does not prove domain mapping. |
| Candidate filtering and publication eligibility | [App use cases](../crates/app/tests/use_cases.rs) | Keep zero-balance native assets, remove zero-balance tokens, and reject publication when a later candidate fails after earlier observations succeeded. |
| Config deletion, repeat publication and dependent execution across transports | Managed [client e2e](../bin/rest-api/tests/client_execution_e2e.rs) | App scenarios own hostile provenance and lost repository acknowledgement, not another full transport lifecycle. |
| Bounded serialization and physical append limits | [Bounded JSON](../crates/kernel/canonical/src/bounded.rs), [Journal frames](../crates/kernel/journal/tests/frame_contract.rs), [Store scenarios](../crates/kernel/store/tests/support/scenarios.rs) | [Runtime size projection](../crates/kernel/runtime/tests/original_encoding.rs) owns caller-visible size classification; small-limit comparison tests alone do not prove every caller's wiring. |

## Local cost measurements

Measured on 2026-09-16 in the default Nix shell, with warm build artifacts and the default
unoptimized test profile. Each test ran alone twice; the table reports libtest execution seconds,
excluding compilation and Nix startup. Other workspace tasks were active. These are local
comparisons, not uncontended benchmarks. A dash means the unchanged scenario was not individually
remeasured after setup reuse.

| Test | Before setup reuse (seconds) | After setup reuse (seconds) |
| --- | --- | --- |
| `bound_routes_reject_duplicates_and_over_capacity` | 0.00, 0.00 | — |
| `snapshot_starts_from_exact_revision_and_returns_holdings` | 12.60, 12.84 | — |
| `snapshot_progresses_after_an_interrupted_read` | 10.29, 10.43 | — |
| `snapshot_records_a_durable_provider_failure` | 9.43, 9.39 | — |
| `snapshot_token_holdings_and_typed_read_failures` | 32.87, 31.92 | 21.40, 22.83 |
| `config_rejects_malformed_unbound_and_forged_rows_before_admission` | 18.70, 18.37 | — |
| `config_delete_does_not_revoke_an_admitted_run` | 10.22, 10.20 | — |
| `indeterminate_start_and_progress_carry_recovery_identity` | 18.84, 18.57 | 12.34, 12.71 |
| `enrichment_keeps_native_and_nonzero_candidates` | 43.39, 43.15 | 34.44, 34.08 |
| `enrichment_publish_rejects_incomplete_forged_and_lost_ack` | 28.04, 27.35 | — |
| `shipping_metadata_constructor_cases_reach_native_materialization_after_schema_admission` | 29.32, 28.74 | — |

Repeat a measurement with:

```bash
nix develop -c cargo test -p mfm-app --test use_cases <test-name> -- --exact
```

A temporary probe timed the public API phases of one native snapshot twice:

| Phase | Seconds |
| --- | --- |
| Compose Application and bind provider | 3.76, 3.77 |
| Parse config document | 0.00091, 0.00094 |
| Import config, including planning | 2.60, 2.67 |
| Start run, including planning and execution | 3.73, 3.81 |

The probe was removed after measurement. The result supports reusing one Application and imported
revision inside a scenario when only the provider outcome changes. Each run still has its own
RunId, and separate tests retain separate mutable backends and providers. No test was removed.
The enrichment scenario now reaches a later balance failure and checks publication rejection, so
its comparison includes stronger coverage. Production caching or schema changes need separate
profiling; these phase measurements do not identify an internal algorithm to change.

Verification: all 11 tests passed together with `nix develop -c cargo test -p mfm-app --test
use_cases` (34.52 seconds), and the three changed scenarios each passed twice individually. Format
and `nix develop -c cargo clippy -p mfm-app --test use_cases -- -D warnings` passed. No production
code, public API, persistence, manifest or task-graph change selects a composed CI run here.

## Organization assessment

Keep one `use_cases` Cargo test target. A module split into config, snapshot, enrichment and
hostile restoration scenarios would improve navigation without compiling shared fixtures into
multiple integration-test binaries. Keep the common provider in one support module and each
hostile Store or repository beside its scenarios; do not introduce a generic fixture framework.

The file split is deferred while concurrent edits are coordinated. The current test names and
file links remain valid. Setup reuse and more precise failure placement do not depend on the split.

## Material uncertainties

Local timings include concurrent workspace activity and are not CI budgets or production latency
estimates. The assumption is that repeated local measurements identify worthwhile setup reductions;
contention and the unoptimized test profile make the absolute costs uncertain. If that assumption
is wrong, the measured speedup will not transfer to CI. Validate CI impact with comparable
before/after task evidence before changing verification budgets.
