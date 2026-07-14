# EVM Contract Lifecycle

Status: implemented public contract.

This document describes the current EVM contract lifecycle model. It is not an RFC or migration
plan. For the generic typed-core authority contract, see `docs/design.md`; for crate placement and
boundary rules, see `docs/architecture.md`; for live EVM route configuration, see
`docs/evm-rpc-routing.md`.

## Contract

EVM contract lifecycle runs are context-bound. A lifecycle run declares one
`EvmContractContext`, and deploy, configure, validate, and import states execute inside that
certified context.

The context carries semantic identity such as:

- `network.network_id`
- `network.expected_chain_id`
- `contract_profile`

`network_id` is semantic workflow data. It is not an RPC endpoint, source id, credential, or runtime
route. Live transports resolve it through runtime config and verify the observed chain id for each
provider-bound EVM network binding before executing operation-only requests.

Phase actions are local work descriptions. They do not carry independent network authority:

- `DeployAction`
- `ConfigureAction`
- `ValidateAction`

Lifecycle resources are produced as context-bound typed values, not as raw continuation seeds:

- deployed contract instances are produced by deploy or import-deployed states
- configured contract instances are produced by configure or import-configured states
- validate consumes a configured instance and produces validation/public-output evidence

Rendered public output is a user surface only. It is not lifecycle continuation authority.

## Entry Points

The public entry-point operation names are:

- `evm_contract_deploy`
- `evm_contract_configure`
- `evm_contract_validate`
- `evm_contract_lifecycle`

Deploy-only config:

```json
{
  "context": { "...": "EvmContractContext JSON" },
  "deploy": { "...": "DeployAction JSON" }
}
```

Configure-only config:

```json
{
  "context": { "...": "EvmContractContext JSON" },
  "import_deployed": { "...": "ImportDeployedSpec JSON" },
  "configure": { "...": "ConfigureAction JSON" }
}
```

Validate-only config:

```json
{
  "context": { "...": "EvmContractContext JSON" },
  "import_configured": { "...": "ImportConfiguredSpec JSON" },
  "validate": { "...": "ValidateAction JSON" }
}
```

Full lifecycle config:

```json
{
  "context": { "...": "EvmContractContext JSON" },
  "deploy": { "...": "DeployAction JSON" },
  "configure": { "...": "ConfigureAction JSON" },
  "validate": { "...": "ValidateAction JSON" }
}
```

Configure-only and validate-only starts are import-plus-transition workflows. They do not accept
old raw typestate seeds.

## Imports

Imports are the only public continuation boundary for lifecycle material that did not come from an
earlier node in the same run.

### From MFM Run

`ImportDeployedSpec::FromMfmRun` and `ImportConfiguredSpec::FromMfmRun` require both `source` and
retained `evidence`.

Identifier-only imports are invalid. The source must be verified from retained typed authority,
including the source certified spec artifact, source certificate artifact, committed source stream,
source terminal event, source value evidence, descriptor identities, stage, producer, context ref,
context descriptor, value digest, and import policy digest.

Replay verifies the retained evidence against the compiled certification registry. It does not query
a live store, runtime config, source registry, or public-output renderer.

This import proves MFM provenance only when the retained source evidence verifies.

### Adopt External Address

`AdoptExternalAddress` admits an externally known address under the certified context. This is
honest adoption, not proof that MFM deployed or configured the contract.

The adoption policy is hash-defining config. It controls required evidence such as observed chain
identity, code existence, expected code hash, block anchoring, initial read assertions, and initial
event assertions. Runtime and replay must use the certified policy and retained evidence; they must
not infer missing policy fields from current defaults.

Public output must preserve that provenance. An externally adopted address must not be rendered as
MFM-deployed or MFM-configured unless verified source evidence proves it.

## Authority Rules

The certifier makes context binding hash-defining:

- context table entries are content addressed by `ContextRef`
- nodes declare required context or no-context
- context-bound cells declare resource kind, stage, context ref, and producer constraints
- context-bound inputs must match the consuming node context, resource kind, stage, and approved
  producer
- raw seeds cannot produce context-bound lifecycle resources unless the certified producer
  constraint explicitly allows that source

Runtime materializes `CertifiedContext<EvmContractContext>` from the certified spec, not from
runtime config or payload fields. Before invoking a runner, runtime rechecks that context-bound
inputs match the certified source cell and consuming node context. Output admission checks the
committed `CellProduced` context and staged artifact evidence against the certified cell
constraints.

Adapters derive EVM capability requests from the certified context plus context-bound inputs.
Transports only prove live route behavior, including observed chain id. Transport source evidence
does not prove that a contract instance belongs to a lifecycle context.

Replay uses the stored certified spec, typed run stream, retained artifacts, side-effect evidence,
import evidence, validation evidence, and public-output evidence. Replay must not use live RPC,
runtime config, projection rows, rendered JSON, or current public-output rendering as authority.

## Rejected Shapes

The old standalone shapes are intentionally unsupported:

```json
{ "config": "ConfigurePhaseConfig", "deployed": "DeployedContract" }
```

```json
{ "config": "ValidatePhaseConfig", "configured": "ConfiguredContract" }
```

These also fail closed:

- per-phase network fields as lifecycle authority
- raw deployed/configured typestate seeds
- import from rendered public JSON
- import from projection rows
- import from identifier-only source-run references
- copied `context_ref` payload fields without certified graph authority

The invariant is:

```text
the certified graph cannot express a lifecycle transition that consumes a resource outside its context
```

It is not:

```text
compare phase.network_id with typestate.network_id at each call site
```

## Code Map

- Entry config ownership: `crates/ops/evm-contract-lifecycle-op`; reusable state config types:
  `crates/states/evm-contracts`
- Domain model and evidence types: `crates/evm-contract-model`
- Operation topology: `crates/ops/evm-contract-lifecycle-op`
- State contracts: `crates/states/evm-contracts`
- Runner binding and replay verifier: `crates/adapters/evm-contracts`
- Live EVM routing: `crates/transports/evm`
- Public entry-point registry: `crates/app`

When changing this surface, update the CLI and REST docs if public request or response shapes
change, and keep `docs/design.md` authoritative for generic kernel/runtime semantics.
