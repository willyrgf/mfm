# FSM scheduler closeout validation, 2026-06-18

This file records branch-local evidence for the RFC scheduler refactor closeout. It is intentionally
small and points to exact commands rather than relying on transient terminal scrollback.

## Focused Cargo validation

Passed locally after closing the reviewer gaps:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p mfm-runtime`
- `cargo check -p mfm-runtime -p mfm-transports-proof -p mfm-adapters-portfolio -p mfm-app`
- `cargo test -p mfm-transports-proof`
- `cargo test -p mfm-adapters-portfolio`
- `cargo test -p mfm-app`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- `cargo test -p mfm-integration-tests --lib typed_certified_slice_acceptance_passes_required_contract`
- `cargo test -p mfm-integration-tests --lib compensated_certified_slice_replays_and_reports_public_status`
- `cargo test -p mfm-integration-tests --lib replay_artifacts_reject`

The runtime test set includes regression coverage for:

- storage/artifact authority failures leaving started attempts open for recovery
- open-attempt interruption being committed by `AttemptRecoveryLifecycle`
- resume-time runner executable identity enforcement
- capability implementation bindings missing from the runtime registry
- capability implementation descriptors differing from the certified descriptor
- saga terminal proofs minted from the current post-start prefix
- async resource-lane blocking before artifact staging

## Nix CI validation

Passed locally after closing the reviewer gaps:

- Command: `NIXFIED_STATE_DIR=/tmp/mfm-nixfied-ci.Bxhm7U nix run .#ci`
- Run id: `run-1003492-1781795941626317257`
- Computed model hash:
  `d6fdf033ef9f6dce4b2236d94c27c96cffb7d1ec57cd7189dcf4cb8e56aca34a`
- Durable summary JSON:
  `docs/validation/fsm-scheduler-closeout-2026-06-18-run-summary.json`

Passed nodes:

- `ci.check.fmt`
- `ci.check.clippy`
- `ci.check.cargo-metadata-contract`
- `ci.check.architecture-namespace-contract`
- `ci.test.workspace-tests.nextest-run`
- `ci.test.workspace-tests.doc-tests`
- `ci.parity-cli-keystore`
- `ci.test-db.postgres-sqlx-check`
- `ci.test-db.parity-postgres-state-events`
- `ci.test-db.parity-postgres-rest-api`
- `ci.parity-reth-contracts`
- `ci.parity-reth-portfolio`

The managed Postgres and Reth parity suites are evidenced by the Nix CI run; no external
`DATABASE_URL` was required in this shell.

## Prepared-boundary closeout addendum

Additional focused validation after closing the prepared-boundary and public-status coverage gaps:

- `cargo fmt --all -- --check`
- `cargo clippy -p mfm-runtime -p mfm-app --all-targets -- -D warnings`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-app`
- `cargo test -p mfm-integration-tests --test rest_api_run_control`

These checks cover phase-aware side-effect interruption before `SideEffectInvocationPrepared`,
explicit open-attempt recovery operational blocks, concrete side-effect recovery classifications,
ordinary attempt phase witnesses, live REST run-status coverage, and store-backed resource-lane
public status projection. They are focused Cargo checks for this addendum; they do not replace the
earlier recorded Nix CI run.

## Post-review merge-readiness addendum

Additional post-review cleanup after the multi-agent merge review:

- Async attempt lifecycle now prechecks resource lanes through the store-owned status projection
  before staging runner artifacts. The regression test now uses inline side-effect artifact bytes so
  it fails if staging happens before the lane block.
- Nixfied `test-db` now includes the CLI Postgres status contract
  (`cargo test -p mfm --features parity-tests --test status_contract_postgres -- --nocapture`) under
  managed Postgres.
- CLI, REST, app rustdoc, RFC, plan, and design docs now distinguish persisted live resource-lane
  status from internal no-append scheduler waiters, and document `scheduler_status`.

Focused validation run after this addendum:

- `cargo fmt --all -- --check`
- `cargo test -p mfm-runtime async_runtime_blocks_exclusive_lane_before_staging_artifact -- --nocapture`
- `cargo test -p mfm-runtime`
- `cargo check --workspace --all-targets --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm --test json_output_integration test_run_status_json_contract_exposes_attempt_dispositions_and_saga_blocks`
- `cargo test -p mfm-integration-tests --test rest_api_run_control`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-app async_status_with_projection_filters_unreferenced_global_resource_lanes`
- `cargo test -p mfm-app semantic_status_exposes_resource_lanes_from_store_projection`
- `nix run .#model-check`
- `NIXFIED_STATE_DIR="$(mktemp -d)" nix -L -v --log-format bar-with-logs run .#ci`

The first full `.#ci` rerun after this addendum hit host disk exhaustion while writing Nixfied task
summary output, after the earlier nodes had passed. After removing generated old Nixfied temp state,
the full service-backed gate passed at HEAD:

- Run id: `run-1750288-1781830168956305202`
- Computed model hash:
  `04800959b8ca2f7103bb33049d37f4ee4f79a74d74bdced5f972a301ee587c04`
- Managed services: Postgres at `127.0.0.1:28080`, Reth at `127.0.0.1:28082`
- Newly added `ci.test-db.parity-cli-postgres-status` passed under managed Postgres.
