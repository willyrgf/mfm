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
not a real capability. Signing is broader than DCV. EVM read authorities are broader than DCV and
have distinct evidence profiles. EVM transaction submission is broader than DCV.

Correct capability names should describe authority:

- `mfm.signing.sign`
- `mfm.evm.chain_identity.read`
- `mfm.evm.call.read`
- `mfm.evm.logs.read`
- `mfm.evm.nonce.read`
- `mfm.evm.transaction.submit`

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
- `expected_chain_id`: chain identity returned by the network
- `source_id`: process-local runtime source routing key, not workflow semantic config unless the
  selected source itself is domain intent
- `control_scope`: MFM execution partition, only if truly semantic

Local reth should not be represented as `ethereum-mainnet` unless it is actually mainnet.

### Transport Boundary

The current repo has two EVM JSON-RPC implementations embedded in feature transports:

- `crates/transports/evm-dcv`
- `crates/transports/portfolio`

Both parse `MFM_EVM_RPC_SOURCES_JSON`. Both route by `network_id`. Both make JSON-RPC calls through
feature-local clients. That duplication is a signal that the protocol contract and transport
implementation boundaries are wrong.

Generic EVM capability contracts must exist before any workflow-specific adapter uses live EVM
JSON-RPC. The split is:

- `crates/evm-capabilities`: EVM capability traits, capability specs, request/response contract
  types, source-ref/evidence types, and redaction-safe error contracts
- `crates/transports/evm`: live JSON-RPC implementation, runtime source registry, HTTP/client
  behavior, endpoint/auth redaction, and implementations of the capability traits

### Signer Boundary

Keystore signing currently lives inside the EVM DCV transport. That makes a local MFM keystore an
implementation detail of one contract lifecycle runner.

Signing is not DCV-specific and not EVM lifecycle-specific. It also must not become "sign arbitrary
bytes." The target shape is:

- generic signing crate: signer refs, typed algorithm/domain/purpose identifiers, request/result
  traits, expected public identity verification, private constructors for secret-bearing values,
  and error redaction
- keystore signer crate: MFM keystore-backed implementation
- EVM signing adapter: converts EVM transaction prehashes into signing requests and verifies the
  expected EVM address
- lifecycle adapter: asks for a signature, encodes submit-time raw transaction bytes transiently,
  and records redacted evidence

Raw signed transactions are bearer mutation material. They must remain transient submit-time bytes.
Provider APIs must not expose decrypted private keys.

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
- implement protocol traits defined by capability contract crates
- support multiple adapters and workflows

Transports must not:

- use workflow recipe names
- define the protocol contract that states depend on
- depend on workflow operation crates
- know about contract lifecycle topology
- open keystores or own signer behavior
- persist typed semantic events by themselves

### Signer

Signers provide generic key material and signature capabilities.

Signer crates may:

- define `SignerRef`
- define typed signing request/result traits with algorithm, domain, and purpose separation
- implement MFM keystore-backed signing
- map runtime signer ids to keystore entries, hardware signers, remote signers, or future wallets
- verify expected public identity for a signing domain
- redact all secret-bearing details from errors

Signer crates must not:

- depend on contract lifecycle states
- depend on EVM workflow operations
- expose "sign arbitrary bytes" APIs
- expose decrypted private keys through provider APIs
- leak private keys, mnemonics, passwords, password paths, keystore paths, signatures, or raw signed
  transactions into typed semantic surfaces

### Configuration

Typed config contains semantic intent and non-secret references only.

Typed config may contain:

- contract artifact reference or inline artifact
- ABI call intent
- expected chain id
- network id
- source/oracle id only when source selection is domain intent, not local runtime routing
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

- `mfm.evm.chain_identity.read`
- `mfm.evm.block.read`
- `mfm.evm.call.read`
- `mfm.evm.logs.read`
- `mfm.evm.nonce.read`
- `mfm.evm.fee.read`
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

Semantic config describes domain intent. Runtime process config chooses concrete local resources.
Typed workflow config must not contain process-local `source_id` values unless choosing a source is
itself domain intent, such as selecting a named oracle. In ordinary EVM execution, typed config
names the semantic network and expected chain identity; runtime config maps that intent to a local
source and records redacted source evidence.

Example semantic config:

```json
{
  "network": {
    "network_id": "reth-dev",
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
| `crates/transports/evm-dcv` owns JSON-RPC contracts, live JSON-RPC, signing, lifecycle runner, replay | `crates/evm-capabilities` owns EVM capability contracts; `crates/transports/evm` owns live JSON-RPC; `crates/adapters/evm-contracts` owns lifecycle execution |
| states depend on `crates/transports/evm` | states depend on `crates/evm-capabilities` traits and capability specs only |
| `EvmDcvRpcClient` | `EvmJsonRpcClient` |
| `DeployConfigureValidateSignerConfig` with keystore env names | `SignerRef` plus runtime signer registry |
| `EvmDcvSignerCapability` | `SigningCapability` or `SignerCapability` |
| `EvmDcvReadCapability` | segregated EVM capability specs such as `EvmCallReadCapability`, `EvmLogsReadCapability`, and `EvmChainIdentityCapability` |
| `mfm.evm.dcv.value.abi_json` | `mfm.evm.contract.value.abi_json` or `mfm.evm.abi.value.abi_json` |
| `mfm.evm.dcv.config.deploy` | `mfm.evm.contract.config.deploy` |
| `mfm evm dcv deploy-configure-validate` | `mfm evm contracts lifecycle` |
| `/v1/evm/dcv/validate` | `/v1/evm/contracts/validate` |
| typed workflow config chooses local `source_id = "reth_local"` | runtime config maps `source_id = "reth_local"` to semantic `network_id = "reth-dev"` and verifies `expected_chain_id` |

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
- signing purpose identifiers
- signing request/result traits
- expected public identity verification contracts
- private constructors for secret-bearing or transient signing material
- redaction-safe signing errors

It should not know about:

- EVM contract lifecycle
- REST
- CLI
- keystore file paths
- password file paths
- arbitrary untyped byte signing

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
- per-request signing audit behavior
- redaction-safe errors

It should not know about:

- deploy/configure/validate
- EVM lifecycle states
- CLI request shapes
- REST request shapes
- APIs that return decrypted private keys

### EVM Capability Contracts

Add:

```text
crates/evm-capabilities
```

It should own:

- EVM capability specs and traits
- EVM source reference and redacted source evidence types
- request/response contract types
- chain identity read capability
- block read capability
- call read capability
- log read capability
- nonce read capability
- fee/gas read capability
- transaction submission capability
- receipt polling capability
- redaction-safe EVM RPC errors

It should not know about:

- DCV
- contract lifecycle operation topology
- HTTP
- reqwest
- live RPC endpoint URLs
- authorization headers
- MFM keystore internals
- public CLI or REST request kinds

### Generic EVM Transport

Add:

```text
crates/transports/evm
```

It should own:

- `EvmJsonRpcClient`
- live JSON-RPC request execution
- runtime source registry parsing
- `source_id` routing for process-local sources
- endpoint and authorization redaction
- request/response wire parsing
- implementations of `crates/evm-capabilities` traits
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

It should not know about:

- DCV
- contract lifecycle operation topology
- MFM keystore internals
- public CLI or REST request kinds
- state graph topology

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

### EVM Contract Phase Config

Rename or replace:

```text
crates/evm-deploy-configure-validate-config
```

with:

```text
crates/evm-contract-config
```

It should own:

- authored/canonical deploy phase config
- authored/canonical configure phase config
- authored/canonical validate phase config
- non-secret signer refs
- semantic network and expected-chain intent
- receipt policy
- validation assertions

It should not own:

- full `deploy -> configure -> validate` aggregate topology
- workflow machine ids
- process-local source ids unless source choice is domain intent
- keystore env names
- password-file env names
- RPC URLs
- authorization headers
- raw transaction bytes

The full lifecycle aggregate config belongs in `crates/ops/evm-contract-lifecycle-op`, because the
aggregate is operation topology. Lower config crates may own reusable phase config only.

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

with a lifecycle adapter that depends on shared capability contracts and runtime-provided
implementations:

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

- generic EVM capability contracts
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
- Does a state depend only on capability contract crates, not live transport crates?
- Does an operation perform runtime behavior or ambient IO?
- Does a transport depend on a workflow operation crate?
- Does a signer depend on a domain workflow crate?
- Does typed config contain only semantic intent and non-secret references?
- Are `network_id`, `source_id`, and `expected_chain_id` distinct?
- Is `source_id` absent from typed workflow config unless source choice is itself domain intent?
- Are EVM capabilities segregated by authority and evidence profile?
- Are RPC URLs, authorization headers, keystore paths, password paths, raw tx bytes, and signatures
  absent from typed configs, events, artifacts, public outputs, fixtures, and error details?
- Are public CLI/REST/schema names domain-language names?
- Do tests enforce boundaries, or do they simply assert the current implementation shape?

If the answer is unclear, the change is not ready.

## Tests And CI Rules To Add

The repo already has useful kernel dependency checks. It needs similar checks for domain/platform
boundaries. These tests should be added first with explicit allowlists for existing violations, then
the allowlists should shrink as migration work lands.

Add cargo-metadata tests that assert:

- `crates/states/*` do not depend on `crates/transports/*`, `crates/app`, `bin/*`, or storage
  implementations
- `crates/states/*` may depend on capability contract crates such as `crates/evm-capabilities`
- `crates/ops/*` do not depend on transports, signers, app, binaries, or storage implementations
- generic transports do not depend on workflow operation crates
- signer crates do not depend on workflow operation or state crates
- app may assemble transports and certification descriptors, but does not own workflow semantics

Add namespace tests that assert:

- no new public schema namespace uses recipe acronyms
- capability names are authority names, not workflow names
- public route kinds do not include implementation recipe names

Add source/API checks that assert:

- EVM capability traits, request/response contracts, and capability specs live in
  `crates/evm-capabilities`
- live EVM JSON-RPC implementation code lives in the generic EVM transport
- keystore path/password resolution lives in the keystore signer provider
- workflow typed config does not contain `source_id`, `keystore_path_env`, `password_file_env`,
  `rpc_url`, or `authorization`, except for a documented source-selection domain intent
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

- portfolio runner depends on EVM capability contracts; runtime/app wiring supplies the live EVM
  transport implementation
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

- document EVM capability contracts and generic EVM runtime source registry
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

The migration order should be enforcement-first, then capability-first:

1. Add allowlisted architecture boundary tests for dependency direction, namespaces, capabilities,
   and forbidden typed-config fields.
2. Add generic signing boundary.
3. Add MFM keystore signer provider.
4. Add `crates/evm-capabilities`.
5. Add generic EVM JSON-RPC transport implementation.
6. Rename/split EVM contract model and reusable phase config.
7. Rename EVM contract state crate and schema namespaces.
8. Split lifecycle adapter into `crates/adapters/evm-contracts`.
9. Move full lifecycle aggregate config/topology into the operation crate.
10. Rename operation crate and compile APIs.
11. Replace CLI and REST surfaces with contract lifecycle language.
12. Rebaseline reth integration tests with correct network/source language.
13. Shrink architecture-test allowlists and update docs.

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
- EVM capability contracts live outside live transport implementation crates
- generic EVM JSON-RPC behavior is reusable by portfolio and contract lifecycle
- keystore signing is behind generic signer provider boundaries
- reusable contract phase config contains signer refs, not keystore path/password env refs
- full lifecycle aggregate config/topology lives in the operation crate
- contract lifecycle states own domain semantics without ambient IO
- lifecycle adapter lives under `crates/adapters/evm-contracts` and records side-effect evidence
  through runtime/store APIs
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

## Appendix B: Repo-Wide Audit And Crate-By-Crate Impact

This appendix is the current repo-wide audit ledger. It is intentionally concrete. It records where
the architecture failure is already hardened in crate boundaries, public APIs, schema names, tests,
docs, Nixfied wiring, and support code.

Not every item below is equally broken. Some entries are correct behavior in the wrong crate, some
are correct crates with leaked runtime-resource fields, and some are tests that should survive after
renaming. The purpose is to make the real blast radius explicit before migration work starts.

### Audit Labels

- `replace`: the current public boundary is wrong and should be superseded, not carried forward
- `split`: useful behavior exists, but the crate owns more than one durable abstraction
- `rename`: useful behavior can survive under domain/platform names
- `move`: code belongs under another boundary
- `preserve`: the invariant is good and should be kept under the new boundary
- `audit`: the same flaw may exist, but the exact target depends on a later design pass

### B.1 Primary Blast Radius

The direct DCV stack is not a local naming problem. The repo currently makes DCV a first-class
architecture axis through all of these surfaces:

- workspace members in `Cargo.toml`
- generated package names in `Cargo.lock`
- model schemas in `crates/evm-dcv-model`
- workflow config schemas in `crates/evm-deploy-configure-validate-config`
- state schemas, capabilities, public outputs, and adapter identity in `crates/states/evm-dcv`
- EVM JSON-RPC, transaction submission, signer resolution, lifecycle runner, artifact reader, and
  replay verifier in `crates/transports/evm-dcv`
- operation topology and public operation ids in `crates/ops/evm-deploy-configure-validate-op`
- app registration in `crates/app`
- CLI commands in `bin/cli/src/commands/evm/dcv.rs`
- REST routes and request kinds in `bin/rest-api/src/lib.rs`
- parity tests in `tests/integration`
- docs catalog, repo map, docs.rs readiness notes, CLI docs, REST docs, and EVM RPC routing docs
- Nixfied publish-docs and reth source wiring

The same underlying pattern also appears outside the DCV stack:

- `crates/transports/portfolio` duplicates generic EVM JSON-RPC behavior and source routing
- portfolio wallet/source models include runtime-provider and signer-adjacent concepts
- core config mixes semantic network/auth config with endpoint URLs and keystore references
- CLI keystore support owns signing behavior that should be behind a signer provider
- `crates/transports/proof` depends on an operation crate, reversing the operation/transport
  dependency direction
- tests assert current implementation shape more strongly than architecture invariants

### B.2 Direct Contract Lifecycle Stack

| Area | Impact | Required correction |
|---|---|---|
| `Cargo.toml` | `replace`: workspace promotes DCV model/config/state/transport/op crates as durable taxonomy. | Replace workspace members with contract model, contract lifecycle config/state/op, generic EVM transport, generic signing, keystore signer, and lifecycle adapter crates. |
| `Cargo.lock` | `replace`: generated package graph hardens old package names. | Regenerate only after crate split/rename work lands. |
| `crates/evm-dcv-model` | `rename`: reusable contract model behavior is named after the DCV recipe. | Replace with `crates/evm-contract-model`; move generic ABI helpers to `crates/evm-core` if that produces a cleaner boundary. |
| `crates/evm-dcv-model/src/lib.rs` | `replace`: ABI, bytecode, deployed/configured values, assertions, and validation reports use `mfm.evm.dcv.*` schema namespaces and `Dcv` naming. | Use `mfm.evm.contract.*`, `mfm.evm.abi.*`, and `mfm.evm.contract.lifecycle.*` namespaces. |
| `crates/evm-dcv-model/README.md` | `rename`: package docs teach the wrong model name. | Rewrite around EVM contract artifact, ABI, lifecycle, and validation values. |
| `crates/evm-deploy-configure-validate-config` | `split`: config crate is named after workflow topology and owns both phase config and aggregate lifecycle config. | Replace lower-layer config with reusable contract phase config, likely `crates/evm-contract-config`; move full `deploy -> configure -> validate` aggregate config into `crates/ops/evm-contract-lifecycle-op`. |
| `crates/evm-deploy-configure-validate-config/src/lib.rs` | `split`: config mixes contract intent, `network_id`, `control_scope`, signer implementation, keystore entry id, keystore path env vars, password-file env vars, receipt policy, and client-version assertions. | Keep semantic phase config only. Move signer material resolution to signer runtime config. Move process-local RPC source routing to EVM transport runtime config. Move runtime client assertions to adapter policy or test fixtures. |
| `DeployConfigureValidateSignerConfig` | `replace`: typed config embeds `keystore_path_env` and `password_file_env`. | Replace with `SignerRef` plus optional expected public address. Runtime signer registry maps signer refs to provider-specific details. |
| `crates/states/evm-dcv` | `rename`: state crate owns reusable contract lifecycle states under a recipe name. | Replace with `crates/states/evm-contracts` or another durable contract lifecycle state crate. |
| `crates/states/evm-dcv/src/lib.rs` | `replace`: state namespace, adapter identity, capability ids, public outputs, values, evidence, and operation output types use `mfm.evm.dcv`, `typed-evm-dcv`, `EvmDcv*`, and `Dcv*`. | Rename to contract lifecycle state and value language. Adapter identity should move to the adapter crate. |
| `EvmDcvReadCapability` | `replace`: multiple EVM authorities are collapsed under a DCV read name. | Replace with segregated EVM capabilities for chain identity, block reads, calls, logs, nonce, fee/gas, receipt polling, and any other distinct authority/evidence profile. |
| `EvmDcvSignerCapability` | `replace`: generic signing authority is named after DCV. | Replace with `mfm.signing.sign`. |
| `EvmDcvTransactionSubmitCapability` | `replace`: generic EVM transaction submission authority is named after DCV. | Replace with `mfm.evm.transaction.submit`. |
| `EvmDcvReadBackend` | `move`: generic EVM read behavior is state-local. | Move reusable protocol behavior to the generic EVM transport/capability boundary. |
| state raw-transaction protection tests | `preserve`: raw signed transaction secrecy is the right invariant. | Keep the invariant under lifecycle/signing boundaries; stop anchoring it to `evm-dcv`. |
| `crates/transports/evm-dcv` | `split`: one recipe transport owns EVM capability contracts, generic EVM JSON-RPC, source routing, signer resolution, keystore access, signing, transaction submission, receipt polling, lifecycle runner, artifact reader, and replay verifier. | Split into `crates/evm-capabilities`, `crates/transports/evm`, `crates/signing`, `crates/signers/keystore`, and `crates/adapters/evm-contracts`. |
| `EvmDcvRpcClient` | `move`: generic EVM JSON-RPC client and protocol contract are workflow-local. | Move capability traits/request-response contracts to `crates/evm-capabilities`; extract live `EvmJsonRpcClient` to `crates/transports/evm`. |
| `MFM_EVM_RPC_SOURCES_JSON` parsing in DCV transport | `move`: runtime source registry is owned by a workflow transport. | Move to generic EVM transport runtime config. Route by `source_id`, not `network_id`; verify `expected_chain_id`. |
| keystore signing in DCV transport | `move`: transport opens keystore files, reads password files, extracts private keys, derives addresses, signs transactions, and encodes raw transactions. | Move signing request/result traits to `crates/signing`; move MFM keystore implementation to `crates/signers/keystore`; lifecycle adapter asks for signatures. |
| `verify_evm_dcv_replay` and `EvmDcvReplayVerifier` | `rename`: replay verifier is contract lifecycle evidence behavior, not DCV transport behavior. | Move/rename to lifecycle adapter replay verifier; replay must not read env vars, RPC, or signer providers. |
| `mfm-transports-evm-dcv` and `typed-evm-dcv` executable identity | `replace`: executable/source identity is named after recipe scaffolding. | Use generic transport identity for EVM RPC and lifecycle adapter identity for contract lifecycle runners. |
| `crates/ops/evm-deploy-configure-validate-op` | `rename`: operation crate can own topology, but the public name is recipe-shaped and leaks lower-layer DCV types. | Replace with `crates/ops/evm-contract-lifecycle-op`; expose planning API and domain outputs only. |
| operation public ids/scopes | `replace`: `mfm.evm.dcv.operation.*`, `evm_dcv`, `DCV_OPERATION_VERSION`, and `PUBLIC_OUTPUT_KEY = "evm_dcv"` harden recipe names. | Use contract lifecycle operation ids, scopes, versions, and public output keys. |
| operation compile APIs | `rename`: `compile_dcv_*`, `dcv_program_draft`, `Dcv*` APIs expose scaffolding. | Rename to lifecycle draft/compile/output names. |
| operation typestate tests | `preserve`: phase ordering tests are valid. | Keep compile-fail coverage under lifecycle operation/config types. |

### B.3 App, CLI, And REST Public Surface

| Area | Impact | Required correction |
|---|---|---|
| `crates/app/Cargo.toml` | `replace`: app depends directly on DCV op and DCV transport. | Depend on lifecycle op, lifecycle adapter, generic EVM transport, and signer providers. |
| `crates/app/src/lib.rs` | `replace`: app registers `EvmDcvArtifactReader`, DCV runners, DCV certification descriptors, and DCV replay verifier. | Register generic transports and signer providers separately from lifecycle adapter/descriptors. |
| `bin/cli/Cargo.toml` | `replace`: CLI imports recipe op crate. | Depend on lifecycle op/config only; do not depend on transport implementation names. |
| `bin/cli/src/commands/evm/mod.rs` | `replace`: public command group is `mfm evm dcv`. | Use `mfm evm contracts` or another durable contract lifecycle command group. |
| `bin/cli/src/commands/evm/dcv.rs` | `replace`: public types and outputs use `DcvCommand`, `DcvRequestArgs`, `EvmDcvResponse`, and `mfm.cli.evm_dcv.typed.v1`. | Rename around contract lifecycle. CLI should parse input, start/resume runs, and render outputs only. |
| `bin/cli/README.md` | `replace`: documents DCV command surface and EVM RPC source env examples that route local reth as mainnet. | Rewrite around contract lifecycle commands and generic EVM source registry. |
| `bin/rest-api/Cargo.toml` | `replace`: REST imports recipe op crate. | Depend on lifecycle op/config only. |
| `bin/rest-api/src/lib.rs` | `replace`: routes expose `/v1/evm/dcv/deploy`, `/configure`, `/validate`, and `/deploy-configure-validate`; request kinds and errors use `evm_dcv_*`. | Replace with contract lifecycle routes, request kinds, error codes, and framework versions. |
| `bin/rest-api/README.md` | `replace`: documents DCV REST contract and signer env references in requests. | Rewrite with lifecycle routes and `SignerRef` request shape. |

Breaking changes are required here. Compatibility aliases would preserve the wrong public
architecture and should not be added.

### B.4 Missing Platform Primitives

The audit found missing reusable capability crates. These should be introduced before renaming the
public lifecycle API, otherwise code will just move the same boundary violations under new names.

| Missing primitive | Why it is required | Proposed owner |
|---|---|---|
| generic signing API | Signing is broader than contract lifecycle and broader than EVM. | `crates/signing` |
| keystore signer provider | MFM keystore resolution, password handling, unlock, audit, and redaction are provider behavior. | `crates/signers/keystore` |
| EVM capability contracts | States and adapters need EVM traits, capability specs, request/response contracts, source-ref/evidence types, and redacted errors without depending on a live transport implementation. | `crates/evm-capabilities` |
| generic EVM JSON-RPC transport | `evm-dcv` and `portfolio` both implement live EVM JSON-RPC and parse the same source env. | `crates/transports/evm` |
| EVM source registry | Runtime endpoint routing must be reusable and redacted. | `crates/transports/evm` |
| runtime source reference | Runtime config needs a non-secret `source_id` distinct from semantic network identity; typed workflow config may use it only when source selection is domain intent. | `crates/evm-capabilities` for the type/evidence contract; `crates/transports/evm` for process-local resolution |
| lifecycle adapter | Contract lifecycle side-effect execution is domain-specific adapter behavior, not generic transport behavior. | `crates/adapters/evm-contracts` |

Minimum target types:

- `SignerRef`
- `SigningRequest`
- `SigningResult`
- `SigningCapability`
- `EvmSourceRef`
- `EvmRpcSource`
- `EvmJsonRpcClient`
- `EvmChainIdentityCapability`
- `EvmCallReadCapability`
- `EvmLogsReadCapability`
- `EvmNonceReadCapability`
- `EvmFeeReadCapability`
- `EvmTransactionSubmitCapability`
- `expected_chain_id` as a chain check, not an endpoint routing key

### B.5 Same Flaw Outside The DCV Stack

These areas should be audited before they are extended. Some are lower risk because they are not
yet public contract lifecycle APIs, but they show the same boundary weakness.

| Area | Impact | Required correction |
|---|---|---|
| `crates/transports/portfolio/src/lib.rs` | `split`: portfolio transport parses `MFM_EVM_RPC_SOURCES_JSON`, owns `PortfolioRpcClient`, implements `eth_blockNumber`, `eth_getBalance`, `eth_call`, ERC-20 reads, and routes by `network_id`. | Delete duplicate EVM RPC once `crates/evm-capabilities` and `crates/transports/evm` exist. Portfolio state/adapter should depend on EVM capability contracts while runtime wiring supplies the live transport. |
| `crates/states/portfolio/src/lib.rs` | `audit`: `PortfolioReadCapability` hides generic protocol authority behind portfolio naming; prepared sources and resolved subjects can carry `control_scope` and implementation kind. | Split portfolio semantic observation from generic EVM/Bitcoin/source capabilities. State outputs should carry public subject identity and evidence, not provider implementation names. |
| `crates/portfolio/model/src/portfolio.rs` | `audit`: `NetworkConfig` mixes `network_id`, `chain_id`, and `control_scope`. | Keep semantic network identity and expected chain identity distinct from runtime routing. |
| `crates/portfolio/model/src/wallet.rs` | `audit`: wallet config includes `KeystoreEntry`, `NodeManagedAccount`, `ExternalSigner`, `WalletSignerConfig`, `WalletCapabilities`, and `ResolvedWallet`. | Read-only portfolio config should describe wallet subjects only. Mutation/signing authority should use generic signer refs and runtime signer registry. |
| `crates/portfolio/model/src/symbol.rs` | `audit`: `PriceSourceRef.source_id` is documented as runtime environment source id. | Decide whether this is semantic oracle identity or runtime source routing; do not mix both in one field. |
| `crates/ops/portfolio-tracker-op` | `audit`: operation-level workflow naming is allowed, but it must not become a transport/source/signer boundary. | Keep planning-only. Ensure portfolio op does not own runtime EVM routing or generic source concepts. |
| `docs/PORTFOLIO_SNAPSHOT_DETAILED.md` | `replace`: portfolio docs own EVM RPC bootstrap details and public mainnet fallback behavior. | Point to generic EVM transport/source registry docs after that boundary exists. |
| `docs/UPGRADE.md` | `replace`: upgrade notes document `MFM_EVM_RPC_SOURCES_JSON` and public mainnet fallback as workflow setup. | Rewrite with source registry semantics and explicit source/network/chain distinction. |
| `crates/core/src/config/mod.rs` | `audit`: top-level config mixes networks, wallets, auth, `rpc_urls`, `rpc_url`, and chain id. | Quarantine as legacy application config or split into domain registry plus runtime process config. Do not reuse as typed workflow config. |
| `crates/core/src/config/authentication/mod.rs` | `audit`: `KeystoreReference` includes local paths. | Public/typed config should carry signer refs. Runtime config may resolve paths. |
| `crates/core/src/config/network.rs` | `audit`: static network config includes RPC URLs. | Static registries should keep semantic network identity only; transports own endpoints. |
| `crates/core/src/config/dexes.rs` | `audit`: DEX config includes endpoint-like runtime data mixed with static protocol data. | Keep protocol/address identity separate from runtime endpoint selection. |
| `crates/core/src/keystore/mod.rs` | `audit`: `get_private_key` returns decrypted signing authority to callers. | Prefer provider APIs that accept signing requests and return signatures/public metadata without exposing keys. |
| `crates/core/src/keystore/README.md` | `replace`: docs teach private key retrieval and path-based runtime config. | Rewrite around signer-provider usage once provider exists. |
| `bin/cli/src/support/keystore.rs` | `move`: CLI owns EIP-1559 signing, key lookup, password env/file handling, raw signed tx file output, and test KDF downgrade behavior. | Move signing and secret-file hardening into keystore signer provider. CLI remains parser/renderer and utility wrapper. |
| `bin/cli/src/commands/keystore/mod.rs` | `audit`: boundary tests currently require direct typed keystore helpers. | Invert tests so CLI-owned signing and broad private-key extraction become review failures. |
| `bin/cli/src/presentation/output.rs` | `audit`: public error output can print arbitrary messages. | Add central redaction discipline for secret-bearing/runtime-resource errors. |
| `crates/kernel/values/src/lib.rs` | `audit`: secret marker checks are heuristic and do not reject runtime-resource field names. | Add persisted-surface deny rules for fields such as `rpc_url`, `authorization`, `password_file_env`, `keystore_path`, and raw transaction material. |
| `crates/kernel/program-derive/src/lib.rs` | `audit`: derive checks reject secret wrapper misuse, but not runtime-resource references in typed config. | Add schema/derive/CI checks for forbidden typed config field names. |
| `crates/collectors/btc-jsonrpc-http` | `audit`: Bitcoin JSON-RPC is protocol-shaped but lives under collectors and carries `rpc_password` as plain `String`. | Before Bitcoin workflows grow, decide transport taxonomy and use redacted/zeroized runtime config. |
| `crates/collectors/proof` | `audit`: proof values, capabilities, typed state specs, public outputs, and replay-verifier interfaces live under a `collectors` directory even though the crate is domain/state-contract shaped. | Classify explicitly: either keep it as a conformance fixture with that role documented, or move durable proof state/domain contracts to a state/domain crate such as `crates/states/proof`; keep the current no-live-IO dependency boundary. |
| `crates/transports/proof` | `replace`: transport depends on `mfm-op-proof`, reversing operation/transport direction. | Move op-based conformance helpers to tests/support or app fixture layer; transports must not depend on op crates. |
| `crates/transports/process-exec` | `preserve`: no duplicate workflow-specific protocol issue found in this audit. | Keep watching dependency direction and public capability names. |
| `crates/evm-core` | `preserve`: appears to be utility/domain support, not a workflow transport. | Use it as a possible home for generic ABI/EVM helpers if that preserves crate focus. |

### B.6 Public Names And Identifiers To Retire

These names should disappear from public API, schema, crate, docs, and test surfaces except in
historical migration notes:

- `dcv`
- `Dcv*`
- `EvmDcv*`
- `DeployConfigureValidate*` below the operation topology layer
- `deploy_configure_validate` as a durable schema, route, framework, or runner identity
- `crates/evm-dcv-model`
- `crates/evm-deploy-configure-validate-config`
- `crates/states/evm-dcv`
- `crates/transports/evm-dcv`
- `crates/ops/evm-deploy-configure-validate-op`
- `mfm.evm.dcv.*`
- `mfm.evm.dcv.operation.*`
- `mfm.evm.dcv.config.*`
- `mfm.evm.dcv.value.*`
- `mfm.evm.dcv.fact.*`
- `mfm.evm.dcv.capability.*`
- `mfm evm dcv`
- `/v1/evm/dcv/*`
- `evm_dcv_*` request kinds, error codes, public output keys, and test helper names
- `mfm.cli.evm_dcv.typed.v1`
- `mfm.rest_api.evm_dcv.typed.v1`
- `mfm-transports-evm-dcv`
- `typed-evm-dcv`
- `EvmDcvRpcClient`
- `EvmDcvReplayVerifier`
- `verify_evm_dcv_replay`

Names that may survive only at operation topology level if MFM deliberately chooses that public
workflow language:

- `deploy`
- `configure`
- `validate`
- `deploy -> configure -> validate`

The preferred durable public language is contract lifecycle.

### B.7 Tests And Tooling That Currently Enforce The Wrong Shape

| Area | Current problem | Required correction |
|---|---|---|
| `crates/app/tests/typed_transport_boundaries.rs` | Requires `mfm-transports-evm-dcv` and blesses the wrong transport crate. | Invert into architecture tests that reject workflow-specific generic transports, signer leakage, and `mfm.evm.dcv` public ids. |
| `tests/integration/tests/cargo_metadata_contract.rs` | Checks only narrow kernel dependency boundaries. | Add cargo-metadata rules for operation/state/adapter/transport/signer/config dependency direction. |
| `tests/integration/Cargo.toml` | Integration tests depend on DCV op/transport crates. | Depend on lifecycle op/adapter and generic EVM transport. |
| `tests/integration/src/test_support.rs` | Test wallet helpers emit DCV signer JSON with keystore env refs. | Emit `SignerRef`; configure test signer registry separately. |
| `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs` | Asserts `/v1/evm/dcv/*`, `evm_dcv_*`, DCV replay APIs, and local reth as `ethereum-mainnet`. | Test contract lifecycle routes/kinds, generic signer/source registry, local network identity, and explicit expected chain id. Add negative checks for retired DCV routes. |
| `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs` | Uses local reth as `ethereum-mainnet`. | Use local/dev network identity, process-local runtime `source_id`, and expected chain check. |
| `tests/integration/tests/typed_portfolio_snapshot_local.rs` | Has a better local network fixture, but still relies on source env keyed by network id. | Preserve as a positive fixture after runtime source registry migrates to `source_id`. |
| `crates/ops/evm-deploy-configure-validate-op/tests` | Compile-fail tests are useful but anchored to recipe crate/types. | Keep phase-ordering coverage under lifecycle operation/config names. |
| `crates/states/evm-dcv/tests` | Raw transaction protection is useful but anchored to old crate name. | Preserve under lifecycle/signing boundary and add checks for signer/ref leakage. |
| source-scan tests | Some scans assert implementation structure instead of architecture. | Prefer cargo metadata, schema tests, compile-fail tests, and type-boundary tests; use scans only as secondary guardrails. |

New architecture tests should reject:

- state crates depending on transport, signer provider, app, binary, or storage implementation crates
- operation crates depending on transport, signer provider, app, binary, or storage implementation
  crates
- transport crates depending on operation crates
- signer crates depending on workflow operation/state crates
- workflow config schemas containing `keystore_path_env`, `password_file_env`, `rpc_url`,
  `authorization`, raw transaction fields, private keys, mnemonics, or password fields
- public schema namespaces using recipe acronyms
- capability names using workflow names instead of authority names
- local reth fixtures using `ethereum-mainnet` unless the fixture is actually mainnet

### B.8 Docs, Catalogs, And Generated Metadata

| Area | Impact | Required correction |
|---|---|---|
| `docs/evm-rpc-routing.md` | `replace`: current runbook mixes DCV, portfolio, runtime routing, `network_id`, `control_scope`, and signer env references. | Rewrite around generic EVM source registry and separate `network_id`, `source_id`, `expected_chain_id`, and `control_scope`. |
| `docs/docs-rs-readiness.md` | `replace`: lists DCV model/config/state/transport/op as publishable units. | Reframe around platform primitives and contract lifecycle crates. |
| `crates/docs/catalog.toml` | `replace`: docs.rs catalog publishes DCV packages. | Replace entries after split/rename. |
| `crates/docs/README.md` | `replace`: umbrella docs list DCV packages. | Regenerate after catalog change. |
| `docs/repo-index.json` | `replace`: generated index lists old crate paths and descriptions. | Regenerate after crate changes. |
| `docs/repo-map.md` | `replace`: repo map teaches old placement. | Regenerate or rewrite after taxonomy changes. |
| `bin/cli/README.md` | `replace`: public docs bless DCV CLI and mixed EVM RPC source examples. | Rewrite after CLI and EVM source registry changes. |
| `bin/rest-api/README.md` | `replace`: public docs bless DCV REST routes. | Rewrite after REST route change. |
| `nixfied/project/module.nix` | `replace`: local reth source aliases include `ethereum-mainnet`; publish-docs examples reference `mfm-transports-evm-dcv`. | Use local/dev source and network ids with explicit chain checks; update docs package list after split. |

`docs/ops-and-states.md` has already been removed. No new architecture doc should refer to it.

### B.9 Target Crate Map

This is the target crate boundary map implied by the audit. Names can still be reviewed, but the
responsibility split should not be weakened.

| Current boundary | Target boundary |
|---|---|
| `crates/evm-dcv-model` | `crates/evm-contract-model`; optional generic ABI helpers in `crates/evm-core` |
| `crates/evm-deploy-configure-validate-config` phase config | reusable phase config in `crates/evm-contract-config` |
| `crates/evm-deploy-configure-validate-config` aggregate lifecycle config | aggregate topology config in `crates/ops/evm-contract-lifecycle-op` |
| `crates/states/evm-dcv` | `crates/states/evm-contracts` or contract lifecycle state crate |
| `crates/transports/evm-dcv` EVM traits/capability specs | `crates/evm-capabilities` |
| `crates/transports/evm-dcv` live EVM RPC implementation | `crates/transports/evm` |
| `crates/transports/evm-dcv` signing behavior | `crates/signing` and `crates/signers/keystore` |
| `crates/transports/evm-dcv` lifecycle runner and replay verifier | `crates/adapters/evm-contracts` |
| `crates/ops/evm-deploy-configure-validate-op` | `crates/ops/evm-contract-lifecycle-op` |
| `bin/cli/src/commands/evm/dcv.rs` | contract lifecycle CLI command module |
| `/v1/evm/dcv/*` | `/v1/evm/contracts/*` or equivalent lifecycle REST routes |
| `crates/transports/portfolio` EVM client | generic EVM transport plus portfolio adapter behavior |
| portfolio wallet signer model | generic signer refs/capabilities plus portfolio subject model |
| core path/url-bearing config reused by workflows | runtime process config only; typed workflow config gets non-secret refs |
| CLI keystore signing support | keystore signer provider plus thin CLI wrapper |
| proof transport depending on proof op | transport independent of op; conformance helpers in tests/support or app fixture layer |

### B.10 Migration Worklist From The Audit

Perform the correction enforcement-first, then capability-first, then public-surface-first. Do not
start with string renames while generic EVM capability contracts, live EVM transport, and signing
are still embedded in workflow crates.

1. Add allowlisted architecture tests for dependency direction, schema namespaces, capability
   names, typed config forbidden fields, signer boundaries, and transport boundaries.
2. Add `crates/signing`.
3. Add `crates/signers/keystore`.
4. Add `crates/evm-capabilities`.
5. Add `crates/transports/evm` with source registry, `EvmJsonRpcClient`, source-id routing, and
   expected chain id verification.
6. Move DCV and portfolio EVM RPC use to EVM capability contracts plus the generic live transport.
7. Replace workflow signer config with `SignerRef` and runtime signer registry setup.
8. Rename/split `evm-dcv-model` into contract/ABI model namespaces.
9. Rename/split lifecycle config so reusable phase config contains semantic intent and non-secret
   refs only, while aggregate lifecycle topology/config lives in the operation crate.
10. Rename state crate, schemas, capabilities, state values, and public outputs to contract
    lifecycle language.
11. Extract lifecycle adapter into `crates/adapters/evm-contracts`.
12. Rename operation crate and compile APIs to contract lifecycle language.
13. Replace CLI and REST public surfaces with contract lifecycle routes, commands, request kinds,
    error codes, and framework versions.
14. Rebaseline reth/Nixfied fixtures so local reth is not represented as `ethereum-mainnet`.
15. Rewrite docs, catalog, repo map, docs.rs readiness notes, CLI docs, REST docs, and EVM routing
    docs.
16. Shrink architecture-test allowlists as each old boundary is removed.
17. Remove stale DCV public names once the new boundaries are in place.

### B.11 Audit Completion Definition

The audit-driven correction is complete when all of these are true:

- workspace members no longer include DCV-named platform/domain crates
- public schema namespaces no longer use `mfm.evm.dcv.*`
- public CLI and REST surfaces no longer expose `dcv`
- EVM capability contracts live in `crates/evm-capabilities`
- generic live EVM JSON-RPC exists once and is reused by portfolio and contract lifecycle through
  capability contracts
- runtime source routing uses `source_id` and verifies `expected_chain_id`
- local reth tests do not use `ethereum-mainnet` unless they are actually connected to mainnet
- keystore signing is behind a signer provider and no lifecycle transport opens keystores directly
- typed workflow config contains signer refs and semantic network/chain intent, not
  path/env/url/auth fields or process-local source refs
- full lifecycle aggregate config/topology lives in the operation crate
- operation crates are planning-only
- state crates do not own runtime IO or signer/provider resolution
- transport crates do not depend on operation crates
- signer crates do not depend on workflow crates
- tests enforce the architecture doctrine instead of the current implementation shape
