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
- `MFM_EVM_RPC_URL`: EVM JSON-RPC URL (required for `op_id = "evm_read"`)
- `MFM_EVM_RPC_AUTHORIZATION`: optional Authorization header value for the EVM RPC transport

## API

All responses are JSON envelopes:

- success: `{"status":"success","data": ...}`
- error: `{"status":"error","error":{"code":"...","message":"..."}}`

Endpoints:

- `GET /v1/health`
- `POST /v1/runs/start`
- `POST /v1/runs/:run_id/resume`
- `GET /v1/runs/:run_id/status`
- `GET /v1/runs/:run_id/events?from_seq=1&to_seq=<optional>`
- `GET /v1/artifacts/:artifact_id`

Start a run:

```bash
curl -s "http://127.0.0.1:3001/v1/runs/start" \
  -H "content-type: application/json" \
  -d '{"op_id":"proof","op_version":"v1","op_config":{}}'
```

Supported `op_id` values (current):

- `proof` (default)
- `evm_read`
- `nix_app`

Docs:

- Design contract: [`../../REDESIGN.md`](../../REDESIGN.md)
- Architecture overview: [`../../ARCHITECTURE.md`](../../ARCHITECTURE.md)
- CLI docs (parallel surface): [`../cli/README.md`](../cli/README.md)
