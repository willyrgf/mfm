# Fix CI Slots/Envs Hardening Plan

## Goal

Stabilize parity CI runs (especially `parity-aave-v3-reth`) by removing cross-run `reth` interference, hardening `reth` liveness under reuse, and surfacing actionable failures.

## Root Cause Summary

1. CI setup currently kills `reth` by fixed p2p port `30303`, which can terminate a reused `reth` from another run/slot.
2. `parity-aave-v3-reth` can fail if node lifetime is shorter than step lifetime (infra failure, not blockchain semantics).
3. For the incident analyzed on 2026-02-18, `reth` was terminated by an external SIGTERM source; framework teardown/cleanup logic was ruled out.
4. Chain state reuse with the same root wallet is valid in this pipeline (dynamic nonce handling, receipt-derived addresses, CREATE2 idempotency, and sufficient dev balances).

## Deep Investigation Results (2026-02-18 Incident)

Why `reth` died mid-step:

1. Framework-level causes were ruled out (no between-unit cleanup path, no fixture-owned stop, no `process::stop --scope slot-env` targeting `reth`, no relevant `AUTO_STOP_CONFLICTING` path, no overlapping cross-run slot collision, no step script stop of `reth`).
2. `reth` process exited after receiving SIGTERM from an unknown external source and shut down gracefully (`RC=0` / graceful peer persistence log).
3. Current artifacts do not record signal sender PID, so exact source attribution is not possible from existing diagnostics alone.

Implication:

1. Primary failure mode for this incident is external process liveness loss, not chain-state invalidity.
2. Hardening should prioritize liveness detection/recovery + signal attribution, while still fixing known slot-isolation hazards.

## Service Audit: Slots + Ephemeralness

Scope of investigation: whether services other than `reth` violate slot isolation or ephemeral behavior.

1. `postgres`, `minio`, `helios`, `nginx`, and `supervisor` appear slot-aware in framework lifecycle/config paths (slot-derived ports and slot-scoped service dirs).
2. No additional fixed global listener was found in service lifecycle definitions; the known outlier remains `reth` p2p (`30303`) behavior.
3. CI setup listener cleanup is not only about `reth`; it also kills conflicting listeners for MinIO ports. This is expected cleanup, but must remain strictly slot-scoped.
4. Ephemeralness is not universal by default: base runtime/storage paths can be persistent unless explicitly forced to per-run ephemeral roots in CI steps.
5. Conclusion: the primary slot-isolation bug is still `reth` p2p/global cleanup, but hardening should also enforce slot-safe cleanup patterns and explicit ephemeral roots for parity-critical steps.

## Landing Strategy: Two Commits (Same Branch)

Land this work in **two commits** (not two PRs), in this order:

### Commit 1: CI slot/env lifecycle isolation hardening

Scope:

1. Make `reth` p2p port slot-scoped in framework reth lifecycle.
2. Remove or scope the global `30303` conflicting-listener kill so it cannot kill foreign slot/run `reth`.
3. Keep step-level defense-in-depth: isolate `parity-aave-v3-reth` via per-step service policy override (`never` reuse, local/ephemeral ownership for that step).
4. Add/adjust framework tests for slot-safe service lifecycle behavior.
5. Add guardrails to keep listener cleanup slot-scoped for non-`reth` services too (for example MinIO ports).
6. Ensure parity-critical steps pin explicit ephemeral runtime roots (instead of relying on persistent default base dirs).

Primary files:

1. `nixfied/.framework/reth/lifecycle.nix`
2. `nixfied/project/ci/scripts/setup.nix`
3. `nixfied/project/ci.nix`
4. `nixfied/.framework/internal/test.nix` (as needed)

Expected outcome:

1. No cross-run/slot `reth` termination from CI setup.
2. `parity-aave-v3-reth` does not inherit brittle service reuse behavior.
3. Other services remain slot-safe under cleanup/restart paths, and parity steps use explicit ephemeral roots.

### Commit 2: Reth liveness + diagnostics hardening

Scope:

1. Add `reth` liveness guard before parity deploy/test steps (fast health check + fail-fast reason).
2. Add restart-on-dead behavior for reused `reth` (same slot/env) with bounded retry (single retry).
3. Add safe exec failure metadata (exit code/program/timeout/signal) for `nix_app` path.
4. Include enriched failure details in parity diagnostics output (last health probe, last known pid, termination signal when available).
5. Add lifecycle trap logging improvements to capture signal context (`PID`, signal name/number, timestamp).
6. Keep nonce-serialization as optional follow-up only if future evidence shows real nonce-race incidents.

Primary files:

1. `nixfied/project/ci/scripts/steps/parity-aave-v3-reth.nix`
2. `nixfied/.framework/reth/lifecycle.nix`
3. `crates/machine/src/exec_transport.rs`
4. `crates/ops/common/src/states/nix.rs`
5. `tests/integration/tests/parity_aave_v3_reth_scenario.rs`
6. `nixfied/project/conf.nix` (only if additional probe/log env toggles are included)

Expected outcome:

1. Parity step detects dead `reth` early and recovers once when safe.
2. External process termination incidents become diagnosable from CI artifacts.
3. `exec_failed` diagnostics become actionable without leaking secrets.

## Acceptance Criteria

1. Repeated `parity` runs do not show foreign `reth` kills from setup.
2. `parity-aave-v3-reth` passes reliably under normal CI load.
3. Concurrent CI runs across slots do not terminate each other's `reth`.
4. When failures happen, diagnostics clearly distinguish infra/process exit from protocol/deploy logic errors.
5. No service cleanup path performs cross-slot process kills.
6. Parity-critical steps prove ephemeral runtime roots in logs/artifacts.
7. At least one artifact/log line records reth termination context (signal + pid) when a signal-driven stop occurs.

## Validation Commands

1. `nix run .#check`
2. `nix run .#test`
3. `nix run .#ci -- --parity --summary`
4. Optional stress: run two CI commands concurrently on different slots and verify no cross-slot `reth` termination.

## Suggested Commit Messages

1. `ci/framework: harden slot-env service lifecycle and reth port isolation`
2. `parity/reth: harden reused-node liveness checks and enrich exec failure diagnostics`
