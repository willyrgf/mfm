# Repository Map

Generated from `docs/repo-index.json`.

## Start Here
- `docs/repo-index.json` - Canonical deterministic repository index.
- `docs/repo-map.md` - LLM-facing repository map generated from docs/repo-index.json.
- `README.md` - Primary repository overview and command entrypoints.
- `AGENTS.md` - Agent instructions and collaboration constraints.
- `docs/architecture.md` - Project documentation.
- `docs/redesign.md` - Project documentation.

## Components
- `Cargo.toml` (rust-cargo)
- `bin/cli/Cargo.toml` (rust-cargo)
- `bin/rest-api/Cargo.toml` (rust-cargo)
- `crates/app/Cargo.toml` (rust-cargo)
- `crates/collectors/evm-jsonrpc-http/Cargo.toml` (rust-cargo)
- `crates/collectors/evm/Cargo.toml` (rust-cargo)
- `crates/core/Cargo.toml` (rust-cargo)
- `crates/machine-derive/Cargo.toml` (rust-cargo)
- `crates/machine-test-support/Cargo.toml` (rust-cargo)
- `crates/machine/Cargo.toml` (rust-cargo)
- `crates/ops/common/Cargo.toml` (rust-cargo)
- `crates/ops/evm-deploy-configure-validate-op/Cargo.toml` (rust-cargo)
- `crates/ops/evm-read-op/Cargo.toml` (rust-cargo)
- `crates/ops/evm-write-op/Cargo.toml` (rust-cargo)
- `crates/ops/keystore-admin-op/Cargo.toml` (rust-cargo)
- `crates/ops/keystore-op/Cargo.toml` (rust-cargo)
- `crates/ops/keystore-tx-op/Cargo.toml` (rust-cargo)
- `crates/ops/nix-app-op/Cargo.toml` (rust-cargo)
- `crates/ops/portfolio-tracker-op/Cargo.toml` (rust-cargo)
- `crates/ops/proof-op/Cargo.toml` (rust-cargo)
- `crates/sdk/Cargo.toml` (rust-cargo)
- `crates/storages/artifact-store-fs/Cargo.toml` (rust-cargo)
- `crates/storages/artifact-store-s3/Cargo.toml` (rust-cargo)
- `crates/storages/artifact-store-secret/Cargo.toml` (rust-cargo)
- `crates/storages/event-store-mem/Cargo.toml` (rust-cargo)
- `crates/storages/event-store-postgres/Cargo.toml` (rust-cargo)
- `crates/storages/indexer/Cargo.toml` (rust-cargo)
- `crates/tools/architecture-verify/Cargo.toml` (rust-cargo)
- `flake.nix` (nix-flake)
- `tests/integration/Cargo.toml` (rust-cargo)

## Command Surfaces
- `ci` from `nixfied/project/ci.nix`

## Sensitive Zones
- `crates/core/src/keystore` - Security-sensitive key handling, tamper detection, and persisted keystore compatibility. (checks: nix run .#check, nix run .#test, nix run .#ci -- --audit --summary)
- `crates/machine` - Recovery, replay, and deterministic state-machine runtime semantics. (checks: nix run .#check, nix run .#test, nix run .#ci -- --parity --summary)
- `nixfied/.framework` - Framework internals; avoid direct edits in installed repos. (checks: nix run .#help)
- `nixfied/project/ci.nix` - CI pipeline behavior and release gates. (checks: nix run .#ci -- --summary)
- `nixfied/project/conf.nix` - Project identity, environment names, and port contract. (checks: nix run .#check, nix run .#ci -- --summary)
- `nixfied/project/quality.nix` - Quality checks and discovery drift enforcement. (checks: nix run .#check)

## Canonical Commands
- `nix run .#help`
- `nix run .#dev`
- `nix run .#test`
- `nix run .#build`
- `nix run .#check`
- `nix run .#ci -- --summary`

## Invariants
- Treat `nixfied/project/` as the primary customization surface.
- Keep command metadata (`api`) aligned with script behavior.
- Keep this map and `docs/repo-index.json` in sync via `nix run .#check`.
