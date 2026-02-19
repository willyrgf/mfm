# MFM REST API Documentation

Experimental REST API (Axum) for starting/resuming runs and inspecting run events/artifacts.

## Running (Nixfied)

```bash
# Local dev workflow (starts Postgres + MinIO + REST API)
nix run .#dev

# Run the server only (requires DATABASE_URL)
nix run .#mfm_rest_api
```

## Configuration

Environment variables:

- `MFM_REST_API_ADDR`: bind address (default: `127.0.0.1:3001`)
- `DATABASE_URL`: Postgres URL for the event store (required)
- `MFM_ARTIFACT_BACKEND`: `fs` (default) or `s3`
- `MFM_ARTIFACT_ROOT`: artifact root dir when using `fs` (default: `~/.mfm/run_artifacts`)
- `MFM_S3_ENSURE_BUCKET`: if set (any value), ensure the S3 bucket exists on startup
- `MFM_EVM_RPC_SOURCES_JSON`: optional JSON array of source objects for `namespace="evm"` routing.
  Example:
  ```json
  [
    {"id":"helios_local","rpc_url":"http://127.0.0.1:8545","kind":"local"},
    {"id":"drpc_public","rpc_url":"https://eth.drpc.org","kind":"remote_public"}
  ]
  ```
- `MFM_EVM_RPC_PREFERRED_ORDER`: optional comma-separated source IDs used as routing preference.
- `MFM_EVM_RPC_STRATEGY`: optional routing strategy (`failover` or `hedged_light`; default `hedged_light`).
- `MFM_EVM_RPC_HEDGE_DELAY_MS`: optional hedge delay in milliseconds for `hedged_light`.
- `MFM_EVM_RPC_UNHEALTHY_COOLDOWN_CALLS`: optional unhealthy cooldown in logical call-count units.
- `MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS`: optional comma-separated source IDs that must pass `eth_getProof` probe.
- `MFM_EVM_RPC_LOGS_MAX_BLOCK_SPAN`: optional initial max block span for `eth_getLogs` chunking.
- `MFM_EVM_RPC_LOGS_MIN_BLOCK_SPAN`: optional minimum block span for adaptive `eth_getLogs` splitting.
- `MFM_EVM_RPC_LOGS_MAX_CHUNKS_PER_CALL`: optional chunk/retry budget cap for a single `eth_getLogs` request.
- Legacy compatibility:
  - `MFM_EVM_RPC_URL`: single-source fallback endpoint (mapped to source id `user_primary`).
  - `MFM_EVM_RPC_AUTHORIZATION`: optional Authorization header for that legacy single source.
- `MFM_PORTFOLIO_TOKENS_JSON`: optional JSON array of ERC-20 token specs used by `feature_id = "portfolio.snapshot"`

## API

All responses are JSON envelopes:

- success: `{"status":"success","data": ...}`
- error: `{"status":"error","error":{"code":"...","message":"..."}}`

Endpoints:

- `GET /v1/health`
- `GET /v1/ready`
- `GET /v1/features`
- `POST /v1/features/:feature_id/execute`
- `POST /v1/runs/start`
- `POST /v1/runs/:run_id/resume`
- `GET /v1/runs/:run_id/status`
- `GET /v1/runs/:run_id/events?from_seq=1&to_seq=<optional>`
- `GET /v1/artifacts/:artifact_id`

`namespace="evm"` routing notes:

- EVM read transport routes by source ID (`route.source_id`) and runtime source config.
- Per-request `rpc_url` override is rejected for all EVM calls.
- EVM routing runbook: [`../../docs/evm-rpc-routing.md`](../../docs/evm-rpc-routing.md)

Probe semantics:

- `/v1/health`: liveness only (process is running)
- `/v1/ready`: readiness (event store + artifact store probes must succeed)

Start a run:

```bash
curl -s "http://127.0.0.1:3001/v1/runs/start" \
  -H "content-type: application/json" \
  -d '{"op_id":"proof","op_version":"v1","op_config":{}}'
```

Generic feature execution:

```bash
curl -s "http://127.0.0.1:3001/v1/features/run.start/execute" \
  -H "content-type: application/json" \
  -d '{"payload":{"op_id":"proof","op_version":"v1","op_config":{}}}'
```

Portfolio snapshot feature:

```bash
curl -s "http://127.0.0.1:3001/v1/features/portfolio.snapshot/execute" \
  -H "content-type: application/json" \
  -d '{"payload":{"address":"0x000000000000000000000000000000000000dead","chain_id":1,"tokens":[]}}'
```

Supported `op_id` values (current):

- `proof` (default)
- `evm_read`
- `keystore_tx_sign`
- `keystore_tx_send_raw`
- `portfolio_tracker`
- `nix_app`

There are no dedicated keystore tx endpoints. Use generic run APIs (`/v1/runs/start`, `/v1/runs/:run_id/resume`) with those `op_id` values.

Supported `feature_id` values (current):

- `portfolio.snapshot`

Docs:

- Design contract: [`../../docs/redesign.md`](../../docs/redesign.md)
- Architecture overview: [`../../docs/architecture.md`](../../docs/architecture.md)
- CLI docs (parallel surface): [`../cli/README.md`](../cli/README.md)
