# CI Process Race Condition Investigation

Date: 2026-02-21  
Repo: `mfm2` (`/Users/willyrgf/dev/rust/src/github.com/willyrgf/mfm2`)

## Summary

The CI instability observed during cleanup validation was caused by process/runtime collisions, not by the Rust cleanup changes themselves.

The primary issue was overlapping and orphaned Nixfied CI/model processes across repositories sharing the same runtime namespace (`project.id = "mfm"`), which produced:

- stuck/long-running `shell-app-contracts` steps,
- canceled CI steps due to fail-fast propagation,
- intermittent missing-file failures in ephemeral paths.

## Symptoms Observed

- `nix run .#ci -- --mode full --summary` repeatedly appeared to hang around `shell-app-contracts`.
- `shell-app-contracts.log` often stayed empty or only contained:
  - `building ... nixfied-introspect ...`
- CI summaries showed cancellation cascades (fail-fast).
- One recorded run showed `architecture-verify` failing with missing files in ephemeral source paths.

## Key Evidence

1. `shell-app-contracts` runs model introspection:

- File: `nixfied/project/module.nix:804`
- Command in step: `nix run "path:$ROOT"#model >/dev/null`

2. Runtime namespace is shared by project id:

- File: `nixfied/project/conf.nix:33` -> `project.id = "mfm"`
- File: `nixfied/project/conf.nix:180` -> `registryRoot = "/tmp/nixfied-runtime/${project.id}"`

3. No CI/Nix wiring changes were introduced by cleanup:

- `git diff --name-only -- nixfied/project nixfied/.framework flake.nix` returned empty.

4. Direct proof of cross-repo process interference:

- `lsof -p 64458` showed cwd:
  - `/Users/willyrgf/dev/rust/src/github.com/willyrgf/mfm`
- This process was a live `shell-app-contracts` worker while investigation was run from `mfm2`.

5. Evidence of concurrent/orphan runs:

- Multiple simultaneous `nixfied-orchestrator` / `nixfied-executor` chains for `task.ci` (`--mode full` and `--mode basic`) were active concurrently.
- Orphan `nix run path:/private/tmp/mfm-ephemeral-.../source#model` processes with `PPID=1` were present.

6. Concrete failed run artifact showing ephemeral source disappearance:

- File: `/tmp/nixfied-runtime/mfm/artifacts/run-1b3ea6cf9b5e7a33ed57ef5c/architecture-verify.log`
- Failure included:
  - missing `/private/tmp/mfm-ephemeral-XO8TR9/source/crates/app/src/lib.rs`
  - missing `/private/tmp/mfm-ephemeral-XO8TR9/source/crates/states/keystore/src/states/tx.rs`

Those files exist in repo, so the failure pattern matches runtime race/removal, not source-level deletion from cleanup.

## Root Cause Chain

1. Cleanup validation involved repeated CI starts/stops and interrupted runs.
2. Interrupted runs left orphan Nixfied worker/model processes.
3. `mfm` and `mfm2` both use `project.id = "mfm"`, sharing `/tmp/nixfied-runtime/mfm` and related transient state.
4. Processes from different repos/attempts contended on shared runtime artifacts and temporary roots.
5. `shell-app-contracts` and `#model` operations overlapped and blocked/competed, while fail-fast canceled sibling steps.
6. In some runs, ephemeral content paths were no longer valid at read time, yielding false “missing file” diagnostics.

## Why It Appeared “After Cleanup”

Cleanup itself did not introduce CI logic changes.  
The issue surfaced during cleanup because that session generated a high volume of interrupted/overlapping CI runs, which amplified an existing namespace/process isolation weakness.

## Impact

- Unreliable CI signal during validation.
- False attribution risk to code cleanup.
- Time loss from non-deterministic CI behavior.

## Immediate Corrective Actions Taken During Investigation

- Enumerated and terminated orphaned:
  - `nixfied-orchestrator`,
  - `nixfied-executor`,
  - `ephemeral-orchestrator-executor`,
  - `nix run ...#model`,
  - stale `shell-app-contracts` worker processes.
- Verified standalone `nix run .#model` succeeds when run in isolation.

## Recommended Permanent Fixes

1. Isolate runtime namespace per repo:
  - Update `nixfied/project/conf.nix` to use a unique `project.id` for `mfm2` (for example `mfm2`).
2. Add timeout for model introspection in `shell-app-contracts`:
  - wrap `nix run "path:$ROOT"#model` with bounded timeout/fail-fast behavior.
3. Add CI preflight stale-process guard:
  - detect and fail if active `task.ci*` or `#model` workers already exist for same runtime namespace.
4. Operational discipline:
  - avoid concurrent CI runs across repos sharing the same `project.id`.
  - avoid starting a new CI while previous orchestration is still active.

## Verification Plan After Fix

1. Ensure no active stale workers in `/tmp/nixfied-runtime/<project-id>`.
2. Run:
  - `nix run .#check`
  - `nix run .#test`
  - `nix run .#ci -- --mode full --summary`
3. Confirm:
  - `shell-app-contracts.log` completes with explicit pass line,
  - `summary.json` exists and includes no canceled fail-fast artifacts caused by orchestration contention.
