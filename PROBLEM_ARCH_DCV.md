# Architecture Correction: Prevent Workflow Recipes From Becoming Platform Boundaries

Status: review draft.

This document replaces the earlier DCV-specific migration note. The DCV surface is still the
motivating case, but the problem is broader: MFM allowed a workflow recipe to become a crate,
module, API, schema, capability, transport, signer, and test boundary.

Breaking changes are allowed. Compatibility with the current DCV names is not a goal. The goal is
to make this class of architecture failure harder to introduce again.

## Executive Verdict

The current EVM deploy/configure/validate implementation proves valuable runtime behavior, but it
establishes the wrong architecture.

The useful behavior is:

- typed contract lifecycle states
- typed deploy/configure/validate typestate values
- side-effect evidence with intent, idempotency, preparation, submission, receipt, confirmation,
  and output evidence
- replay from recorded evidence only
- redaction of raw signed transactions, RPC URLs, and secret material from public outputs
- reth-backed production-path coverage

The architectural failure is:

- a workflow recipe became the durable unit of ownership
- generic EVM JSON-RPC behavior became feature-specific transport
- keystore signing became embedded inside an EVM lifecycle runner
- public CLI, REST, schema, and executable identities mirrored implementation scaffolding
- config mixed domain intent, runtime routing, and signer material references
- operation, state, adapter, transport, signer, and config responsibilities blurred
- tests validated the implemented shape instead of enforcing architecture boundaries

DCV is a symptom. The root problem is that MFM lacks an enforceable taxonomy for deciding what
deserves a crate, schema namespace, capability, route, command, adapter, transport, or config type.

## Root Cause

MFM has strong typed-runtime authority rules, but weak domain and platform taxonomy rules.

The typed core protects important things:

- certified specs are the runtime contract
- append-only run streams are authoritative
- stores own atomic commit and event ordering
- typed configs and values are no-float and no-secret surfaces
- replay must use recorded evidence
- operations plan, states own typed contracts, and binaries stay thin

Those rules are necessary, but they are not enough. They do not answer the product architecture
question:

```text
what kind of thing is this new unit?
```

When that question is not answered first, the first working implementation becomes the taxonomy.
That is what happened here. A parity slice named "deploy/configure/validate" became the organizing
principle for crates, schemas, capabilities, adapters, transports, CLI, REST, docs, tests, and
executable identities.

The missing rule is:

```text
only operation crates may be named after workflow topology;
everything below an operation must be named after durable domain behavior
or reusable platform capability.
```

## Missing Architecture Rules

### Platform Primitive vs Workflow Recipe

MFM did not have a hard distinction between platform primitives and workflow recipes.

A platform primitive is reusable infrastructure:

- signer reference
- signing request
- keystore-backed signing provider
- EVM JSON-RPC client
- EVM RPC source routing
- transaction submission
- receipt polling
- event log reads
- artifact store access

A workflow recipe is an arrangement of states:

- deploy then configure then validate
- prepare sources then observe balances then assemble snapshot
- read proof fact then apply proof mutation then assemble proof output

Workflow recipes may own operation crates because operations are deterministic topology. Workflow
recipes must not own protocol clients, signer providers, reusable state families, public schema
namespaces, or generic capabilities.

### Capability Boundaries

The current effect/capability system enforces role shape, for example that a side-effect state has
exactly one external mutation authority. It does not enforce that the capability itself is the right
abstraction.

That allowed names like:

- `EvmDcvReadCapability`
- `EvmDcvSignerCapability`
- `EvmDcvTransactionSubmitCapability`

The role count may be correct while the capability taxonomy is wrong. A DCV signer capability is
not a real capability. Signing is broader than DCV. EVM RPC reads are broader than DCV. EVM
transaction submission is broader than DCV.

Correct capability names should describe authority:

- `mfm.signing.sign`
- `mfm.evm.rpc.read`
- `mfm.evm.transaction.submit`
- `mfm.evm.logs.read`

The state or adapter using a capability may be contract-lifecycle-specific. The capability should
not be.

### Config Boundary

Typed config is semantic input. It may contain non-secret intent and non-secret references. It must
not contain process-local runtime routing.

Current DCV config mixes:

- contract lifecycle intent
- network selection
- receipt policy
- signer selection
- keystore entry id
- environment variable names for keystore paths
- environment variable names for password-file paths

The signer fields do not persist raw passwords or private keys, but they still make workflow config
responsible for resolving local secret-bearing systems. That is the wrong boundary.

The target split is:

- workflow config contains `SignerRef`
- process runtime config maps `SignerRef` to keystore, hardware signer, remote signer, or future
  wallet implementation
- typed artifacts, events, public outputs, and error details contain only redacted signer identity
  and public verification material

Similarly, typed config must distinguish:

- `network_id`: semantic network label
- `source_id`: local runtime source routing key
- `expected_chain_id`: chain identity returned by the network
- `control_scope`: MFM execution partition, only if truly semantic

Local reth should not be represented as `ethereum-mainnet` unless it is actually mainnet.

### Transport Boundary

The current repo has two EVM JSON-RPC implementations embedded in feature transports:

- `crates/transports/evm-dcv`
- `crates/transports/portfolio`

Both parse `MFM_EVM_RPC_SOURCES_JSON`. Both route by `network_id`. Both make JSON-RPC calls through
feature-local clients. That duplication is a signal that the transport boundary is wrong.

Generic EVM JSON-RPC must be a reusable transport primitive before any workflow-specific adapter
uses it.

### Signer Boundary

Keystore signing currently lives inside the EVM DCV transport. That makes a local MFM keystore an
implementation detail of one contract lifecycle runner.

Signing is not DCV-specific and not EVM lifecycle-specific. The target shape is:

- generic signing crate: signer refs, request/result traits, error redaction
- keystore signer crate: MFM keystore-backed implementation
- EVM signing adapter: converts EVM transaction prehashes into signing requests and verifies the
  expected EVM address
- lifecycle adapter: asks for a signature, encodes submit-time raw transaction bytes transiently,
  and records redacted evidence

Raw signed transactions are bearer mutation material. They must remain transient submit-time bytes.

### Public API Boundary

CLI commands, REST routes, request kinds, schema ids, error codes, executable identities, and
public-output schema ids are durable contracts. They must describe domain behavior, not temporary
implementation recipes.

Disallowed public language:

- `dcv`
- `deploy_configure_validate` when exposed as the durable API unit
- `typed-evm-dcv`
- `mfm.evm.dcv.*`
- `/v1/evm/dcv/*`
- `evm_dcv_*`

Allowed public language:

- contract deployment
- contract configuration
- contract validation
- contract lifecycle
- EVM transaction intent
- EVM RPC source
- signer reference
- signing provider

## Where Naming Became Architecture

These names encoded temporary implementation intent into durable contracts:

- `crates/evm-dcv-model`
- `crates/evm-deploy-configure-validate-config`
- `crates/states/evm-dcv`
- `crates/transports/evm-dcv`
- `crates/ops/evm-deploy-configure-validate-op`
- `mfm.evm.dcv.*`
- `EvmDcv*`
- `Dcv*`
- `DeployConfigureValidate*`
- `mfm evm dcv`
- `/v1/evm/dcv/*`
- `evm_dcv_*` request kinds
- `mfm-transports-evm-dcv`
- `typed-evm-dcv`
- `verify_evm_dcv_replay`
- `EvmDcvRpcClient`

Tests hardened the same abstraction:

- REST parity tests assert `/v1/evm/dcv/*`
- integration tests name local reth as `ethereum-mainnet`
- app boundary tests require `mfm-transports-evm-dcv`
- state tests guard raw transaction secrecy but do not guard capability taxonomy
- operation tests guard phase ordering but do not guard recipe leakage into lower layers

The fix is not to rename strings one by one. The fix is to introduce boundaries that make the
wrong strings impossible or obviously review-blocking.

## Boundary Contract

### Operation

Operations are deterministic planning only.

Operations may:

- parse and validate planning config
- lower authored config into typed config
- assemble state graphs
- attach domain keys and lineage
- bind public outputs
- certify program drafts through trusted descriptors

Operations must not:

- open RPC connections
- open keystores
- read password files
- sign payloads
- submit transactions
- poll receipts
- implement replay
- own reusable protocol APIs
- own generic signer APIs

Workflow recipe names are allowed at this layer only.

### State

States own reusable domain lifecycle behavior.

States may:

- define typed config, input, output, and public-output contracts
- construct deterministic transaction intent
- validate ABI/config shape
- define validation request and response semantics
- define typestate values such as deployed/configured/validated outputs
- require generic capabilities

States must not:

- build HTTP clients
- parse runtime source env vars
- open files
- open keystores
- read passwords
- sign transactions
- submit raw transactions
- persist runtime evidence directly

State names must be domain behavior names, not recipe names.

### Adapter

Adapters connect state-owned domain intent to runtime capabilities.

Adapters may:

- bind certified state descriptors to executable runners
- call generic transports
- call generic signer providers
- encode submit-time raw transaction bytes transiently
- record side-effect evidence through runtime/store APIs
- implement replay verifiers for the domain evidence model

Adapters must not:

- define generic protocol clients under workflow names
- own signer material resolution directly
- bypass runtime/store authority
- place semantic behavior outside state-owned functions

Adapters may be domain-specific. They should not be recipe-specific unless the recipe is truly the
domain.

### Transport

Transports implement reusable protocols and live/replay capability backends.

Transports may:

- parse runtime-only source configuration
- redact endpoints and authorization material
- execute protocol calls
- expose protocol traits and request/response types
- support multiple adapters and workflows

Transports must not:

- use workflow recipe names
- depend on workflow operation crates
- know about contract lifecycle topology
- open keystores or own signer behavior
- persist typed semantic events by themselves

### Signer

Signers provide generic key material and signature capabilities.

Signer crates may:

- define `SignerRef`
- define signing request/result traits
- implement MFM keystore-backed signing
- map runtime signer ids to keystore entries, hardware signers, remote signers, or future wallets
- redact all secret-bearing details from errors

Signer crates must not:

- depend on contract lifecycle states
- depend on EVM workflow operations
- leak private keys, mnemonics, passwords, password paths, keystore paths, signatures, or raw signed
  transactions into typed semantic surfaces

### Configuration

Typed config contains semantic intent and non-secret references only.

Typed config may contain:

- contract artifact reference or inline artifact
- ABI call intent
- expected chain id
- network id
- source id when source selection is semantic
- signer id
- expected public address
- receipt policy
- validation assertions

Typed config must not contain:

- private keys
- mnemonics
- passwords
- password file paths
- keystore file paths
- environment variable names that resolve secret-bearing paths
- RPC URLs
- RPC authorization headers
- raw signed transaction bytes
- signature material

Runtime configuration owns those resolutions.

## Target Architecture Doctrine

### Rule 1: One Stable Abstraction Per Crate

A crate must own one durable abstraction. It must not own a parity slice.

Good crate reasons:

- reusable protocol transport
- reusable signer provider
- reusable domain state family
- deterministic workflow topology
- pure domain model
- authored/canonical config pipeline
- storage implementation
- runtime/kernel primitive

Bad crate reasons:

- "this was the scope of the parity test"
- "these functions were implemented together"
- "the CLI command needs a place to put code"
- "the recipe name is convenient"
- "this is experimental so naming does not matter"

### Rule 2: Workflow Names Stop At Operations

Workflow topology names may appear in:

- operation crate names
- operation type names
- operation tests
- CLI/REST convenience commands only when the public workflow itself is durable domain language

Workflow topology names must not appear in:

- generic transport crates
- signer crates
- protocol clients
- capability names
- domain model namespaces
- state crate names when the states are independently reusable
- public schema namespaces when the behavior has broader domain meaning

### Rule 3: Capability Names Describe Authority

Capability names must describe the external authority being granted.

Allowed:

- `mfm.evm.rpc.read`
- `mfm.evm.transaction.submit`
- `mfm.signing.sign`
- `mfm.artifact.read`

Disallowed:

- `mfm.evm.dcv.read`
- `mfm.evm.dcv.signer`
- `mfm.evm.dcv.transaction_submit`
- `mfm.portfolio.evm_rpc` if the capability is generic EVM RPC

### Rule 4: Public APIs Use Domain Language

Public names should describe what the user means, not how the implementation was assembled.

Allowed:

```text
mfm evm contracts deploy
mfm evm contracts configure
mfm evm contracts validate
mfm evm contracts lifecycle

POST /v1/evm/contracts/deploy
POST /v1/evm/contracts/configure
POST /v1/evm/contracts/validate
POST /v1/evm/contracts/lifecycle
```

Disallowed:

```text
mfm evm dcv deploy
mfm evm dcv deploy-configure-validate

POST /v1/evm/dcv/deploy
POST /v1/evm/dcv/deploy-configure-validate
```

### Rule 5: Runtime Routing Is Not Semantic Config

Semantic config may refer to runtime systems by stable non-secret ids. Runtime process config maps
those ids to concrete local resources.

Example semantic config:

```json
{
  "network": {
    "network_id": "reth-dev",
    "source_id": "reth_local",
    "expected_chain_id": 31337
  },
  "signer": {
    "signer_id": "deployer",
    "expected_address": "0x000000000000000000000000000000000000dead"
  }
}
```

Example runtime config:

```json
{
  "sources": [
    {
      "source_id": "reth_local",
      "network_id": "reth-dev",
      "rpc_url_env": "MFM_RETH_RPC_URL",
      "authorization_env": "MFM_RETH_AUTHORIZATION"
    }
  ],
  "signers": [
    {
      "signer_id": "deployer",
      "provider": "mfm_keystore",
      "entry_id": "550e8400-e29b-41d4-a716-446655440000",
      "keystore_path_env": "MFM_DEPLOYER_KEYSTORE",
      "password_file_env": "MFM_DEPLOYER_KEYSTORE_PASSWORD_FILE"
    }
  ]
}
```

The runtime config above is process-local. It must not become typed workflow config, event payload,
artifact payload, public output, or replay input.

## Allowed And Disallowed Boundaries

| Disallowed | Allowed |
|---|---|
| `crates/transports/evm-dcv` owns JSON-RPC, signing, lifecycle runner, replay | `crates/transports/evm` owns JSON-RPC; `crates/transports/evm-contracts` owns lifecycle runner |
| `EvmDcvRpcClient` | `EvmJsonRpcClient` |
| `DeployConfigureValidateSignerConfig` with keystore env names | `SignerRef` plus runtime signer registry |
| `EvmDcvSignerCapability` | `SigningCapability` or `SignerCapability` |
| `EvmDcvReadCapability` | `EvmRpcReadCapability` |
| `mfm.evm.dcv.value.abi_json` | `mfm.evm.contract.value.abi_json` or `mfm.evm.abi.value.abi_json` |
| `mfm.evm.dcv.config.deploy` | `mfm.evm.contract.config.deploy` |
| `mfm evm dcv deploy-configure-validate` | `mfm evm contracts lifecycle` |
| `/v1/evm/dcv/validate` | `/v1/evm/contracts/validate` |
| local reth routed as `ethereum-mainnet` | local reth routed as `reth-dev` with `expected_chain_id` checked |

## Target Shape

### Generic Signing

Add a generic signing boundary:

```text
crates/signing
```

It should own:

- `SignerRef`
- signing algorithm identifiers
- signing domain identifiers
- signing request/result traits
- redaction-safe signing errors

It should not know about:

- EVM contract lifecycle
- REST
- CLI
- keystore file paths
- password file paths

### Keystore Signer Provider

Add:

```text
crates/signers/keystore
```

It should own:

- mapping runtime signer ids to MFM keystore entries
- env-var based path/password resolution
- password trimming and zeroization
- keystore unlock/load/sign behavior
- redaction-safe errors

It should not know about:

- deploy/configure/validate
- EVM lifecycle states
- CLI request shapes
- REST request shapes

### Generic EVM Transport

Add:

```text
crates/transports/evm
```

It should own:

- `EvmJsonRpcClient`
- `EvmRpc` trait
- `EvmSourceRef`
- `EvmRpcSource`
- source routing
- request/response parsing
- `eth_chainId`
- `web3_clientVersion`
- `eth_blockNumber`
- `eth_getTransactionCount`
- fee/gas calls
- `eth_estimateGas`
- `eth_sendRawTransaction`
- `eth_getTransactionReceipt`
- receipt polling helpers
- `eth_call`
- `eth_getLogs`
- redaction-safe EVM RPC errors

It should not know about:

- DCV
- contract lifecycle operation topology
- MFM keystore internals
- public CLI or REST request kinds

### EVM Contract Model

Rename or replace:

```text
crates/evm-dcv-model
```

with:

```text
crates/evm-contract-model
```

It should own:

- contract artifact config
- ABI JSON wrappers
- bytecode JSON wrappers
- ABI calldata preparation
- event assertion preparation
- read assertion preparation
- `DeployedContract`
- `ConfiguredContract`
- validation report values

It should not own:

- workflow recipe names
- signer resolution
- EVM RPC clients
- keystore access

### EVM Contract Lifecycle Config

Rename or replace:

```text
crates/evm-deploy-configure-validate-config
```

with:

```text
crates/evm-contract-lifecycle-config
```

It should own:

- authored/canonical contract lifecycle config
- phase config types
- non-secret signer refs
- network/source refs
- receipt policy
- validation assertions

It should not own:

- keystore env names
- password-file env names
- RPC URLs
- authorization headers
- raw transaction bytes

### EVM Contract States

Rename or replace:

```text
crates/states/evm-dcv
```

with:

```text
crates/states/evm-contracts
```

Keep the durable state concepts:

- `DeployContractState`
- `ConfigureContractState`
- `ValidateContractState`
- `DeployedContract`
- `ConfiguredContract`
- `ValidationReport`

Rename recipe-shaped values:

- `EvmDcvTransactionIntent` -> `EvmTransactionIntent` or `ContractTransactionIntent`
- `EvmDcvDeployIntent` -> `ContractDeployIntent`
- `EvmDcvConfigureIntent` -> `ContractConfigureIntent`
- `EvmDcvValidateReadRequest` -> `ContractValidationReadRequest`
- `EvmDcvValidateReadResponse` -> `ContractValidationReadResponse`
- `DcvPublicOutputs` -> `ContractLifecyclePublicOutputs`

### Lifecycle Adapter

Rename or replace:

```text
crates/transports/evm-dcv
```

with a lifecycle adapter that depends on shared capabilities:

```text
crates/transports/evm-contracts
```

or, if MFM chooses to create an adapter namespace:

```text
crates/adapters/evm-contracts
```

It should own:

- runner binding for contract lifecycle states
- side-effect phase progression
- prepared invocation evidence
- transaction hash consistency checks
- receipt and confirmation evidence
- lifecycle replay verifier

It should not own:

- generic JSON-RPC client implementation
- signer provider implementation
- keystore path/password resolution
- workflow operation topology

### Lifecycle Operation

Rename or replace:

```text
crates/ops/evm-deploy-configure-validate-op
```

with:

```text
crates/ops/evm-contract-lifecycle-op
```

It should own deterministic topology:

```text
deploy -> configure -> validate
```

It may expose standalone operation helpers:

- deploy only
- configure only
- validate only
- full lifecycle

It must remain planning-only.

## Reviewer Checklist

Before accepting a new op, state, transport, adapter, signer, or config, reviewers must ask:

- What category is this: platform primitive, domain model, state, adapter, transport, signer,
  config, operation, storage, app assembly, or binary surface?
- Is the crate named after a durable abstraction?
- Is any crate named after a temporary workflow recipe?
- Could another workflow reuse this protocol behavior?
- Could another workflow reuse this signing behavior?
- Does a state depend on runtime, store implementation, app, binary, transport implementation, or
  signer implementation?
- Does an operation perform runtime behavior or ambient IO?
- Does a transport depend on a workflow operation crate?
- Does a signer depend on a domain workflow crate?
- Does typed config contain only semantic intent and non-secret references?
- Are `network_id`, `source_id`, and `expected_chain_id` distinct?
- Are RPC URLs, authorization headers, keystore paths, password paths, raw tx bytes, and signatures
  absent from typed configs, events, artifacts, public outputs, fixtures, and error details?
- Are public CLI/REST/schema names domain-language names?
- Do tests enforce boundaries, or do they simply assert the current implementation shape?

If the answer is unclear, the change is not ready.

## Tests And CI Rules To Add

The repo already has useful kernel dependency checks. It needs similar checks for domain/platform
boundaries.

Add cargo-metadata tests that assert:

- `crates/states/*` do not depend on `crates/transports/*`, `crates/app`, `bin/*`, or storage
  implementations
- `crates/ops/*` do not depend on transports, signers, app, binaries, or storage implementations
- generic transports do not depend on workflow operation crates
- signer crates do not depend on workflow operation or state crates
- app may assemble transports and certification descriptors, but does not own workflow semantics

Add namespace tests that assert:

- no new public schema namespace uses recipe acronyms
- capability names are authority names, not workflow names
- public route kinds do not include implementation recipe names

Add source/API checks that assert:

- EVM JSON-RPC code lives in the generic EVM transport
- keystore path/password resolution lives in the keystore signer provider
- workflow typed config does not contain `keystore_path_env`, `password_file_env`, `rpc_url`, or
  `authorization`
- replay code does not read env vars, open live RPC, open keystores, or access signer providers

Avoid source scans as the only proof. Prefer cargo metadata, type boundaries, compile-fail tests,
schema golden tests, and production-path integration tests. Source scans may be useful as guardrails
only when paired with stronger checks.

## Repo Areas To Audit Next

### EVM DCV Surface

Audit and replace:

- `crates/evm-dcv-model`
- `crates/evm-deploy-configure-validate-config`
- `crates/states/evm-dcv`
- `crates/transports/evm-dcv`
- `crates/ops/evm-deploy-configure-validate-op`
- `bin/cli/src/commands/evm/dcv.rs`
- `/v1/evm/dcv/*` routes in `bin/rest-api`
- `mfm.evm.dcv.*` schema namespaces
- `EvmDcv*`, `Dcv*`, and `DeployConfigureValidate*` public types

### Portfolio Transport

Audit `crates/transports/portfolio`.

Current concern:

- it embeds an EVM JSON-RPC client
- it parses `MFM_EVM_RPC_SOURCES_JSON`
- it routes EVM reads by `network_id`

Target:

- portfolio runner depends on generic EVM read transport/capability
- portfolio state semantics remain portfolio-specific
- generic EVM RPC behavior is not duplicated

### Portfolio Wallet And Signer Model

Audit portfolio wallet implementation fields.

Current concern:

- `WalletImplementationConfig` includes signer-shaped concepts
- `WalletSignerConfig` and `WalletCapabilities` may become workflow-local signer abstractions if
  extended casually

Target:

- reusable signer references and signer capabilities come from the generic signing boundary
- portfolio config uses non-secret signer refs only where signing is truly semantic
- portfolio read-only flows do not imply signer authority

### EVM RPC Routing Runbook

Audit `docs/evm-rpc-routing.md`.

Current concern:

- it documents runtime routing but still normalizes mixed source/network/control-scope behavior
- examples route local reth as `ethereum-mainnet`
- it treats EVM RPC routing as a typed transport runbook for multiple workflows before the generic
  transport exists

Target:

- document generic EVM runtime source registry
- keep `network_id`, `source_id`, `expected_chain_id`, and `control_scope` separate
- remove DCV language

### Nixfied And Integration Tests

Audit:

- `nixfied/project/module.nix`
- parity reth integration tests
- CLI/REST tests
- docs catalog and repo map

Current concern:

- local reth source aliases include `ethereum-mainnet`
- tests assert DCV routes and request kinds
- tests assert linked DCV transport crates

Target:

- local reth uses a local/dev network id
- expected chain id is checked explicitly
- tests assert contract lifecycle public API names
- boundary tests reject recipe-specific transport/signer leaks

## Migration Principles

Do not iterate on the DCV public surface. Use the working implementation as a behavioral prototype
and move the behavior into correct boundaries.

The migration order should be capability-first:

1. Add generic signing boundary.
2. Add MFM keystore signer provider.
3. Add generic EVM JSON-RPC transport.
4. Rename/split EVM contract model and lifecycle config.
5. Rename EVM contract state crate and schema namespaces.
6. Split lifecycle adapter from generic EVM transport and keystore signing.
7. Rename operation crate and compile APIs.
8. Replace CLI and REST surfaces with contract lifecycle language.
9. Rebaseline reth integration tests with correct network/source language.
10. Add architecture boundary tests and update docs.

Each step should preserve or strengthen:

- no raw signed transaction persistence
- no secret-bearing error messages
- replay from recorded evidence only
- side-effect idempotency and recovery behavior
- certified typed spec authority
- append-only stream authority

## Acceptance Criteria For The Correction

This architecture correction is complete only when:

- no public API exposes `dcv` except historical migration notes
- generic EVM JSON-RPC behavior is reusable by portfolio and contract lifecycle
- keystore signing is behind generic signer provider boundaries
- contract lifecycle config contains signer refs, not keystore path/password env refs
- contract lifecycle states own domain semantics without ambient IO
- lifecycle adapter records side-effect evidence through runtime/store APIs
- operations remain deterministic topology only
- CLI and REST expose contract lifecycle language
- local reth tests do not pretend to be `ethereum-mainnet`
- tests enforce architecture boundaries rather than the old implementation shape

## Appendix A: DCV Case Study

The old DCV implementation made the following useful concepts visible:

- `DeployContractState`
- `ConfigureContractState`
- `ValidateContractState`
- `DeployedContract`
- `ConfiguredContract`
- `ConfiguredContractRef`
- `ValidationReport`
- side-effect evidence phases
- replay evidence checks
- redaction tests
- reth-backed lifecycle coverage

Those concepts should survive.

The following should not survive as public or crate architecture:

- `dcv`
- `deploy_configure_validate` as a durable lower-layer namespace
- `EvmDcv*`
- `Dcv*`
- `mfm.evm.dcv.*`
- `crates/transports/evm-dcv`
- `EvmDcvRpcClient`
- `DeployConfigureValidateSignerConfig`
- `/v1/evm/dcv/*`
- `mfm evm dcv`
- `mfm-transports-evm-dcv`
- `typed-evm-dcv`

This is not because "DCV is a bad acronym." It is because DCV was a recipe and the architecture let
a recipe become a platform boundary.

## Appendix B: Crate-By-Crate Impact

Intentionally deferred.

Use this appendix in the next review pass to spell out the concrete impact for each current crate,
including rename/delete/split decisions, dependency changes, public type changes, schema namespace
changes, tests, and docs.
