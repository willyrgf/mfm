# Keystore CLI Reth Tx Sign/Send Integration (2026-02-14)

## Scope
- Implement plan from `KEYSTORE_CLI_INTEGRATION_TEST.md`.
- Exclude plan file itself from commit.

## Initial Exploration
- Keystore command registry: `bin/cli/src/commands/keystore/mod.rs`.
- Existing keystore commands: `bin/cli/src/commands/keystore/{import.rs,list.rs,delete.rs}`.
- Command result/error contract: `bin/cli/src/commands/result.rs`.
- Output formatting contract: `bin/cli/src/presentation/output.rs`.
- Keystore unlock/path helpers: `bin/cli/src/support/keystore_manager.rs`.
- Existing parity pipeline wiring: `nixfied/project/ci.nix`.
- Existing parity test patterns: `tests/integration/tests/parity_*.rs`.

## Decisions In Progress
- Add dedicated CLI support modules for tx signing and rpc submission:
  - `bin/cli/src/support/evm_tx_signing.rs`
  - `bin/cli/src/support/evm_rpc.rs`
- Reuse shared key selector semantics in `KeystoreManager` (`--id` or `--by-label`).
- Gate new heavy CLI parity integration tests with `parity-tests` feature in `bin/cli/Cargo.toml`.

## Implemented Decisions
- Added CLI feature gate: `bin/cli/Cargo.toml` -> `[features] parity-tests = []`.
- Added keystore commands:
  - `bin/cli/src/commands/keystore/tx_sign.rs`
  - `bin/cli/src/commands/keystore/tx_send_raw.rs`
- Added support helpers:
  - `bin/cli/src/support/evm_tx_signing.rs`
  - `bin/cli/src/support/evm_rpc.rs`
- Added shared key selector resolution:
  - `bin/cli/src/support/keystore_manager.rs::resolve_key_id`
- Added parity integration test module:
  - `bin/cli/tests/parity_keystore_reth_tx_send.rs` (`#![cfg(feature = "parity-tests")]`)
- Added dedicated parity CI step in `nixfied/project/ci.nix`:
  - `parity-keystore-reth-tx-sign-send`
- Updated user-facing docs:
  - `bin/cli/README.md` (new `keystore tx-sign` + `keystore tx-send-raw` sections)

## Validation
- Build-only compile for new parity test:
  - `cargo test -p mfm --features parity-tests --test parity_keystore_reth_tx_send --no-run`
  - Result: pass
- Full CLI compile (feature-gated):
  - `cargo test -p mfm --features parity-tests --no-run`
  - Result: pass
- Quality checks:
  - `nix run .#check`
  - Result: pass (existing warning in `crates/core/src/keystore/mod.rs` about `OpenOptions` truncate behavior)
- Full parity CI mode:
  - `nix run .#ci -- --parity --summary`
  - Result: pass
  - Summary artifact: `/tmp/ci-artifacts/20260214-133718-75806a4a/summary.json`

## Debugging Notes
- Initial parity run failure:
  - Symptom: `rpc call eth_sendTransaction failed: {"code":-32602,"message":"invalid transaction request"}`
  - Fix: include explicit transfer gas and gas price in funding transaction request in `bin/cli/tests/parity_keystore_reth_tx_send.rs`.

## Next Time
- Add a shared parity RPC test helper crate/module to avoid per-test JSON-RPC boilerplate.
- Add one unit test around funding tx request shape to catch `eth_sendTransaction` schema regressions early.
- Consider a lightweight CI mode selector for running only one parity step while iterating.
