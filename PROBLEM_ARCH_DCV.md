# Architecture Review: Replace EVM DCV With Smart Contract Lifecycle Boundaries

Review target: commit `a28052d3fa40c36e23490a78258aab603e730740`
(`implement keystore backed evm dcv flow`).

This document is an architecture review, not a line-by-line code review. Breaking changes are
allowed and expected. The current implementation proves several useful runtime behaviors, but it
also establishes the wrong public and crate boundaries around "DCV".

## Architectural Verdict

The commit is partially correct but not acceptable as the final architecture.

The direction is correct where it models deploy, configure, and validate as typed lifecycle states:

- `DeployContractState`
- `ConfigureContractState`
- `ValidateContractState`
- `DeployedContract`
- `ConfiguredContract`
- `ValidationReport`

These are useful blockchain domain concepts and should be kept.

The direction is incorrect where "EVM DCV" becomes the architectural unit. DCV is not blockchain
terminology, and it should not be a crate name, schema namespace, CLI subcommand, REST path,
capability name, adapter name, replay verifier name, or transport name.

The commit is also mis-layered because it places reusable EVM JSON-RPC behavior and keystore-backed
transaction signing inside a DCV-specific transport crate.

## Pieces Worth Keeping

Keep the following ideas and most of their behavior:

- Typed lifecycle states:
  - `DeployContractState`
  - `ConfigureContractState`
  - `ValidateContractState`
- Typestate values:
  - `DeployedContract`
  - `ConfiguredContract`
  - `ConfiguredContractRef`
  - `ValidationReport`
- Side-effect evidence model:
  - intent persisted
  - idempotency input hash
  - claim
  - prepared invocation
  - invocation started
  - submission observed or unknown
  - receipt observed
  - confirmation observed
  - state output
- Replay approach:
  - side-effect replay verifies recorded evidence only
  - validation replay recomputes the read request from certified config and typed input
  - replay does not use live RPC or signer access
- Reth-backed integration coverage.
- Tests proving public output does not contain raw signed transactions, RPC URLs, or local secret
  paths.
- Low-level ABI and transaction helpers in `crates/evm-core`.

## Primary Architectural Problems

### 1. DCV Is The Wrong Abstraction

Current code makes `dcv` pervasive:

- `crates/evm-dcv-model`
- `crates/states/evm-dcv`
- `crates/transports/evm-dcv`
- `mfm.evm.dcv` schema namespaces
- `EvmDcv*` types
- `mfm evm dcv` CLI commands
- `/v1/evm/dcv/*` REST routes
- `mfm-transports-evm-dcv` executable identity
- `verify_evm_dcv_replay`
- `EvmDcvRpcClient`

This is not a domain model. It is an acronym for one recipe: deploy, configure, validate. In the
blockchain domain, these are normal smart-contract lifecycle actions.

The target language should be:

- smart contract deployment
- contract call/configuration
- contract validation
- contract lifecycle workflow
- EVM transaction intent
- EVM JSON-RPC transport
- signer reference
- signing provider

The operation should be thin. It should assemble reusable states. It should not define a new
workflow silo around DCV.

### 2. Keystore Signing Is In The Wrong Place

The current signing path lives in:

- `crates/transports/evm-dcv/src/lib.rs`
  - `sign_legacy_transaction_with_signer`
  - `sign_legacy_transaction_with_signer_blocking`

Those functions:

- parse a DCV-specific signer config
- read env vars
- read a password file
- open `mfm_core::keystore::Keystore`
- unlock it
- load private key material
- derive an EVM address
- sign the EVM transaction hash
- encode a signed legacy EVM transaction

This is not DCV-specific behavior. It is signer and transaction signing infrastructure.

Signing should not be modeled as a lifecycle state. It is execution support for a mutation adapter.
A deploy/configure state should declare non-secret signer intent. A runtime adapter should ask a
generic signing provider to sign a network-specific payload.

The reusable shape should be:

- `crates/signing`
  - generic signer references and signing request/result traits
  - no EVM lifecycle concepts
  - no REST or CLI concepts
- `crates/signers/keystore`
  - MFM keystore-backed implementation of the generic signer provider
  - owns env-var resolution, password-file reading, keystore open/unlock, key lookup, and redacted
    error messages
- `crates/evm-signing` or `crates/transports/evm` module
  - EVM transaction-specific signing adapter
  - turns `LegacyTxToSign` or `Eip1559TxToSign` into a generic signing request
  - verifies the expected EVM address
  - encodes raw signed transaction bytes transiently

This design supports EVM today and Bitcoin or future networks later. Bitcoin-specific sighash
rules should live in a Bitcoin adapter over the same signer provider, not in an EVM lifecycle crate.

### 3. There Should Be No `EvmDcvRpcClient`

Current code has:

- `crates/transports/evm-dcv/src/lib.rs`
  - `struct EvmDcvRpcClient`
  - `impl EvmDcvRpcClient`

It implements generic EVM JSON-RPC behavior:

- `eth_chainId`
- `web3_clientVersion`
- `eth_blockNumber`
- `eth_getTransactionCount`
- `eth_gasPrice`
- `eth_estimateGas`
- `eth_sendRawTransaction`
- `eth_getTransactionReceipt`
- receipt polling
- confirmations
- `eth_call`
- `eth_getLogs`

None of this is DCV-specific.

The target should be a reusable EVM transport:

- `crates/transports/evm`
  - `EvmJsonRpcClient`
  - `EvmRpcSource`
  - `EvmNetworkRef`
  - `EvmReceipt`
  - `EvmLogFilter`
  - `EvmCallRequest`
  - `EvmTransactionRequest`
  - `PreparedEvmTransaction`
  - `EvmRpcError` with redaction-safe messages

The lifecycle runner should depend on a trait, not on a concrete DCV RPC client:

```rust
pub trait EvmRpc: Send + Sync {
    async fn chain_id(&self, source: &EvmSourceRef) -> Result<u64, EvmRpcError>;
    async fn transaction_count(&self, source: &EvmSourceRef, address: &str) -> Result<u64, EvmRpcError>;
    async fn estimate_gas(&self, source: &EvmSourceRef, tx: &EvmCallLike) -> Result<u64, EvmRpcError>;
    async fn send_raw_transaction(&self, source: &EvmSourceRef, raw_tx_hex: &str) -> Result<String, EvmRpcError>;
    async fn transaction_receipt(&self, source: &EvmSourceRef, tx_hash: &str) -> Result<Option<EvmReceipt>, EvmRpcError>;
    async fn eth_call(&self, source: &EvmSourceRef, call: &EvmCallRequest) -> Result<String, EvmRpcError>;
    async fn get_logs(&self, source: &EvmSourceRef, filter: &EvmLogFilter) -> Result<Vec<EvmLog>, EvmRpcError>;
}
```

The concrete client should hide RPC URLs and authorization values from errors, logs, artifacts,
events, CLI output, and REST responses.

### 4. Boundaries Are Blurred

The intended boundaries should be:

- Operation:
  - deterministic planning only
  - assembles state graph
  - no ambient IO
  - no RPC
  - no keystore
  - no signing
- State:
  - executable lifecycle semantics
  - typed input/output
  - builds transaction intent
  - validates config shape
  - evaluates validation semantics through an explicit read backend
- Transport:
  - reusable network/protocol implementation
  - EVM JSON-RPC only
  - no lifecycle-specific naming
- Adapter:
  - converts lifecycle state intent into EVM transport and signer calls
  - records runtime evidence through MFM runtime/store APIs
  - owns side-effect ledger phase progression
- Configuration:
  - typed, non-secret input
  - signer references
  - source/network references
  - contract artifact references or inline build artifacts
  - ABI/config intent
  - no raw secrets

Current violations:

- `crates/transports/evm-dcv` combines lifecycle adapter, EVM JSON-RPC client, signer adapter,
  keystore access, transaction preparation, replay verifier, and artifact loading.
- `DeployConfigureValidateSignerConfig` is lifecycle config but contains keystore runtime
  resolution details.
- `PreparedTransactions` persists signer config in the prepared invocation artifact. It does not
  persist raw signing material, which is good, but it still persists runtime signer resolution
  references. The target should persist a redacted signer reference or signer identity, not
  keystore path env names and password file env names.
- `ValidateContractState::run` uses an unavailable backend, while the real validation read path is
  implemented by the erased transport runner. That can be acceptable in the current runtime binding
  model only if the state crate still owns validation semantics through a callable function. The
  target should make the executable semantic path explicit: state owns semantics, adapter supplies
  capabilities and records evidence.

## Proposed Target Crate Names

Delete or rename:

| Current | Target |
|---|---|
| `crates/evm-dcv-model` | `crates/evm-contract-model` |
| `crates/evm-deploy-configure-validate-config` | `crates/evm-contract-lifecycle-config` |
| `crates/states/evm-dcv` | `crates/states/evm-contracts` |
| `crates/transports/evm-dcv` | split into `crates/transports/evm` and `crates/transports/evm-contracts` |
| `crates/ops/evm-deploy-configure-validate-op` | `crates/ops/evm-contract-lifecycle-op` |
| `mfm.evm.dcv.*` schema namespace | `mfm.evm.contract.*` or `mfm.evm.contract_lifecycle.*` |
| `EvmDcv*` types | `EvmContract*`, `ContractLifecycle*`, or plain `Evm*` where generic |

Add:

- `crates/signing`
- `crates/signers/keystore`
- optionally `crates/evm-signing` if EVM signing should stay separate from `crates/transports/evm`

## Proposed Target Types

### Generic Signing

```rust
pub struct SignerRef {
    pub signer_id: String,
    pub expected_public_identity: Option<String>,
}

pub enum SigningAlgorithm {
    Secp256k1Recoverable,
    Secp256k1Schnorr,
    Ed25519,
}

pub struct SigningRequest {
    pub signer_ref: SignerRef,
    pub algorithm: SigningAlgorithm,
    pub domain: SigningDomain,
    pub prehash: [u8; 32],
}

pub struct SigningResponse {
    pub signer_identity: String,
    pub signature: SignatureBytes,
}

pub trait SigningProvider: Send + Sync {
    fn sign_prehash(&self, request: SigningRequest) -> SigningFuture<'_>;
}
```

The keystore adapter should map `SignerRef` to a local MFM keystore entry through process-local
runtime configuration. That mapping is not part of contract lifecycle config.

### EVM Signing Adapter

```rust
pub trait EvmTransactionSigner: Send + Sync {
    async fn sign_legacy(
        &self,
        signer: &SignerRef,
        expected_from: &str,
        tx: &LegacyTxToSign,
    ) -> Result<SignedEvmTransaction, EvmSigningError>;
}

pub struct SignedEvmTransaction {
    pub transaction_hash: String,
    pub raw_transaction_hex: Zeroizing<String>,
}
```

`raw_transaction_hex` must remain transient. It should not be serialized into normal typed values,
public outputs, manifests, events, durable artifacts, or error messages.

### EVM Transport

```rust
pub struct EvmSourceRef {
    pub source_id: String,
    pub network_id: String,
}

pub struct EvmNetworkCheck {
    pub expected_chain_id: u64,
}

pub trait EvmRpc: Send + Sync {
    async fn chain_id(&self, source: &EvmSourceRef) -> Result<u64, EvmRpcError>;
    async fn client_version(&self, source: &EvmSourceRef) -> Result<String, EvmRpcError>;
    async fn block_number(&self, source: &EvmSourceRef) -> Result<u64, EvmRpcError>;
    async fn transaction_count(&self, source: &EvmSourceRef, from: &str) -> Result<u64, EvmRpcError>;
    async fn estimate_gas(&self, source: &EvmSourceRef, tx: &EvmCallLike) -> Result<u64, EvmRpcError>;
    async fn send_raw_transaction(&self, source: &EvmSourceRef, raw_tx_hex: &str) -> Result<String, EvmRpcError>;
    async fn transaction_receipt(&self, source: &EvmSourceRef, tx_hash: &str) -> Result<Option<EvmReceipt>, EvmRpcError>;
    async fn eth_call(&self, source: &EvmSourceRef, call: &EvmCallRequest) -> Result<String, EvmRpcError>;
    async fn get_logs(&self, source: &EvmSourceRef, filter: &EvmLogFilter) -> Result<Vec<EvmLog>, EvmRpcError>;
}
```

The concrete transport should support source config such as:

```json
[
  {
    "source_id": "reth_local",
    "network_id": "ethereum-dev",
    "rpc_url_env": "MFM_RETH_RPC_URL",
    "authorization_env": "MFM_RETH_AUTHORIZATION"
  }
]
```

Prefer env names over literal RPC URLs and authorization strings in process config. Runtime errors
must not include the resolved URL or authorization header.

### Contract Lifecycle Config

```rust
pub struct ContractDeploymentConfig {
    pub source: EvmSourceRef,
    pub expected_chain_id: u64,
    pub artifact: ContractArtifactRef,
    pub from: String,
    pub signer: SignerRef,
    pub constructor_args: Vec<AbiArgumentValue>,
    pub value_wei: Option<String>,
    pub receipt_policy: ReceiptPolicy,
}

pub struct ContractConfigurationConfig {
    pub source: EvmSourceRef,
    pub expected_chain_id: u64,
    pub from: String,
    pub signer: SignerRef,
    pub calls: Vec<ContractCallConfig>,
    pub confirmations: ContractConfirmationConfig,
    pub receipt_policy: ReceiptPolicy,
}

pub struct ContractValidationConfig {
    pub source: EvmSourceRef,
    pub expected_chain_id: u64,
    pub require_client_substring: Option<String>,
    pub assertions: ContractValidationAssertions,
}
```

`network_id` and `source_id` must be distinct:

- `network_id`: semantic domain label such as `ethereum-mainnet`, `ethereum-sepolia`, or
  `reth-dev`.
- `source_id`: local process routing key for a concrete RPC endpoint.
- `expected_chain_id`: on-chain replay-protection identity returned by `eth_chainId`.

Do not route local reth as `network_id = "ethereum-mainnet"` unless it is actually mainnet.

## Smart Contract Lifecycle States

Target state crate: `crates/states/evm-contracts`.

States:

- `DeployContractState`
  - effect: side effect
  - input: `()`
  - output: `DeployedContract`
  - prepares a contract creation transaction intent
  - validates artifact, constructor args, sender, signer reference, expected chain id
- `ConfigureContractState`
  - effect: side effect
  - input: `DeployedContract`
  - output: `ConfiguredContract`
  - prepares one or more contract call transaction intents
  - requires confirmation reads or event assertions when configuration is intended to be proven
- `ValidateContractState`
  - effect: read external
  - input: `ConfiguredContract`
  - output: `ContractValidationReport`
  - validates configured intent against live reads/events
  - validates additional assertions

The state crate should own:

- transaction intent construction from ABI/artifact/config
- typestate output construction from confirmation evidence
- validation report construction from read backend responses
- schema definitions for lifecycle values

It should not own:

- HTTP client construction
- RPC source env parsing
- authorization headers
- keystore opening
- password file reading
- raw transaction submission

## Thin Operation Shape

Target op crate: `crates/ops/evm-contract-lifecycle-op`.

Standalone operations:

- `DeployContractOperation`
- `ConfigureContractOperation`
- `ValidateContractOperation`

Composed operation:

- `ContractLifecycleOperation`

The composed operation should do only deterministic planning:

```rust
deploy = builder.state::<DeployContractState>("deploy_contract", config.deploy, ())
configured = builder.state::<ConfigureContractState>("configure_contract", config.configure, deploy)
report = builder.state::<ValidateContractState>("validate_contract", config.validate, configured)
```

Any authored-config normalization should happen before certification, or in a pure config build
step. If the operation must fill shared artifact refs into phase configs, keep that as pure,
deterministic config lowering and make it explicit in the config crate.

## Public API Proposal

### CLI

Replace:

```text
mfm evm dcv deploy
mfm evm dcv configure
mfm evm dcv validate
mfm evm dcv deploy-configure-validate
```

With:

```text
mfm evm contracts deploy --request-file <REQUEST_FILE>
mfm evm contracts configure --request-file <REQUEST_FILE> --deployed-contract-file <DEPLOYED_JSON>
mfm evm contracts validate --request-file <REQUEST_FILE> --configured-contract-file <CONFIGURED_JSON>
mfm evm contracts lifecycle --request-file <REQUEST_FILE>
```

CLI error codes should use contract lifecycle language:

- `InvalidEvmContractRequest`
- `EvmContractCompileInvalid`
- `SignerUnavailable`
- `SignerAddressMismatch`
- `EvmRpcSourceUnavailable`
- `EvmChainIdMismatch`

Avoid `EvmDcv*`.

### REST

Replace:

```text
POST /v1/evm/dcv/deploy
POST /v1/evm/dcv/configure
POST /v1/evm/dcv/validate
POST /v1/evm/dcv/deploy-configure-validate
```

With:

```text
POST /v1/evm/contracts/deploy
POST /v1/evm/contracts/configure
POST /v1/evm/contracts/validate
POST /v1/evm/contracts/lifecycle
```

Request kind values:

```json
{ "kind": "evm_contract_deploy_start_v1" }
{ "kind": "evm_contract_configure_start_v1" }
{ "kind": "evm_contract_validate_start_v1" }
{ "kind": "evm_contract_lifecycle_start_v1" }
```

The response shape can remain:

```json
{
  "run": {},
  "public_schema_id": "...",
  "public_output": null
}
```

Do not include RPC URLs, authorization details, keystore paths, password-file paths, raw
transactions, or signatures.

### Config Schema

Composed lifecycle request:

```json
{
  "machine_id": "evm_contract_lifecycle",
  "pipeline_version": "v1",
  "input": {},
  "network": {
    "network_id": "reth-dev",
    "source_id": "reth_local",
    "expected_chain_id": 31337
  },
  "contract": {
    "artifact": {
      "abi": [],
      "bytecode": "0x..."
    }
  },
  "deploy": {
    "from": "0x...",
    "signer": {
      "signer_id": "deployer",
      "expected_address": "0x..."
    },
    "constructor_args": [],
    "value_wei": "0"
  },
  "configure": {
    "from": "0x...",
    "signer": {
      "signer_id": "deployer",
      "expected_address": "0x..."
    },
    "calls": [],
    "confirmation_read_assertions": [],
    "confirmation_event_assertions": []
  },
  "validate": {
    "read_assertions": [],
    "event_assertions": []
  }
}
```

The signer reference is non-secret. The mapping from `signer_id` to keystore path, password-file
path, hardware signer, remote signer, or future Bitcoin wallet is process runtime configuration,
not workflow config.

## Migration Plan Split Into Reviewable Commits

### Commit 1: Add Generic Signing Boundary

Add:

- `crates/signing`
- signer reference and signing request/result traits
- redaction-safe signing error type
- tests for redaction and address mismatch surfaces

No EVM lifecycle behavior changes yet.

Docs:

- update `docs/architecture.md` with signing boundary

Tests:

- unit tests in `crates/signing`

### Commit 2: Add MFM Keystore Signer Adapter

Add:

- `crates/signers/keystore`
- runtime mapping from signer id to keystore entry
- env-var based path/password resolution
- password trimming and zeroization
- keystore unlock/load/sign tests

Move logic out of `crates/transports/evm-dcv/src/lib.rs`:

- `env_value`
- password-file read
- keystore open/unlock
- key id parse
- expected-address verification support

Tests:

- wrong password
- missing signer id
- wrong expected address
- error does not echo env values, paths, password, private key, mnemonic

### Commit 3: Add Generic EVM Transport

Add:

- `crates/transports/evm`
- `EvmJsonRpcClient`
- `EvmRpc` trait
- RPC source parsing
- redaction-safe RPC errors

Move generic logic out of `EvmDcvRpcClient`:

- `chain_id`
- `client_version`
- `block_number`
- `transaction_count`
- `gas_price` or fee policy
- `estimate_gas`
- `send_raw_transaction`
- `transaction_receipt`
- `wait_receipt`
- `eth_call`
- `get_logs`

Tests:

- unit tests for response parsing
- reth smoke test for `eth_chainId`, transaction count, receipt polling where available
- errors do not include RPC URL or authorization header

### Commit 4: Rename Model And Config Crates

Rename:

- `crates/evm-dcv-model` -> `crates/evm-contract-model`
- `crates/evm-deploy-configure-validate-config` -> `crates/evm-contract-lifecycle-config`

Rename schema namespace:

- from `mfm.evm.dcv.*`
- to `mfm.evm.contract.*` or `mfm.evm.contract_lifecycle.*`

Breaking change: no compatibility aliases.

Tests:

- schema IDs compile
- canonical JSON artifact ids remain deterministic under the new schema
- authored JSON/TOML decode into the new config shape

Docs:

- update crate catalog docs
- update `docs/ops-and-states.md`

### Commit 5: Rename State Crate And Types

Rename:

- `crates/states/evm-dcv` -> `crates/states/evm-contracts`
- `EvmDcvTransactionIntent` -> `EvmTransactionIntent` or `ContractTransactionIntent`
- `EvmDcvDeployIntent` -> `ContractDeployIntent`
- `EvmDcvConfigureIntent` -> `ContractConfigureIntent`
- `EvmDcvValidateReadRequest` -> `ContractValidationReadRequest`
- `EvmDcvValidateReadResponse` -> `ContractValidationReadResponse`
- `DcvPublicOutputs` -> `ContractLifecyclePublicOutputs`

Keep:

- `DeployContractState`
- `ConfigureContractState`
- `ValidateContractState`

Tests:

- state registry descriptors
- operation lowering to deploy/configure/validate nodes
- validation rejects configured-contract scope mismatch before reads
- validation confirms configured intent against live reads

### Commit 6: Split Lifecycle Adapter From EVM Transport

Rename/split:

- `crates/transports/evm-dcv` -> `crates/transports/evm-contracts`
- lifecycle adapter depends on:
  - `crates/transports/evm`
  - `crates/signing`
  - `crates/signers/keystore`

Remove:

- `EvmDcvRpcClient`
- direct `Keystore` usage from lifecycle adapter
- DCV-specific signer config

Keep:

- side-effect phase progression
- prepared invocation hash check
- submission unknown evidence
- receipt and confirmation staging
- replay verifier shape

Tests:

- prepared invocation excludes raw transaction and signature material
- prepared invocation excludes keystore paths and password-file paths
- regenerated transaction hash must match prepared hash before submit
- replay verifier rejects mixed deploy/configure evidence

### Commit 7: Rename Operation Crate And Compile API

Rename:

- `crates/ops/evm-deploy-configure-validate-op`
- to `crates/ops/evm-contract-lifecycle-op`

Rename functions:

- `dcv_program_draft` -> `contract_lifecycle_program_draft`
- `dcv_deploy_program_draft` -> `contract_deploy_program_draft`
- `dcv_configure_program_draft` -> `contract_configure_program_draft`
- `dcv_validate_program_draft` -> `contract_validate_program_draft`
- `compile_dcv_program` -> `compile_contract_lifecycle_program`
- `compile_dcv_deploy_program` -> `compile_contract_deploy_program`
- `compile_dcv_configure_program` -> `compile_contract_configure_program`
- `compile_dcv_validate_program` -> `compile_contract_validate_program`
- `register_dcv_certification_descriptors` -> `register_evm_contract_certification_descriptors`

Tests:

- operation lowering stays three nodes for lifecycle
- standalone phase programs compile with seed boundaries

### Commit 8: Replace CLI And REST Surfaces

CLI:

- delete `bin/cli/src/commands/evm/dcv.rs`
- add `bin/cli/src/commands/evm/contracts.rs`
- replace `EvmCommand::Dcv` with `EvmCommand::Contracts`

REST:

- replace `/v1/evm/dcv/*` routes with `/v1/evm/contracts/*`
- replace `EvmDcv*StartKind` with `EvmContract*StartKind`
- replace error codes using `Dcv`

Docs:

- update `bin/cli/README.md`
- update `bin/rest-api/README.md`

Tests:

- CLI JSON output shape
- REST route happy paths
- old routes are gone

### Commit 9: Rebaseline Reth Integration

Update:

- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
- helper names in `tests/integration/src/test_support.rs`
- env var names if needed

Fix network language:

- do not use `ethereum-mainnet` for local reth
- use `network_id = "reth-dev"` or similar
- assert `expected_chain_id` matches `eth_chainId`

Tests:

- deploy/configure/validate phase REST flow
- composed lifecycle operation
- restart boundaries
- replay rejects tampered fact/output/configured-contract artifacts
- public output does not leak secret paths, RPC URLs, raw tx, signatures

### Commit 10: Documentation And Inventory Cleanup

Update:

- `docs/architecture.md`
- `docs/ops-and-states.md`
- `docs/evm-rpc-routing.md`
- `docs/repo-map.md`
- `crates/docs/README.md`
- `crates/docs/catalog.toml`
- crate READMEs

Remove all public references to `dcv` unless documenting historical migration.

Run:

```bash
nix run .#check
nix run .#test
nix run .#ci -- --mode parity --summary
nix run .#ci -- --mode full --summary
```

## Risks And Invariants

### Secret Handling

Must never persist, log, or render:

- private keys
- mnemonics
- passwords
- password-file contents
- keystore file paths
- password-file paths
- RPC authorization headers
- raw signed transaction bytes
- signature scalars
- signer backend internals

Allowed durable references:

- signer id
- expected public address
- transaction hash
- content-addressed artifact ids
- receipt and confirmation evidence

Prepared invocation artifacts may include unsigned transaction fields and transaction hashes, but
they must not include raw signed transactions or enough signer backend details to locate local
secret files.

### Replay And Determinism

Replay must use recorded evidence only:

- no live RPC
- no signer access
- no keystore access
- no environment variable reads
- no filesystem reads except typed artifact store reads by recorded artifact id/evidence

Operation expansion must remain deterministic and pure.

State intent construction must not depend on ambient IO.

### Artifact, Event, And Output Stability

The rename is intentionally breaking. After the migration, schemas should stabilize under the new
contract lifecycle names.

Events and artifacts should retain the same evidence pattern:

- intent artifact
- idempotency hash
- prepared invocation artifact
- submission artifact
- receipt artifact
- confirmation artifact
- fact response artifact
- state output artifact
- public output artifact

The stable public language should be contract lifecycle, not DCV.

### Transport Reuse

The EVM transport must be usable by future EVM states that have nothing to do with contract
lifecycle, for example:

- ERC-20 transfers
- allowance approvals
- balance reads
- event indexing
- NFT minting
- governance calls
- generic contract calls

If a future state imports `mfm-transports-evm-contracts` only to get JSON-RPC, the split failed.

### Test Realism Against Reth

The reth tests are valuable and should be preserved.

They should assert:

- local reth source is routed by `source_id`
- expected chain id is checked before signing
- deploy and configure do not duplicate mutations across restart boundaries
- validation proves configured intent against live reads/events
- replay detects tampered fact response artifacts
- replay detects tampered state output artifacts
- replay detects tampered configured-contract input artifacts
- public output remains redaction-safe

## Concrete Current References

Files and symbols that should be renamed, split, or deleted:

- `bin/cli/src/commands/evm/mod.rs`
  - `EvmCommand::Dcv`
- `bin/cli/src/commands/evm/dcv.rs`
  - delete or replace with `contracts.rs`
- `bin/rest-api/src/lib.rs`
  - `/v1/evm/dcv/*`
  - `EvmDcvDeployStartKind`
  - `EvmDcvConfigureStartKind`
  - `EvmDcvValidateStartKind`
  - `EvmDcvFullStartKind`
  - `launch_compiled_evm_dcv`
- `crates/ops/evm-deploy-configure-validate-op/src/lib.rs`
  - `dcv_program_draft`
  - `compile_dcv_program`
  - `register_dcv_certification_descriptors`
  - operation names using `mfm.evm.dcv`
- `crates/states/evm-dcv/src/lib.rs`
  - `NAMESPACE = "mfm.evm.dcv"`
  - `EvmDcvReadCapability`
  - `EvmDcvSignerCapability`
  - `EvmDcvTransactionSubmitCapability`
  - `EvmDcvReadBackend`
  - `EvmDcvTransactionIntent`
  - all `Dcv*PublicOutputs`
- `crates/transports/evm-dcv/src/lib.rs`
  - `EvmDcvArtifactReader`
  - `register_evm_dcv_runners`
  - `EvmDcvRpcClient`
  - `sign_legacy_transaction_with_signer`
  - `sign_legacy_transaction_with_signer_blocking`
  - `verify_evm_dcv_replay`
- `crates/evm-deploy-configure-validate-config/src/lib.rs`
  - `DeployConfigureValidateSignerConfig`
  - `DeployConfigureValidate*Config`
  - schema namespace `mfm.evm.dcv`
- `crates/evm-dcv-model/src/lib.rs`
  - schema namespace `mfm.evm.dcv`
- `crates/app/src/lib.rs`
  - `register_evm_dcv_runners`
  - `register_dcv_certification_descriptors`
  - `verify_evm_dcv_replay`
- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
  - `NETWORK_ID = "ethereum-mainnet"` for local reth
  - `/v1/evm/dcv/*`
  - `evm_dcv_*` request kinds

## Final Recommendation

Do not iterate on the current DCV surface. Use the working implementation as a behavioral
prototype, then migrate it into the target boundaries above.

The intended end state is:

- generic signing and keystore adapter
- generic EVM JSON-RPC transport
- reusable EVM contract lifecycle states
- thin lifecycle operations
- CLI/REST using contract lifecycle language
- replay and side-effect evidence model preserved
- no DCV public API

