# Helios Mainnet Local RPC Setup (Nixfied)

This repo already ships a Nixfied `helios` service. The goal here is:

- run Helios locally
- have it serve JSON-RPC on `127.0.0.1:<port>`
- back it with **Ethereum mainnet** data using upstream providers

## Configuration

Nixfied auto-loads a root `.env` file (if present) for `nix run ...` commands.

This repo's `.env` is intentionally gitignored (`/.env` in `.gitignore`).

Recommended `.env` values (see also `.env.example`):

- `HELIOS_NETWORK=mainnet`
- `HELIOS_EXECUTION_RPC_URL=https://eth.drpc.org`
- `HELIOS_CONSENSUS_RPC_URL=https://www.lightclientdata.org` (optional; defaults to this for mainnet if omitted)
- `HELIOS_CHECKPOINT=0x...` (optional; if omitted, Nixfied derives one from the consensus endpoint at start time)
- `ETHEREUM_MAINNET_RPC_WSS=wss://ethereum-rpc.publicnode.com` (optional; not used unless you point Helios at it)

### Deriving a Checkpoint Manually (Optional)

If you want to pin the weak-subjectivity checkpoint explicitly:

```bash
CONS="https://www.lightclientdata.org"
slot=$(curl -fsS "$CONS/eth/v1/beacon/headers/finalized" | jq -r '.data.header.message.slot|tonumber')
epoch_start=$(( slot - (slot % 32) ))
checkpoint=$(curl -fsS "$CONS/eth/v1/beacon/headers/$epoch_start" | jq -r '.data.root')
echo "HELIOS_CHECKPOINT=$checkpoint"
```

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

The final JSON now includes both metadata and the decoded snapshot payload at
`data.result.snapshot` (in addition to `snapshot_artifact_id`).

`MFM_KEEP_SERVICES` is no longer supported by this app.
To keep Helios/Postgres running after the command exits (so Helios sync progress keeps advancing),
start them explicitly first:

```bash
MFM_ENV=dev NIX_ENV=0 nix run .#service::postgres::start
MFM_ENV=dev NIX_ENV=0 nix run .#service::helios::start
nix run .#mfm::portfolio::snapshot -- 0x000000000000000000000000000000000000dead
```

Stop them manually when done:

```bash
MFM_ENV=dev NIX_ENV=0 nix run .#service::helios::stop
MFM_ENV=dev NIX_ENV=0 nix run .#service::postgres::stop
```

Inspect service ownership/reuse state with:

```bash
nix run .#process::status -- --all
nix run .#service::helios::events -- --limit 100
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
- If you don't set `HELIOS_CONSENSUS_RPC_URL` for mainnet, the Nixfied Helios wrapper defaults it to `https://www.lightclientdata.org`.
- `HELIOS_CHECKPOINT` is part of the weak-subjectivity trust model. If you want explicit, deterministic control, pin `HELIOS_CHECKPOINT` in `.env` instead of relying on auto-derivation.
- Helios requires the upstream execution RPC to support `eth_getProof` for **explicit block numbers** (not only `latest`).
  - Some free RPCs return `distance to target block exceeds maximum proof window` for `eth_getProof` at numeric blocks (e.g. `eth.llamarpc.com`).
  - If `mfm::portfolio::snapshot` fails before writing `snapshot_artifact_id`, switch to an execution RPC that supports proofs for recent numeric blocks (example: `https://eth.drpc.org`).
