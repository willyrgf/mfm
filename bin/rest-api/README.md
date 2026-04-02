# MFM REST API Documentation

Experimental REST API (Axum) for starting/resuming runs and inspecting run stream records/artifacts.

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
- `DATABASE_URL`: Postgres URL for the stream store (required)
- `MFM_ARTIFACT_BACKEND`: `fs` (default) or `s3`
- `MFM_ARTIFACT_ROOT`: artifact root dir when using `fs` (default: `~/.mfm/run_artifacts`)
- `MFM_S3_ENSURE_BUCKET`: if set (any value), ensure the S3 bucket exists on startup
- `MFM_EVM_RPC_SOURCES_JSON`: optional JSON array of bootstrap source objects for the canonical
  `rpc.control` source catalog.
  Example:
  ```json
  [
    {"id":"helios_local","network_id":"ethereum-mainnet","rpc_url":"http://127.0.0.1:8545","kind":"local"},
    {"id":"drpc_public","network_id":"ethereum-mainnet","rpc_url":"https://eth.drpc.org","kind":"remote_public"}
  ]
  ```
- `MFM_EVM_RPC_PREFERRED_ORDER`: optional comma-separated source IDs used as bootstrap preference.
- `MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS`: optional comma-separated source IDs that must pass
  `eth_getProof` during `rpc.control` preparation.
- Every `MFM_EVM_RPC_SOURCES_JSON` source must declare a `network_id`.

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
- `GET /v1/runs/:run_id/stream?from_seq=1&to_seq=<optional>`
- `GET /v1/artifacts/:artifact_id`

`rpc.control` routing notes:

- Canonical EVM reads route through `namespace="rpc.control"` with `network_id`.
- `control_scope` defaults to `shared` unless the caller explicitly isolates the request flow.
- Canonical managed requests do not expose caller-controlled source pinning.
- Per-request `rpc_url` override is rejected for managed calls.
- EVM routing runbook: [`../../docs/evm-rpc-routing.md`](../../docs/evm-rpc-routing.md)

Probe semantics:

- `/v1/health`: liveness only (process is running)
- `/v1/ready`: readiness (stream store + artifact store probes must succeed)

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
  -d '{
    "payload": {
      "portfolio": {
        "...": "canonical PortfolioConfig"
      },
      "valuation_source_registry": {
        "...": "canonical ValuationSourceRegistry"
      }
    }
  }'
```

Successful `portfolio.snapshot` responses include the canonical `report` when the run completes.
`report.wallet_summaries[*].totals_by_quote[*]` and `report.totals_by_quote[*]` carry the derived
per-quote `assets_value_dec`, `collateral_value_dec`, `debt_value_dec`, `staked_value_dec`, and
`net_value_dec` summary fields.

`payload.portfolio.networks[*].control_scope` is optional and defaults to `shared`. Set it when
one portfolio flow must isolate managed `rpc.control` source state from another flow on the same
network.

Supported public root `op_id` values for `/v1/runs/start` and feature `run.start` (current):

- `proof` (default)
- `keystore_import`
- `keystore_list`
- `keystore_delete`
- `keystore_tx_sign`
- `evm_read`
- `evm_contract_from_nix`
- `evm_deploy`
- `evm_configure`
- `evm_validate`
- `evm_deploy_configure_validate_config_build`
- `evm_deploy_configure_validate_execute`
- `evm_deploy_configure_validate`
- `portfolio_config_build`
- `portfolio_execute`
- `portfolio_tracker`
- `aave_v3_origin_stack_config_build`
- `aave_v3_origin_stack_execute`
- `aave_v3_origin_stack`
- `nix_app`
- `aave_v3_origin_adapt_deploy`

Notes:
- `evm_read` executes through the shared `rpc.control`-backed EVM read states.
- Planner-internal semantic ids such as `portfolio_prepare_execution_sources`,
  `portfolio_project_report`, `aave_v3_origin_publish_artifact`, and
  `aave_v3_origin_project_report` remain registered for recursive expansion but are rejected by
  public single-op entrypoints with `op_not_public`.

There are no dedicated keystore tx endpoints. Use generic run APIs (`/v1/runs/start`,
`/v1/runs/:run_id/resume`) with those `op_id` values.

Supported `feature_id` values (current):

- `portfolio.snapshot`

Docs:

- Design contract: [`../../docs/design.md`](../../docs/design.md)
- Architecture overview: [`../../docs/architecture.md`](../../docs/architecture.md)
- CLI docs (parallel surface): [`../cli/README.md`](../cli/README.md)
