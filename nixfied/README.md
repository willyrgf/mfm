# Nixfied

> WARNING: Nixfied is in active development. Breaking changes can happen in any commit.

Nixfied is a Nix-first framework for codifying project workflows (dev, test, build, check, CI), runtime environments, and optional infrastructure modules from a single configuration surface.

## Contents

- [Quick start](#quick-start)
- [General architecture](#general-architecture)
- [Public APIs](#public-apis)
- [Install and upgrade](#install-and-upgrade)
- [Configuration model](#configuration-model)
- [CI and ephemeral execution](#ci-and-ephemeral-execution)
- [Modules](#modules)
- [Framework tests](#framework-tests)
- [Sanity checks](#sanity-checks)

## Quick start

```bash
nix run .#help
nix run .#dev
nix run .#test
nix run .#build
nix run .#check
nix run .#ci
nix run .#format
```

Template defaults are placeholders for `dev`, `test`, and `build`.

## General architecture

Nixfied composes project config, framework internals, and optional modules in `flake.nix`, then validates and exposes a final `apps` surface.

### Evaluation flow

1. Load project config from `nixfied/project/`.
2. Initialize slot/env helpers (`nixfied/.framework/slots.nix`).
3. Conditionally load enabled service modules (`postgres`, `nginx`, `minio`, `reth`, `helios`).
4. Validate service contracts (`publicApi.version = 3`) for enabled modules.
5. Build hook env exports (`nixfied/.framework/hooks.nix`).
6. Build framework lib (`nixfied/.framework/lib/`).
7. Build app sets:
   - Core apps (`help`, project commands, etc.)
   - Generated module apps (`svc::*`, `process::*`, supervisor, utility)
   - CI app
   - Isolation apps (`validate-env`, `test-isolation`)
   - Framework-only apps (`framework::*`) when `nixfied/.framework/.workspace` exists
   - Local extension apps from `nixfied/local/`
8. Validate all apps against the app API contract (`api.version = 2`, `appContract.version = 2`).

### Repository layout and ownership

- `flake.nix`: top-level composition and app/package outputs.
- `nixfied/project/`: primary customization surface.
  - `conf.nix`: envs, ports, module toggles, runtime defaults.
  - `dev.nix`, `test.nix`, `prod.nix`, `quality.nix`, `ci.nix`, `format.nix`: command declarations.
  - `default.nix`: merges all project files.
- `nixfied/.framework/`: framework internals (vendored in installed repos).
- `nixfied/local/`: user-owned local extensions (`apps`, `packages`, `devShells`).
- `tests/framework/`: framework integration tests and fixtures.

### Runtime model (slots, envs, ports)

- `PROJECT_ENV` selects logical environment (`dev`, `test`, `prod`, or custom).
- `NIX_ENV` selects slot index (`0` by default when unset).
- Effective ports are computed per slot and env offset:

```text
effective_port = base_port + slot + env_offset
```

Slot helpers exported to runtime:
- `SLOT_INFO`, `SLOT_INFO_JSON`
- `REQUIRE_SLOT_ENV`, `REQUIRE_SLOT_ENV_JSON`

## Public APIs

This section defines the user-facing interfaces intended for automation and integration.

### Command namespaces

- Core apps:
  - `help`, `dev`, `test`, `build`, `check`, `format`, `ci`
- Isolation apps:
  - `validate-env`, `test-isolation`
- Utility apps:
  - `ports`, `check-ports`
- Process registry apps:
  - `process::status`, `process::slots`, `process::runs`, `process::inspect`, `process::stop`, `process::gc`
- Service apps (generated from service contract):
  - `svc::<service>::<operation>`
- Supervisor apps (when `supervisor.enable = true`):
  - `up`, `down`, `svc-status`, `svc-health`, `svc-logs`, `svc-restart`
- Framework workspace apps (framework repo only):
  - `framework::install`, `framework::upgrade`, `framework::prompt-plan`, `framework::test`

Primary command discovery and docs are available via:

```bash
nix run .#help
nix run .#help -- <command>
```

Note: `validate-env` and `test-isolation` are executable apps but are not listed by the current `help` renderer.

### App API contract (`version = 2`)

Every app exposed in `flake.nix` must provide API metadata that passes validation:

- Project commands: `commands.<name>.api`
- Generated/internal apps: `app.meta.nixfied.api` (or via framework builders)

Required API shape:
- `api.version = 2`
- `summary`, `details`, `usage`
- `appContract` with `version = 2`

`appContract` defines machine-readable command behavior:
- `commandClass`: `typed` | `passthrough` | `json` | `batch-runner`
- runtime primitives in `appContract.env`:
  - `LOG_LEVEL` (`error|warn|info|debug|trace`, alias: `NIXFIED_LOG_LEVEL`)
  - `OUTPUT_MODE` (`stdout|logs|both`, alias: `NIXFIED_OUTPUT_MODE`)
- argument/env specs
- output contract mode (`appContract.outputs.mode`: `text|kv|json`)
- failure codes
- idempotence metadata

Validation fails flake evaluation when metadata is missing or invalid.

### Service API contract (`publicApi.version = 3`)

For enabled services, `module.publicApi` is required and validated during flake evaluation.

Required fields:
- `version = 3`
- `service` (must match module key)
- `summary`, `details`
- `artifacts` (metadata)
- `runtimePrimitives`:
  - `version = 1`
  - `logLevel` contract (`LOG_LEVEL`, alias `NIXFIED_LOG_LEVEL`, enum values)
  - `outputMode` contract (`OUTPUT_MODE`, alias `NIXFIED_OUTPUT_MODE`, enum values)
- `operations` attrset with required lifecycle ops:
  - `start`, `stop`, `status`

Each operation includes script and docs metadata and can expose:
- a generated app: `svc::<service>::<op>`
- a generated hook env var: `SVC_<SERVICE>_<OP>`

### Environment and policy API

Core env interface:
- `PROJECT_ENV`: required for service/supervisor apps.
- `NIX_ENV`: slot index (defaults to `0` when unset).
- `LOG_LEVEL`: runtime logging level (`error|warn|info|debug|trace`).
- `OUTPUT_MODE`: runtime log routing (`stdout|logs|both`).
- default coupling: when `OUTPUT_MODE` is unset and `LOG_LEVEL=debug`, output defaults to `both`.
- strict alias consistency: conflicting canonical/alias values fail fast (`LOG_LEVEL` vs `NIXFIED_LOG_LEVEL`, `OUTPUT_MODE` vs `NIXFIED_OUTPUT_MODE`).
- strict empty handling: explicitly setting `LOG_LEVEL=""` or `OUTPUT_MODE=""` fails; unset the variable to use defaults.

Important distinction:
- `OUTPUT_MODE` controls where log lines are emitted.
- `appContract.outputs.mode` controls command payload format (`text|kv|json`).

Service reuse and discovery controls:
- `SERVICE_REUSE_POLICY=never|same-root|same-slot|cross-run`
- `SERVICE_OWNER_SCOPE=ephemeral|persistent`
- `SERVICE_DISCOVERY_SCOPE=local|global`

Defaults are context-sensitive, and invalid combinations fail fast.

### Stability and breaking changes policy

Nixfied is currently development-stage software:
- Breaking changes can happen in any commit.
- Treat command output and interfaces as evolving unless pinned.

For upgrades and compatibility checks:
- Review `README.md` and `nixfied/VENDORED.txt`.
- Compare revisions with `git log` and `git diff` when updating framework versions.

## Install and upgrade

Install into an existing repository:

```bash
cd my-project
nix run github:willyrgf/nixfied#framework::install
```

Upgrade framework files in an existing install:

```bash
cd my-project
nix run github:willyrgf/nixfied#framework::upgrade -- --force
```

Key behavior:
- Default install target branch is `nixfied`.
- `--worktree` installs into a separate git worktree.
- `--force` allows applying install/upgrade on the current branch.
- `--filter=...` selects project template files during fresh install.
- `--reset-project` overwrites project templates during upgrade.
- Prompt plan generation is enabled by default (`NIXFIED_PROMPT_PLAN.md`).

Vendoring boundaries:
- Framework-owned (overwritten on upgrade): `flake.nix`, `flake.lock`, `nixfied/.framework/`
- User-owned (preserved by default): `nixfied/project/`, `nixfied/local/`

Canonical boundary record: `nixfied/VENDORED.txt`.

## Configuration model

### `nixfied/project/conf.nix`

Defines:
- project identity and env var names
- environment offsets and slot behavior
- port roles
- runtime/tooling defaults
- module toggles
- CI/isolation/ephemeral/process registry defaults

### `nixfied/project/*.nix` command files

Each file contributes to `commands` and is merged by `nixfied/project/default.nix`.

Command schema (minimum):

```nix
commands.dev = {
  api = {
    version = 2;
    summary = "Start dev workflow";
    details = "Runs project dev workflow.";
    usage = [ "nix run .#dev" ];
    appContract = {
      version = 2;
      name = "dev";
      commandClass = "typed";
      allowUnknownArgs = false;
      args = [ ];
      env = [ ];
      outputs = { mode = "text"; };
      failureCodes = {
        generic = 1;
        usage = 2;
        precondition = 3;
        unavailable = 4;
        timeout = 5;
      };
      idempotent = true;
    };
  };
  env = { PROJECT_ENV = "dev"; };
  useDeps = true;
  script = ''
    echo "custom dev flow"
  '';
};
```

Runtime primitive reuse example in a custom app script:

```bash
# LOG_LEVEL / OUTPUT_MODE are already validated and normalized by Nixfied.
case "${LOG_LEVEL}" in
  debug|trace)
    export MY_APP_VERBOSE=1
    ;;
  *)
    export MY_APP_VERBOSE=0
    ;;
esac

case "${OUTPUT_MODE}" in
  logs|both)
    export MY_APP_AUDIT_LOGS=1
    ;;
  *)
    export MY_APP_AUDIT_LOGS=0
    ;;
esac
```

## CI and ephemeral execution

CI is configured in `nixfied/project/ci.nix`.

Common usage:

```bash
nix run .#ci
nix run .#ci -- --summary
nix run .#ci -- --mode app
nix run .#ci -- --bg
```

Highlights:
- Modes and steps are defined declaratively (`ci.modes`, `ci.steps`).
- `summary.json` is produced per run in CI artifacts.
- `--bg` uses the run registry for detached execution.

Ephemeral mode (`ephemeral.enable = true`) adds:
- isolated temporary roots
- slot locking for concurrent runs
- cleanup on success and retained artifacts on failure

## Modules

Detailed module docs moved out of README:

- `docs/modules/README.md`
- `docs/modules/postgres.md`
- `docs/modules/nginx.md`
- `docs/modules/minio.md`
- `docs/modules/reth.md`
- `docs/modules/helios.md`

All generated service apps use:

```bash
PROJECT_ENV=<env> NIX_ENV=<slot> nix run .#svc::<service>::<operation>
```

## Framework tests

Run framework integration tests:

```bash
nix run .#framework::test
```

Examples:

```bash
nix run .#framework::test -- --profile ci
nix run .#framework::test -- --jobs 3
nix run .#framework::test -- --serial
nix run .#framework::test -- --list-shards
nix run .#framework::test -- --shard installer
```

## Sanity checks

```bash
nix run .#help
nix flake show
nix flake check --no-build
```
