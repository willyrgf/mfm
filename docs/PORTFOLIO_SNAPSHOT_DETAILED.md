# Portfolio Snapshot Wiring

This document describes the current Nixfied wrapper for `mfm::portfolio::snapshot`.

The wrapper is intentionally thin:

- it validates the request file path
- it starts or reuses Postgres through modeled service hooks
- it ensures the snapshot database exists
- it configures EVM RPC bootstrap environment when the caller has not provided one
- it runs the packaged `mfm_cli --output-format json portfolio snapshot --request-file`

## RPC Defaults

The preferred runtime contract is explicit RPC configuration through
`MFM_EVM_RPC_SOURCES_JSON`.

When `MFM_EVM_RPC_SOURCES_JSON` is unset, the wrapper provides a public Ethereum mainnet
fallback:

```json
{
  "sources": [
    {
      "id": "publicnode-ethereum-mainnet",
      "expected_chain_id": 1,
      "rpc_url": "https://ethereum-rpc.publicnode.com",
      "authorization": null
    }
  ],
  "policies": [
    {
      "id": "publicnode-ethereum-mainnet",
      "ordered_sources": ["publicnode-ethereum-mainnet"]
    }
  ]
}
```

## Service Policy

Postgres lifecycle is controlled by the standard service policy environment:

- `SERVICE_OWNER_SCOPE=ephemeral|persistent`
- `SERVICE_DISCOVERY_SCOPE=local|global`

The removed legacy controls `MFM_KEEP_SERVICES` and `SERVICE_REUSE_POLICY` are rejected by
the wrapper. Use the service policy variables instead.

## Files And Outputs

The wrapper writes a temporary JSON result file, validates the stable CLI envelope, prints
the final JSON response to stdout, and removes the temporary file on exit.

The snapshot itself uses the certified typed run-event store, typed artifact store, typed fact
evidence, and runtime-only RPC source configuration described in `docs/design.md`.
