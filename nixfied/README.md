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
- `nix run .#framework::upgrade`
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
- `nix run .#runs [-- <run-id>]`
- `nix run .#stop-run -- <run-id>`
- `nix run .#stop-all-runs`

Introspection surfaces:

- `nix run .#docs`
- `nix run .#features`
- `nix run .#model`
- `nix run .#stateHash`
- `nix run .#services`
- `nix run .#tasks`
- `nix run .#task::<id>`
- `nix run .#schema`

## Configuration Surface

Primary edit points:

- `nixfied/project/conf.nix` for project identity, envs, ports, and module settings.
- `nixfied/project/module.nix` for project-layer composition, plus `nixfied/project/{tasks,workflows}.nix` for project-owned command surfaces.
- `nixfied/modules/` for typed module options.

Environment defaults:

- `NIX_ENV=0`
- `PROJECT_ENV=dev`

Service configuration and selectors:

- Configure services in `nixfied/project/conf.nix` under `services.<name>` (`enable`, `ports`, `sources`, `defaultSource`).
- Service probes accept composable selectors on health/readiness checks: `--service <name|all>` and optional `--source <key>`.
- `nix run .#health -- --service <name|all> [--source <key>]`
- `nix run .#ready -- --service <name|all> [--source <key>]`
- See `docs/modules/README.md` for service-specific configuration details.

Ephemeral execution defaults (configured in `nixfied/project/conf.nix`):

- `ephemeral.copyMode = "git-files"`: copies tracked + non-ignored untracked files.
- `ephemeral.excludePatterns = [...]`: fallback excludes for `static-excludes` copy mode.
- `ephemeral.keepFailures = true`: failed ephemeral roots are retained for debugging.
- `ephemeral.maxFailedRoots = 8`: cap on retained failed roots.
- `ephemeral.maxFailedRootAgeHours = 72`: age-based pruning for retained failed roots.
- `ephemeral.maxCopyBytes = 0`: disabled by default; set `> 0` to enforce a pre-copy size cap.
- `ephemeral.minFreeBytesAfterCopy = 0`: disabled by default; set `> 0` to enforce free-space floor after copy estimate.

Ephemeral runtime behavior:

- Success path: ephemeral root is cleaned.
- Failure path: root is preserved (or dropped if `keepFailures = false`), then pruned by age/count policy.
- Budget checks run before copy and fail fast with `ERROR:` if limits are exceeded.

## Architecture Summary

- Modules: `lib.evalModules` + typed options from `nixfied/modules/*.nix`.
- Compiler: deterministic pass pipeline in `nixfied/compiler/*.nix`.
- Hashing: `stateHash = sha256(toCanonicalNix(model))`.
- Runner: dispatcher routes to orchestrator, then executor (`dispatcher -> orchestrator -> executor`).
- Registry: append-only NDJSON event stream with replay support.
- Ownership: framework-owned code lives under `nixfied/framework/{core,runtime,install,presets}`; `nixfied/project/` is the downstream composition/customization layer. `nixfied/{lib,install,runner,registry}` remain transitional compatibility shims, and new internal code should prefer canonical `nixfied/framework/...` paths.

## Docs

- `docs/ARCHITECTURE.md` for high-level architecture.
- `docs/DETAILED.md` for model, runtime, workflow, and registry contracts.
- `docs/UPGRADE.md` for downstream upgrade notes and behavior changes.
- `docs/repo-map.md` for the generated repository map.
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

`localOverrides` is explicit. The default repository flake passes `[]`, so `nixfied/local/default.nix` is preserved template space, not an auto-loaded module.

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
