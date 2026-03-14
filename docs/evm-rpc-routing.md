# EVM RPC Routing

Status: `rpc.control` is the canonical state-facing RPC ingress in the default app bundle.

This document is the operator and contributor runbook for two related surfaces:

- `namespace = "rpc.control"`: the canonical managed ingress used by shared runtime states and
  built-in app flows
- `namespace = "evm"`: the internal/direct executor surface retained for explicit low-level tests
  and control-plane internals

Normative architecture references:
- `docs/redesign.md`
- `docs/architecture.md`
- `RPC_CONTROL_PLANE_WIRE_UP.md`

## 1. Canonical Runtime Contract

Canonical runtime callers should use `rpc.control`.

Managed EVM read/write request envelope:

```json
{
  "kind": "evm_call",
  "network_id": "ethereum-mainnet",
  "method": "eth_chainId",
  "params": []
}
```

Control-plane preflight/setup request envelope:

```json
{
  "kind": "prepare_sources",
  "network_id": "ethereum-mainnet"
}
```

Notes:
- Canonical callers should supply `network_id` whenever they know the stable network context.
- `route.source_id` remains available only as a migration/compatibility escape hatch. It is not the
  final canonical read contract.
- Per-request raw `rpc_url` overrides are rejected.
- Canonical app wiring no longer exposes the raw `evm` transport as the default state-facing
  routing authority.

## 2. Bootstrap Source Configuration

The control plane bootstraps its source catalog from runtime-only environment variables:

- `MFM_EVM_RPC_SOURCES_JSON`: JSON array of source objects:
  - `id`
  - `rpc_url`
  - optional `authorization`
  - optional `kind` (`local`, `remote_user`, `remote_public`)
  - optional `network_id`
  - optional `require_get_proof_probe`
- `MFM_EVM_RPC_PREFERRED_ORDER`: comma-separated source IDs used as base ordering hints.
- `MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS`: comma-separated source IDs that must pass `eth_getProof`
  probing.

Compatibility fallback:
- `MFM_EVM_RPC_URL`
- `MFM_EVM_RPC_AUTHORIZATION`

When only the legacy single-source fallback is configured, the control plane maps it to source id
`user_primary`.

Legacy compatibility surface:
- `MFM_EVM_RPC_SOURCE_ID` remains only for the low-level `keystore tx-send-raw` compatibility
  command. It is not part of the canonical portfolio/symbol/Aave read contract.

## 3. Runtime Behavior

`rpc.control` owns the managed path for:

- source-pool membership bootstrap
- source probing and capability checks
- durable ranking snapshots
- managed source selection for unpinned EVM calls
- best-effort runtime observation writes after live calls

Current managed behavior:
- `prepare_sources` syncs membership, probes stale or missing sources, computes ranked order, and
  persists source-pool state in the Postgres control-plane store.
- `evm_call` selects a source from the durable pool unless the call is explicitly route-pinned for
  compatibility reasons.
- If multiple network-specific bootstrap catalogs exist, managed callers must provide `network_id`.

The raw `evm` transport remains useful for:

- the internal executor embedded by `mfm-transports-rpc-control`
- explicit low-level routing/failover tests
- direct helper code that intentionally exercises executor behavior outside the canonical app bundle

## 4. Error and Diagnostic Shape

Common `rpc.control` error codes include:

- `rpc_control_no_sources`
- `rpc_control_network_required`
- `rpc_control_network_invalid`
- `rpc_control_source_unknown`
- `rpc_control_source_invalid`
- `rpc_control_pool_invalid`

`prepare_sources` returns ranked-source summaries that expose:

- `available_source_ids`
- `ranked_source_ids`
- per-source health/cooldown/get-proof diagnostics

## 5. Contributor Guidance

- Shared runtime states should use the typed `mfm-collectors-rpc-control` client.
- New canonical read APIs should pass `network_id`, not `rpc_source_id`.
- New canonical write APIs should not introduce fresh caller-controlled source selection.
- If you need to test raw executor behavior directly, do so explicitly and document that it is a
  direct/internal test rather than a canonical app-routing path.
