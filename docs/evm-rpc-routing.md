# Typed EVM Runtime Config

Status: typed transport runbook for EVM-backed portfolio and contract-lifecycle workflows.

EVM RPC endpoints and signer provider bindings are live runtime inputs. They are not semantic run
authority and must not be persisted in manifests, events, artifacts, public outputs, fixtures, or
replay inputs.

Normative architecture references:

- `docs/design.md`
- `docs/architecture.md`

## Runtime Config File

Live CLI start/resume accepts `--runtime-config <PATH>`. CLI and REST also read
`MFM_RUNTIME_CONFIG_FILE` when no explicit path is provided. Read-only commands and REST startup do
not load this file.

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

[evm.signers.deployer]
provider = "keystore"
entry_id = "00000000-0000-0000-0000-000000000000"
keystore_path = "/run/mfm/deployer.keystore"
unlock_file = "/run/mfm/deployer.password"
```

JSON with the same shape is also accepted by `mfm-runtime-config`.

Runtime config validation rejects source-level chain ids. Expected chain id comes from workflow
semantics: portfolio `NetworkConfig.chain_id` or contract lifecycle `network.expected_chain_id`.

## Guarded Requests

Adapters derive an `EvmChainGuard` from workflow config for every live EVM call. The guard contains:

- semantic `network_id`
- expected EVM chain id

The EVM transport owns route and source resolution. It resolves `network_id` through runtime config,
selects a configured source/policy, probes chain identity, and returns redacted evidence containing
the semantic network id, expected chain id, observed chain id, selected source ref, and policy id.

The selected source and policy ids are audit provenance only. Replay and public output must not
resolve them against current runtime config.

## Signing

Contract lifecycle configs carry only signer intent: non-secret `signer_ref` and expected signer
address. App assembly resolves `signer_ref` through the runtime config signer registry when a
mutation workflow needs signing. Validation-only workflows do not require signer bindings.

Keystore paths, unlock files, passwords, private keys, mnemonics, signed material, and raw
transactions remain runtime-only and must be redacted from diagnostics.

## Replay

Replay uses the stored certified spec, typed run stream, typed artifacts, and replay verifiers. It
must not open live RPC connections or consult runtime config.

EVM contract replay recomputes expected validation-read requests from certified config and typestate
artifacts, then checks stored fact evidence and terminal output artifacts against that expected
request.

## Contributor Guidance

- Keep runtime config parsing in `mfm-runtime-config`.
- Keep live EVM source and route resolution in `mfm-transports-evm`.
- Keep workflow-specific guard derivation in adapters.
- Keep binaries limited to parsing and passing runtime config paths.
- Add tests that prove replay uses recorded evidence and fails closed on missing or mismatched
  facts, receipts, confirmations, artifacts, or verifier identities.
