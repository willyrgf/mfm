# mfm-collectors-evm-jsonrpc-http

HTTP live transport for the `namespace = "evm"` IO surface (EVM JSON-RPC).

## Current Behavior

- Source-id based runtime routing (`EvmJsonRpcHttpConfig.sources` + `preferred_order`)
- Fallible factory construction via `EvmJsonRpcHttpTransportFactory::try_new`, with config and
  HTTP-client build errors returned before a transport can be registered.
- Method classification:
  - `read_light`: optional hedging (`hedged_light`)
  - `read_heavy`: sequential failover
  - `write_or_side_effect`: primary-only single dispatch
  - `eth_getLogs`: failover-only adaptive chunking (`max/min span` + `chunk budget`)
- Health behavior:
  - startup/lazy probes (`eth_chainId`, `eth_blockNumber`, optional `eth_getProof`)
  - unhealthy cooldown by logical call count
- Diagnostics:
  - include safe source IDs and coarse classes only
  - do not include URL/auth secrets

## Request Shape

Transport request payload:

```json
{
  "method": "eth_chainId",
  "params": [],
  "route": { "source_id": "helios_local" }
}
```

Notes:

- `route.source_id` is optional.
- Per-request `rpc_url` override is rejected.

## Routing Analysis Operation

For full routing visibility without issuing network requests, call:

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
- target method + param-size metadata
- resolved method class and dispatch mode
- selected source order (or selection error details)
- ranked source state snapshot (score, health, cooldown, probe status)

## Debug Telemetry

At `debug` level, transport logs include:
- call-level routing decision (`method_class`, `dispatch_mode`, `source_order`, `route_source_id`)
- per-request dispatch/result fields (`source_id`, `source_kind`, `rpc_endpoint`, `rpc_method`, `rpc_request_id`, `http_status`)

Telemetry keeps diagnostics safe:
- no full RPC URLs
- no auth headers
- no secret query strings
- `rpc_endpoint` is sanitized to `scheme://host:port`

Docs:
- [`../../../docs/design.md`](../../../docs/design.md)
- [`../../../docs/evm-rpc-routing.md`](../../../docs/evm-rpc-routing.md)
