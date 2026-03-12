# Replan: `mfm::portfolio::snapshot` as Workflow + Packaged `mfm_cli`

Date: 2026-03-12

## Review Summary

The framework upgrade improved documentation around service readiness and health, but it did not change the core workflow constraint:

- workflows still forward one shared passthrough argv to `preRun`, units, and `postRun`
- workflow units still do not support per-step args, env, or outputs
- the generated docs now explicitly say to use wrapper tasks when an app needs different step args or a strict public stdout contract

Implication:

- `mfm::portfolio::snapshot` should become a public wrapper around an internal workflow
- it should not become a public `workflowRef` directly

## Current Reuse Surface

What is reusable now:

- `task.ops.ready`
- `task.ops.health`
- model-derived service directories and service config env
- internal workflow execution via `run-workflow`

What is still not a usable public surface in this repo:

- per-step workflow args/env/outputs
- public `svc::postgres::*` / `svc::helios::*` apps in the flake app surface

## Problems in Current Snapshot Implementation

The current implementation in `nixfied/project/module.nix` still carries too much app-local orchestration:

- Postgres startup/bootstrap logic is in `task.mfm.portfolio.services-start`
- Helios startup/checkpoint/bootstrap logic is also in `task.mfm.portfolio.services-start`
- `mfm_cli` is run through `cargo run`, so the snapshot path pulls in the Rust toolchain at runtime
- the public task owns stdout correctly, but orchestration is still more monolithic than necessary

## Replanned Target

### Public Surface

Keep:

- public `task.mfm.portfolio.snapshot`

Responsibility:

- parse `--help` / `<ADDRESS>`
- create temp session paths
- export shared workflow env:
  - `MFM_SNAPSHOT_ADDRESS`
  - `MFM_SNAPSHOT_HANDOFF_FILE`
  - `MFM_SNAPSHOT_RESULT_FILE`
- reserve stdout for the final JSON payload
- invoke internal workflow
- validate final JSON envelope
- replay only the final JSON to stdout

This keeps the existing public contract stable.

### Internal Workflow

Add:

- `workflow.mfm.portfolio.snapshot`

Shape:

1. `preRun.tasks`
   - `task.mfm.portfolio.snapshot.preflight`
2. units
   - `postgres-start`
   - `postgres-ready`
   - `helios-start`
   - `helios-ready`
   - `snapshot-exec`
3. `postRun.tasks`
   - `task.mfm.portfolio.snapshot.services-stop`
   - `alwaysRun = true`

Reason:

- this is the most "proper workflow" shape available under current framework semantics
- task-specific data moves through shared env/file paths created by the public wrapper

## Task Reorganization

Replace the current large `services-start` task with smaller wrapper tasks:

- `task.mfm.portfolio.snapshot.preflight`
  - validate required env
  - reject `MFM_KEEP_SERVICES`
  - enforce `HELIOS_NETWORK=mainnet`
  - ensure session file env is present

- `task.mfm.portfolio.snapshot.postgres-start`
  - start or reuse Postgres
  - write ownership metadata into handoff file

- `task.mfm.portfolio.snapshot.postgres-ready`
  - call `task.ops.ready --service postgres --source local`

- `task.mfm.portfolio.snapshot.helios-start`
  - start or reuse Helios
  - derive checkpoint if needed
  - update handoff ownership metadata

- `task.mfm.portfolio.snapshot.helios-ready`
  - call `task.ops.ready --service helios --source local`

- `task.mfm.portfolio.snapshot.exec`
  - reconstruct runtime env for the CLI
  - run packaged `mfm_cli`
  - write JSON only to `MFM_SNAPSHOT_RESULT_FILE`

- `task.mfm.portfolio.snapshot.services-stop`
  - stop only services owned by this invocation
  - respect ownership/cleanup markers from handoff

## `ready` / `health` Usage

Use `task.ops.ready` directly for dedicated wrapper tasks only:

- one task for Postgres ready
- one task for Helios ready

Do not try to use bare `task.ops.ready` in workflow `preRun` for both services at once:

- one service selector is accepted per invocation
- workflow phase tasks still share one argv

`task.ops.health` is still useful for diagnostics and CI, but it is not necessary in the hot path of snapshot execution.

## Minimal `mfm_cli` Packaging

### Goal

Stop using:

- `cargo run -q -p mfm --bin mfm_cli -- ...`

Instead:

- package a minimal `mfm_cli` binary once
- use that package in snapshot and CI mainnet paths

### Proposed Package

Add a package in project config, for example:

- `conf.packages.mfm-cli`

Implementation direction:

- use `pkgs.rustPlatform.buildRustPackage`
- build from workspace root with `Cargo.lock`
- only build:
  - package `mfm`
  - binary `mfm_cli`

Expected shape:

- `pname = "mfm-cli"`
- `src = ../..`
- `cargoLock.lockFile = ../../Cargo.lock`
- `cargoBuildFlags = [ "-p" "mfm" "--bin" "mfm_cli" ]`
- `mainProgram = "mfm_cli"`

Use cases:

- `task.mfm_cli`
- `task.mfm.portfolio.snapshot.exec`
- `task.ci.mainnet-portfolio-snapshot-helios`

### Expected Benefits

- smaller runtime closure for snapshot execution
- no `cargo run` startup cost
- no Rust toolchain requirement in the snapshot runtime path
- clearer separation between build-time and runtime concerns

## Recommended Implementation Order

1. Add packaged `mfm_cli`.
2. Switch `task.mfm_cli` to the packaged binary.
3. Add `task.mfm.portfolio.snapshot.exec` using the packaged binary.
4. Split current `services-start` into smaller workflow-oriented tasks.
5. Add `workflow.mfm.portfolio.snapshot`.
6. Reduce public `task.mfm.portfolio.snapshot` to:
   - parse args
   - create env/file handoff
   - run workflow
   - validate/replay JSON
7. Switch `task.ci.mainnet-portfolio-snapshot-helios` to the packaged binary too.

## Non-Goals

- do not make the public app itself a direct workflow app
- do not depend on undocumented builder-only hook env
- do not persist secret-bearing env in handoff files
- do not widen the JSON contract beyond the current `mfm_cli` envelope

## Outcome

After this replan:

- `mfm::portfolio::snapshot` is still a thin public JSON app
- orchestration is represented as a real internal workflow
- service checks reuse `task.ops.ready`
- CLI execution uses a packaged binary instead of `cargo run`
- the remaining custom logic is limited to ownership/handoff and public stdout control
