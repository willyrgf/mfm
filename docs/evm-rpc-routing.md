# Typed EVM RPC Routing

Status: typed transport runbook for EVM-backed portfolio and contract-lifecycle workflows.

The current EVM RPC path is not a separate semantic stream or workflow runtime. RPC source
configuration is runtime-only capability input consumed by typed transport backends. Certified run
semantics are still defined by the certified spec and typed run stream.

Normative architecture references:

- `docs/design.md`
- `docs/architecture.md`

## Runtime Configuration

Typed EVM transports discover RPC sources from environment variables:

- `MFM_EVM_RPC_SOURCES_JSON`: JSON source registry with endpoint-bearing `sources` and ordered
  fallback `policies`.
- `MFM_EVM_CONTRACT_SOURCE_REF`: optional source id used by contract lifecycle routes when the
  workflow config does not select a process-local source explicitly.
- `MFM_EVM_CONTRACT_SOURCE_POLICY_ID`: optional source policy id used by contract lifecycle routes
  when the workflow config does not select a process-local policy explicitly.
- `MFM_EVM_SIGNERS_JSON`: JSON array of runtime signer-provider entries used by contract lifecycle
  routes.

Source object fields:

- `id`: stable source id used only for runtime selection and diagnostics
- `expected_chain_id`: chain id that must be observed before the source can be used
- `rpc_url`: endpoint URL
- `authorization`: optional runtime-only auth metadata

Policy object fields:

- `id`: stable policy id used only for runtime selection and diagnostics
- `ordered_sources`: source ids tried in order

Example:

```json
{
  "sources": [
    {
      "id": "reth-local",
      "expected_chain_id": 31337,
      "rpc_url": "http://127.0.0.1:8545",
      "authorization": null
    },
    {
      "id": "publicnode-ethereum-mainnet",
      "expected_chain_id": 1,
      "rpc_url": "https://ethereum-rpc.publicnode.com",
      "authorization": null
    }
  ],
  "policies": [
    {
      "id": "reth-local",
      "ordered_sources": ["reth-local"]
    },
    {
      "id": "publicnode-ethereum-mainnet",
      "ordered_sources": ["publicnode-ethereum-mainnet"]
    }
  ]
}
```

Do not persist `rpc_url` or authorization material in typed values, specs, events, artifacts, public
outputs, or fixtures.

## `control_scope`

Portfolio configs still carry a non-secret `control_scope`. In the typed runtime it is part of the
domain request identity and source-selection partition, not independent store authority. Use a
distinct scope when two workflows on the same network should not share runtime RPC source preference
or diagnostics.

`network_id` remains required for every configured source and every typed workflow request that
uses an EVM RPC backend.

## Live Execution

Live typed transports may:

- select a configured source for the certified network and scope
- probe source health before use
- perform read calls required by certified read states
- submit side-effect transactions required by certified side-effect states
- persist typed fact, receipt, confirmation, and artifact evidence through runtime/store APIs

Live typed transports must not:

- add uncertified state nodes
- rewrite a certified spec
- create an independent run stream or side-effect stream
- persist secrets or raw signing material in typed semantic surfaces
- accept per-request raw RPC URL overrides from workflow configs

Contract lifecycle configs carry only signer intent: a non-secret `signer_ref` and expected signer
address. `MFM_EVM_SIGNERS_JSON` maps `signer_ref` to a process-local provider entry, for example:

```json
[
  {
    "signer_ref": "deployer",
    "entry_id": "00000000-0000-0000-0000-000000000000",
    "keystore_env": "MFM_KEYSTORE_PATH",
    "unlock_file_env": "MFM_KEYSTORE_PASSWORD_FILE"
  }
]
```

The referenced paths, passwords, private keys, and signed raw transactions remain runtime-only.

## Replay

Replay uses the stored certified spec, typed run stream, typed artifacts, and replay verifiers.
Replay must not open live RPC connections or consult runtime source configuration.

EVM contract replay recomputes expected validation-read requests from certified config and typestate
artifacts, then checks stored fact evidence and terminal output artifacts against that expected
request.

## Contributor Guidance

- Add new EVM read behavior as typed state contracts plus typed transport runner support.
- Add new mutation behavior as typed side-effect states with intent, idempotency input, receipt or
  recovery evidence, and replay verifier coverage.
- Keep RPC endpoints and auth material runtime-only.
- Keep `network_id` and non-secret scope information in typed configs when they are part of
  semantic request identity.
- Add tests that prove replay uses recorded evidence and fails closed on missing or mismatched
  facts, receipts, confirmations, artifacts, or verifier identities.
