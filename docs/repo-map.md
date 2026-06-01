# Repository Map

Generated from `docs/repo-index.json`.

## Start Here
- `docs/repo-index.json` - Canonical deterministic repository index.
- `docs/repo-map.md` - LLM-facing repository map generated from docs/repo-index.json.
- `README.md` - Primary repository overview and command entrypoints.
- `docs/DETAILED.md` - Detailed model architecture and contracts.
- `docs/UPGRADE.md` - Downstream upgrade notes for behavioral and path contract changes.
- `AGENTS.md` - Agent instructions and collaboration constraints.
- `docs/architecture.md` - Project documentation.
- `docs/code-quality.md` - Project documentation.
- `docs/design.md` - Project documentation.
- `docs/ops-and-states.md` - Project documentation.

## Components
- `Cargo.toml` (rust-cargo)
- `bin/cli/Cargo.toml` (rust-cargo)
- `bin/rest-api/Cargo.toml` (rust-cargo)
- `crates/app/Cargo.toml` (rust-cargo)
- `crates/authored-config/Cargo.toml` (rust-cargo)
- `crates/collectors/btc-jsonrpc-http/Cargo.toml` (rust-cargo)
- `crates/collectors/proof/Cargo.toml` (rust-cargo)
- `crates/core/Cargo.toml` (rust-cargo)
- `crates/docs/Cargo.toml` (rust-cargo)
- `crates/evm-core/Cargo.toml` (rust-cargo)
- `crates/evm-dcv-model/Cargo.toml` (rust-cargo)
- `crates/evm-deploy-configure-validate-config/Cargo.toml` (rust-cargo)
- `crates/kernel/canonical/Cargo.toml` (rust-cargo)
- `crates/kernel/capabilities/Cargo.toml` (rust-cargo)
- `crates/kernel/certify/Cargo.toml` (rust-cargo)
- `crates/kernel/effects/Cargo.toml` (rust-cargo)
- `crates/kernel/events/Cargo.toml` (rust-cargo)
- `crates/kernel/ids/Cargo.toml` (rust-cargo)
- `crates/kernel/program-derive/Cargo.toml` (rust-cargo)
- `crates/kernel/program/Cargo.toml` (rust-cargo)
- `crates/kernel/replay/Cargo.toml` (rust-cargo)
- `crates/kernel/runtime/Cargo.toml` (rust-cargo)
- `crates/kernel/spec/Cargo.toml` (rust-cargo)
- `crates/kernel/store/Cargo.toml` (rust-cargo)
- `crates/kernel/values/Cargo.toml` (rust-cargo)
- `crates/ops/evm-deploy-configure-validate-op/Cargo.toml` (rust-cargo)
- `crates/ops/portfolio-tracker-op/Cargo.toml` (rust-cargo)
- `crates/ops/proof-op/Cargo.toml` (rust-cargo)
- `crates/portfolio-config/Cargo.toml` (rust-cargo)
- `crates/portfolio/model/Cargo.toml` (rust-cargo)
- `crates/states/evm-dcv/Cargo.toml` (rust-cargo)
- `crates/states/portfolio/Cargo.toml` (rust-cargo)
- `crates/storages/artifact-store-fs/Cargo.toml` (rust-cargo)
- `crates/storages/stream-store-postgres/Cargo.toml` (rust-cargo)
- `crates/tools/publish-docs/Cargo.toml` (rust-cargo)
- `crates/transports/evm-dcv/Cargo.toml` (rust-cargo)
- `crates/transports/portfolio/Cargo.toml` (rust-cargo)
- `crates/transports/process-exec/Cargo.toml` (rust-cargo)
- `crates/transports/proof/Cargo.toml` (rust-cargo)
- `flake.nix` (nix-flake)
- `tests/integration/Cargo.toml` (rust-cargo)

## Command Surfaces
- `ci` from `nixfied/project/module.nix`

## Features
- (none detected)

## Dispatcher and Introspection
- `run-task -- <task-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`
- `run-workflow -- <workflow-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`
- `run-workflow-parallel -- <workflow-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`
- `runs [run-id]` from `nixfied/framework/runtime/dispatcher.nix`
- `stop-run -- <run-id>` from `nixfied/framework/runtime/dispatcher.nix`
- `stop-all-runs` from `nixfied/framework/runtime/dispatcher.nix`
- `features` from `nixfied/framework/runtime/dispatcher.nix`
- `introspect`, `stateHash`, `schema` from `nixfied/framework/core/mkNixfied.nix`

## Sensitive Zones
- `crates/core/src/crypto.rs` - Security-sensitive Ethereum private-key parsing, address derivation, and recoverable signing. (checks: nix run .#check, nix run .#test, nix run .#ci -- --audit --summary)
- `crates/core/src/keystore` - Security-sensitive key handling, tamper detection, and persisted keystore compatibility. (checks: nix run .#check, nix run .#test, nix run .#ci -- --audit --summary)
- `nixfied/framework` - Framework internals; avoid direct edits in installed repos. (checks: nix run .#help)
- `nixfied/project/conf.nix` - Project identity, environment names, and port contract. (checks: nix run .#check, nix run .#ci -- --summary)
- `nixfied/project/module.nix` - Modeled tasks, workflows, CI pipeline behavior, quality checks, and discovery drift enforcement. (checks: nix run .#check, nix run .#ci -- --summary)

## Canonical Commands
- `nix run .#help`
- `nix run .#dev`
- `nix run .#test`
- `nix run .#build`
- `nix run .#check`
- `nix run .#format`
- `nix run .#ci -- --summary`
- `nix run .#validate-env`
- `nix run .#test-isolation`
- `nix run .#ports`
- `nix run .#check-ports`
- `nix run .#features`
- `nix run .#framework::test`
- `nix run .#framework::install`
- `nix run .#framework::upgrade`

## Invariants
- Treat `nixfied/project/` as the primary customization surface.
- Keep command metadata aligned with script behavior.
- Keep this map and `docs/repo-index.json` in sync when command surfaces or key docs change.
