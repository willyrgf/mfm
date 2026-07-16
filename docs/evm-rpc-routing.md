# Typed EVM Runtime Config

Status: typed transport runbook for EVM-backed portfolio and contract-lifecycle workflows.

EVM RPC endpoints and signer provider bindings are live runtime inputs. They are not semantic run
authority and must not be persisted in manifests, events, artifacts, public outputs, fixtures, or
replay inputs.

Normative architecture references:

- `docs/design.md`
- `docs/architecture.md`
- `docs/evm-contract-lifecycle.md`

## Runtime Config File

Live CLI start accepts `--runtime-config <PATH>`. A resume needs it only when verified history still
has a pending EVM live-source node. CLI and REST also read `MFM_RUNTIME_CONFIG_FILE` when no
explicit path is provided. Read-only commands and REST startup do not load this file.

Example TOML:

```toml
[evm.sources.reth-local]
rpc_url = "http://127.0.0.1:8545"

[evm.sources.mainnet-primary]
rpc_url_file = "/run/mfm/mainnet-rpc-url"
auth_header_file = "/run/mfm/mainnet-auth-header"

[evm.policies.mainnet]
ordered_sources = ["mainnet-primary"]

[evm.routes.reth-dev]
source_ref = "reth-local"

[evm.routes.ethereum-mainnet]
source_ref = "mainnet-primary"
policy_id = "mainnet"

[keystores.default]
keystore_path = "/run/mfm/deployer.keystore"
unlock_file = "/run/mfm/deployer.password"

[signers.deployer]
provider = "keystore"
keystore_ref = "default"
entry_id = "00000000-0000-0000-0000-000000000000"
```

JSON with the same shape is also accepted by `mfm-runtime-config`.

Runtime config validation rejects source-level chain ids. Expected chain id comes from workflow
semantics: portfolio `NetworkConfig.chain_id` or contract lifecycle `network.expected_chain_id`.

## Provider-Bound Requests

App assembly creates one process-local live transport runtime from runtime config and caches the
derived `EvmJsonRpcClient`. Runners and adapters derive an `EvmNetworkBinding` from certified
workflow semantics, validate that binding without network IO, and bind it to an
`EvmJsonRpcNetworkProvider` before any live call.

EVM capability requests are operation-only. They carry operation parameters such as block selectors,
accounts, calldata, log filters, signed payloads, or transaction hashes. They do not carry
`network_id`, expected chain id, source refs, policy ids, endpoints, or credentials.

The bound provider owns route and source resolution. It resolves the bound `network_id` through
runtime config, selects a configured source/policy, probes chain identity, and returns redacted
evidence containing the semantic network id, expected chain id, observed chain id, selected source
ref, and policy id. A successful provider response has already enforced the provider binding and
operation-specific identity checks.

The selected source and policy ids are audit provenance only. Replay and public output must not
resolve them against current runtime config.

Portfolio snapshots pin EVM views by chain id, block number, and block hash. Later EVM balance and
contract-call reads use the pinned block hash as an EIP-1898 block selector with
`requireCanonical: true`; the stored block number is audit context and must not be used as the
provider read selector. ERC-20 metadata and balance states use the existing generic `eth_call`
capability only: `decimals()` and `balanceOf(address)` retain the destination, exact calldata,
canonical hash selector, raw return bytes, and redacted certified-source identity as external-read
evidence. Each call is followed by an exact hash block re-verification before a fact is recorded.

Transport failures, HTTP status failures, JSON-RPC error objects, malformed responses, and source
mismatches are classified as redacted provider diagnostics. Diagnostics may carry the stable EVM
operation id and numeric status/error code, but never endpoint URLs, authorization headers, provider
messages, response bodies, or runtime config paths.

## Signing

Contract lifecycle configs carry only signer intent: non-secret `signer_ref` and expected signer
address. App assembly resolves `signer_ref` through the runtime config signer registry when a
mutation workflow needs signing. Validation-only workflows do not require signer bindings.

Keystore paths, unlock files, passwords, private keys, mnemonics, signed material, and raw
transactions remain runtime-only and must be redacted from diagnostics.

## Replay

Replay uses the stored certified spec, typed run stream, typed artifacts, and replay verifiers. It
must not open live RPC connections or consult runtime config.

EVM collector replay recomputes native and ERC-20 reads from retained evidence without a live
provider: it validates the destination, calldata, canonical hash selector, raw return bytes,
source binding, re-verified anchor, decoded output, and recorded fact evidence. EVM contract replay
recomputes the certified semantic binding from the lifecycle context and context-bound artifacts,
then checks stored fact evidence, side-effect evidence, import evidence, validation evidence, and
terminal output artifacts against that expected authority.

## Contributor Guidance

- Keep runtime config parsing in `mfm-runtime-config`.
- Keep live EVM source and route resolution in `mfm-transports-evm`.
- Bind live providers from certified semantic source intent before issuing operation-only requests.
- Keep workflow-specific operation construction in adapters.
- Keep binaries limited to parsing and passing runtime config paths.
- Add tests that prove replay uses recorded evidence and fails closed on missing or mismatched
  facts, receipts, confirmations, artifacts, or verifier identities.
