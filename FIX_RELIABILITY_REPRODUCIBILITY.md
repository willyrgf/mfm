# Fix Reliability and Reproducibility

## Goal

Eliminate flaky behavior caused by process I/O scheduling races and shell/platform variance, while preserving current security constraints (especially no secret leakage in persisted error surfaces).

## Problem Summary

The recent flake (`exec_stdin_write_failed` vs `exec_failed`) confirmed we still have timing-sensitive behavior in process execution code.

Current reliability risks are not only test-level:

1. Path allowlist checks use raw string prefixes and can be bypassed with path traversal/symlink tricks.
2. Timeout starts after stdin write, so a blocked stdin write can exceed timeout budget.
3. Timeout handling relies on `kill_on_drop`, which is not explicit enough for deterministic cleanup.
4. `wait_with_output()` buffers full output and can create memory/DoS risk with large stdout/stderr.
5. The same execution pattern exists in both `exec_transport` and `nix-exec`, so fixes should be shared.
6. Some tests still depend on `/bin/sh` behavior and fixed `sleep` timing.

## What We Will Implement and Why

## 1) Canonical path enforcement in `exec_transport`

Implementation:

1. Canonicalize `program_path` before policy validation.
2. Validate allowlist membership against canonical paths, not raw user input.
3. Execute canonical path only.

Why:

1. Prevents prefix-bypass execution like `/nix/store/.../../../bin/echo`.
2. Removes symlink/path normalization ambiguity across OS/filesystem behaviors.
3. Makes allowlist enforcement deterministic and security-correct.

## 2) Deadline that covers entire execution lifecycle

Implementation:

1. Start one deadline at spawn time.
2. Apply it to stdin write + stdin close + process wait.
3. If deadline is exceeded at any stage, fail with timeout semantics.

Why:

1. Prevents hangs where stdin write blocks forever before timeout starts.
2. Makes timeout behavior stable under CI load and scheduler variability.
3. Ensures timeout means total wall-clock budget, not just post-write wait time.

## 3) Explicit timeout termination and reaping

Implementation:

1. On timeout, explicitly kill child process.
2. Explicitly reap/wait the child before returning error.
3. Avoid relying solely on `kill_on_drop` for cleanup ordering.

Why:

1. Reduces zombie/cleanup races.
2. Produces deterministic lifecycle completion across Linux/macOS runners.
3. Prevents post-timeout stragglers affecting later tests or steps.

## 4) Broken-pipe-aware stdin write classification

Implementation:

1. If stdin write fails due to early child exit, check child status immediately.
2. Classify as `exec_failed` with exit metadata when child already exited non-zero.
3. Reserve `exec_stdin_write_failed` for true transport-level failures.

Why:

1. Eliminates the specific race that produced inconsistent error codes.
2. Aligns error classification with user-visible outcome (program exit vs transport failure).
3. Improves debuggability and stability of retry/failure handling.

## 5) Bounded process I/O to avoid memory blowups

Implementation:

1. Enforce max stdin byte size before write.
2. Read stdout/stderr with bounded collectors rather than unbounded `wait_with_output()` buffering.
3. Add explicit overflow error codes and safe metadata (sizes, limits, program path).

Why:

1. Prevents OOM-style failures from large process output.
2. Makes behavior predictable under pathological programs and CI contention.
3. Improves reliability without relaxing security constraints.

## 6) Shared hardened process runner

Implementation:

1. Extract shared process-execution primitive (deadline, bounded I/O, cleanup, classification).
2. Use it in:
   - `crates/machine/src/exec_transport.rs`
   - `crates/collectors/nix-exec/src/lib.rs`

Why:

1. Avoids duplicated race-prone logic.
2. Keeps behavior and error semantics consistent across execution paths.
3. Reduces maintenance drift and future regressions.

## 7) Strict flake allowlist matching

Implementation:

1. Replace raw `starts_with` for flake refs with structured matching.
2. Enforce boundary-aware matching for host/org/repo/ref segments.

Why:

1. Prevents false allowlist matches such as `github:willyrgf/mfm-malicious`.
2. Improves policy correctness and reproducibility of authorization behavior.

## 8) Deterministic process tests

Implementation:

1. Replace shell script timing tests with a Rust helper binary/fixture behavior.
2. Use handshake/block-until-signal mechanics instead of `sleep`.
3. Add regression tests for:
   - stdin broken-pipe classification
   - timeout during write
   - timeout cleanup behavior
   - allowlist traversal bypass attempts
   - oversized stdin/stdout/stderr

Why:

1. Removes dependence on `/bin/sh` differences (`dash` vs `bash`).
2. Removes scheduler-speed assumptions from assertions.
3. Ensures failures are reproducible and actionable.

## Implementation Order

1. `exec_transport` hardening (items 1-5).
2. extract shared runner and migrate `nix-exec` (item 6 + item 7).
3. test harness upgrades and new regression tests (item 8).
4. run full validation (`nix run .#check`, `nix run .#test`, `nix run .#ci -- --mode basic --summary`, then full mode before commit).

## Acceptance Criteria

1. Error classification for early-exit stdin race is deterministic (`exec_failed` with safe exit metadata).
2. Timeout applies to full lifecycle and always leaves no live child process.
3. Path traversal/symlink allowlist bypass attempts are rejected.
4. Large output/input paths fail with deterministic bounded-size errors, not memory-pressure failures.
5. `exec_transport` and `nix-exec` use the same hardened execution semantics.
6. Repeated CI-mode runs show stable results across Linux/macOS.
