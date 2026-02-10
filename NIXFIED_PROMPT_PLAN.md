# NIXFIED PROMPT PLAN

Create a PROMPT PLAN in Markdown for integrating this project with the Nixfied framework.

Requirements:
- Be concise and actionable.
- Use headings: "PROMPT PLAN", "Project Snapshot", "Current Behavior", "Integration Steps",
  "Key Files to Edit", "Validation Checklist", "Open Questions", and "Next Prompts".
- Ground every step in the provided context; do not guess missing details.
- Treat Nixfied as the single entrypoint for dev/test/build/check/ci and (optionally) db/nginx/supervisor:
  nix run .#help, .#dev, .#test, .#build, .#check, .#ci
- Call out the key file-to-command mapping (do not assume "prod" is a command):
  - nixfied/project/dev.nix -> commands.dev
  - nixfied/project/test.nix -> commands.test
  - nixfied/project/prod.nix -> commands.build (build/prod workflow)
  - nixfied/project/quality.nix -> commands.check
  - nixfied/project/ci.nix -> CI pipeline DSL config (ci.modes/ci.steps) + CI command metadata
  - nixfied/project/conf.nix -> project identity, envs/ports, module toggles, ephemeral config
  - nixfied/project/default.nix -> merges all project files; update if new files are added
- Mention the primary customization surface is nixfied/project/ (avoid editing flake.nix unless the plan proves it's necessary).
- Reference relevant framework features (only if applicable to this project):
  - CI pipeline DSL (modes/steps, artifacts, summary.json; supports --summary, --mode/--<mode>, --bg)
  - Ephemeral environments (slot locking, source copy, conditional cleanup; ci.useEphemeral)
  - Module apps + hooks (db-*, nginx-*, supervisor apps; postgres backups/migrations)
  - Run registry (used by CI --bg mode)
- In "Integration Steps", start with high-level goals (behavior parity with the current dev/test/build/check/ci workflows, avoid regressions), then list concrete wiring steps with exact file paths.
- In "Key Files to Edit", list each file and the specific changes needed.
- In "Validation Checklist", include concrete smoke checks (nix run .#help/.#dev/.#test/.#build/.#check/.#ci -- --summary) and any project-specific checks from the docs.
- Include documentation alignment goals (README.md plus any agent instruction docs like CLAUDE.md/AGENTS.md should make Nixfied the canonical entrypoint).
- If docs conflict on command names or behavior, call it out and ask which source is authoritative.
- If info is missing, list it in "Open Questions".

Sources:
- Project docs: PROJECT_README.md, PROJECT_CLAUDE.md, PROJECT_AGENTS.md
- Nixfied docs: NIXFIED_FRAMEWORK_README.md

Context (project docs + framework README) follows:


<<< FILE: PROJECT_README.md >>>
# MFM

A WIP platform for on-chain operations.

### ALERT: Experimental project, not for production or mainnet use. Mostly AI-generated with barely any human review.


<<< END OF PROJECT_README.md >>>

<<< FILE: PROJECT_AGENTS.md >>>
# MFM Development Guide for AI Agents

This document is the source-of-truth for AI agents working in this repository.
It is inspired by the practices used in large Rust codebases: modular crates, strong typing, careful performance work, and a bias toward small, reviewable changes.

## Read First (Non-Negotiables)

- Keep changes small and local; prefer 1 logical change per PR/commit.
- Match CI: use `cargo +nightly fmt --all`, `cargo +nightly clippy --workspace --lib --examples --tests --benches --all-features`, and `cargo nextest run --workspace`.
- Never log, print, or persist secrets (passwords, mnemonics, private keys).
- Preserve crate boundaries: libraries stay usable without the CLI.
- If you touch security-sensitive code (keystore/crypto), add or strengthen tests.

## Project Overview

MFM is a WIP toolkit for on-chain operations.

What exists today:

- A security-hardened Ethereum keystore (persisted JSON, tamper detection, DoS guards).
- A generic async recoverable state machine for multi-step workflows.
- A scriptable CLI that currently exposes keystore management.
- YAML configuration models for networks/tokens/DEXes (not fully wired end-to-end yet).

Status: experimental, not production-ready, and not intended for mainnet use.

## Repository Map

Workspace root: `Cargo.toml`

- `mfm_core/`: core library (keystore + config models). Security-critical.
- `mfm_machine/`: async recoverable state machine framework.
- `mfm_machine_derive/`: proc-macro derive for state metadata.
- `mfm_cli/`: CLI binary (package name `mfm`, bin name `mfm_cli`).

Key docs:

- `README.md`: project disclaimer.
- `mfm_cli/README.md`: CLI behavior and JSON output contract.
- `mfm_machine/README.md`: state machine concepts and usage.
- `CURRENT_STATE.md`, `CLEANUP_PLAN.md`: deeper design notes (may lag code).

## Architecture Overview

### Core Components

1. `mfm_core/`: core library
   - `mfm_core::keystore`: encrypted key storage and signing utilities
   - `mfm_core::config`: YAML config models (networks/tokens/DEX/auth)
2. `mfm_machine/`: async state machine framework
   - `state`: `Tag`, `Label`, metadata, typed context (`SafeContext`)
   - `state_machine`: engine, scheduler, tracker, recovery
3. `mfm_machine_derive/`: proc-macro derive
   - `#[derive(StateMetadataReqs)]` reduces boilerplate for state metadata
4. `mfm_cli/`: CLI binary (package `mfm`, bin `mfm_cli`)
   - clap command tree, keystore subcommands, stable JSON/text outputs

Crate dependency graph:

```text
mfm (CLI) -> mfm_core -> mfm_machine -> mfm_machine_derive
```

## Key Design Principles

- Modularity: each crate should be usable as a library with minimal coupling.
- Type Safety: strong typing throughout with minimal use of dynamic dispatch
- Explicit boundaries: keep public APIs stable; avoid leaking implementation details.
- Performance as a feature: avoid accidental allocations in hot paths; measure first.
- Extensibility: prefer traits + generic types when it improves composability.
- Correctness and security first: especially in security related modules (e.g. `mfm_core::keystore`).
- Output stability: treat CLI JSON/text formats as public API.

## Development Workflow

### Code Style and Standards

1. **Formatting**: Always use nightly rustfmt
```bash
cargo +nightly fmt --all
```

2. **Linting**: Run clippy with all features
```bash
cargo +nightly clippy --workspace --lib --examples --tests --benches --all-features
```

3. **Testing**: Use nextest for faster test execution
```bash
cargo nextest run --workspace
```

### Security Audit

CI runs `cargo audit`.

```bash
cargo audit
```

### Nix (Optional, CI Parity)

The repo ships a Nix flake (`flake.nix`). CI runs:

```bash
nix flake check
nix build
nix run .#mfm_cli -- --help
```

## Common Contribution Types

These are typical, review-friendly change patterns (focus on a single outcome).

1. Small bug fixes (1-20 lines)
   - Fix off-by-one / validation edge case
   - Tighten error messages or error variants
   - Add missing `#[serde(default)]` for backward compatibility

2. Security hardening
   - Strengthen keystore tamper checks
   - Add stricter file size/shape validation
   - Reduce secret copies / ensure zeroization

3. Adding comprehensive tests
   - Regression tests for previously failing inputs
   - Corruption/tamper tests for persisted formats
   - CLI e2e tests for command workflows

4. Making components more generic / reusable
   - Prefer traits + bounds over hard-coded types when it improves reuse
   - Keep crate boundaries intact (no `mfm_core` -> `mfm_cli` coupling)

5. Feature additions
   - New CLI subcommand with stable JSON output
   - New state machine scheduler/tracker implementation with tests


## Code Style & API Guidelines

### Rust Style

- Prefer explicit, readable code over cleverness.
- Avoid panics in library code. Use `Result` and typed errors.
- Keep public APIs documented and consistent (names, error behavior, invariants).
- Prefer `&str`/borrowing where possible; avoid cloning in hot paths.

### Error Handling

- Libraries:
  - Prefer typed errors (e.g. `thiserror`) for stable, testable behavior.
  - Use `anyhow` primarily for glue code or when error typing adds little value.
- CLI:
  - Preserve stable, machine-readable error codes (see `mfm_cli/README.md`).
  - Avoid breaking the JSON output schema.

### Logging

- Library crates:
  - Prefer `log` to avoid forcing a subscriber on downstream users.
- Binaries:
  - Prefer `tracing` (CLI initializes `tracing_subscriber` in `mfm_cli/src/main.rs`).
- Never log secrets (passwords, mnemonics, private keys, raw ciphertext).

### Unsafe

Avoid `unsafe` where possible. If you must use it:

- Explain the safety invariants (why it is safe, what must remain true).
- Add tests that would fail if invariants are violated.

## Domain-Specific Guidance

### Keystore (`mfm_core/src/keystore/`)

This is security-sensitive code. Treat changes here as high risk.

Rules:

- Do not introduce secret-bearing debug output.
- Keep secrets in `zeroize::Zeroizing` (or equivalent), minimize copies, and aggressively drop temporary buffers.
- Preserve constant-time comparisons where used (`subtle`).
- Keep the threat model in mind: tamper detection, swap attacks, DoS via file size.
- Do not make `Keystore` `Send`/`Sync` without a deliberate redesign.
- Be careful with format compatibility: keystore files are persisted JSON. If you add fields, use `#[serde(default)]` for backwards compatibility.

When adding features (export, password change, etc.):

- Add unit tests and corruption/tamper tests.
- Prefer explicit, test-backed behavior over implicit "best effort".

### CLI (`mfm_cli/`)

The CLI is designed to be scriptable and AI-friendly.

- Respect global `--output-format` and `MFM_OUTPUT_FORMAT`.
- When adding commands, maintain both:
  - human-readable `text` output
  - machine-readable `json` output with stable structure
- Provide non-interactive modes (`--stdin`, `--yes`, env vars) where appropriate.
- Keep outputs stable; treat output format as part of the public API.

Run locally:

```bash
cargo run -p mfm --bin mfm_cli -- --help
cargo run -p mfm --bin mfm_cli -- keystore list
```

### State Machine (`mfm_machine/`)

The state machine is async and uses a typed tag/label system.

- Keep states deterministic unless explicitly tagged as side-effecting.
- Do not block the async runtime:
  - use async I/O where possible
  - use `tokio::task::spawn_blocking` for CPU-bound or blocking work
- Prefer `SafeContext::{read_typed, write_typed}` over ad-hoc JSON.
- If you change scheduling/recovery semantics, add tests that assert behavior.

### Proc-Macros (`mfm_machine_derive/`)

- Optimize for clear compile-time errors.
- Avoid expanding to surprising code (keep generated impls small and idiomatic).
- Add targeted tests in consuming crates when macro behavior changes.

## Testing Guidance

Prefer tests that lock in behavior at boundaries:

- Unit tests for pure functions and small components.
- Integration tests for interactions between components like:
  - keystore persistence and file integrity
  - CLI command workflows (happy path + failure modes)
  - state machine execution and recovery

Optional but useful in the right places:

- Property tests for invariants (parsing/validation code).
- Fuzz tests for parsers/decoders/serialization.
- Benchmarks for performance-critical paths (crypto, hot loops, large files).

If a bug can reappear, write a regression test.

## What To Avoid

- Introducing new dependencies without strong justification.
- Changing CLI output schemas casually.
- Storing secrets in repo files, logs, fixtures, or test snapshots.

## Performance Considerations

- Avoid accidental allocations in hot paths (crypto, parsing, tight loops).
- Prefer borrowing (`&[u8]`, `&str`) to cloning, especially for large buffers.
- In async code, do not block the runtime; use `tokio::task::spawn_blocking`.
- Be mindful of lock scope for `SafeContext` (keep read/write lock holds short).

## Common Pitfalls

- Logging secrets (passwords, private keys, mnemonics, decrypted buffers).
- Breaking persisted formats without `#[serde(default)]` or migration strategy.
- Holding locks across `.await` (can deadlock or stall progress).
- Introducing `unwrap()`/`expect()` in library code where errors should be surfaced.
- Changing CLI output schema or error codes without updating docs and tests.

## CI Requirements

Before opening a PR (or finishing a change), ensure [Code Style and Standards](#code-style-and-standards) are met.

If you use Nix or need CI parity, also run: `nix flake check && nix build`.

## Debugging Tips

- Backtraces: set `RUST_BACKTRACE=1` (CI already does).
- CLI logging: use `tracing::{debug, info, warn, error}` with a clear target.
- Library logging: use `log::{debug, info, warn, error}`.
- Tests: prefer isolated temp dirs/files via `tempfile`.


## Commenting Guidelines (Keep Future Readers in Mind)

Write comments that remain valuable after the PR merges.

Good comments explain WHY / constraints / invariants:

```rust
// AAD binds ciphertext to entry id, preventing swap attacks.
// Any change here must remain constant-time.
```

Avoid comments that just restate code or describe the PR.

`unsafe` blocks must always be documented.

DO:

```rust
// HashMap provides O(1) lookups during keystore list filtering.
// This avoids repeated linear scans in the CLI hot path.
```

DONT:

```rust
// Changed from Vec to HashMap for O(1) lookups.
```

## Quick Reference

### Essential Commands

```bash
# Format code
cargo +nightly fmt --all

# Run lints
cargo +nightly clippy --workspace --lib --examples --tests --benches --all-features

# Run tests
cargo nextest run --workspace

# Run specific benchmark
cargo bench --bench bench_name

# Build optimized binary
cargo build --release --features "jemalloc asm-keccak"

# Check compilation for all features
cargo check --workspace --all-features

# Check documentation
cargo docs --document-private-items 
```

<<< END OF PROJECT_AGENTS.md >>>

<<< FILE: NIXFIED_FRAMEWORK_README.md >>>
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
```

Template defaults are safe no-ops: `dev`, `test`, `build`, and `check` print a
placeholder and exit 0. The CI pipeline is enabled and runs placeholder steps.
Replace each command in its file under `nixfied/project/`.

## Install into an existing repo

From your target repository:

```bash
cd my-app
nix run github:willyrgf/nixfied#framework::install
```

Safety behavior:
- If the repo name does not end with `_nixified`, the installer copies the repo
  to `<repo>_nixified` (or `--target`) and re-runs itself there.
- It refuses to install/upgrade unless the repo name ends with `_nixified`.
- It installs only `flake.nix`, `flake.lock`, and `nixfied/`.

Force overwrite:

```bash
nix run github:willyrgf/nixfied#framework::install -- --force
# or
NIXFIED_INSTALL_FORCE=1 nix run github:willyrgf/nixfied#framework::install
```

Fast install (optional, avoids copying large repos):

```bash
nix run github:willyrgf/nixfied#framework::install -- --worktree
```

Custom target directory:

```bash
nix run github:willyrgf/nixfied#framework::install -- --target /path/to/my-app_nixified
```

If the target already exists and you want to refresh it from the source repo:

```bash
nix run github:willyrgf/nixfied#framework::install -- --force --sync
```

Upgrade an existing `_nixified` repo (preserves `nixfied/project/` by default):

```bash
cd my-app_nixified
nix run github:willyrgf/nixfied#framework::upgrade -- --force
```

To overwrite project templates during upgrade:

```bash
nix run github:willyrgf/nixfied#framework::upgrade -- --force --reset-project
```

Filter which project files are installed (conf is always included):

```bash
nix run github:willyrgf/nixfied#framework::install -- --filter=conf,test,ci
```

Prompt plan (optional):
- Generate manually:

```bash
nix run github:willyrgf/nixfied#framework::prompt-plan
```

- Or generate as part of install/upgrade:

```bash
nix run github:willyrgf/nixfied#framework::install -- --prompt-plan
```

- Disable with `NIXFIED_PROMPT_PLAN=0`.
- Overwrite with `--prompt-plan-force` or `NIXFIED_PROMPT_PLAN_OVERWRITE=1`.

Framework-only apps:
- The `framework::install`, `framework::upgrade`, `framework::prompt-plan`
  (prompt generator), and `framework::test` apps are
  only exposed when the repository contains `nixfied/.framework`.
- The installer removes this marker in target repos so `nix flake show` will
  not list those apps after installation.
- If you want to run framework tests from an installed repo, create the marker
  file (`touch nixfied/.framework`) locally.

Framework workspace:
- Framework maintenance commands live under the `framework::` namespace so they
  don't collide with project commands (e.g. `nix run .#framework::test`).
  This workspace only exists when `nixfied/.framework` is present.

## Repository layout

```
flake.nix
nixfied/
  .framework          # marker: enables framework::* apps
  internal/
    core.nix           # dev/test/build/check/help apps
    install.nix        # installer + upgrade + prompt-plan
    test.nix           # framework test runner
    isolation.nix      # parallel isolation stress test
    module-apps.nix    # auto-generated module apps (db-*, nginx-*, supervisor)
  project/
    conf.nix           # base configuration
    dev.nix            # dev command
    test.nix           # test command
    prod.nix           # build/prod command(s)
    quality.nix        # check command
    ci.nix             # CI command + pipeline DSL
    default.nix        # merges the files above
  lib/
    default.nix        # aggregator re-exporting all lib functions
    helpers.nix        # shell helper script generation
    builders.nix       # mkApp / mkAppScript / withTiming
    summary.nix        # summary parser for CI output
    run-registry.nix   # run tracking with meta.json + background mode
    parallel.nix       # parallel runner generation
    process.nix        # signal handler + process manager
    port-utils.nix     # port cleanup + conflict checker
  postgres/
    default.nix        # aggregator
    lifecycle.nix      # init/start/stop
    config.nix         # postgresql.conf generation
    backup.nix         # backup/restore/list
    migration.nix      # migration runner
    migration-safety.nix # pre-migration safety checks
    port-management.nix  # port conflict resolution
    rollback.nix       # rollback support
  nginx/
    default.nix        # aggregator
    lifecycle.nix      # init/start/stop/reload
    templates.nix      # config templates
    site-management.nix # add/remove/enable/disable sites
    ssl.nix            # certificate management
  supervisor/
    default.nix        # aggregator
    config.nix         # process-compose YAML generation
    lifecycle.nix      # start/stop/restart
    status.nix         # status/isRunning/logs
    management.nix     # daemon management
  ci.nix               # pipeline runner (ephemeral, summary.json)
  ephemeral.nix        # ephemeral environments (slot locking, cleanup)
  slots.nix            # slot/env/port logic
  hooks.nix            # exported hook env vars
  devshell.nix         # nix develop shell
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

ports = {
  backend = 3000;
  frontend = 3100;
  http = 8080;
  https = 8443;
  postgres = 5432;
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
  script = ''
    echo "hello"
  '';
};
```

Files by convention:
- `dev.nix` -> `dev`
- `test.nix` -> `test`
- `prod.nix` -> `build`
- `quality.nix` -> `check`
- `ci.nix` -> `ci`

## Execution environment

Every command is wrapped by `nixfied/lib.nix` and gets:
- `COMMAND_NAME` set to the command name.
- `.env` loaded if present (does not override existing env vars).
- `tooling.runtimePackages` added to `PATH`.
- `install.deps` (if `useDeps = true`).
- Framework helper functions (see below).
- Hook environment variables from `nixfied/hooks.nix`.

## Slots, environments, and ports

- `PROJECT_ENV` selects the environment (`dev`, `test`, `prod`).
- `NIX_ENV` selects the slot (0-9).

Ports are computed as:

```
computed_port = base_port + slot + env_offset
```

`nixfied/slots.nix` exposes helper scripts:
- `SLOT_INFO` prints `SLOT`, `ENV`, `BASE_DIR`, `LOG_DIR`, `RUN_DIR`,
  `CONFIG_DIR`, `STATE_DIR`, and all computed ports.
- `REQUIRE_SLOT_ENV` validates env/slot, prints values, and prompts in TTY
  if `PROJECT_ENV` or `NIX_ENV` were not explicitly set.

Example usage:

```bash
eval "$(${SLOT_INFO})"
echo "Backend port: $BACKEND_PORT"
```

## Framework helpers (shell)

Every command sources a helper script generated by `nixfied/lib.nix`.

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
  - When used with command substitution (`PID=$(start_service ...)`), cleanup is
    not auto-registered; call `stop_service` yourself.
- `stop_service PID [name]`
  - Stop a background service.
- `with_service NAME [start opts] -- <start command...> --run <command...>`
  - Start a service, then run a command; cleanup is automatic.
- `run_hook ENV_VAR [args...]`
  - Execute the command stored in `ENV_VAR`.
- `artifact_dir`
  - Return the CI artifacts directory.
- `artifact_path NAME`
  - Return a full path inside the artifacts directory.

Service example:

```bash
PID=$(start_service backend --log /tmp/backend.log --wait-port 3000 -- ./start-backend)
./run-tests
stop_service "$PID" backend
```

## Nix helper functions (lib)

`nixfied/lib/` is a directory of Nix modules re-exported through
`nixfied/lib/default.nix`. Import it in your Nix wiring:

```nix
lib = import ./nixfied/lib { inherit pkgs project hooks; };
```

Exported functions:

- **builders.nix** - `mkApp`, `mkAppWithDeps`, `mkAppScript`, `withTiming`
- **helpers.nix** - `loadEnv`, `helpersScript`, `hookExports`
- **summary.nix** - `summaryParser`
- **parallel.nix** - `mkParallelRunner`
- **port-utils.nix** - `mkPortCleanup`, `mkPortConflictChecker`
- **process.nix** - `mkSignalHandler`, `mkProcessManager`
- **run-registry.nix** - `runRegistryStart` (run tracking with meta.json)

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
      requires = [ "nginx" ];
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
- `requires` (list of module names, e.g., `postgres`, `nginx`)

CLI usage:

```bash
nix run .#ci                 # default mode
nix run .#ci -- --summary    # summary output
nix run .#ci -- --mode app   # select mode
nix run .#ci -- --app        # shorthand for mode "app"
nix run .#ci -- --bg         # run in background via run registry
```

The CI runner writes `summary.json` to the artifacts directory after each run,
containing mode, exit code, and per-step results (name, status, duration).

CI environment variables available inside steps:
- `CI_MODE`
- `CI_SUMMARY`
- `CI_ARTIFACTS_DIR`
- `CI_KEEP_ARTIFACTS_ON_FAILURE`
- `CI_KEEP_ARTIFACTS_ON_SUCCESS`

Artifacts:
- Stored in `ci.artifacts.dir` and also in `CI_ARTIFACTS_DIR`.
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
`nixfied/internal/module-apps.nix`. These are listed under "Module Apps" in
`nix run .#help`.

Postgres apps (when `modules.postgres.enable = true`):
- `db-start`, `db-stop`, `db-init`, `db-setup`, `db-full-start`
- `db-backup`, `db-restore`, `db-list-backups`
- `db-test-migrations`, `db-check-port`, `db-list-instances`

Nginx apps (when `modules.nginx.enable = true`):
- `nginx-start`, `nginx-stop`, `nginx-init`, `nginx-reload`
- `nginx-site-add`, `nginx-site-remove`, `nginx-site-list`
- `nginx-site-enable`, `nginx-site-disable`
- `nginx-cert-obtain`, `nginx-cert-renew`

Supervisor apps (when `supervisor.enable = true`):
- `up`, `down`, `supervisor-status`, `supervisor-logs`, `supervisor-restart`

Utility apps (always available):
- `check-ports`, `ports`

## Run registry

The run registry (`nixfied/lib/run-registry.nix`) provides durable run tracking.
Each run creates a directory with `meta.json` (status, timing, exit code) and
`output.log`.

Used by CI `--bg` mode to detach runs into the background. The runs root
defaults to `/tmp/<project-id>-runs` (configurable via `ci.runsRoot`).

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
- `POSTGRES_INIT`, `POSTGRES_START`, `POSTGRES_STOP`, `POSTGRES_SETUP_DB`
- `POSTGRES_FULL_START`, `POSTGRES_FULL_START_TEST`
- `POSTGRES_BACKUP`, `POSTGRES_RESTORE`, `POSTGRES_LIST_BACKUPS`
- `POSTGRES_TEST_MIGRATIONS`, `POSTGRES_ENSURE_MIGRATION_TESTED`
- `POSTGRES_CHECK_PORT`, `POSTGRES_KILL_PORT`, `POSTGRES_LIST_INSTANCES`

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
- `NGINX_INIT`, `NGINX_START`, `NGINX_STOP`, `NGINX_RELOAD`
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

The `nixfied/supervisor/` module generates a process-compose YAML and provides
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

To include the isolation runner in framework tests:

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

<<< END OF NIXFIED_FRAMEWORK_README.md >>>
