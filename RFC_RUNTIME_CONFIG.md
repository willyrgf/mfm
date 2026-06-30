# RFC: Runtime Configuration File

## Status

Draft.

## Problem

MFM currently mixes runtime-only process wiring across several environment variables:

- `MFM_EVM_RPC_SOURCES_JSON`
- `MFM_EVM_NETWORK_ROUTES_JSON`
- `MFM_EVM_SIGNERS_JSON`
- `DATABASE_URL`

The EVM variables are especially awkward because they carry structured configuration in environment
values. That makes local development, review, shell history, Nixfied task wiring, and service
deployment harder than necessary.

The current shape also exposes implementation details at the user boundary. A caller must understand
the split between EVM sources, source policies, semantic network routes, and signer bindings before
they can run a portfolio snapshot or contract lifecycle workflow against a live endpoint. Those
concepts are valid internally, but the primary runtime configuration surface should be a readable
file, not a collection of JSON blobs in the environment.

There is also an inconsistent validation model:

- Portfolio runner setup can treat missing EVM RPC configuration as an unavailable provider.
- Contract lifecycle runner setup validates EVM RPC routing eagerly.

That split makes failures depend on workflow internals instead of a single runtime-configuration
contract. If a workflow requires EVM RPC, missing or invalid runtime wiring should fail consistently,
with redacted diagnostics, before live execution depends on it.

At the same time, RPC endpoints, provider auth, keystore paths, unlock files, and store connection
strings must not move into authored workflow config. Authored `--config` remains semantic run input:
it is validated, certified, hashed into run semantics, replayed, and may influence public artifacts.
Runtime process wiring is local capability configuration and must remain outside typed values, specs,
events, artifacts, and public outputs.

## Goals

- Provide one runtime-only configuration file for process wiring, including store selection and live
  capability bindings.
- Keep authored workflow config semantic and free of endpoints, auth material, keystore paths, and
  other local process resources.
- Make portfolio and contract lifecycle runtime validation consistent.
- Make signer validation effect-scoped: read-only workflows can use public addresses without signer
  runtime bindings, while signing-capable side-effect workflows must have signer bindings.
- Keep runtime config out of replay. Replay must use recorded evidence and retained artifacts, not
  live endpoints.
- Make Nixfied managed development tasks pass runtime wiring through the same file surface users can
  inspect and reproduce.
- Prefer TOML for hand-authored local configuration, while allowing JSON where machine generation is
  useful.
- Allow runtime-local direct values when users do not want environment indirection, while preserving
  strict redaction and no-persistence rules.

## Non-Goals

- Do not add per-request RPC URL overrides to workflow configs.
- Do not persist runtime endpoints, auth material, database URLs, signer paths, passwords, private
  keys, or signed raw transactions in run streams, artifacts, public outputs, or fixtures.
- Do not make EVM routes part of domain semantics unless selecting a named source is explicitly a
  domain-level requirement in a future workflow.
- Do not preserve environment JSON blobs as the primary API merely for compatibility.

## Proposed Solution

Introduce a single runtime configuration file and pass it explicitly to process entry points.

CLI example:

```sh
mfm run start --op portfolio_snapshot --config portfolio.toml --runtime-config runtime.toml
```

REST example:

```sh
mfm-rest-api --runtime-config runtime.toml
```

If a process manager needs environment-based wiring, the environment should point to the file:

```sh
MFM_RUNTIME_CONFIG_FILE=runtime.toml mfm-rest-api
```

The environment value is only a path. Structured runtime configuration belongs in the file.

## Runtime Config Shape

Common single-source local development example:

```toml
[store.postgres]
database_url = "postgresql://postgres:postgres@127.0.0.1:5432/mfm_dev"

[evm.sources.reth-local]
expected_chain_id = 31337
rpc_url = "http://127.0.0.1:8545"

[evm.routes.reth-local]
source_ref = "reth-local"
# policy_id is optional here; it defaults to source_ref and synthesizes a same-id
# single-source policy for the common case.
```

Advanced fallback example:

```toml
[store.postgres]
database_url_env = "MFM_DATABASE_URL"

[evm.sources.primary-mainnet]
expected_chain_id = 1
rpc_url_env = "MFM_MAINNET_PRIMARY_RPC_URL"
auth_header_env = "MFM_MAINNET_PRIMARY_AUTH_HEADER"

[evm.sources.secondary-mainnet]
expected_chain_id = 1
rpc_url_env = "MFM_MAINNET_SECONDARY_RPC_URL"

[evm.policies.ethereum-mainnet]
ordered_sources = ["primary-mainnet", "secondary-mainnet"]

[evm.routes.ethereum-mainnet]
source_ref = "primary-mainnet"
policy_id = "ethereum-mainnet"
```

Signer example:

```toml
[evm.signers.deployer]
provider = "keystore"
entry_id = "00000000-0000-0000-0000-000000000000"
keystore_path = "/path/to/keystore.json"
unlock_file = "/path/to/password-file"
```

The runtime config may contain direct runtime-local values or point to values through environment or
file indirection. Direct values are useful for local development and single-file process deployment.
Indirection remains recommended for managed secret injection. In all cases, runtime config is a
secret-bearing process-local file: it must not be committed as a fixture, emitted in logs, recorded in
run streams or artifacts, or returned through CLI/REST/public-output surfaces. RPC URLs, RPC auth
values, database URLs, keystore paths, unlock-file paths, passwords, and private-key material are
always redacted in diagnostics. Private keys and mnemonics must not be represented directly in this
file.

## Semantics

`store.postgres` defines how the process obtains the production Postgres connection string. Supported
sources should include a direct `database_url` for local/single-file setup and indirection such as
`database_url_env` or `database_url_file` / `database_url_file_env` for managed deployments. The
resolved URL is secret-bearing and must be redacted.

`evm.sources` defines process-local EVM JSON-RPC endpoints. Source ids are local routing keys.
`expected_chain_id` is a runtime assertion about that source and must match the semantic chain id
required by any workflow routed through the source.

Each EVM source should support direct endpoint/auth values and indirection. Examples include
`rpc_url`, `rpc_url_env`, `rpc_url_file`, `rpc_url_file_env`, `auth_header`, `auth_header_env`,
`auth_header_file`, and `auth_header_file_env`. Resolved endpoint and authorization values are
secret-bearing for redaction purposes, even if a given local endpoint is not sensitive.

`evm.policies` defines ordered fallback across sources. Explicit policies must have non-empty,
duplicate-free source lists. If a route omits `policy_id`, the runtime derives a same-id
single-source policy from `source_ref` for the common case. Explicit policies should be used when
fallback is intended.

`evm.routes` maps semantic workflow `network_id` values to process-local source and policy ids. This
keeps domain network labels separate from local endpoint names. `source_ref` is the preferred
starting source within `policy_id`; it must be a member of that policy's `ordered_sources`. Routes do
not carry authoritative chain ids, because chain identity is semantic workflow input.

`evm.signers` maps semantic `signer_ref` values to local signer providers. The config may reference
keystore paths and unlock files directly or through indirection, but those references remain
runtime-only. Direct unlock-file paths are allowed; direct password values are not. Expected public
signer identity remains semantic typed config and non-secret evidence, not runtime config.

## Validation Model

Runtime validation should be shared by all EVM-backed workflows, but it should be layered so generic
runtime-config parsing does not learn workflow semantics.

### Runtime Config Shape Validation

When the file is parsed, the runtime config validator should check deployment-local shape:

- every configured store source is well-formed and redaction-safe;
- every EVM source id, policy id, route key, and signer ref uses the checked local-id grammar;
- every EVM source has exactly one endpoint source (`rpc_url`, `rpc_url_env`, or file indirection);
- direct RPC URLs are syntactically valid and do not contain URL userinfo;
- RPC authorization has at most one source, whether direct or indirect;
- every EVM source has a non-zero `expected_chain_id`;
- every explicit policy has at least one unique source;
- every policy source exists;
- every route references an existing source and policy;
- every route `source_ref` is a member of the selected policy;
- every signer entry has a supported provider shape and checked local sources;
- signer entries do not contain private keys, mnemonics, passwords, password values, or raw signing
  material.

All parse and validation diagnostics must be closed-category and redacted. It is acceptable to name
non-secret ids such as `network_id`, `source_ref`, `policy_id`, and `signer_ref`; diagnostics must
not include RPC URLs, auth material, database URLs, passwords, private keys, keystore paths, password
paths, or signed transactions, regardless of whether those values came directly from the file or from
indirection.

### Workflow Requirement Validation

Before `RunAdmitted` for live execution, adapter-owned runner ingress should derive runtime
requirements from certified nodes and launch config artifacts:

- EVM read requirements: `(network_id, expected_chain_id)` for portfolio reads and contract
  validation reads.
- EVM mutation requirements: `(network_id, expected_chain_id, signer_ref)` for signing-capable
  side-effect nodes such as contract deploy/configure submit nodes.

The common validator should then check:

- every required semantic network has a runtime route;
- every route's selected policy has only sources whose `expected_chain_id` matches the required
  semantic chain id;
- the same `network_id` is not required with conflicting `expected_chain_id` values in one
  certified workflow;
- every EVM mutation requirement has a configured signer provider for its `signer_ref`.

Signer validation is effect-scoped. Public addresses in read-only portfolio or validation configs
are query subjects, not signer authority, and must not require `[evm.signers]`. For mutation
workflows, preflight verifies that a signer binding exists. It should not unlock keystores or probe
private key material during admission unless a future explicit read-only signer-public-identity
capability is introduced. The actual expected public identity check happens when the signing request
is materialized: typed config supplies the expected public identity, the signer result verifies that
identity, and raw signed transaction bytes remain transient.

Portfolio and contract lifecycle should therefore fail the same way for missing EVM runtime wiring.
Non-EVM portfolio configs should not require EVM runtime configuration, and read-only EVM portfolio
configs should not require signer runtime configuration.

Portfolio runners should implement runner-ingress validation for EVM read requirements instead of
accepting an unavailable provider until first use. Contract lifecycle runners should keep validating
read-only phases without signer bindings and mutation phases with signer bindings.

Adapters and transports should still validate defensively when used, but the primary user-facing
failure should come from this common preflight.

Live `eth_chainId` checks may run as transport readiness or first-use validation, but they are not
certification or replay authority. Static preflight compares semantic expected chain ids to runtime
source `expected_chain_id`; transports still verify that the live endpoint reports that configured
chain id before using it.

## Evidence and Replay

Replay must not load runtime config, construct live transports, call live signers, or inspect
keystores. Replay verifies from certified spec/certificate artifacts, the append-only run stream,
retained artifacts, and recorded read/side-effect evidence.

Status, stream inspection, replay, and public-output rendering services must be constructible without
valid live runtime capability configuration. A malformed or missing runtime config may block live
start/resume/drive of nodes that need those capabilities, but it must not block evidence-only read or
replay paths.

Recorded EVM evidence may include redacted `source_ref`, `policy_id`, observed `chain_id`, semantic
`network_id`, and semantic expected chain id as audit provenance. Replay must not resolve those
runtime refs against the current runtime config. Adapters that build typed outputs from EVM reads
must validate observed chain id against semantic expected chain id before producing output, including
portfolio pin/read paths.

## Architecture Placement

The runtime config parser should live in a small dedicated runtime-config crate if reuse warrants it,
or at the app/process boundary while the surface is still small. It should not live in workflow
operation crates or state crates.

Suggested ownership:

- `mfm-runtime-config` or `mfm-app`: parse runtime configuration and validate deployment-local
  shape.
- `mfm-transports-evm`: construct clients from explicit typed source registries; no direct env
  parsing in the primary path; validate route/policy membership and live chain id defensively.
- `mfm-adapters-*`: receive explicit capabilities, routes, and signer providers; derive
  workflow-specific runtime requirements in runner ingress from certified nodes and launch config
  artifacts.
- `mfm-signers-*`: construct signer providers from explicit typed signer registries; verify expected
  public identities at signing time without leaking secret-bearing details.
- `mfm-app`: orchestrate common preflight from adapter-owned requirements and runtime-config
  validators; do not own workflow planning or state semantics; keep evidence-only service assembly
  independent from live runtime capability validity.
- binaries: read CLI/env file path, pass parsed runtime config into app assembly.
- Nixfied: generate or pass a runtime config file for managed dev services.

## Migration Direction

Because MFM is pre-production, correctness and clarity should take priority over preserving awkward
environment JSON APIs.

Recommended migration:

1. Add `--runtime-config <PATH>` to `mfm run start` and `mfm run resume`.
2. Add REST runtime config file support.
3. Introduce typed runtime config parsing and validation.
4. Normalize existing `--database-url`, `DATABASE_URL`, `--evm-rpc-sources`, and EVM environment JSON
   compatibility inputs into the same typed runtime config model before app assembly. Direct values
   and indirect sources should resolve through the same redacted runtime value types.
5. Replace direct reads of `MFM_EVM_RPC_SOURCES_JSON`, `MFM_EVM_NETWORK_ROUTES_JSON`, and
   `MFM_EVM_SIGNERS_JSON` with explicit runtime config plumbing.
6. Update Nixfied Reth/Postgres workflows to generate or pass runtime config files.
7. Update docs and tests to use `runtime.toml`.
8. Remove the environment JSON blobs as documented primary APIs.

Temporary compatibility can exist only as an implementation bridge if needed, but it should not be
documented as the preferred surface.

## Open Questions

- Should direct signer paths be limited to local development, with deployment docs requiring
  environment or file indirection?
- Should runtime config support JSON from day one, or should TOML be the only authored format until a
  machine-generated JSON use case appears?
- Should a separate readiness command perform live `eth_chainId` checks for every configured source,
  distinct from launch preflight and replay?
