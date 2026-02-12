# Helios Mainnet Local RPC Setup (Nixfied)

This repo already ships a Nixfied `helios` service. The goal here is:

- run Helios locally
- have it serve JSON-RPC on `127.0.0.1:<port>`
- back it with **Ethereum mainnet** data using upstream providers

## Configuration

Nixfied auto-loads a root `.env` file (if present) for `nix run ...` commands.

This repo's `.env` is intentionally gitignored (`/.env` in `.gitignore`).

Current `.env` values:

- `HELIOS_NETWORK=mainnet`
- `HELIOS_EXECUTION_RPC_URL=https://eth.llamarpc.com`
- `HELIOS_CONSENSUS_RPC_URL=https://www.lightclientdata.org`
- `ETHEREUM_MAINNET_RPC_WSS=wss://ethereum-rpc.publicnode.com` (optional; not used unless you point Helios at it)

## Start Helios (Mainnet)

Run Helios in one terminal (it stays in the foreground):

```bash
MFM_ENV=dev NIX_ENV=0 nix run .#service::helios::check-config
MFM_ENV=dev NIX_ENV=0 nix run .#service::helios::start
```

To stop it from another terminal:

```bash
MFM_ENV=dev NIX_ENV=0 nix run .#service::helios::stop
```

## RPC URL and Ports

Helios binds to `127.0.0.1` only.

With `MFM_ENV=dev NIX_ENV=0`, the default port mapping in `nixfied/project/conf.nix` gives:

- base `heliosRpc=8547`
- dev offset `+10`
- slot stride `+100 * NIX_ENV`

So for `NIX_ENV=0`, Helios JSON-RPC is:

- `http://127.0.0.1:8557`

If you change env/slot, check:

```bash
MFM_ENV=dev NIX_ENV=0 nix run .#service::helios::status
```

## Smoke RPC Calls

### Chain ID

```bash
curl -fsS \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
  "http://127.0.0.1:8557" \
  | jq
```

Expected: `.result == "0x1"` (Ethereum mainnet).

### Latest Block Number

```bash
curl -fsS \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
  "http://127.0.0.1:8557" \
  | jq
```

### Balance (Polling-Friendly)

```bash
ADDR="0x000000000000000000000000000000000000dead"
curl -fsS \
  -H 'content-type: application/json' \
  --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBalance\",\"params\":[\"$ADDR\",\"latest\"]}" \
  "http://127.0.0.1:8557" \
  | jq
```

## Use With MFM (Portfolio Snapshot)

### One-shot (Nix App)

This repo provides an app that starts Postgres + Helios and then snapshots a single address:

```bash
nix run .#mfm::portfolio::snapshot -- 0x000000000000000000000000000000000000dead
```

### Manual (CLI)

`mfm_cli portfolio snapshot` currently requires:

- `DATABASE_URL` (Postgres-backed event store)
- `MFM_EVM_RPC_URL` (point this at your local Helios HTTP RPC)

Start Postgres:

```bash
MFM_ENV=dev NIX_ENV=0 nix run .#service::postgres::start
```

Then run a snapshot against Helios:

```bash
export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5442/mfm"
export MFM_EVM_RPC_URL="http://127.0.0.1:8557"

nix run .#mfm_cli -- portfolio snapshot 0x000000000000000000000000000000000000dead --chain-id 1 --output-format json
```

## Notes / Gotchas

- Helios mainnet requires a **consensus** endpoint (Beacon API / light client updates provider). An Ethereum JSON-RPC endpoint (HTTP/WSS) is not a consensus endpoint.
- Helios requires the upstream execution RPC to support `eth_getProof`. If Helios fails to start due to provider limitations, try switching `HELIOS_EXECUTION_RPC_URL` to the websocket endpoint in `.env`:
  - `HELIOS_EXECUTION_RPC_URL=wss://ethereum-rpc.publicnode.com`
