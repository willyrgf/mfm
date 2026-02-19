# EVM RPC Routing

Status: implemented and validated in repository (2026-02-19).

This document is the operator and contributor runbook for the `namespace="evm"` live IO routing
path implemented in this repository.

Normative architecture references:
- `docs/redesign.md`
- `docs/architecture.md`

## 1. Transport Contract

The state-facing IO namespace remains unchanged:
- `namespace = "evm"`

Transport request envelope:

```json
{
  "method": "eth_chainId",
  "params": [],
  "route": { "source_id": "helios_local" }
}
```

Notes:
- `route.source_id` is optional.
- Per-request `rpc_url` override is rejected for all EVM calls with `evm_request_invalid`.
- URL/auth values are runtime-only config and are never persisted in facts/events/artifacts.

Routing analysis operation (local-only, no network dispatch):

```json
{
  "method": "mfm_debugRoutingAnalysis",
  "params": {
    "method": "eth_chainId",
    "params": [],
    "route": { "source_id": "helios_local" }
  }
}
```

Response includes:
- target method metadata
- resolved method class and dispatch mode
- selected source order (or structured selection error details)
- ranked source state snapshot (score, health, cooldown window, probe status)

## 2. Runtime Source Configuration

Primary source-pool configuration:
- `MFM_EVM_RPC_SOURCES_JSON`: JSON array of source objects:
  - `id`
  - `rpc_url`
  - optional `authorization`
  - optional `kind` (`local`, `remote_user`, `remote_public`)
  - optional `require_get_proof_probe`
- `MFM_EVM_RPC_PREFERRED_ORDER`: comma-separated source IDs.
- `MFM_EVM_RPC_STRATEGY`: `hedged_light` (default) or `failover`.
- `MFM_EVM_RPC_HEDGE_DELAY_MS`: hedge delay for `hedged_light`.
- `MFM_EVM_RPC_UNHEALTHY_COOLDOWN_CALLS`: cooldown window after source failure.
- `MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS`: comma-separated IDs requiring `eth_getProof` probe.
- `MFM_EVM_RPC_LOGS_MAX_BLOCK_SPAN`: initial max block span for `eth_getLogs` chunking.
- `MFM_EVM_RPC_LOGS_MIN_BLOCK_SPAN`: minimum block span before chunking stops splitting.
- `MFM_EVM_RPC_LOGS_MAX_CHUNKS_PER_CALL`: retry/chunk budget cap for a single logs request.

Compatibility fallback (single source):
- `MFM_EVM_RPC_URL` and optional `MFM_EVM_RPC_AUTHORIZATION` map to source id `user_primary`.

`tx-send-raw` write path source selection:
- CLI arg: `--source-id`
- env fallback: `MFM_EVM_RPC_SOURCE_ID`

## 3. Method Classification and Dispatch

Methods are classified into:
- `read_light`
- `read_heavy`
- `write_or_side_effect`

Current policy:
- `read_light`: optional two-source hedge (`hedged_light`) or failover (`failover` mode).
- `read_heavy`: sequential failover only.
- `write_or_side_effect`: primary-only single dispatch (no hedge, no cross-source write fanout).
- `eth_getLogs`: adaptive chunking over block ranges with failover-only dispatch.

Classification highlights:
- Heavy by default unless explicitly allowlisted.
- Explicit heavy set includes `eth_getLogs`, `trace_*`, `debug_*`.
- Write/side-effect includes `eth_sendRawTransaction`, `eth_sendTransaction`, and
  `personal_*`/`admin_*`/`miner_*`/`txpool_*`/`engine_*`.
- `eth_call` is downgraded to heavy when encoded params exceed threshold
  (`hedge_max_eth_call_params_bytes`).
- `eth_getLogs` chunking behavior:
  - requires filter-based block range (`fromBlock`/`toBlock`)
  - splits using `logs_max_block_span`
  - on retryable failures, bisects down to `logs_min_block_span`
  - enforces `logs_max_chunks_per_call` budget

## 4. Health, Probing, and Ordering

Probes:
- `eth_chainId`
- `eth_blockNumber`
- optional `eth_getProof` when required for selected sources

Health behavior:
- Source failures can mark source unhealthy for a cooldown window.
- Score decay/recovery is weighted by failure class and method class.
- Unhealthy sources are skipped unless explicitly routed by `route.source_id` (which then returns
  `evm_source_unhealthy` when unavailable).

Ordering when route hint is absent:
1. health score (higher first)
2. source kind priority (`local` preferred over remote kinds)
3. configured base order (`MFM_EVM_RPC_PREFERRED_ORDER`)

## 5. Stable Error Codes and Diagnostics

Pool-related codes:
- `evm_source_unhealthy`
- `evm_no_healthy_source`
- `evm_hedge_exhausted`
- `evm_route_source_unknown`
- `evm_logs_chunking_invalid_range`
- `evm_logs_chunking_exhausted`

Diagnostics rules:
- Include safe source IDs and coarse error metadata only.
- Never include full RPC URLs, authorization headers, or secret-bearing query parameters.

Debug telemetry (when `LOG_LEVEL=debug` or equivalent filter enables this target):
- call-level routing decision:
  - `rpc_method`, `method_class`, `dispatch_mode`, `source_order`, `route_source_id`
- per-request dispatch/result:
  - `source_id`, `source_kind`, `rpc_endpoint`, `rpc_method`, `rpc_request_id`, `http_status`
  - `rpc_endpoint` is sanitized to `scheme://host:port` only (no path/query/auth)

## 6. Replay and Determinism

No new state-facing client was introduced.

Existing state logic continues to use the standard IO provider path:
- live mode records facts
- replay mode serves recorded facts

Replay invariants validated:
- failover/hedging live captures replay deterministically without new network calls
- missing fact key behavior remains `MissingFactKey` with stable `missing_fact_key` code

## 7. Migration Notes

Breaking change:
- Per-request EVM `rpc_url` request override is removed from runtime behavior.

CLI migration for `keystore tx-send-raw`:
- old: `--rpc-url <URL>`
- new: `--source-id <ID>` (or `MFM_EVM_RPC_SOURCE_ID`)

Report compatibility:
- `keystore_tx_send_raw` still exposes `rpc_url_host` field name, but value carries source ID.
