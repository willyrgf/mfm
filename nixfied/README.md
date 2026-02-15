# Nixfied

A Nix-first framework for single codified source of truth for packaging, distribution, and environments setup.

## Overview

**Motivation:** Nixfied is built for code agents and humans who want *one*
codified source of truth for packaging, distribution, and environment setup.
The goal is reproducibility across dev, test, prod, CI, and config layers—no
hand-crafted snowflakes, just repeatable environments everywhere.

### Install (from your repo):

```bash
cd my-project
nix run github:willyrgf/nixfied#framework::install
```

### Prompt your AI (optional):

```bash
nix run github:willyrgf/nixfied#framework::prompt-plan
```

**This generates `NIXFIED_PROMPT_PLAN.md`. Paste it into your AI prompt and ask
the model to follow it.**

## Contents

- [Overview](#overview)
- [Quick start](#quick-start)
- [Install into an existing repo](#install-into-an-existing-repo)
- [Repository layout](#repository-layout)
- [Configuration model](#configuration-model)
- [Execution environment](#execution-environment)
- [Slots, environments, and ports](#slots-environments-and-ports)
- [Framework helpers (shell)](#framework-helpers-shell)
- [Nix helper functions (lib)](#nix-helper-functions-lib)
- [CI pipeline DSL](#ci-pipeline-dsl)
- [Ephemeral environments](#ephemeral-environments)
- [Module apps](#module-apps)
- [Run registry](#run-registry)
- [Process-first visibility and reuse semantics](#process-first-visibility-and-reuse-semantics)
- [Optional modules](#optional-modules)
- [Supervisor (process-compose)](#supervisor-process-compose)
- [Dev shell and packages](#dev-shell-and-packages)
- [Framework tests](#framework-tests)
- [Sanity checks](#sanity-checks)

## Quick start

```bash
nix run .#help
nix run .#dev
nix run .#test
nix run .#build
nix run .#ci
nix run .#check
nix run .#format
```

Template defaults are safe no-ops for `dev`, `test`, and `build`.
`check` first validates discovery artifacts (`docs/repo-index.json`,
`docs/repo-map.md`) and then runs the project quality script.
If artifacts are missing or stale, run:

```bash
nix run .#check -- --refresh-discovery
```

`format` runs `nixfmt` over `*.nix` files in the repo. The CI pipeline is
enabled and runs placeholder steps.
Replace each command in its file under `nixfied/project/`.

## App API contract

Nixfied requires any app exposed in `flake.nix` to define structured metadata at
`commands.<name>.api` (or `app.meta.nixfied.api` for generated/internal apps).
This is used to power `nix run .#help`, and missing/invalid metadata fails
flake evaluation.

Minimal example:

```nix
commands.dev.api = {
  version = 2;
  summary = "Start the dev workflow";
  details = "Longer docs (can be multi-line).";
  usage = [ "nix run .#dev" ];
  appContract = {
    version = 2;
    name = "dev";
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
```

## Service API contract

For supported services (`postgres`, `nginx`, `minio`, `reth`, `helios`), Nixfied enforces a
service contract at `module.publicApi` during flake evaluation.

Required shape:
- `version = 1`
- `service = "<name>"`
- `profiles = [ "dev" "prod" "test" "ci" ]`
- `coreOps` with required ops:
  `init`, `start`, `stop`, `restart`, `status`, `health`, `check-config`
- `artifacts` metadata

Built-in services also expose a `READY` extension hook (`<SERVICE>_READY`) to
gate "usable" state, which may be stricter than "alive".
- `health` means process/API liveness.
- `ready` means the service can satisfy its intended workload checks.
  For example, Helios `HELIOS_HEALTH` validates `eth_chainId`, while
  `HELIOS_READY` validates `eth_blockNumber`.

Generated service apps are exposed as:
- `service::<service>::<operation>`

Generated service hooks (`<SERVICE>_<OPERATION>`) and service apps share the
same launcher path:
- both enforce explicit env and resolve slot via `REQUIRE_SLOT_ENV` (slot defaults to `0` when unset)
- both forward CLI arguments to the underlying operation script

Examples:
- `PROJECT_ENV=dev NIX_ENV=0 nix run .#service::postgres::start`
- `PROJECT_ENV=dev NIX_ENV=0 nix run .#service::nginx::site-add -- example.localhost 127.0.0.1 3000`
- `PROJECT_ENV=dev NIX_ENV=0 nix run .#service::minio::bucket-list`
- `PROJECT_ENV=dev NIX_ENV=0 nix run .#service::reth::start`
- `PROJECT_ENV=dev NIX_ENV=0 nix run .#service::helios::start`

## Install into an existing repo

From your target repository:

```bash
cd my-app
nix run github:willyrgf/nixfied#framework::install
```

Safety behavior:
- Installs into the `nixfied` branch (creates it from your current HEAD if missing).
- By default, if you are on another branch, it switches to `nixfied` before writing files
  (refuses to switch if the working tree is dirty unless `--force`).
- With `--force` (and without `--worktree`), install/upgrade run on your current branch
  instead of switching to `nixfied`.
- It installs only `flake.nix`, `flake.lock`, and `nixfied/`.
- Customize your project in `nixfied/project/` and `nixfied/local/` (avoid editing framework code).

Vendoring boundaries (relevant for upgrades):
- Framework-owned (overwritten on `framework::upgrade`): `flake.nix`, `flake.lock`, `nixfied/.framework/`.
- User-owned (preserved on `framework::upgrade`): `nixfied/project/` and `nixfied/local/`.
- Canonical doc: `nixfied/VENDORED.txt` (includes the framework source revision used by install/upgrade).

Force overwrite:

```bash
nix run github:willyrgf/nixfied#framework::install -- --force
# or
NIXFIED_INSTALL_FORCE=1 nix run github:willyrgf/nixfied#framework::install
```

Worktree install (optional, keeps current checkout unchanged):

```bash
nix run github:willyrgf/nixfied#framework::install -- --worktree
```

Custom worktree directory:

```bash
nix run github:willyrgf/nixfied#framework::install -- --worktree --target /path/to/my-app_nixfied
```

Upgrade an existing install (upgrades framework files, preserves `nixfied/project/` and `nixfied/local/` by default):

```bash
cd my-app
nix run github:willyrgf/nixfied#framework::upgrade -- --force
```

With `--force` (and without `--worktree`), upgrade runs in your current branch.

To overwrite project templates during upgrade:

```bash
nix run github:willyrgf/nixfied#framework::upgrade -- --force --reset-project
```

Filter which project files are installed (conf is always included):

```bash
nix run github:willyrgf/nixfied#framework::install -- --filter=conf,test,ci
```

Prompt plan:
- Generated automatically as part of install/upgrade (best effort).
- Generate manually:

```bash
nix run github:willyrgf/nixfied#framework::prompt-plan
```

- Skip with `--no-prompt-plan` or `NIXFIED_PROMPT_PLAN=0`.
- Overwrite with `--prompt-plan-force` or `NIXFIED_PROMPT_PLAN_OVERWRITE=1`.

Framework-only apps:
- The `framework::install`, `framework::upgrade`, `framework::prompt-plan`
  (prompt generator), and `framework::test` apps are
  only exposed when the repository contains `nixfied/.framework/.workspace`.
- The installer removes this marker in target repos so `nix flake show` will
  not list those apps after installation.
- If you want to run framework tests from an installed repo, create the marker
  file (`touch nixfied/.framework/.workspace`) locally.

Framework workspace:
- Framework maintenance commands live under the `framework::` namespace so they
  don't collide with project commands (e.g. `nix run .#framework::test`).
  This workspace only exists when `nixfied/.framework/.workspace` is present.

## Repository layout

```
flake.nix
nixfied/
  .framework/
    .workspace         # marker: enables framework::* apps
    internal/
      core.nix         # dev/test/build/check/help apps
      install.nix      # installer + upgrade + prompt-plan
      test.nix         # framework test runner
      isolation.nix    # parallel isolation stress test
      module-apps.nix  # auto-generated module apps (service::<service>::<op>, supervisor)
    lib/
      default.nix      # aggregator re-exporting all lib functions
      helpers.nix      # shell helper script generation
      builders.nix     # mkApp / mkAppScript / withTiming
      summary.nix      # summary parser for CI output
      run-registry.nix # run tracking with meta.json + background mode
      process-registry.nix # global process/service/run registry + diagnostics
      parallel.nix     # parallel runner generation
      process.nix      # signal handler + process manager
      port-utils.nix   # port cleanup + conflict checker
      service-api.nix  # service API contract validation + app/hook generation
    postgres/
      default.nix      # aggregator
      lifecycle.nix    # init/start/stop
      config.nix       # postgresql.conf generation
      backup.nix       # backup/restore/list
      migration.nix    # migration runner
      migration-safety.nix # pre-migration safety checks
      port-management.nix  # port conflict resolution
      rollback.nix     # rollback support
    nginx/
      default.nix      # aggregator
      lifecycle.nix    # init/start/stop/reload
      templates.nix    # config templates
      site-management.nix # add/remove/enable/disable sites
      ssl.nix          # certificate management
    minio/
      default.nix      # aggregator
      config.nix       # minio config defaults
      lifecycle.nix    # init/start/stop/restart/status/health/check-config
      bucket-management.nix # bucket/policy operations
    reth/
      default.nix      # aggregator
      config.nix       # reth config defaults
      lifecycle.nix    # init/start/stop/restart/status/health/check-config
    helios/
      default.nix      # aggregator
      package.nix      # vendored helios package wrapper
      config.nix       # helios config defaults
      lifecycle.nix    # init/start/stop/restart/status/health/check-config
    supervisor/
      default.nix      # aggregator
      config.nix       # process-compose YAML generation
      lifecycle.nix    # start/stop/restart
      status.nix       # status/isRunning/logs
      management.nix   # daemon management
    ci.nix             # pipeline runner (ephemeral, summary.json)
    ephemeral.nix      # ephemeral environments (slot locking, cleanup)
    slots.nix          # slot/env/port logic
    hooks.nix          # exported hook env vars
    devshell.nix       # nix develop shell
  local/
    default.nix        # user-owned extensions (extra apps/packages/devShells)
  project/
    conf.nix           # base configuration
    dev.nix            # dev command
    test.nix           # test command
    prod.nix           # build/prod command(s)
    quality.nix        # check/format commands
    ci.nix             # CI command + pipeline DSL
    default.nix        # merges the files above
tests/
  framework/
    fixtures/
      ci/              # CI DSL fixtures
      modules/         # module hook fixtures
      helpers/         # helper function fixtures
      slots/           # slot/env fixtures
      ephemeral/       # ephemeral slot locking fixtures
      registry/        # run registry fixtures
```

## Configuration model

All project configuration lives in `nixfied/project/`.
`nixfied/project/default.nix` merges the files below via `recursiveUpdate`.

### `nixfied/project/conf.nix`

Base configuration and module toggles.

```nix
project = {
  name = "Nixfied Project";
  id = "nixfied-project";
  description = "Reusable Nix development framework";
  envVar = "PROJECT_ENV";  # env name (dev/test/prod)
  slotVar = "NIX_ENV";     # slot number (0-9)
};

envs = {
  prod = { offset = 0; };
  dev  = { offset = 10; };
  test = { offset = 20; };
};

slots = {
  max = 9;
  stride = 1;
  default = 0; # used when NIX_ENV is unset
};

ports = {
  backend = 3000;
  frontend = 3100;
  http = 8080;
  https = 8443;
  postgres = 5432;
  minioApi = 9000;
  minioConsole = 9001;
  rethHttp = 8545;
  rethWs = 8546;
  rethAuth = 8551;
  heliosRpc = 8547;
};

directories.base = "${XDG_DATA_HOME:-$HOME/.local/share}/${project.id}";

tooling.runtimePackages = [ pkgs.coreutils pkgs.gnused ];
# tooling.devShellPackages = [ pkgs.nodejs_20 ];
# tooling.devShellHook = ''echo "dev shell ready"'';

install.deps = ''
  # language/package manager install
  # e.g. npm install, bun install, pip install -r requirements.txt
'';

supervisor.enable = true;
supervisor.services = { };

modules.postgres.enable = false;
modules.nginx.enable = false;
modules.minio.enable = false;
modules.reth.enable = false;
modules.helios.enable = false;

packages = { };
```

### `nixfied/project/*.nix` command files

Each file exports a `commands` attrset. You can add new commands anywhere as
long as they are merged in `nixfied/project/default.nix`.

Command schema:

```nix
commands.<name> = {
  description = "Shown in nix run .#help";
  env = { PROJECT_ENV = "dev"; };
  useDeps = true;  # runs install.deps before the script
  fixtures = {
    services = [
      {
        name = "postgres";
        profile = "test";
      }
    ];
    env = {
      DATABASE_URL = {
        from = "postgres.url";
        database = "app_test";
      };
    };
  };
  script = ''
    echo "hello"
  '';
};
```

Files by convention:
- `dev.nix` -> `dev`
- `test.nix` -> `test`
- `prod.nix` -> `build`
- `quality.nix` -> `check`, `format`
- `ci.nix` -> `ci`

## Execution environment

Every command is wrapped by framework lib helpers and gets:
- `COMMAND_NAME` set to the command name.
- `.env` loaded if present (does not override existing env vars).
- `tooling.runtimePackages` added to `PATH`.
- `install.deps` (if `useDeps = true`).
- Framework helper functions (see below).
- Hook environment variables from `nixfied/.framework/hooks.nix`.

## Slots, environments, and ports

- `PROJECT_ENV` selects the environment (`dev`, `test`, `prod`).
- `NIX_ENV` selects the slot (0-9, defaults to `0` if unset).

Ports are computed as:

```
computed_port = base_port + slot + env_offset
```

`nixfied/.framework/slots.nix` exposes helper scripts:
- `SLOT_INFO` prints `SLOT`, `ENV`, `BASE_DIR`, `LOG_DIR`, `RUN_DIR`,
  `CONFIG_DIR`, `STATE_DIR`, and all computed ports.
- `REQUIRE_SLOT_ENV` validates env/slot and hard-fails if `PROJECT_ENV` is
  missing or if slot/env values are invalid.

Example usage:

```bash
export PROJECT_ENV=dev
export NIX_ENV=0
eval "$(${SLOT_INFO})"
echo "Backend port: $BACKEND_PORT"
```

## Framework helpers (shell)

Every command sources a helper script generated by `nixfied/.framework/lib/`.

- `require_env VAR [message]`
  - Fail if `VAR` is missing or empty.
- `skip_if_missing VAR [reason]`
  - Return 1 if `VAR` is missing so callers can skip work.
- `wait_http URL [timeout] [interval]`
  - Poll a URL until it responds or timeout expires.
- `wait_port PORT [timeout] [interval]`
  - Wait for a TCP port to listen (uses `lsof` or `nc`).
- `log_capture LOGFILE -- <command...>`
  - Capture stdout/stderr to a file (`LOG_TEE=1` also streams to stdout).
- `summary_parse LOGFILE DURATION EXIT_CODE`
  - Print a compact run summary (used by CI `--summary`).
- `with_cleanup CMD`
  - Register a cleanup command (runs on EXIT/INT/TERM in LIFO order).
- `start_service NAME [opts] -- <command...>`
  - Run a background service with optional logging and readiness checks.
  - On readiness failure, the spawned process is always stopped before returning.
- `start_service_into PID_VAR NAME [opts] -- <command...>`
  - Start a service and assign the PID into `PID_VAR` in the current shell.
- `stop_service PID [name]`
  - Stop a background service.
- `with_service NAME [start opts] -- <start command...> --run <command...>`
  - Start a service, then run a command; cleanup is automatic.
- `fixture_start_service SERVICE [profile] [timeout] [interval] [logfile] [keep_running]`
  - Fixture-oriented service startup using service hook contracts.
  - Readiness checks prefer `*_READY` hooks, then fall back to `*_HEALTH`.
  - If wrapper startup exits early, `*_STATUS` is consulted before failing.
  - `keep_running=1` skips cleanup registration so tests can intentionally keep
    service state across cleanup boundaries.
  - Fixture preludes map policy env vars to `keep_running` automatically:
    `SERVICE_OWNER_SCOPE=persistent`, `SERVICE_REUSE_POLICY=same-slot|cross-run`,
    or `SERVICE_DISCOVERY_SCOPE=global` keep services running across command exit.
- `run_hook ENV_VAR [args...]`
  - Execute the command stored in `ENV_VAR`.
  - For service hooks, this is behaviorally equivalent to running the matching
    `service::<service>::<operation>` app.
- `artifact_dir`
  - Return the CI artifacts directory.
- `artifact_path NAME`
  - Return a full path inside the artifacts directory.

Service example:

```bash
start_service_into PID backend --log /tmp/backend.log --wait-port 3000 -- ./start-backend
./run-tests
stop_service "$PID" backend
```

## Nix helper functions (lib)

`nixfied/.framework/lib/` is a directory of Nix modules re-exported through
`nixfied/.framework/lib/default.nix`. Import it in your Nix wiring:

```nix
lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
```

Exported functions:

- **builders.nix** - `mkApp`, `mkAppWithDeps`, `mkAppScript`, `withTiming`
- **helpers.nix** - `loadEnv`, `helpersScript`, `hookExports`
- **summary.nix** - `summaryParser`
- **parallel.nix** - `mkParallelRunner`
- **port-utils.nix** - `mkPortCleanup`, `mkPortConflictChecker`
- **process.nix** - `mkSignalHandler`, `mkProcessManager`
- **run-registry.nix** - `runRegistryStart` (run tracking with meta.json)
- **process-registry.nix** - `emitEvent`, `processStatus`, `processSlots`,
  `processRuns`, `processInspect`, `processGc`, `serviceStatus`,
  `serviceEvents`, `serviceLogs` (global process-first visibility)

## CI pipeline DSL

The CI runner is enabled by default and defined in `nixfied/project/ci.nix`.

Top-level config:

```nix
ci = {
  enable = true;
  defaultMode = "basic";
  env = { PROJECT_ENV = "test"; };
  useDeps = true;
  setup = "";      # runs once before steps
  teardown = "";   # runs once after steps
  artifacts = {
    dir = "/tmp/ci-artifacts";
    keepOnFailure = true;
    keepOnSuccess = false;
  };
  modes = {
    basic = { steps = [ "quality" "tests" ]; };
    app = { steps = [ "quality" "tests" "system" ]; };
  };
  steps = {
    quality = {
      description = "Quality checks";
      run = ''
        LOGFILE=$(artifact_path "quality.log")
        log_capture "$LOGFILE" -- ./lint
      '';
    };
    system = {
      description = "System tests";
      skipIfMissing = [ "API_KEY" ];
      fixtures = {
        services = [
          {
            name = "nginx";
            profile = "test";
          }
        ];
      };
      when = "[ \"$PROJECT_ENV\" = test ]";
      cleanup = ''
        echo "cleanup after step"
      '';
      env = { FOO = "bar"; };
      run = ''
        ./run-system-tests
      '';
    };
  };
};
```

Step fields:
- `description` (string)
- `run` (shell script)
- `when` (shell condition, optional)
- `cleanup` (shell script, runs after `run` even on failure)
- `env` (attrset of env vars)
- `skipIfMissing` (list of required env vars)
- `fixtures` (attrset, optional)
  - `services` (list of service fixtures: string names or `{ name, profile?, timeout?, interval?, logs?, exports?, bootstrap?, ... }`)
  - `env` (derived/static env exports for step body)
  - `artifacts` (fixture log controls, e.g. `{ logs = true; prefix = "step-name"; }`)

CLI usage:

```bash
nix run .#ci                 # default mode
nix run .#ci -- --summary    # summary output
nix run .#ci -- --mode app   # select mode
nix run .#ci -- --app        # shorthand for mode "app"
nix run .#ci -- --bg         # run in background via run registry
```

The CI runner writes `summary.json` to the artifacts directory after each run,
containing mode, exit code, per-step results (name, status, duration), and a
`timing` block (`total_duration`, `setup_duration`, `steps_duration`,
`teardown_duration`, `accounted_duration`, `untracked_duration`).

CI environment variables available inside steps:
- `CI_MODE`
- `CI_SUMMARY`
- `CI_ARTIFACTS_DIR`
- `CI_KEEP_ARTIFACTS_ON_FAILURE`
- `CI_KEEP_ARTIFACTS_ON_SUCCESS`

Artifacts:
- Stored per run under `ci.artifacts.dir/<run-id>` by default.
- `ci.artifacts.dir/latest` points to the most recent run directory.
- Set `CI_ARTIFACTS_DIR` to force a fixed artifacts directory for a specific invocation.
- Removed automatically unless `keepOnFailure`/`keepOnSuccess` are true.

## Ephemeral environments

Ephemeral mode provides fully isolated, deterministic execution environments.
All mutable state goes to a temporary directory that is cleaned up on success
and preserved on failure for debugging.

Enable in `nixfied/project/conf.nix`:

```nix
ephemeral = {
  enable = true;
  excludePatterns = [ ".git" "node_modules" ".next" "dist" ];
  extraDirs = [ ];
};
```

Features:
- **Slot locking**: Each ephemeral run acquires an exclusive lock on a slot
  (0-9) to prevent port conflicts between concurrent runs.
- **Conditional cleanup**: State is cleaned on success, preserved on failure.
- **Source copy**: The project is rsync'd to the ephemeral root (excluding
  configured patterns).
- **CI integration**: When `ci.useEphemeral = true` (default), CI steps run
  inside an ephemeral wrapper via `mkEphemeralWrapper`.

## Module apps

When modules are enabled, the framework auto-generates convenience apps via
`nixfied/.framework/internal/module-apps.nix`. These are listed under "Module Apps" in
`nix run .#help`.

Service apps (when modules are enabled):
- `service::postgres::<operation>` (for example: `start`, `setup-db`, `backup`, `shell`)
- `service::nginx::<operation>` (for example: `start`, `site-add`, `site-list`, `cert-renew`)
- `service::minio::<operation>` (for example: `start`, `bucket-create`, `bucket-list`, `policy-apply`)
- `service::reth::<operation>` (for example: `start`, `health`, `ready`, `check-config`)
- `service::helios::<operation>` (for example: `start`, `health`, `ready`, `check-config`)

Supervisor apps (when `supervisor.enable = true`):
- `up`, `down`, `svc-status`, `svc-logs`, `svc-restart`

Utility apps (always available):
- `check-ports`, `ports`
- `process::status`, `process::slots`, `process::runs`, `process::inspect`, `process::stop`, `process::gc`

All module apps and supervisor apps require explicit env selection.
Set `PROJECT_ENV` before running `up`, `down`, `svc-*`, or any
`service::<service>::<operation>` app. `NIX_ENV` is optional and defaults to
slot `0` when unset (an `INFO:` line is printed when the default is used).

## Run registry

The run registry (`nixfied/.framework/lib/run-registry.nix`) provides durable run tracking.
Each run creates a directory with `meta.json` (status, timing, exit code) and
`output.log`.

Used by CI `--bg` mode to detach runs into the background. The runs root
defaults to `/tmp/<project-id>-runs` (configurable via `ci.runsRoot`).

## Process-first visibility and reuse semantics

Nixfied now includes a process-first runtime registry to make cross-run
observability explicit, especially when CI runs inside ephemeral roots.

Problem this solves:
- `MFM_KEEP_SERVICES=1` style behavior is local to one command invocation.
- Ephemeral runs use isolated roots and often clean up on success, which can
  hide runtime metadata from later status queries.
- Service status checks based only on slot/env-local PID/state files can miss
  processes started from another root.
- Long CI readiness waits can look "stuck" without a global view of active runs
  and wait reasons.

### Command surface

Process commands:
- `nix run .#process::status`
- `nix run .#process::status -- --all`
- `nix run .#process::slots`
- `nix run .#process::runs`
- `nix run .#process::inspect -- <id>`
- `nix run .#process::stop -- --run-id <id>`
- `nix run .#process::stop -- --run-id <id> --scope slot-env`
- `nix run .#process::stop -- --run-id <id> --dry-run`
- `nix run .#process::gc`
- `nix run .#process::gc -- --apply`

Service-level observability extensions:
- `nix run .#service::<name>::log -- [--lines N] [--follow]`
- `nix run .#service::<name>::logs -- [--lines N] [--follow]` (compat alias)
- `nix run .#service::<name>::events -- [--limit N]`
- `nix run .#service::<name>::status` (local + global registry merge)

Extended `service::<name>::status` fields:
- `scope`
- `owner_run_id`
- `owner_scope`
- `ephemeral_root`
- `registry_state`
- `slot_owner`
- `wait_reason`
- `log_path`

### Reuse and discovery policy controls

Public env controls:
- `SERVICE_REUSE_POLICY=never|same-root|same-slot|cross-run`
- `SERVICE_OWNER_SCOPE=ephemeral|persistent`
- `SERVICE_DISCOVERY_SCOPE=local|global`

Defaults are inferred with this precedence and can be overridden:
- explicit `SERVICE_OWNER_SCOPE` / `SERVICE_DISCOVERY_SCOPE` env vars
- explicit `SERVICE_REUSE_POLICY` fills missing scopes:
  - `same-slot` or `cross-run` => `persistent`, `global`
  - `same-root` => `ephemeral`, `local`
- execution context fallback:
  - ephemeral execution defaults to `same-root`, `ephemeral`, `local`
  - persistent execution defaults to `same-slot`, `persistent`, `global`

Examples:
- concise slot reuse in persistent context:
  `PROJECT_ENV=dev NIX_ENV=0 SERVICE_REUSE_POLICY=same-slot nix run .#<command>`
- fully explicit equivalent:
  `PROJECT_ENV=dev NIX_ENV=0 SERVICE_OWNER_SCOPE=persistent SERVICE_DISCOVERY_SCOPE=global SERVICE_REUSE_POLICY=same-slot nix run .#<command>`

Validation rules:
- `cross-run` requires `SERVICE_OWNER_SCOPE=persistent` and
  `SERVICE_DISCOVERY_SCOPE=global`
- `same-root` requires `SERVICE_OWNER_SCOPE=ephemeral` and
  `SERVICE_DISCOVERY_SCOPE=local`
- invalid combinations fail fast with an `ERROR:` and corrective hint

### Registry model

Default registry root:
- `/tmp/nixfied-runtime/<project-id>` (configurable via
  `project.process.registryRoot` in `nixfied/project/conf.nix`)

Storage:
- `events.jsonl` (append-only event log)
- `snapshot.json` (materialized query snapshot)
- `locks/*.lock` (coordination)

Core event types:
- `run_started`, `run_finished`
- `slot_acquired`, `slot_released`
- `service_starting`, `service_ready`, `service_degraded`, `service_stopped`,
  `service_orphaned`
- `readiness_progress`

Representative event fields:
- `event_id`, `event_type`, `timestamp`, `run_id`, `command_name`, `project_id`
- `service`, `slot`, `env`, `profile`, `pid`, `pgid`, `state`
- `owner_scope`, `reuse_policy`, `discovery_scope`, `ephemeral_root`
- `readiness` metadata, `wait_reason`, `log_path`

### Delivery and migration

Phase plan:
- Phase 1: registry writes + `process::*` command namespace
- Phase 2: dual-source service status + service events/log operations
- Phase 3: explicit policy semantics with strict validation
- Phase 4: GC/orphan hardening + deprecation guidance for older implicit keep
  semantics

Compatibility:
- existing defaults remain non-breaking
- `runtime::*` aliases remain available during migration
- status field additions are additive key/value fields

Acceptance goals:
- active services/runs are always listable regardless of ephemeral roots
- slot ownership and contention reason are visible from one command
- reuse behavior is explicit, documented, and test-covered
- cross-root status is no longer silently misleading
- CI + registry data provides postmortem traceability

## Optional modules

Enable modules in `nixfied/project/conf.nix`.

### Postgres

Config:

```nix
modules.postgres = {
  enable = true;
  database = "app";
  testDatabase = "app_test";
  extensions = [ "pgcrypto" ];
  portKey = "postgres";  # key in ports
  dataDirName = "postgres";
  extraConfig = "";
};
```

Hooks (exported env vars):
- `POSTGRES_INIT`, `POSTGRES_START`, `POSTGRES_STOP`, `POSTGRES_RESTART`
- `POSTGRES_STATUS`, `POSTGRES_HEALTH`, `POSTGRES_READY`, `POSTGRES_CHECK_CONFIG`
- `POSTGRES_SETUP_DB`
- `POSTGRES_FULL_START`, `POSTGRES_FULL_START_TEST`
- `POSTGRES_BACKUP`, `POSTGRES_RESTORE`, `POSTGRES_LIST_BACKUPS`
- `POSTGRES_TEST_MIGRATIONS`, `POSTGRES_ENSURE_MIGRATION_TESTED`
- `POSTGRES_CHECK_PORT`, `POSTGRES_KILL_PORT`, `POSTGRES_LIST_INSTANCES`
- `POSTGRES_SHELL`

Example:

```bash
run_hook POSTGRES_FULL_START
run_hook POSTGRES_SETUP_DB
run_hook POSTGRES_BACKUP my-backup
run_hook POSTGRES_LIST_INSTANCES
```

### Nginx

Config:

```nix
modules.nginx = {
  enable = true;
  portKeyHttp = "http";
  portKeyHttps = "https";
  dataDirName = "nginx";
};
```

Hooks:
- `NGINX_INIT`, `NGINX_START`, `NGINX_STOP`, `NGINX_RESTART`
- `NGINX_STATUS`, `NGINX_HEALTH`, `NGINX_READY`, `NGINX_CHECK_CONFIG`, `NGINX_RELOAD`
- `NGINX_SITE_PROXY`, `NGINX_SITE_STATIC`
- `NGINX_SITE_ADD`, `NGINX_SITE_REMOVE`, `NGINX_SITE_LIST`
- `NGINX_SITE_ENABLE`, `NGINX_SITE_DISABLE`
- `NGINX_CERT_OBTAIN`, `NGINX_CERT_RENEW`, `NGINX_CERT_STATUS`

Example:

```bash
run_hook NGINX_INIT
run_hook NGINX_SITE_PROXY example.localhost 127.0.0.1 3000
run_hook NGINX_START
run_hook NGINX_SITE_LIST
```

### MinIO

Config:

```nix
modules.minio = {
  enable = true;
  package = pkgs.minio;
  clientPackage = pkgs.minio-client;
  portKeyApi = "minioApi";
  portKeyConsole = "minioConsole";
  dataDirName = "minio";
  rootUser = "minioadmin";
  rootPassword = "minioadmin";
  browser = true;
};
```

Hooks:
- `MINIO_INIT`, `MINIO_START`, `MINIO_STOP`, `MINIO_RESTART`
- `MINIO_STATUS`, `MINIO_HEALTH`, `MINIO_READY`, `MINIO_CHECK_CONFIG`
- `MINIO_FULL_START`, `MINIO_FULL_START_TEST`
- `MINIO_EXPORT_S3_ENV`, `MINIO_BUCKET_ENSURE`
- `MINIO_BUCKET_CREATE`, `MINIO_BUCKET_DELETE`, `MINIO_BUCKET_LIST`
- `MINIO_POLICY_APPLY`

Example:

```bash
run_hook MINIO_INIT
run_hook MINIO_START
run_hook MINIO_BUCKET_LIST
run_hook MINIO_STOP
```

### Reth

Config:

```nix
modules.reth = {
  enable = true;
  package = pkgs.reth;
  portKeyHttp = "rethHttp";
  portKeyWs = "rethWs";
  portKeyAuth = "rethAuth";
  dataDirName = "reth";
  network = "local";
  devMode = true;
  extraArgs = [ ];
};
```

Hooks:
- `RETH_INIT`, `RETH_START`, `RETH_STOP`, `RETH_RESTART`
- `RETH_STATUS`, `RETH_HEALTH`, `RETH_READY`, `RETH_CHECK_CONFIG`
- `RETH_FULL_START`, `RETH_FULL_START_TEST`

Example:

```bash
run_hook RETH_INIT
run_hook RETH_CHECK_CONFIG
run_hook RETH_START
run_hook RETH_HEALTH
run_hook RETH_STOP
```

### Helios

Config:

```nix
modules.helios = {
  enable = true;
  package = pkgs.callPackage ../.framework/helios/package.nix { };
  portKeyRpc = "heliosRpc";
  dataDirName = "helios";
  network = "local";
  executionRpcPortKey = "rethHttp";
  executionRpcUrl = "";
  consensusRpcUrl = "";
  defaultConsensusRpcUrl = "https://www.lightclientdata.org";
  checkpoint = "";
  extraArgs = [ ];
};
```

Hooks:
- `HELIOS_INIT`, `HELIOS_START`, `HELIOS_STOP`, `HELIOS_RESTART`
- `HELIOS_STATUS`, `HELIOS_HEALTH`, `HELIOS_READY`, `HELIOS_CHECK_CONFIG`
- `HELIOS_FULL_START`, `HELIOS_FULL_START_TEST`

Note:
- The vendored default package is a wrapper. Set `HELIOS_BIN` or override
  `modules.helios.package` to point at a real Helios binary.
- For `network = "mainnet"`, if `consensusRpcUrl` is unset, Helios defaults to
  `defaultConsensusRpcUrl` (`https://www.lightclientdata.org` by default).
- For non-local networks, if `checkpoint` is unset, startup derives a recent
  weak-subjectivity checkpoint from the consensus endpoint and validates
  `light_client/bootstrap`; on mainnet + lightclientdata it also falls back to
  `https://lodestar-mainnet.chainsafe.io`.
- For deterministic control, pin `checkpoint` explicitly.
- `HELIOS_READY` tunables:
  - `HELIOS_READY_TIMEOUT_SECS` (default `300`)
  - `HELIOS_READY_INTERVAL_SECS` (default `1`)

Example:

```bash
run_hook HELIOS_INIT
run_hook HELIOS_CHECK_CONFIG
run_hook HELIOS_START
run_hook HELIOS_HEALTH
run_hook HELIOS_STOP
```

## Supervisor (process-compose)

Supervisor is intended for production orchestration. Configure services in
`nixfied/project/conf.nix`:

```nix
supervisor = {
  enable = true;
  services = {
    app = {
      command = ''./start-app'';
      workingDir = ".";
      env = { PORT = "3000"; };
      readiness = {
        type = "http";
        host = "127.0.0.1";
        port = "3000";
        path = "/health";
      };
    };
  };
};
```

The `nixfied/.framework/supervisor/` module generates a process-compose YAML and provides
lifecycle scripts.

Hooks (exported env vars when supervisor is enabled):
- `SUPERVISOR_START`, `SUPERVISOR_STOP`, `SUPERVISOR_START_DAEMON`
- `SUPERVISOR_STATUS`, `SUPERVISOR_IS_RUNNING`
- `SUPERVISOR_LOGS`, `SUPERVISOR_RESTART`

Example:

```bash
run_hook SUPERVISOR_START
run_hook SUPERVISOR_STATUS
run_hook SUPERVISOR_STOP
```

## Dev shell and packages

- `nix develop` uses `tooling.devShellPackages` and `tooling.devShellHook`.
- `project.packages` is exposed via `flake.packages` for custom outputs.

## Framework tests

Run the integration test suite for the framework itself:

```bash
nix run .#framework::test
```

Default behavior:
- Uses the `ci` profile.
- Runs shard groups in parallel (`--jobs 2`).

Examples:

```bash
nix run .#framework::test -- --profile ci
nix run .#framework::test -- --jobs 3
nix run .#framework::test -- --serial
nix run .#framework::test -- --list-shards
nix run .#framework::test -- --shard installer
```

To include isolation in framework tests:

```bash
FRAMEWORK_ISOLATION=1 nix run .#framework::test
```

The test runner is fully packaged with Nix tools; it does not depend on system
utilities.

See `tests/framework/README.md` for fixtures, example snippets, and an examples index.

## Sanity checks

```bash
nix flake show
nix flake check --no-build
```
