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
