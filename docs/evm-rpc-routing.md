# Typed EVM RPC Routing

Status: typed transport runbook for EVM-backed portfolio and deploy/configure/validate workflows.

The current EVM RPC path is not a separate semantic stream or workflow runtime. RPC source
configuration is runtime-only capability input consumed by typed transport backends. Certified run
semantics are still defined by the certified spec and typed run stream.

Normative architecture references:

- `docs/design.md`
- `docs/architecture.md`

## Runtime Configuration

Typed EVM transports discover RPC sources from environment variables:

- `MFM_EVM_RPC_SOURCES_JSON`: JSON array of source objects.
- `MFM_EVM_RPC_PREFERRED_ORDER`: comma-separated source IDs used as selection preference.
- `MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS`: comma-separated source IDs that must pass `eth_getProof`
  probing before use.

Source object fields:

- `id`: stable source id used only for runtime selection and diagnostics
- `network_id`: typed workflow network id
- `rpc_url`: endpoint URL
- `kind`: optional `local`, `remote_user`, or `remote_public`
- `authorization`: optional runtime-only auth metadata
- `require_get_proof_probe`: optional per-source probe requirement

Example:

```json
[
  {
    "id": "reth_local",
    "network_id": "ethereum-mainnet",
    "rpc_url": "http://127.0.0.1:8545",
    "kind": "local"
  },
  {
    "id": "public_eth",
    "network_id": "ethereum-mainnet",
    "rpc_url": "https://ethereum-rpc.publicnode.com",
    "kind": "remote_public"
  }
]
```

Do not persist `rpc_url` or authorization material in typed values, specs, events, artifacts, public
outputs, or fixtures.

## `control_scope`

Portfolio and EVM DCV configs still carry a non-secret `control_scope`. In the typed runtime it is
part of the domain request identity and source-selection partition, not independent store
authority. Use a distinct scope when two workflows on the same network should not share runtime RPC
source preference or diagnostics.

`network_id` remains required for every configured source and every typed workflow request that
uses an EVM RPC backend.

## Live Execution

Live typed transports may:

- select a configured source for the certified network and scope
- probe source health before use
- perform read calls required by certified read states
- sign with runtime-only MFM keystore access and submit side-effect transactions required by
  certified side-effect states
- persist typed fact, receipt, confirmation, and artifact evidence through runtime/store APIs

Live typed transports must not:

- add uncertified state nodes
- rewrite a certified spec
- create an independent run stream or side-effect stream
- persist secrets or raw signing material in typed semantic surfaces
- accept per-request raw RPC URL overrides from workflow configs

Deploy/configure signer config carries only non-secret keystore references: entry id and the names
of environment variables that point to the keystore path and password file. The referenced paths,
passwords, private keys, and signed raw transactions remain runtime-only.

## Replay

Replay uses the stored certified spec, typed run stream, typed artifacts, and replay verifiers.
Replay must not open live RPC connections or consult runtime source configuration.

EVM DCV replay recomputes expected validation-read requests from certified config and typestate
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
