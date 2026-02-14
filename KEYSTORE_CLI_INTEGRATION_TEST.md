# Keystore CLI Integration Test Plan (Reth)

## Summary

Add a new parity integration workflow that proves this end-to-end path on live Reth:

1. Generate a fresh wallet private key.
2. Import it into MFM keystore through CLI.
3. Fund that wallet from Reth dev account 0 via JSON-RPC.
4. Sign an EIP-1559 transaction via new CLI signing command (using keystore-managed key).
5. Submit via separate CLI send command.
6. Validate on Reth with receipt plus balance-delta checks.

This includes both:
- new CLI functionality (`tx-sign` + `tx-send-raw`)
- a dedicated parity integration test plus dedicated parity CI step

## Scope and Locked Decisions

- Test scope: live parity integration against Reth.
- Key source: generate fresh key per test run.
- Keystore surface exercised: CLI path.
- Signing model: pure CLI with new command(s), not API fallback.
- Command shape: separate sign and send commands.
- Tx type: EIP-1559 only (v1).
- Output policy: tx hash plus metadata only (no raw signed tx in stdout/stderr).
- Funding path: `eth_sendTransaction` from unlocked Reth dev account 0.
- CI wiring: dedicated new parity step.

## Public CLI Interface Changes

Add new subcommands under `keystore`.

### `mfm_cli keystore tx-sign`

Purpose:
- unlock keystore key and sign an EIP-1559 tx payload, writing signed raw tx to an output file with restrictive permissions

Required args:
- key selector: `--id <uuid>` or `--by-label <label>`
- `--to <address>`
- `--value-wei <u128-as-decimal-or-hex>`
- `--chain-id <u64>`
- `--nonce <u64>`
- `--max-fee-per-gas <u128>`
- `--max-priority-fee-per-gas <u128>`
- `--gas-limit <u64>`
- `--out <path>`

Optional args:
- `--data <0xhex>` default `0x`
- `--keystore <path>`

Output contract:
- text/json stable response with `from`, `to`, `nonce`, `chain_id`, `tx_type=0x2`, `payload_hash`, `out_path`
- must not include private key, signature bytes, or raw tx hex

### `mfm_cli keystore tx-send-raw`

Purpose:
- submit signed raw transaction from file to RPC and return tx hash

Required args:
- `--rpc-url <url>` (or fallback `MFM_EVM_RPC_URL`)
- `--in <path>`

Output contract:
- text/json stable response with `tx_hash`, `rpc_url_host`, submit timestamp
- must not print raw tx payload

## Internal Implementation Plan

### 1) CLI command modules

Files:
- `bin/cli/src/commands/keystore/mod.rs`
- `bin/cli/src/commands/keystore/tx_sign.rs` (new)
- `bin/cli/src/commands/keystore/tx_send_raw.rs` (new)

Tasks:
- Register new subcommands and clap args.
- Enforce existing output format contract (`text` + `json`).
- Reuse existing `CommandResult`/`CommandError` patterns.

### 2) Transaction signing support in CLI layer

Files:
- `bin/cli/src/support/evm_tx_signing.rs` (new)
- `bin/cli/src/support/mod.rs` (if needed)

Tasks:
- Build EIP-1559 signing payload (typed tx `0x02`) with deterministic encoding.
- Use keystore key retrieval (`get_private_key`) for signing.
- Derive recovery id deterministically from signature plus hash plus expected address.
- Compose final raw tx bytes and hex-encode.
- Write raw tx to `--out` path with restrictive permissions (`0600` Unix).
- Enforce no secret logging or debug leakage.

### 3) RPC submission helper

Files:
- `bin/cli/src/commands/keystore/tx_send_raw.rs`
- optionally `bin/cli/src/support/evm_rpc.rs` (new helper)

Tasks:
- Submit with `eth_sendRawTransaction`.
- Parse JSON-RPC error shape into stable CLI error codes/messages.
- Return tx hash only on success.

### 4) Keystore manager and key selection reuse

Files:
- `bin/cli/src/support/keystore_manager.rs`
- potentially shared selector extraction from existing keystore commands

Tasks:
- Reuse consistent key selection semantics (`--id` or `--by-label`).
- Fail on ambiguous/missing label with stable code.

## Integration Test Plan (Reth)

### Test file

- `bin/cli/tests/parity_keystore_reth_tx_send.rs` (new)
- gated with feature: `#![cfg(feature = "parity-tests")]`

### Test flow

1. Read `MFM_EVM_RPC_URL`.
2. Query `eth_chainId`, `eth_accounts`; pick account0 as funder.
3. Generate random secp256k1 private key and derive address.
4. Create temp keystore path and password file.
5. Run CLI `keystore import --import-type privatekey --stdin` with generated key.
6. Fund generated address via `eth_sendTransaction` from account0 and wait for receipt status `0x1`.
7. Read recipient balance before send.
8. Query sender nonce via `eth_getTransactionCount`.
9. Query fee inputs (`eth_maxPriorityFeePerGas`, fallback policy if unavailable, plus `eth_gasPrice`) and set gas limit.
10. Run `mfm_cli keystore tx-sign ... --out signed.tx`.
11. Run `mfm_cli keystore tx-send-raw --in signed.tx --rpc-url ...`.
12. Wait for receipt and assert:
    - status `0x1`
    - `from == generated address`
    - `to == intended recipient`
    - tx type is EIP-1559 (`0x2` semantics)
13. Verify recipient balance delta is positive (or exact expected increase where deterministic).
14. Assert JSON output schema and assert no raw tx/private data appears.

### Negative scenarios in same module

- `tx-sign` fails with wrong keystore password.
- `tx-sign` fails with missing key selector.
- `tx-sign` fails with ambiguous label.
- `tx-send-raw` fails with malformed input file.
- `tx-send-raw` fails with missing rpc url and missing env var.
- error messages do not leak secret material.

## CI and Nixfied Wiring

### CLI feature

File:
- `bin/cli/Cargo.toml`

Task:
- Add `parity-tests` feature for gating heavy integration tests.

### Dedicated parity step

File:
- `nixfied/project/ci.nix`

Task:
- Add step name (example): `parity-keystore-reth-tx-sign-send`
- Services: at least `reth`
- Command:
  - `cargo nextest run -p mfm --features parity-tests --test parity_keystore_reth_tx_send`
- Capture logs/artifacts per existing parity conventions.

## Test Cases and Acceptance Criteria

### Command-level acceptance

- `keystore tx-sign` writes output file with restrictive permissions and valid typed tx bytes.
- `keystore tx-send-raw` returns tx hash accepted by Reth.
- JSON output schema is stable and machine-parseable.
- No secret leakage in stdout/stderr or error strings.

### End-to-end acceptance

- New key imported into keystore via CLI.
- Transaction signed from keystore key is mined on Reth.
- Receipt confirms successful execution.
- Recipient balance increases.
- Test passes reliably under parity CI environment.

## Assumptions and Defaults

- Reth parity environment exposes `MFM_EVM_RPC_URL`.
- Reth has unlocked dev account 0 available via `eth_accounts` and `eth_sendTransaction`.
- EIP-1559 fields are available; if `eth_maxPriorityFeePerGas` is unavailable, fallback logic is implemented.
- Output policy remains strict: no raw tx/private key in stdout/stderr.
- Tests provide non-interactive keystore password via `MFM_KEYSTORE_PASSWORD_FILE`.
