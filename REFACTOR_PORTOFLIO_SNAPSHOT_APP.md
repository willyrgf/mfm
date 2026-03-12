# Refactor `mfm::portfolio::snapshot` App

## Purpose

Handoff for rewriting `nix run .#mfm::portfolio::snapshot -- <ADDRESS>` so it works on the executor runtime, preserves the current JSON output contract, and stops depending on builder-only hook env.

## Recommendation

Keep the Rust portfolio execution path unchanged.

Refactor the Nixfied app so that:

- `portfolio_tracker` remains the domain op.
- `mfm_cli portfolio snapshot` remains the output-contract authority.
- service/bootstrap logic moves out of one large shell blob into explicit internal task surfaces.
- the public app becomes a thin operational wrapper over those internal tasks.

Do not move Postgres or Helios lifecycle into Rust ops/states.

## Why This Is The Best Fit

Architecture constraints already point this way:

- binaries stay transport-only: `docs/architecture.md:86`
- ops stay planning-only: `docs/architecture.md:116`
- states execute through explicit IO boundaries: `docs/redesign.md:62`
- `portfolio_tracker` is already the right op and already uses reusable EVM read states plus justified local output/report states: `docs/ops-and-states.md:47`, `docs/ops-and-states.md:69`, `docs/ops-and-states.md:81`

The existing Rust path is already thin and correct:

- CLI command only parses args and calls the app service: `bin/cli/src/commands/portfolio/snapshot.rs:10`
- app service only validates env, starts `portfolio_tracker`, and extracts the final report: `crates/app/src/lib.rs:856`
- event store bootstrap already requires `DATABASE_URL` before a run can even start: `crates/app/src/lib.rs:341`

That means service lifecycle is a pre-run operational concern, not op/state logic.

## What Broke

`task.mfm.portfolio.snapshot` is currently defined as an executor task in `nixfied/project/module.nix:714`, but its shell body still assumes builder/runtime hook env:

- `SLOT_INFO_JSON`
- `SVC_POSTGRES_ENSURE`
- `SVC_POSTGRES_STOP`
- `SVC_HELIOS_ENSURE`
- `SVC_HELIOS_STOP`

Those assumptions are visible in `nixfied/project/module.nix:767` and `nixfied/project/module.nix:794`.

The executor runtime does not inject those variables. It injects:

- model-derived port env such as `POSTGRES_PORT` and `HELIOSRPC_PORT`:
  `nixfied/framework/runtime/env-sandbox.nix:806`
- per-service runtime dirs such as `NIXFIED_SERVICE_POSTGRES_DATA_DIR` and
  `NIXFIED_SERVICE_HELIOS_LOG_DIR`: `nixfied/framework/runtime/env-sandbox.nix:823`
- executor self-entrypoint `NIXFIED_EXECUTOR_SELF` for calling other compiled
  tasks/workflows from inside a task

Important nuance:

- generic executor tasks inherit normalized model port names (`heliosRpc` ->
  `HELIOSRPC_PORT`)
- service-local helpers may derive aliases like `HELIOS_RPC_PORT`, but those are
  not ambient task env by default
- a refactor should alias `HELIOSRPC_PORT` to `HELIOS_RPC_PORT` locally only
  where a helper actually requires that spelling

Direct repro already showed the wrapper fails immediately with:

- `ERROR: SLOT_INFO_JSON is not available in the snapshot runtime`

This is the primary breakage.

Separate issue:

- foreground/background failure observability is also weak
- some failed runs only surfaced `exitCode: 1` in the process registry with no useful terminal output

That logging problem should be fixed too, but it is not the root cause.

## Current Public Surface vs Actual Surface

The docs currently imply a process-first service surface:

- `README.md:104` says service apps like `service::minio::start` exist
- `README.md:144` tells users to start reusable services via `service::*::start`

That does not match the current flake exports for this repo:

- `nix run .#service::postgres::start -- --help` fails
- `nix run .#service::helios::start -- --help` fails

What is actually reusable and executor-safe today:

- `ready`: `nix run .#ready -- --service <name> --source local`
- `health`: `nix run .#health -- --service <name> --source local`
- `ports`: `nix run .#ports`
- `services`: `nix run .#services`
- hidden task `task.ci.services-start`: starts Postgres/MinIO/Reth/Helios for CI
- hidden task `task.ci.services-stop`: stops those CI services

Relevant code:

- `task.ci.services-start`: `nixfied/project/module.nix:1442`
- `task.ci.services-stop`: `nixfied/project/module.nix:1777`

Important limitation:

- `task.ci.services-start` is technically reusable, but semantically wrong for this app because it is CI-shaped, starts extra services, writes CI artifacts, and uses the CI Helios shim path.

## Options Considered

### 1. Re-inject `SLOT_INFO_JSON` and `SVC_*` into executor

Status:

- viable compatibility patch
- not the redesign

Pros:

- smallest code change
- likely restores the app quickly

Cons:

- keeps the monolithic wrapper design
- preserves hidden coupling to builder-only service hooks
- does not improve structure

### 2. Move service orchestration into Rust CLI or `crates/app`

Status:

- not recommended for this change

Pros:

- one place to manage bootstrap logic across surfaces

Cons:

- violates current thin-layer intent
- grows Rust transport/app code into a local process manager
- still has a boot-order problem because `DATABASE_URL` is required before the run starts

### 3. Move service orchestration into Rust op/state logic

Status:

- wrong design

Pros:

- none that outweigh the contract mismatch

Cons:

- violates the op/state boundary
- ambient process/network/bootstrap behavior does not belong in the op planner
- the run cannot bootstrap the event store it needs before `RunStarted`

### 4. Rewrite the app into thin operational tasks

Status:

- recommended

Pros:

- fixes the executor mismatch cleanly
- keeps Rust domain logic unchanged
- reuses executor-safe surfaces that already exist
- gives us clear failure points and logging boundaries

Cons:

- still requires some project-local service start logic because executor-safe `svc::*::start` is not exposed today

## Recommended Design

### Summary

Refactor `mfm::portfolio::snapshot` into:

- one thin public app
- two hidden internal tasks
- optional internal workflow if helpful for CI/reuse

The public app should remain responsible only for:

- help/arg validation
- allocating temp paths for handoff/output files
- invoking internal tasks
- constructing the runtime env needed by `mfm_cli`
- validating the final JSON envelope
- replaying the final JSON payload to the original stdout

The public app should not directly own:

- slot info resolution
- hidden `SVC_*` hook lookup
- Postgres startup logic
- Helios startup logic
- service ownership policy logic
- Helios sync polling implementation

### Internal Task Shape

Recommended hidden tasks:

1. `task.mfm.portfolio.services-start`
2. `task.mfm.portfolio.services-stop`

Optional:

3. `task.mfm.portfolio.snapshot.exec`
4. `workflow.mfm.portfolio.snapshot`

### `task.mfm.portfolio.services-start`

Responsibilities:

- use executor-native env only
- resolve runtime port aliases explicitly when needed (`POSTGRES_PORT`,
  `HELIOSRPC_PORT`, and local `HELIOS_RPC_PORT="$HELIOSRPC_PORT"` only if a
  helper expects that spelling)
- start or reuse Postgres
- start or reuse real Helios mainnet
- run readiness checks
- emit stable handoff data for later cleanup/diagnostics

Inputs:

- inherited `POSTGRES_PORT`
- inherited `HELIOSRPC_PORT`
- inherited `NIXFIED_SERVICE_*` dirs
- `HELIOS_NETWORK` defaulted to `mainnet`
- `HELIOS_EXECUTION_RPC_URL` defaulted from project config
- `HELIOS_CONSENSUS_RPC_URL` defaulted from project config
- process policy envs:
  - `SERVICE_REUSE_POLICY`
  - `SERVICE_OWNER_SCOPE`
  - `SERVICE_DISCOVERY_SCOPE`

Implementation notes:

- use the CI service-start task as the source for executor-safe startup patterns and readiness checks: `nixfied/project/module.nix:1463`
- prefer framework `ready`/`health` surfaces over duplicating a project-local
  Helios sync loop
- Helios readiness is already `strict` in this repo, which means the reusable
  readiness surface already checks `eth_blockNumber`, rejects shim/unknown
  sources, and requires `eth_syncing == false`:
  - `nixfied/framework/core/service-config.nix:541`
  - `nixfied/project/conf.nix:349`
- do not depend on `SLOT_INFO_JSON`
- do not depend on `SVC_*`
- do not boot MinIO or Reth
- do not use the CI Helios shim

Output:

- write a small JSON or env handoff file under a runtime temp dir
- keep the handoff non-secret
- include:
  - `cleanup_required`
  - any ownership markers needed by stop logic
  - useful non-secret diagnostics such as Helios log path/dir

Do not persist:

- `DATABASE_URL`
- `MFM_EVM_RPC_URL`
- `MFM_EVM_RPC_SOURCES_JSON`
- `MFM_EVM_RPC_PREFERRED_ORDER`
- `MFM_EVM_RPC_SOURCE_ID`

### `task.mfm.portfolio.snapshot.exec`

Responsibilities:

- source the non-secret handoff metadata
- reconstruct volatile env in-process
- run the CLI
- write the CLI stdout payload to a designated output file
- print progress/errors to stderr
- validate the output is the expected JSON envelope before returning success

Command shape:

- `cargo run -q -p mfm --bin mfm_cli -- --output-format json portfolio snapshot "$ADDRESS" --chain-id 1`

Notes:

- do not make this task the final stdout authority
- if this task exists at all, the parent task must still replay the validated
  output file to the original stdout
- validating "is JSON" is not enough; validate the current contract shape:
  - top-level `status`
  - top-level `data.feature_id == "portfolio.snapshot"`
  - top-level `data.result` remains the existing `PortfolioSnapshotResponse`
- the simpler immediate shape is to keep CLI execution in the public task and
  omit this hidden task entirely

### `task.mfm.portfolio.services-stop`

Responsibilities:

- stop only services owned by this invocation
- never stop reused persistent services just because snapshot finished
- use the handoff ownership markers from `services-start`

Notes:

- current wrapper used a `reuse_service_root == runtime_service_root` style heuristic: `nixfied/project/module.nix:847`
- that ownership rule should become explicit in the handoff file rather than recomputed implicitly in multiple places

### Public `task.mfm.portfolio.snapshot`

Responsibilities:

- parse `<ADDRESS>`
- set up temp paths for handoff state and final JSON output
- call internal tasks via `NIXFIED_CALLER_PWD="$PWD" "$NIXFIED_EXECUTOR_SELF" run-task ...`
- reconstruct runtime env for `mfm_cli` locally rather than persisting secret-
  bearing env into a handoff file
- remain the only step that writes the final JSON payload to the user's stdout
- validate the exact JSON envelope before reporting success
- trap cleanup to call stop task when needed
- preserve raw JSON stdout by replaying only the validated final payload

Preferred invocation mechanism:

- use `NIXFIED_CALLER_PWD="$PWD" "$NIXFIED_EXECUTOR_SELF" run-task ...`
- avoid recursive `nix run .#...` from inside the task when possible

Reason:

- executor self-call is already the intended internal reuse path
- it is lighter and keeps the execution inside the compiled model/runtime

## Workflow Or Not?

There are two acceptable shapes.

### A. Public thin task that dispatches to internal tasks

This is the safest immediate design.

Why:

- raw JSON stdout contract is easiest to preserve
- stderr progress is easy to keep
- orchestration stays thin, but output control remains explicit
- the public task can keep fd handling / temp-file validation exactly where the
  current implementation already does it

### B. Public app as `workflowRef`

This is attractive structurally, but only if workflow execution can preserve the exact current user contract:

- raw `mfm_cli` JSON unchanged on stdout
- no summary/noise mixed into stdout

At the moment that has not been validated. If the contract matters more than internal purity, prefer option A first.

Practical recommendation:

- extract the monolith into internal tasks now
- keep CLI execution plus stdout replay in the public task first
- add an internal workflow later if it helps CI or orchestration reuse

## Reuse Points Available Today

Executor-safe and immediately reusable:

- `ready`: service readiness checks
- `health`: service health checks
- `ports`: model-derived ports
- `services`: compiled service inventory
- `NIXFIED_EXECUTOR_SELF`: call internal tasks/workflows

Not executor-safe enough for direct composition today:

- generated `svc::*` start/stop apps
- builder-style `SVC_*` hook env
- `SLOT_INFO_JSON`

Semantically reusable but wrong for this app:

- `task.ci.services-start`
- `task.ci.services-stop`

## Suggested Implementation Order

### Phase 1: fix the app structure without changing Rust

1. Add hidden `task.mfm.portfolio.services-start`
2. Add hidden `task.mfm.portfolio.services-stop`
3. Replace the public app body with thin dispatch logic plus direct CLI execution
4. Remove direct `SLOT_INFO_JSON` and `SVC_*` checks from the public app
5. Keep raw JSON stdout unchanged
6. Only add hidden `snapshot.exec` later if it is strictly file-oriented and does not own stdout

### Phase 2: tighten docs and diagnostics

1. Update `README.md` so public service-start claims match reality
2. Update `docs/helios.md` to describe the new task structure
3. Improve failure output for foreground and background runs

### Phase 3: optional framework follow-up

1. Expose executor-safe per-service start/stop surfaces from the framework
2. Rebuild the project-local snapshot tasks on top of those generic service tasks
3. remove duplicated project-local startup logic once the generic surfaces exist

## Concrete Files Likely To Change

Primary:

- `nixfied/project/module.nix`
- `README.md`
- `docs/helios.md`

Possible framework follow-up, not required for the first refactor:

- `nixfied/framework/runtime/helpers/service-api.nix`
- `nixfied/framework/runtime/env-sandbox.nix`
- service runtime files under `nixfied/framework/runtime/services/`

## Acceptance Criteria

The refactor is done when all of the following are true:

- `nix run .#mfm::portfolio::snapshot -- <ADDRESS>` works under executor runtime
- the app no longer reads `SLOT_INFO_JSON`
- the app no longer requires `SVC_POSTGRES_*` or `SVC_HELIOS_*`
- the app relies on executor-native env plus any explicit local aliases it creates itself
- no handoff file persists `DATABASE_URL` or `MFM_EVM_RPC_*`
- stdout contains only the final JSON payload
- that payload still matches the current `mfm_cli` JSON envelope:
  - top-level `status`
  - top-level `data.feature_id == "portfolio.snapshot"`
  - top-level `data.result` shape unchanged
- stderr shows meaningful progress and failure information
- services are not torn down if they were reused and not owned by the current invocation
- docs no longer claim public surfaces that are not exported

## Validation Plan

Smoke:

- `nix run .#mfm::portfolio::snapshot -- --help`

Foreground:

- `NIX_ENV=0 SERVICE_REUSE_POLICY=same-slot HELIOS_NETWORK=mainnet nix run .#mfm::portfolio::snapshot -- 0x53078a4d5618f98123CFcFCdF5c20cb2Cdf97312`

Service probes:

- `nix run .#ready -- --service postgres --source local`
- `nix run .#ready -- --service helios --source local`
- `nix run .#health -- --service helios --source local`
- `nix run .#ports`

Regression checks:

- confirm no direct grep hits remain for `SLOT_INFO_JSON` or `SVC_POSTGRES_ENSURE` in the public snapshot app body
- confirm no handoff file written by the flow persists `DATABASE_URL` or `MFM_EVM_RPC_`
- confirm invalid args still fail with usage code and no partial JSON output
- confirm successful output still contains the existing top-level `status`/`data` envelope and `data.feature_id == "portfolio.snapshot"`
- confirm background mode still records useful failure logs

## Risks / Open Questions

### 1. Real Helios startup without framework service apps

There is no exported executor-safe `service::helios::start` surface today.

Implication:

- Phase 1 likely needs project-local startup logic for real Helios mainnet
- do not wait on a framework-wide service API refactor unless that work is already underway

### 2. Ownership and cleanup policy

The current cleanup behavior is implicit.

Need a clearer rule for:

- whether this invocation started the service
- whether it is allowed to stop the service
- how that decision is passed from start to stop

Recommendation:

- write explicit ownership fields into the handoff file

### 3. Workflow stdout contract

If a workflow is made public, validate output behavior first.

If workflow execution emits summaries or other text to stdout, keep the public app as a thin command task instead of `workflowRef`.

### 4. Port aliasing in executor tasks

Generic executor tasks inherit normalized model port env such as
`HELIOSRPC_PORT`, not necessarily service-local aliases like `HELIOS_RPC_PORT`.

Recommendation:

- make any aliasing explicit inside the task that needs it

### 5. README mismatch

The README currently advertises service-start surfaces that are not exposed by this repo’s flake.

That should be corrected in the same change or immediately after.

## Useful Reference Anchors

Broken public app:

- `nixfied/project/module.nix:714`

Broken env assumptions:

- `nixfied/project/module.nix:767`
- `nixfied/project/module.nix:794`

Current executor-native env injection:

- `nixfied/framework/runtime/env-sandbox.nix:118`
- `nixfied/framework/runtime/env-sandbox.nix:806`
- `nixfied/framework/runtime/env-sandbox.nix:823`

Framework-ready reusable readiness surfaces:

- `nixfied/modules/operations.nix:846`
- `nixfied/modules/operations.nix:863`
- `nixfied/framework/core/service-config.nix:541`

Executor nested task/workflow support:

- `nixfied/framework/runtime/dispatcher.nix:172`
- `nixfied/framework/presets/selfhost.nix:39`

CI service-start template:

- `nixfied/project/module.nix:1442`

Rust CLI surface:

- `bin/cli/src/commands/portfolio/snapshot.rs:10`
- `bin/cli/src/presentation/output.rs:72`

Rust app service:

- `crates/app/src/lib.rs:856`
- `crates/app/src/lib.rs:1731`

Event store bootstrap dependency:

- `crates/app/src/lib.rs:341`

Architecture contract:

- `docs/architecture.md:86`
- `docs/architecture.md:116`
- `docs/redesign.md:28`
- `docs/redesign.md:62`

## Bottom Line

The redesign fix is:

- do not rewrite `portfolio_tracker`
- do not move service orchestration into Rust ops/states
- do rewrite `mfm::portfolio::snapshot` so the public app stays the stdout authority while service/bootstrap work moves into explicit executor-safe internal tasks

The compatibility patch is “inject missing hook env into executor”.

The redesign is “remove the app’s dependency on hook env entirely”.
