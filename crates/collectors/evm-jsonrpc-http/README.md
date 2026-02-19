# mfm-collectors-evm-jsonrpc-http

HTTP live transport for the `namespace = "evm"` IO surface (EVM JSON-RPC).

## Current Behavior (Milestone A Core)

- Source-id based runtime routing (`EvmJsonRpcHttpConfig.sources` + `preferred_order`)
- Method classification:
  - `read_light`: optional hedging (`hedged_light`)
  - `read_heavy`: sequential failover
  - `write_or_side_effect`: primary-only single dispatch
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
- Read-path per-request `rpc_url` override is rejected.
- Legacy write-path direct `rpc_url` is still accepted for compatibility.

Docs: [`../../../docs/redesign.md`](../../../docs/redesign.md)
