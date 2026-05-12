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
`MFM_EVM_RPC_SOURCES_JSON` and `MFM_EVM_RPC_PREFERRED_ORDER`.

When `MFM_EVM_RPC_SOURCES_JSON` is unset, the wrapper provides a public Ethereum mainnet
fallback:

```json
[
  {
    "id": "publicnode_ethereum_mainnet",
    "network_id": "ethereum-mainnet",
    "rpc_url": "https://ethereum-rpc.publicnode.com",
    "kind": "remote_public"
  }
]
```

When `MFM_EVM_RPC_PREFERRED_ORDER` is unset, it defaults to
`publicnode_ethereum_mainnet`.

## Service Policy

Postgres lifecycle is controlled by the standard service policy environment:

- `SERVICE_OWNER_SCOPE=ephemeral|persistent`
- `SERVICE_DISCOVERY_SCOPE=local|global`

The removed legacy controls `MFM_KEEP_SERVICES` and `SERVICE_REUSE_POLICY` are rejected by
the wrapper. Use the service policy variables instead.

## Files And Outputs

The wrapper writes a temporary JSON result file, validates the stable CLI envelope, prints
the final JSON response to stdout, and removes the temporary file on exit.

The snapshot itself uses the normal MFM artifact, stream, fact, and control-plane
contracts described in `docs/design.md`.
