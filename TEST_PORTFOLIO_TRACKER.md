# Testing `portfolio_tracker`

This repo currently exposes portfolio snapshots via:

- Operation: `op_id="portfolio_tracker"`, `op_version="v1"`
- REST feature: `feature_id="portfolio.snapshot"` (wraps a run start)
- CLI: `mfm_cli portfolio snapshot <ADDRESS>`

The goal of this document is a test matrix that catches regressions quickly while keeping CI reliable.

## Quick Commands

Fast local checks:

```bash
# Unit/op tests for the op implementation
cargo test -p mfm-op-portfolio-tracker

# Full repo test suite (nextest)
nix run .#test

# Formatting + clippy (CI parity)
nix run .#check
```

## Coverage Status

- [x] Op: writes output artifact (`snapshot_writes_output_artifact`)
- [x] Op: chain id mismatch fails with stable code `chain_id_mismatch`
- [x] Op: decimals fallback (`decimals()` call) when token config omits `decimals`
- [x] Op: token ordering determinism (state IDs + output ordering)
- [x] REST: `GET /v1/features` exposes `portfolio.snapshot`
- [x] REST: `portfolio.snapshot` missing `MFM_EVM_RPC_URL` => `502` + `evm_rpc_url_missing`
- [x] CLI: `mfm_cli portfolio snapshot --help` works (smoke)
- [x] CLI: invalid `--tokens-json` returns `InvalidJson`
- [x] Parity: local `reth` snapshot pipeline (ETH-only)
- [x] Parity: local `reth` snapshot pipeline with deployed MockERC20 (ERC-20 balance path)

## 1) Deterministic Op Tests (No Network)

These are the most valuable tests because they:

- do not require RPC, Postgres, or MinIO/S3
- validate determinism (pinned block, stable output) and replay friendliness
- run fast and reliably in CI

### Where

- `crates/ops/portfolio-tracker-op/src/lib.rs` (`#[cfg(test)]`)

### What To Test

Add/keep tests for:

1. **Happy path with tokens**
- Mocks:
  - `eth_chainId`
  - `eth_blockNumber`
  - `eth_getBalance`
  - `eth_call` for `balanceOf(address)`
  - optional: `eth_call` for `decimals()` (only when token config sets `decimals: null`)
- Assert:
  - run completes
  - final context snapshot includes `portfolio_tracker.main.snapshot_artifact_id`
  - output artifact exists and is valid JSON
  - output schema contains `wallet_address`, `chain_id`, `block_number`, `generated_at_ms`, `native`, `tokens`, `errors`

2. **Chain id mismatch fails**
- Mock `eth_chainId` != configured `chain_id`
- Assert:
  - run fails
  - error code is stable: `chain_id_mismatch`

3. **Decimals fallback**
- Provide token config without `decimals`
- Assert:
  - the op calls `decimals()` and uses that to compute `amount_dec`

4. **Token ordering determinism**
- Pass tokens unsorted in op_config
- Assert:
  - output `tokens` order is stable (sorted by token address)
  - the state graph contains stable token state IDs (full 40-hex suffix, not truncated)

### Running

```bash
cargo test -p mfm-op-portfolio-tracker
```

## 2) REST API Entry Point Tests (No Network)

These ensure the REST surface stays aligned and stable.

### Where

- `tests/integration/tests/rest_api_run_control.rs`

### What To Add/Ensure

1. **Features list exposes `portfolio.snapshot`**
- Extend `features_list_exposes_builtin_catalog`:
  - assert `features` contains `{id: "portfolio.snapshot"}`

2. **`portfolio.snapshot` fails cleanly when RPC is not configured**
- Note: `mfm_app::make_engine_bundle()` captures `MFM_EVM_RPC_URL` at construction time, so ensure the
  env var is unset before you call `make_engine_bundle()` / `make_app(...)`.
- In the existing in-memory REST harness:
  - call `POST /v1/features/portfolio.snapshot/execute` with payload `{ "address": "0x..." }`
  - do *not* set `MFM_EVM_RPC_URL`
- Assert:
  - status is `502 Bad Gateway`
  - error code is stable (today: `evm_rpc_url_missing`)

### Running

```bash
cargo nextest run -p mfm-integration-tests --test rest_api_run_control
```

## 3) CLI Surface Tests (No Network)

These catch CLI parsing/contract regressions early.

### Where

- `bin/cli/tests/` (integration tests)

### What To Add/Ensure

1. `mfm_cli portfolio snapshot --help` works (smoke)
2. Invalid `--tokens-json` returns `InvalidJson`

### Running

```bash
cargo test -p mfm
```

## 4) Parity Test Against Local `reth` (Real RPC)

Use this when you want an end-to-end signal that:

- the op works against an actual node
- `eth_chainId`, `eth_blockNumber`, `eth_getBalance` behave as expected
- optional (ERC-20 path): `eth_call` for `decimals()` and `balanceOf(address)`

### Important: `portfolio.snapshot` Feature (REST + CLI) vs Local `reth`

The `portfolio.snapshot` feature (used by both the REST API and `mfm_cli portfolio snapshot`) defaults
to `chain_id=1` (Ethereum mainnet).
Local `reth` dev networks do not use chain id 1, so parity tests should pass `chain_id` explicitly
(for example, read from `eth_chainId`) to avoid `chain_id_mismatch`.

### Recommended Structure

Minimal ETH-only parity test (already implemented):

- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`

Extended (ERC-20 path; already implemented):

- `tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs`

If you want a larger, end-to-end pipeline (deploy/configure/validate) test, pattern after:

- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`

Pipeline strategy:

1. Fetch dev account (`eth_accounts[0]`)
2. Deploy a minimal ERC-20 token contract (MockERC20) and mint to the dev account
3. Start `portfolio.snapshot` with:
   - `address = <dev account>`
   - `chain_id = <eth_chainId>`
   - `tokens = [{ address: <mock token address>, symbol: "MOCK", decimals: null }]`
4. Assert output artifact includes:
   - the minted token with expected `raw_u256_dec`
   - `block_number` present and stable

If you don’t yet have a MockERC20 + Nix artifact helper:

- add a new Solidity contract under `contracts/src/`
- add a helper similar to `mfm-contract-artifact-configurable-counter` in `nixfied/project/conf.nix`

### Running

From the CI parity target:

```bash
cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_portfolio_tracker_reth_snapshot

# ERC-20 version:
cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_portfolio_tracker_reth_mock_erc20
```

Or using the Nixfied dev workflow (starts Postgres + MinIO + reth + REST API):

```bash
nix run .#dev
```

## 5) Manual Smoke (CLI + REST)

### CLI

Requires:

- `DATABASE_URL`
- `MFM_EVM_RPC_URL`
- optional: `MFM_PORTFOLIO_TOKENS_JSON`

Example:

```bash
export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5432/mfm"
export MFM_EVM_RPC_URL="http://127.0.0.1:8545"
export MFM_PORTFOLIO_TOKENS_JSON='[
  {"address":"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","symbol":"USDC","decimals":6}
]'

# Default is `--chain-id 1`. Local `reth` dev networks typically require passing a different chain id.
nix run .#mfm_cli -- portfolio snapshot 0x000000000000000000000000000000000000dead --chain-id 1 --output-format json
```

### REST

```bash
curl -s "http://127.0.0.1:3001/v1/features/portfolio.snapshot/execute" \
  -H "content-type: application/json" \
  -d '{"payload":{"address":"0x000000000000000000000000000000000000dead","chain_id":1}}'
```
