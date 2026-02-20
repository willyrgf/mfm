# Nixfied (Model-First)

Nixfied is a model-driven Nix framework.

The canonical source of truth is `nixfiedModel`, compiled from typed modules and exposed through stable flake app surfaces.

## Quick Start

```bash
nix run .#help
nix run .#dev
nix run .#test
nix run .#ci -- --summary
```

## Core Commands

Model-generated app surfaces:

- `nix run .#build`
- `nix run .#check`
- `nix run .#check-ports`
- `nix run .#ci`
- `nix run .#dev`
- `nix run .#format`
- `nix run .#framework::install`
- `nix run .#framework::test`
- `nix run .#health`
- `nix run .#ports`
- `nix run .#ready`
- `nix run .#test`
- `nix run .#test-isolation`
- `nix run .#validate-env`

Dispatcher surfaces:

- `nix run .#run-task -- <task-id> [-- ...]`
- `nix run .#run-workflow -- <workflow-id> [-- ...]`
- `nix run .#run-workflow-parallel -- <workflow-id> [-- ...]`

Introspection surfaces:

- `nix run .#model`
- `nix run .#stateHash`
- `nix run .#tasks`
- `nix run .#task::<id>`
- `nix run .#schema`

## Configuration Surface

Primary edit points:

- `nixfied/project/conf.nix` for project identity, envs, ports, and module settings.
- `nixfied/project/module.nix` for task/workflow modeling and exposed app names.
- `nixfied/modules/` for typed module options.

Environment defaults:

- `NIX_ENV=0`
- `PROJECT_ENV=dev`

## Architecture Summary

- Modules: `lib.evalModules` + typed options from `nixfied/modules/*.nix`.
- Compiler: deterministic pass pipeline in `nixfied/compiler/*.nix`.
- Hashing: `stateHash = sha256(toCanonicalNix(model))`.
- Runner: single dispatcher for task/workflow execution.
- Registry: append-only NDJSON event stream with replay support.

## Docs

- `ARCHITECTURE.md` for high-level architecture.
- `docs/DETAILED.md` for model, runtime, workflow, and registry contracts.
- `REDESIGN.md` for redesign and migration context.
- `docs/repo-map.md` for a repository-oriented index.
- `docs/modules/README.md` for module-specific configuration notes.
- `tests/framework/README.md` for deterministic framework validation.

## API

Primary library entrypoint:

```nix
nixfied.lib.mkNixfied {
  system = "x86_64-linux";
  projectRoot = ./.;
  projectModules = [ ./nixfied/project/module.nix ];
  extraModules = [ ];
  localOverrides = [ ];
}
```

Returned attributes:

```nix
{
  model;
  stateHash;
  tasks;
  workflows;
  apps;
  packages;
  checks;
  devShells;
}
```

## Output Contract

User-facing shell task output should be plain ASCII and prefix-based:

- `INFO:`
- `WARN:`
- `ERROR:`
- `OK:`
- `SKIP:`

## Parallel Worker Reuse Example

Use the parallel runner for multiple workflow runs, and cap worker reuse with `NIXFIED_CI_MAX_WORKERS` (or `CI_MAX_WORKERS`):

```bash
# Run a workflow with its modeled maxWorkers limit.
nix run .#run-workflow-parallel -- workflow.test.parallel.smoke --summary

# Reuse the same parallel runner for another workflow, capped to 2 workers.
NIXFIED_CI_MAX_WORKERS=2 nix run .#run-workflow-parallel -- workflow.ci.full --summary
```

## CI Parallel Integration

`nix run .#ci` uses the CI workflow model, with `execution.parallel = true` in `workflow.ci.*`.
You can cap concurrency with `NIXFIED_CI_MAX_WORKERS` (or `CI_MAX_WORKERS`):

```bash
NIXFIED_CI_MAX_WORKERS=2 nix run .#ci -- --mode full --summary
```

For serial debugging, override the workflow setting:

```bash
NIXFIED_WORKFLOW_PARALLEL=0 nix run .#ci -- --mode full --summary
```

One extra note: GitHub Actions currently runs `nix flake check` + `nix run .#framework::test -- --summary`.
