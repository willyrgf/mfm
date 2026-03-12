# Problems Reusing Nixfied for `mfm::portfolio::snapshot`

Date: 2026-03-12

## Goal

Refactor `mfm::portfolio::snapshot` to look more like CI:

- reuse framework service lifecycle instead of app-local startup glue
- reuse `task.ops.ready` / `task.ops.health`
- express orchestration as steps (`services up`, `ready/check`, `exec`, `services down`)
- keep the public app contract unchanged:
  - raw validated `mfm_cli --output-format json` payload only on stdout

## What Blocked Direct Reuse

### 1. Workflow steps all receive the same passthrough argv

Evidence:

- `nixfied/framework/runtime/executor.nix:1002`
- `nixfied/framework/runtime/executor.nix:1018`
- `nixfied/framework/runtime/executor.nix:1598`
- `nixfied/framework/runtime/executor.nix:1725`
- `nixfied/framework/runtime/executor.nix:1749`

Problem:

- `run-workflow` forwards one shared `passthrough_args` list to `preRun`, workflow units, and `postRun`.
- There is no per-unit `args`, `env`, or `exports` block in `nixfied/schemas/workflow-contract.json`.

Impact on `portfolio::snapshot`:

- `services-start` needs `--handoff-file <path>`
- `ready` would need `--service postgres` and `--service helios`
- `snapshot.exec` needs `<ADDRESS>`
- `services-stop` needs the same `--handoff-file <path>`

That cannot be expressed cleanly as one workflow today without extra wrapper tasks.

Requested capability:

- per-unit fixed args/env in workflow definitions
- or typed step outputs that later steps can consume

### 2. No structured output handoff between workflow steps

Evidence:

- `nixfied/schemas/workflow-contract.json`
- `nixfied/project/module.nix:492`

Problem:

- Workflows model task ordering, but not step outputs.
- There is no first-class way for one step to return a file path / JSON object / env map to another step.

Impact on `portfolio::snapshot`:

- service ownership and cleanup metadata had to be written to an ad hoc handoff file
- the public wrapper still has to create temp paths and manage teardown explicitly

Requested capability:

- step outputs / exports
- dependency inputs derived from prior steps
- a sanctioned temp artifact handoff mechanism

### 3. `task.ops.ready` / `task.ops.health` are reusable, but too coarse for this workflow

Evidence:

- `nixfied/modules/operations.nix:189`
- `nixfied/modules/operations.nix:279`
- `nixfied/modules/operations.nix:865`
- `nix run .#ready -- --help`

Problem:

- `ready` defaults to `--service all`.
- `--source` requires `--service`.
- In a workflow, phase tasks share the same argv, so one phase cannot cleanly run:
  - `ready --service postgres --source local`
  - `ready --service helios --source local`

Impact on `portfolio::snapshot`:

- generic `task.ops.ready` could not be dropped directly into a CI-style `preRun` phase
- app-local helper tasks were still needed

Requested capability:

- workflow phases that can declare multiple task invocations with their own args
- or a service list selector for `ready` / `health`
- or a first-class "ready these services" step primitive

### 4. Public machine-output apps still need a stdout-owning wrapper

Evidence:

- `bin/cli/src/presentation/output.rs`
- `bin/cli/src/commands/portfolio/snapshot.rs`
- `crates/app/src/lib.rs:1731`
- `nixfied/framework/runtime/helpers/shell-contract.nix:422`
- `nixfied/framework/runtime/executor.nix:1680`

Problem:

- `mfm::portfolio::snapshot` is a public machine-output app.
- Its contract is stricter than "valid JSON":
  - `.status == "success"`
  - `.data.feature_id == "portfolio.snapshot"`
  - `.data.result` exists
- Workflow runners are batch-oriented and summary/event oriented.
- There is no framework feature to say "run these steps, but publish only the final task's stdout as the app output".

Impact on `portfolio::snapshot`:

- the public task still has to own fd routing, result capture, validation, and final stdout replay

Requested capability:

- a JSON-safe workflow/app mode
- or an app wrapper primitive that can designate one step as the public stdout result

### 5. Service lifecycle reuse is not a first-class task/workflow primitive

Evidence:

- `nixfied/framework/runtime/helpers/helpers.nix:414`
- `nixfied/framework/runtime/helpers/helpers.nix:477`
- `nixfied/framework/runtime/helpers/service-policy.nix`

Problem:

- The framework has low-level shell helpers:
  - `start_service_should_register_cleanup`
  - `start_service`
  - service policy inference/validation helpers
- But there is no high-level reusable task/workflow primitive like:
  - "start configured services X/Y"
  - "honor current `SERVICE_*` policy"
  - "return ownership / cleanup metadata"
  - "stop only what this invocation owns"

Impact on `portfolio::snapshot`:

- service policy inference and ownership handoff had to be rebuilt inside project code

Requested capability:

- `task.ops.services-start`
- `task.ops.services-stop`
- or a workflow-native managed-service step abstraction

### 6. Port env naming is surprising across generic tasks vs service scripts

Evidence:

- `nixfied/framework/runtime/env-sandbox.nix:118`
- `nixfied/framework/runtime/env-sandbox.nix:815`
- `nixfied/framework/runtime/services/helios/lifecycle.nix:36`
- `nix run .#ports`

Problem:

- Generic executor env is derived from model keys.
- `heliosRpc` becomes `HELIOSRPC_PORT`.
- Some service scripts also synthesize `HELIOS_RPC_PORT`.

Impact on `portfolio::snapshot`:

- assumptions based on `HELIOS_RPC_PORT` were wrong for generic task execution
- app code had to explicitly map `HELIOSRPC_PORT -> HELIOS_RPC_PORT`

Requested capability:

- clearer framework docs for env normalization
- or stable aliases for common service port names

### 7. Public `service::*::start` apps are not guaranteed to exist

Evidence:

- `nix run .#help`
- `nix run .#services`
- `nix run .#service::postgres::start -- --help` fails in this repo

Problem:

- The repo exposes model/service introspection and `ready` / `health`.
- It does not expose public service start apps for Postgres or Helios.

Impact on `portfolio::snapshot`:

- handoff guidance that assumed `service::postgres::start` / `service::helios::start` was wrong

Requested capability:

- document whether public service apps are optional
- or provide a standardized generated surface for start/stop when services are enabled

## Current Project Workaround

Implemented in `nixfied/project/module.nix`:

- public `task.mfm.portfolio.snapshot`
- hidden `task.mfm.portfolio.services-start`
- hidden `task.mfm.portfolio.services-stop`

Why:

- preserves the public JSON stdout contract
- reuses framework `task.ops.ready` inside the hidden startup task
- keeps secret-bearing env out of the persisted handoff file

This is workable, but it is still project-local glue rather than a clean reuse of generic Nixfied orchestration.

## Smallest Useful Framework Improvements

1. Allow workflow units and phase tasks to declare their own args/env.
2. Add step outputs/exports so later steps can consume structured data.
3. Provide first-class managed-service start/stop workflow primitives.
4. Add a way to target a list of services in `ready` / `health`.
5. Add a machine-output workflow mode that can publish one step's stdout as the final app result.
6. Document or alias port env normalization (`HELIOSRPC_PORT` vs `HELIOS_RPC_PORT`).
7. Clarify whether public `service::*` apps should be expected in projects.

## Commands Used While Investigating

```bash
nix run .#help
nix run .#tasks
nix run .#services
nix run .#ports
nix run .#ready -- --help
nix run .#mfm::portfolio::snapshot -- --help
nix run .#run-task -- task.mfm.portfolio.services-start --help
nix run .#run-task -- task.mfm.portfolio.services-stop --help
```
