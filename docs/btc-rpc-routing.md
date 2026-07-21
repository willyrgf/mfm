# Typed Bitcoin Runtime Config

Status: typed transport runbook for Bitcoin-backed portfolio workflows.

Bitcoin RPC endpoints are live runtime inputs. They are not semantic run authority and must not be
persisted in manifests, events, artifacts, public outputs, fixtures, or replay inputs.

Normative architecture references:

- `docs/design.md`
- `docs/architecture.md`
- `docs/persisted-public-surfaces.md`

## Runtime Config File

Live CLI start accepts `--runtime-config <PATH>`. A resume needs it only when verified history still
has a pending Bitcoin live-source node. CLI and REST also read `MFM_RUNTIME_CONFIG_FILE` when no
explicit path is provided. Read-only commands and REST startup do not load this file.

Example TOML:

```toml
[btc.routes.public-bitcoin-core]
rpc_url_env = "MFM_BITCOIN_RPC_URL"
rpc_user_env = "MFM_BITCOIN_RPC_USER"
rpc_password_env = "MFM_BITCOIN_RPC_PASSWORD"
```

JSON with the same shape is also accepted by `mfm-runtime-config`.

The Bitcoin runtime shape is `btc.routes.<source_identity>`. Route keys are semantic
`BtcSourceIdentity` values, not endpoint names, URLs, credential ids, or routing policies.

## Provider-Bound Reads

App assembly creates one process-local live transport runtime from runtime config and caches the
derived `BtcJsonRpcRouter`. Runners and adapters derive a `BtcSourceBinding` from certified
workflow semantics, validate that binding without network IO, and bind it to a
`BtcJsonRpcSourceProvider` before any live call.

BTC capability requests are operation-only. Chain-head requests carry only the requested head
selection. Portfolio balance requests carry only the address, `block_height`, and `block_hash`
obtained from the pinned execution anchor. They do not carry `network_id`, `source_identity`,
expected network tags, endpoints, or credentials.

The bound Bitcoin provider resolves `source_identity` through runtime config, probes
`getblockchaininfo`, and returns redacted evidence containing both expected and observed Bitcoin
network tags. If the observed tag differs from the provider binding, the provider returns a
source-mismatch diagnostic. `validate_source_binding` only checks that the route exists; it does not
perform network IO.

Provider diagnostics may carry reviewed operation ids and public fields such as network tags,
heights, hashes, and numeric status codes. They must never carry RPC URLs, credentials, file paths,
provider messages, request bodies, or response bodies.

Ingress distinguishes absence from invalid input. No runtime file, no `btc` family, or no selected
semantic route yields `RuntimeConfigRequired` with a `provider_configuration_missing` or
`route_unavailable` diagnostic. The diagnostic retains only the certified `network_id`,
`source_identity`, and `bitcoin_network`. An unreadable, malformed, or semantically invalid supplied
file yields `RuntimeConfigInvalid`; it is never presented as a missing route. Admission aggregates
and deduplicates all missing BTC and EVM routes before any `RunAdmitted` event is appended. Resume
performs the same check only for nonterminal live-source nodes.

## Strict Portfolio Snapshots

Portfolio Bitcoin balance reads are exact-anchor requests. The response must verify the same
address, provider source evidence, height, and hash. The live Bitcoin Core provider supports this
only when the requested anchor is the node's current best tip:

1. `getblockchaininfo` must report the requested `block_height` and `block_hash`.
2. `scantxoutset` scans `addr(<address>)`.
3. The scan result `height` and `bestblock` must still match the requested anchor.

If the requested anchor is stale, the node advances during the scan, or the scan does not complete,
the provider returns `operation_incomplete` as a fatal capability failure. The system does not add a
current-only Bitcoin portfolio mode because that would introduce a second snapshot semantics.

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
- Keep one route-based source model and one exact-anchor balance semantic.
