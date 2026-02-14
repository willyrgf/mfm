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
- `MFM_EVM_RPC_URL`: EVM JSON-RPC URL (used by EVM/tx ops when request payload does not override `rpc_url`)
- `MFM_EVM_RPC_AUTHORIZATION`: optional Authorization header value for the EVM RPC transport
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
