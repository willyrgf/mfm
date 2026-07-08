# Typed Bitcoin Runtime Config

Status: typed transport runbook for Bitcoin-backed collector and portfolio workflows.

Bitcoin RPC endpoints are live runtime inputs. They are not semantic run authority and must not be
persisted in manifests, events, artifacts, public outputs, fixtures, or replay inputs.

Normative architecture references:

- `docs/design.md`
- `docs/architecture.md`
- `docs/persisted-public-surfaces.md`

## Runtime Config File

Live CLI start/resume accepts `--runtime-config <PATH>`. CLI and REST also read
`MFM_RUNTIME_CONFIG_FILE` when no explicit path is provided. Read-only commands and REST startup do
not load this file.

Example TOML:

```toml
[btc.routes.bitcoin-mainnet]
rpc_url_env = "MFM_BITCOIN_RPC_URL"
rpc_user_env = "MFM_BITCOIN_RPC_USER"
rpc_password_env = "MFM_BITCOIN_RPC_PASSWORD"
```

JSON with the same shape is also accepted by `mfm-runtime-config`.

The only supported Bitcoin runtime shape is `btc.routes.<source_identity>`. The old singleton
`btc.json_rpc` shape is intentionally rejected. Route keys are semantic `BtcSourceIdentity` values,
not endpoint names, URLs, credential ids, or fallback policies.

## Provider-Bound Reads

App assembly creates a raw `BtcJsonRpcRouter` from runtime config. Runners and adapters derive a
`BtcSourceBinding` from certified workflow semantics, validate that binding without network IO, and
bind it to a `BtcJsonRpcSourceProvider` before any live call.

BTC capability requests are operation-only. Chain-head requests carry only the requested head
selection. Portfolio balance requests carry only the address, `block_height`, and `block_hash`
obtained from the pinned execution anchor. They do not carry `network_id`, `source_identity`,
expected network tags, endpoints, or credentials.

The bound Bitcoin provider resolves `source_identity` through runtime config, probes
`getblockchaininfo`, and returns redacted evidence containing both expected and observed Bitcoin
network tags. If the observed tag differs from the provider binding, the provider returns a
source-mismatch diagnostic. `validate_source_binding` only checks that the route exists; it does not
perform network IO.

Provider diagnostics may carry stable operation ids and closed public fields such as network tags,
heights, hashes, and numeric status codes. They must never carry RPC URLs, credentials, file paths,
provider messages, request bodies, or response bodies.

## Strict Portfolio Snapshots

Portfolio Bitcoin balance reads are exact-anchor requests. The response must verify the same
address, provider source evidence, height, and hash.

Bitcoin Core `scantxoutset` cannot prove an arbitrary prior exact anchor, so the live Bitcoin Core
provider rejects portfolio balance reads with `unsupported_operation` before scanning. This is a
fatal attempt/capability failure, not a portfolio domain observation error. The system does not add a
current-only Bitcoin portfolio mode in this path because that would introduce a second snapshot
semantics.

## Collector Checkpoints

BTC collector state and checkpoint subjects include `bitcoin_network`. Checkpoint queries match the
semantic network id, source identity, head kind, finality policy, confirmation depth, and Bitcoin
network tag so checkpoints cannot cross expected Bitcoin networks.

## Replay

Replay uses the stored certified spec, typed run stream, typed artifacts, and replay verifiers. It
must not open live RPC connections or consult runtime config. Replay providers rebuild the certified
provider binding and verify recorded Bitcoin evidence against that binding and operation request.

## Contributor Guidance

- Keep runtime config parsing in `mfm-runtime-config`.
- Keep live Bitcoin route resolution in `mfm-transports-btc-jsonrpc-http`.
- Bind live providers from certified semantic source intent before issuing operation-only requests.
- Keep workflow-specific operation construction in adapters.
- Keep binaries limited to parsing and passing runtime config paths.
- Do not add fallback source routing, old singleton config compatibility, or current-only balance
  semantics without a deliberate new design.
