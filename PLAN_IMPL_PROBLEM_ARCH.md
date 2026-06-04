# MFM Architecture Correction Implementation Plan

Status: implementation plan.

This plan implements the architecture correction described in
`PROBLEM_ARCH_DCV.md`, `docs/architecture.md`, and `docs/design.md`.

The objective is to refactor MFM so workflow recipes no longer become
crate/module/API/schema/transport/signer/config boundaries. DCV is the
motivating failure case, but the correction is taxonomy-wide.

Breaking changes are allowed. No backward compatibility is required. Do not add
alias crates, alias modules, alias routes, alias CLI commands, deprecated public
types, or compatibility schema ids for the old DCV surface.

## Resolved Planning Decisions

- Adapter identity: use only `crates/adapter-contracts` for now. Do not add a
  separate EVM contract adapter-contract crate unless a later domain split
  proves it is necessary.
- Runtime configuration: define the new runtime source/signer config in this
  refactor. Typed workflow config carries semantic intent and non-secret refs
  only.
- EVM transaction signing: support both legacy and EIP-1559 transactions.
  EIP-1559 is the default. Legacy remains supported because some chains still
  require it.
- Artifact references: use typed evidence refs instead of plain artifact-id
  strings where lifecycle values need durable artifact evidence.
- Keystore/private keys: prefer provider APIs that accept signing requests and
  return signatures plus public metadata. Do not expose decrypted private keys
  through provider APIs.
- Taxonomy for ambiguous crates: classify `crates/core`,
  `crates/collectors/proof`, `crates/collectors/btc-jsonrpc-http`, and any
  other ambiguous crate using the same category rules defined in
  `PROBLEM_ARCH_DCV.md`.

## Executive Summary

Implementation proceeds in four stages:

1. Add taxonomy metadata and enforcement with exact shrinking allowlists.
2. Extract platform primitives: artifact read, signing, EVM signing, keystore
   provider, EVM capabilities, EVM transport, and adapter contracts.
3. Replace the DCV lifecycle stack with contract lifecycle domain/model/config,
   state, adapter, and operation crates.
4. Replace public CLI/REST surfaces and rebaseline tests, docs, Nixfied, and
   generated metadata.

The old DCV implementation should be used only as a temporary behavioral source
during extraction. The final architecture must delete the old public names and
crate boundaries.

## Target Crate Map

| Crate | Category | Responsibility |
|---|---|---|
| `crates/artifact-capabilities` | `capability-contract` | `mfm.artifact.read`, verified artifact read requests, typed evidence refs, redacted artifact errors |
| `crates/signing` | `signer-contract` | `SignerRef`, signing algorithms/domains/purposes, request/result contracts, `mfm.signing.sign`, redacted errors |
| `crates/evm-signing` | `signer-contract` | EVM signing bridge, legacy and EIP-1559 transaction signing requests, address recovery, transient raw-tx encoding |
| `crates/signers/keystore` | `signer-provider` | Runtime keystore signer provider, env/path/password-file resolution, password zeroization, redacted provider errors |
| `crates/evm-capabilities` | `capability-contract` | EVM chain identity, block, call, logs, nonce, fee, gas estimate, submit, receipt capabilities, opaque source refs, redacted source evidence |
| `crates/transports/evm` | `transport` | Live EVM JSON-RPC, endpoint/auth runtime config, source registry, source-id routing, source policy, expected-chain verification |
| `crates/adapter-contracts` | `adapter-contract` | Stable adapter kind/version ids and binding descriptors |
| `crates/evm-contract-model` | `domain-model` | ABI/bytecode wrappers, contract artifact model, deployed/configured/validated typestate values, validation reports |
| `crates/evm-contract-config` | `domain-config` | Reusable deploy/configure/validate phase config with semantic network intent, expected chain id, signer refs, receipt policy |
| `crates/states/evm-contracts` | `state` | Contract deployment, configuration, and validation state contracts and deterministic state behavior |
| `crates/adapters/evm-contracts` | `adapter` | Lifecycle runner binding, side-effect phases, transaction evidence, replay verifier |
| `crates/ops/evm-contract-lifecycle-op` | `operation` | Planning-only deploy/configure/validate/lifecycle topology |
| `crates/adapters/portfolio` | `adapter` | Portfolio runner binding, generic artifact/EVM capability use |

Delete or replace:

- `crates/evm-dcv-model`
- `crates/evm-deploy-configure-validate-config`
- `crates/states/evm-dcv`
- `crates/transports/evm-dcv`
- `crates/ops/evm-deploy-configure-validate-op`
- `crates/transports/portfolio` as a transport boundary, after portfolio runner
  behavior moves to `crates/adapters/portfolio`

## Runtime Configuration Shape

Typed workflow config must not contain runtime endpoint routing or signer
provider resolution. Runtime process config owns those mappings.

Recommended process-local runtime config:

```json
{
  "evm_sources": [
    {
      "source_id": "reth_local",
      "network_id": "reth-dev",
      "expected_chain_id": 31337,
      "rpc_url_env": "MFM_RETH_RPC_URL",
      "authorization_env": "MFM_RETH_AUTHORIZATION",
      "policy_id": "local-first",
      "kind": "local"
    }
  ],
  "evm_source_policies": [
    {
      "policy_id": "local-first",
      "mode": "ordered_fallback",
      "source_ids": ["reth_local"]
    }
  ],
  "signers": [
    {
      "signer_ref": "deployer",
      "provider": "mfm_keystore",
      "entry_id": "550e8400-e29b-41d4-a716-446655440000",
      "keystore_path_env": "MFM_DEPLOYER_KEYSTORE",
      "password_file_env": "MFM_DEPLOYER_KEYSTORE_PASSWORD_FILE"
    }
  ],
  "defaults": {
    "evm_source_policy_id": "local-first",
    "evm_transaction_style": "eip1559"
  }
}
```

Rules:

- `rpc_url_env`, `authorization_env`, `keystore_path_env`, and
  `password_file_env` are runtime-only and must not be typed workflow config,
  events, artifacts, public outputs, schemas, or replay fixtures.
- EVM transport owns endpoint/auth-bearing source structs.
- `crates/evm-capabilities` owns only opaque source refs, source policy ids,
  redacted source evidence, and capability request/response contracts.
- `source_id` is process-local runtime routing unless selecting a source is
  explicitly domain intent.
- Local reth must use a local/dev network id, not `ethereum-mainnet`, unless it
  is actually mainnet.

Typed lifecycle config should look more like:

```json
{
  "network": {
    "network_id": "reth-dev",
    "expected_chain_id": 31337
  },
  "signer": {
    "signer_ref": "deployer",
    "expected_evm_address": "0x000000000000000000000000000000000000dead"
  },
  "transaction_policy": {
    "style": "eip1559"
  }
}
```

## Public API Replacement

Delete old public surfaces. Do not add aliases.

CLI target:

- `mfm evm contracts deploy`
- `mfm evm contracts configure`
- `mfm evm contracts validate`
- `mfm evm contracts lifecycle`

REST target:

- `POST /v1/evm/contracts/deploy`
- `POST /v1/evm/contracts/configure`
- `POST /v1/evm/contracts/validate`
- `POST /v1/evm/contracts/lifecycle`

Request kinds:

- `evm_contract_deploy_start_v1`
- `evm_contract_configure_start_v1`
- `evm_contract_validate_start_v1`
- `evm_contract_lifecycle_start_v1`

Response envelope may keep:

- `run`
- `public_schema_id`
- `public_output`

## Explicit DCV Deletion Plan

The following must disappear from active code, docs, tests, generated metadata,
schema ids, CLI, REST, app wiring, runner ids, and package names, except in
historical migration notes:

- `dcv`
- `Dcv*`
- `EvmDcv*`
- `DeployConfigureValidate*` below operation topology
- `deploy_configure_validate` as a durable schema, route, framework, runner, or
  public API identity
- `crates/evm-dcv-model`
- `crates/evm-deploy-configure-validate-config`
- `crates/states/evm-dcv`
- `crates/transports/evm-dcv`
- `crates/ops/evm-deploy-configure-validate-op`
- `mfm.evm.dcv.*`
- `mfm evm dcv`
- `/v1/evm/dcv/*`
- `evm_dcv_*`
- `mfm.cli.evm_dcv.typed.v1`
- `mfm.rest_api.evm_dcv.typed.v1`
- `mfm-transports-evm-dcv`
- `typed-evm-dcv`
- `EvmDcvRpcClient`
- `EvmDcvArtifactReader`
- `EvmDcvReplayVerifier`
- `verify_evm_dcv_replay`
- `DeployConfigureValidateSignerConfig`

## Parallel Audit Findings

The planning audit used independent read-only slices of the repository:

- DCV/EVM lifecycle crates: DCV is currently encoded as workspace architecture
  through model, config, state, transport, operation, app, tests, docs, and
  Nixfied.
- Transports/capabilities: `mfm-transports-evm-dcv` is overloaded as adapter,
  live EVM transport, signer provider, artifact reader, and replay verifier.
  Portfolio duplicates EVM RPC/source parsing.
- Signer/keystore: typed config carries keystore env names, workflow transport
  opens keystores, and CLI duplicates EVM signing logic.
- Artifact access: DCV and portfolio define workflow-local artifact reader
  traits over `FsTypedArtifactStore`; artifact evidence checks are duplicated
  and incomplete.
- App/CLI/REST: public surfaces expose `mfm evm dcv`, `/v1/evm/dcv/*`,
  `evm_dcv_*`, and DCV framework strings.
- Tests/enforcement: no workspace crate has `package.metadata.mfm.category`;
  metadata tests are kernel-only; an app boundary test currently requires the
  wrong DCV transport.
- Docs/Nixfied/generated metadata: docs catalog, repo map/index, docs.rs
  readiness, EVM routing docs, CLI/REST docs, and Nixfied publish/reth examples
  still publish stale DCV architecture.

## Commit-By-Commit Implementation Plan

### 1. `add mfm crate categories and boundary allowlists`

Purpose:

- Add `package.metadata.mfm.category` to every workspace crate.
- Extend cargo-metadata tests with category parsing and exact allowlisted
  existing violations.

Likely touched:

- `Cargo.toml`
- all workspace `Cargo.toml` manifests
- `tests/integration/tests/cargo_metadata_contract.rs`

Code to delete:

- Kernel-only assumptions as the sole architecture enforcement mechanism.

Code to add or move:

- Closed category enum.
- Path/category consistency checks.
- Category dependency matrix.
- Exact temporary dependency allowlist.
- Synthetic metadata fixtures that inject forbidden edges.

Tests:

- `all_workspace_crates_have_mfm_category`
- `workspace_categories_are_known`
- `category_dependency_rules_reject_forbidden_edges`

Verification:

```bash
cargo test -p mfm-integration-tests --test cargo_metadata_contract
```

Dependencies:

- None.

### 2. `add namespace and typed surface guardrails`

Purpose:

- Add guardrails that reject stale public names, schema namespaces, capability
  names, and forbidden typed-config fields with exact temporary allowlists.

Likely touched:

- `crates/app/tests/typed_transport_boundaries.rs`
- `tests/integration/tests/cargo_metadata_contract.rs`
- possible new integration test file for namespace/schema checks

Code to delete:

- Test requirement that production app must link `mfm-transports-evm-dcv`.

Code to add or move:

- Path-specific namespace allowlist for current violations.
- Checks for `dcv`, `Dcv`, `EvmDcv`, `evm_dcv`, `evm-dcv`,
  `mfm.evm.dcv`, `/v1/evm/dcv`, and `typed-evm-dcv`.
- Checks for forbidden typed-config fields such as `rpc_url`,
  `authorization`, `keystore_path_env`, `password_file_env`,
  `password`, `private_key`, `mnemonic`, `raw_transaction`.

Tests:

- Namespace checks reject synthetic stale names.
- Typed config field scan rejects synthetic forbidden fields.

Verification:

```bash
nix run .#check
```

Dependencies:

- Commit 1.

### 3. `add artifact read capability contract`

Purpose:

- Add a neutral artifact-read capability contract so adapters stop depending on
  concrete artifact-store implementations.

Likely touched:

- `crates/artifact-capabilities/**`
- `crates/storages/artifact-store-fs/**`
- root `Cargo.toml`
- docs catalog later, not in this commit unless required for compile

Code to delete:

- None yet; old workflow-local readers remain temporarily allowlisted.

Code to add or move:

- `ArtifactReadCapability`
- `ArtifactReadRequest`
- `VerifiedArtifactBytes`
- typed evidence refs with digest, role, media type, schema id, semantic type
  id, producer node/seed id, byte length
- redacted `ArtifactReadError`
- constructors from certified config refs, materialized cells, side-effect
  projections, and replay-authorized evidence
- decode helpers layered on verified bytes
- FS implementation in `mfm-artifact-store-fs`

Tests:

- Digest mismatch.
- Role mismatch.
- Schema/semantic mismatch.
- Producer mismatch.
- Byte length mismatch.
- Redaction-safe errors.

Verification:

```bash
cargo test -p mfm-artifact-capabilities -p mfm-artifact-store-fs
```

Dependencies:

- Commit 1.

### 4. `add generic signing contract`

Purpose:

- Add signer contract boundary independent of EVM and lifecycle workflows.

Likely touched:

- `crates/signing/**`
- root `Cargo.toml`

Code to delete:

- None yet.

Code to add or move:

- `SignerRef`
- signing algorithm/domain/purpose identifiers
- `SigningRequest`
- `SigningResult`
- `SigningCapability`
- expected public identity contracts
- redaction-safe signing errors
- private constructors for transient/secret-bearing request internals

Tests:

- `SignerRef` validation.
- Redaction tests.
- Compile-fail tests showing provider runtime config cannot derive
  `MfmConfig`/`MfmValue`.

Verification:

```bash
cargo test -p mfm-signing
```

Dependencies:

- Commit 1.

### 5. `add evm signing bridge`

Purpose:

- Add reusable EVM signing domain logic outside lifecycle adapters and CLI.

Likely touched:

- `crates/evm-signing/**`
- `crates/evm-core/**` only if generic tx helpers need minor relocation
- root `Cargo.toml`

Code to delete:

- None yet; old signing paths remain until consumers migrate.

Code to add or move:

- EVM signing domain and purpose ids.
- Legacy and EIP-1559 transaction signing request construction.
- EIP-155/EIP-1559 sighash handling.
- Signature normalization.
- Recovered-address verification.
- Transient raw transaction encoding.
- Transaction hash calculation.

Tests:

- Legacy tx hash stability.
- EIP-1559 tx hash stability.
- Recovered address matches expected signer address.
- Raw signed transaction type cannot be persisted as a typed value/artifact.

Verification:

```bash
cargo test -p mfm-evm-signing
```

Dependencies:

- Commit 4.

### 6. `add keystore signer provider`

Purpose:

- Add MFM keystore-backed signer provider without exposing decrypted private
  keys to workflow code.

Likely touched:

- `crates/signers/keystore/**`
- `crates/core/src/keystore/**` only for small provider-support APIs if needed
- root `Cargo.toml`

Code to delete:

- None yet.

Code to add or move:

- Runtime signer registry entries mapping `SignerRef` to keystore entry ids.
- Env/path/password-file resolution.
- Password trimming and zeroization.
- Provider API accepting signing requests and returning signatures plus public
  metadata.
- Redaction-safe provider errors.

Tests:

- Provider signs through `SigningRequest` without returning private key.
- Password file trimming and zeroization behavior.
- Error redaction does not leak path/password/key material.
- Tamper/security-sensitive keystore coverage remains intact.

Verification:

```bash
cargo test -p mfm-signers-keystore
```

Dependencies:

- Commit 4.

### 7. `add evm capability contracts`

Purpose:

- Move EVM authority contracts out of state/transport crates.

Likely touched:

- `crates/evm-capabilities/**`
- root `Cargo.toml`

Code to delete:

- None yet.

Code to add or move:

- `EvmSourceRef`
- `EvmSourcePolicyId`
- `RedactedEvmSourceEvidence`
- `EvmChainIdentityCapability`
- `EvmBlockReadCapability`
- `EvmCallReadCapability`
- `EvmLogsReadCapability`
- `EvmNonceReadCapability`
- `EvmFeeReadCapability`
- `EvmGasEstimateCapability`
- `EvmTransactionSubmitCapability`
- `EvmReceiptReadCapability`
- redacted request/response/error contracts

Tests:

- Capability names are authority names, not workflow names.
- Fee-market and gas-estimate capabilities are separate.
- Endpoint/auth-bearing structs are absent from capability contracts.

Verification:

```bash
cargo test -p mfm-evm-capabilities
```

Dependencies:

- Commit 1.

### 8. `add generic evm json-rpc transport`

Purpose:

- Add reusable EVM JSON-RPC transport and runtime source registry.

Likely touched:

- `crates/transports/evm/**`
- root `Cargo.toml`
- maybe Nixfied runtime env examples in a later commit, not here unless needed

Code to delete:

- None yet.

Code to add or move:

- `EvmJsonRpcClient`
- endpoint/auth-bearing runtime source structs
- source registry parser
- source-id routing
- ordered fallback policy
- expected-chain verification before use
- endpoint/auth redaction
- live implementations of EVM capability traits

Tests:

- Selects source by source policy/source id.
- Records redacted source evidence.
- Rejects chain id mismatch.
- Does not expose URL/auth in evidence or errors.
- Supports chain, block, call, logs, nonce, fee, gas estimate, submit, receipt
  calls.

Verification:

```bash
cargo test -p mfm-transports-evm
```

Dependencies:

- Commit 7.

### 9. `add adapter contract ids`

Purpose:

- Move stable adapter kind/version ids to a neutral contract crate.

Likely touched:

- `crates/adapter-contracts/**`
- root `Cargo.toml`

Code to delete:

- None yet; state-owned adapter ids removed when state crate migrates.

Code to add or move:

- stable lifecycle adapter kind/version constructors
- adapter binding descriptors
- no-live-IO validation helpers

Tests:

- Adapter ids are available without depending on adapter implementation crates.
- Category test allows states to depend on adapter contracts, not adapters.

Verification:

```bash
cargo test -p mfm-adapter-contracts
```

Dependencies:

- Commit 1.

### 10. `replace dcv model with evm contract model`

Purpose:

- Replace recipe-named model crate with durable EVM contract model crate.

Likely touched:

- `crates/evm-contract-model/**`
- `crates/evm-dcv-model/**` removed after consumers migrate
- root `Cargo.toml`

Code to delete:

- `mfm.evm.dcv.*` model schemas.
- `Dcv` model names.
- old `crates/evm-dcv-model` once no consumers remain.

Code to add or move:

- ABI JSON wrappers.
- Bytecode JSON wrappers.
- ABI argument wrappers.
- contract artifact config.
- deploy/configure validation assertion types.
- `DeployedContract`
- `ConfiguredContract`
- typed evidence refs for lifecycle artifact evidence.
- `ValidationReport`
- namespaces such as `mfm.evm.contract.*` and `mfm.evm.abi.*`.

Tests:

- Schema golden tests use contract/ABI namespaces only.
- Validation report and typestate behavior preserved.

Verification:

```bash
cargo test -p mfm-evm-contract-model
```

Dependencies:

- Commit 2.

### 11. `replace dcv phase config with evm contract config`

Purpose:

- Replace workflow config crate with reusable phase config crate.

Likely touched:

- `crates/evm-contract-config/**`
- root `Cargo.toml`

Code to delete:

- `DeployConfigureValidateSignerConfig`
- typed `keystore_path_env`
- typed `password_file_env`
- aggregate lifecycle config from lower config crate
- default machine id `evm_deploy_configure_validate`

Code to add or move:

- deploy phase config
- configure phase config
- validate phase config
- `SignerRef` plus expected EVM address
- semantic network intent with `network_id` and `expected_chain_id`
- EIP-1559 default transaction policy with legacy support
- receipt/retry/validation policy

Tests:

- Config denies provider/runtime fields.
- Deploy/configure/validate all require expected chain id.
- EIP-1559 default materializes correctly.
- Legacy transaction style remains accepted.

Verification:

```bash
cargo test -p mfm-evm-contract-config
```

Dependencies:

- Commits 4 and 10.

### 12. `replace evm dcv states with evm contract states`

Purpose:

- Replace recipe-named state crate with reusable contract lifecycle states.

Likely touched:

- `crates/states/evm-contracts/**`
- old `crates/states/evm-dcv/**`
- root `Cargo.toml`

Code to delete:

- `EvmDcvReadCapability`
- `EvmDcvSignerCapability`
- `EvmDcvTransactionSubmitCapability`
- `evm_dcv_adapter_kind`
- `evm_dcv_adapter_version`
- `DcvPublicOutputs`
- `DcvOperationOutputs`
- old state schemas/names under `mfm.evm.dcv`

Code to add or move:

- `DeployContractState`
- `ConfigureContractState`
- `ValidateContractState`
- `ContractTransactionIntent`
- `ContractDeployIntent`
- `ContractConfigureIntent`
- `ContractValidationReadRequest`
- `ContractValidationReadResponse`
- public outputs under contract lifecycle schema names
- adapter bindings from `crates/adapter-contracts`
- capability sets from `crates/evm-capabilities` and `crates/signing`

Tests:

- Raw transaction secrecy trybuild rehomed under contract/signing names.
- States do not depend on transports, signer providers, app, binaries,
  storage implementations, or operations.
- State schemas contain no DCV names.

Verification:

```bash
cargo test -p mfm-state-evm-contracts
```

Dependencies:

- Commits 7, 9, and 11.

### 13. `extract evm contract lifecycle adapter`

Purpose:

- Move lifecycle runners and replay verifier out of workflow-named transport.

Likely touched:

- `crates/adapters/evm-contracts/**`
- old `crates/transports/evm-dcv/**`
- `crates/app/**` later, not necessarily in this commit

Code to delete:

- `EvmDcvArtifactReader`
- `EvmDcvRpcClient`
- keystore opening/signing from lifecycle transport
- JSON-RPC source parsing from lifecycle transport
- `verify_evm_dcv_replay`
- `mfm-transports-evm-dcv` executable identity

Code to add or move:

- lifecycle runner registration
- side-effect phase progression
- prepared invocation evidence using typed evidence refs
- chain identity verification before mutation
- EIP-1559 default signing/submission with legacy fallback/support
- transaction hash consistency checks
- receipt and confirmation evidence
- lifecycle replay verifier using artifact-read contract only

Tests:

- Prepared invocation artifact excludes raw tx, signatures, keystore env names,
  password paths, provider kind, and endpoint data.
- Replay verifier never opens env/RPC/signer providers.
- Replay tamper tests for facts, outputs, domain evidence, missing/mismatched
  artifacts.

Verification:

```bash
cargo test -p mfm-adapters-evm-contracts
```

Dependencies:

- Commits 3, 5, 6, 8, and 12.

### 14. `replace dcv operation with contract lifecycle op`

Purpose:

- Replace recipe-named operation crate with contract lifecycle operation crate.
  Keep aggregate topology at operation layer only.

Likely touched:

- `crates/ops/evm-contract-lifecycle-op/**`
- old `crates/ops/evm-deploy-configure-validate-op/**`
- root `Cargo.toml`

Code to delete:

- DCV op crate.
- DCV re-exports of lower-layer capabilities/adapter internals.
- `mfm.evm.dcv.operation.*`
- `evm_dcv` root scope/public output keys.

Code to add or move:

- deploy-only operation helper
- configure-only operation helper
- validate-only operation helper
- full lifecycle operation helper
- aggregate lifecycle authored/canonical config owned by operation crate
- compile APIs using contract lifecycle language

Tests:

- Typestate ordering compile-fail tests rehomed under lifecycle operation.
- Operation remains planning-only and has no transport/signer-provider/storage
  dependencies.

Verification:

```bash
cargo test -p mfm-op-evm-contract-lifecycle
```

Dependencies:

- Commit 12.

### 15. `wire app assembly through lifecycle capabilities`

Purpose:

- Update app assembly to register new lifecycle operation, adapter, generic EVM
  transport, artifact read contract, and signer provider.

Likely touched:

- `crates/app/Cargo.toml`
- `crates/app/src/lib.rs`
- app tests

Code to delete:

- `mfm_transports_evm_dcv` dependency and runner registration.
- `register_dcv_certification_descriptors`.
- hard call to `verify_evm_dcv_replay`.

Code to add or move:

- production registry wiring for contract lifecycle states/ops.
- capability registry wiring for artifact/EVM/signing.
- lifecycle adapter registration.
- keystore signer provider assembly.
- replay verifier registration under contract lifecycle names.

Tests:

- App starts/resumes/replays lifecycle runs through certified specs.
- App no longer links workflow-specific EVM transport crate.

Verification:

```bash
cargo test -p mfm-app
```

Dependencies:

- Commits 13 and 14.

### 16. `move portfolio runners to adapter boundary`

Purpose:

- Stop treating portfolio runner binding as a transport and delete duplicated
  EVM RPC/source routing.

Likely touched:

- `crates/adapters/portfolio/**`
- old `crates/transports/portfolio/**`
- `crates/app/**`
- portfolio integration tests

Code to delete:

- `PortfolioArtifactReader`
- portfolio-local `PortfolioRpcClient`
- portfolio parsing of `MFM_EVM_RPC_SOURCES_JSON`
- `mfm-artifact-store-fs` dependency from portfolio runner crate

Code to add or move:

- portfolio adapter runner binding consuming artifact-read and EVM capability
  contracts.
- generic EVM transport use through app-provided capabilities.

Tests:

- Portfolio state/adapter tests.
- Local portfolio snapshot integration still passes with runtime source config.

Verification:

```bash
cargo test -p mfm-adapters-portfolio -p mfm-state-portfolio
```

Dependencies:

- Commits 3 and 8.

### 17. `fix proof runner dependency direction`

Purpose:

- Remove known transport-to-operation dependency in proof slice.

Likely touched:

- `crates/transports/proof/**`
- `crates/ops/proof-op/**`
- possible test-support fixture code

Code to delete:

- `mfm-transports-proof -> mfm-op-proof` dependency.

Code to add or move:

- conformance helpers moved to test-support or app fixture layer.

Tests:

- Cargo metadata check rejects transport->operation synthetic edge.
- Proof replay/conformance tests still pass.

Verification:

```bash
cargo test -p mfm-transports-proof -p mfm-integration-tests --test cargo_metadata_contract
```

Dependencies:

- Commit 1.

### 18. `replace cli dcv commands with contract commands`

Purpose:

- Replace CLI public surface in one breaking change, with no aliases.

Likely touched:

- `bin/cli/src/commands/evm/mod.rs`
- `bin/cli/src/commands/evm/contracts.rs`
- remove `bin/cli/src/commands/evm/dcv.rs`
- `bin/cli/Cargo.toml`
- `bin/cli/README.md`
- CLI tests

Code to delete:

- `mfm evm dcv`
- `DcvCommand`
- `DcvRequestArgs`
- `EvmDcvResponse`
- `mfm.cli.evm_dcv.typed.v1`
- `EvmDcvCompileInvalid`

Code to add or move:

- `mfm evm contracts deploy`
- `mfm evm contracts configure`
- `mfm evm contracts validate`
- `mfm evm contracts lifecycle`
- contract lifecycle request/response names
- CLI parser/renderer over lifecycle op compile APIs

Tests:

- `mfm evm contracts --help` exposes expected commands.
- `mfm evm dcv` is absent.
- CLI JSON response envelope remains stable.

Verification:

```bash
cargo test -p mfm --test cli_tests
```

Dependencies:

- Commits 14 and 15.

### 19. `replace rest dcv routes with contract routes`

Purpose:

- Replace REST public surface in one breaking change, with no aliases.

Likely touched:

- `bin/rest-api/src/lib.rs`
- `bin/rest-api/Cargo.toml`
- `bin/rest-api/README.md`
- REST/integration tests

Code to delete:

- `/v1/evm/dcv/*`
- `EvmDcv*StartKind`
- `EvmDcv*StartBody`
- `EvmDcvStartResponse`
- `mfm.rest_api.evm_dcv.typed.v1`
- `InvalidEvmDcvRequest`
- `EvmDcvCompileInvalid`

Code to add or move:

- `/v1/evm/contracts/deploy`
- `/v1/evm/contracts/configure`
- `/v1/evm/contracts/validate`
- `/v1/evm/contracts/lifecycle`
- `evm_contract_*_start_v1` request kinds
- contract lifecycle framework version defaults

Tests:

- New routes pass.
- Old `/v1/evm/dcv/*` routes return 404.
- Old `evm_dcv_*` request kinds reject.

Verification:

```bash
cargo test -p mfm-rest-api
```

Dependencies:

- Commits 14 and 15.

### 20. `rebaseline reth lifecycle integration fixtures`

Purpose:

- Rebaseline production-path parity tests around contract lifecycle language and
  correct runtime source/signer config.

Likely touched:

- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
- `tests/integration/tests/typed_portfolio_snapshot_local.rs`
- `tests/integration/src/test_support.rs`
- `tests/integration/Cargo.toml`
- `nixfied/project/module.nix`

Code to delete:

- local reth as `ethereum-mainnet`
- DCV routes/kinds/framework names
- DCV signer JSON with keystore env refs
- test dependency on `mfm-transports-evm-dcv`

Code to add or move:

- local/dev network id, for example `reth-dev` or `reth-local`
- explicit expected chain id
- runtime source registry config
- runtime signer registry config
- contract lifecycle public routes/kinds

Tests:

- reth-backed deploy/configure/validate/lifecycle coverage.
- replay evidence tests under lifecycle adapter.
- portfolio reth snapshot uses runtime source routing.

Verification:

```bash
nix run .#ci -- --mode parity --summary
```

Dependencies:

- Commits 18 and 19.

### 21. `shrink architecture allowlists after dcv removal`

Purpose:

- Remove temporary DCV exceptions and make architecture checks hard gates.

Likely touched:

- `tests/integration/tests/cargo_metadata_contract.rs`
- namespace/schema tests
- app boundary tests

Code to delete:

- DCV path allowlists.
- DCV dependency allowlists.
- stale public namespace exceptions.

Code to add or move:

- hard checks for final category dependency rules.
- hard checks for no active DCV public names outside historical docs.

Tests:

- Full metadata/category checks.
- Namespace/schema/public route checks.
- Compile-fail tests for forbidden state dependencies.

Verification:

```bash
nix run .#check
```

Dependencies:

- Commit 20.

### 22. `refresh docs catalog and nixfied metadata`

Purpose:

- Update docs, generated metadata, and Nixfied examples after new crates and
  public surfaces exist.

Likely touched:

- `crates/docs/catalog.toml`
- `crates/docs/README.md`
- `docs/repo-map.md`
- `docs/repo-index.json`
- `docs/docs-rs-readiness.md`
- `docs/evm-rpc-routing.md`
- `bin/cli/README.md`
- `bin/rest-api/README.md`
- `nixfied/project/module.nix`

Code to delete:

- active docs/catalog/Nixfied references to DCV crates and routes.
- local reth `ethereum-mainnet` aliases.
- stale publish-docs examples for `mfm-transports-evm-dcv`.

Code to add or move:

- final crate catalog entries.
- generic EVM source registry docs.
- signer registry docs.
- contract lifecycle CLI/REST docs.
- Nixfied runtime-source examples using local/dev network ids.

Tests:

- Umbrella docs are synchronized.
- Docs catalog validates.
- Repo map/index regenerated.

Verification:

```bash
nix run .#publish-docs -- sync-umbrella --check
nix run .#check
```

Dependencies:

- Commit 21.

### 23. `run full architecture correction gate`

Purpose:

- Final cleanup and full CI gate.

Likely touched:

- Any residual stale references found by final checks.

Code to delete:

- Any remaining active old DCV names outside historical docs.
- Any remaining allowlist entries that are no longer needed.

Code to add or move:

- Small docs clarifications if final checks expose missing contributor guidance.

Tests:

- Full CI.

Verification:

```bash
nix run .#ci -- --mode full --summary
```

Dependencies:

- Commit 22.

## Test And Enforcement Plan

### Cargo Metadata Category Checks

Required checks:

- every workspace crate declares `package.metadata.mfm.category`
- category is one of the approved categories
- path and category are consistent
- category dependency matrix is enforced
- allowlists are exact and shrink each commit

Important rules:

- `state` may depend on kernel/domain/config/capability-contract/adapter-contract
  crates only.
- `operation` may depend on states, domain models/configs, and contract crates,
  but not transports, signer providers, app, binaries, runtime scheduling, or
  storage implementations.
- `adapter` may bind states/capability contracts/transports/signer contracts,
  but not operations, app, binaries, or concrete storage implementations.
- `transport` may depend on capability contracts and protocol/domain support,
  but not operations, app, binaries, signer providers, or workflow states.
- `signer-provider` may depend on signer contracts and provider internals, but
  not workflow states or operations.
- artifact-read users must depend on artifact capability contracts, not concrete
  artifact-store implementations.

### Namespace Checks

Reject active use of:

- `dcv`
- `Dcv`
- `EvmDcv`
- `evm_dcv`
- `evm-dcv`
- `mfm.evm.dcv`
- `/v1/evm/dcv`
- `typed-evm-dcv`

Allowed only in:

- `PROBLEM_ARCH_DCV.md`
- this implementation plan until completion
- explicit historical migration notes if added

### Schema Checks

Reject typed schemas/configs/public outputs containing:

- `mfm.evm.dcv`
- provider runtime fields: `rpc_url`, `authorization`, `keystore_path`,
  `keystore_path_env`, `password_file`, `password_file_env`
- secret material fields: `private_key`, `mnemonic`, `password`, signature
  scalars
- raw transaction fields in typed semantic artifacts or public outputs

### Compile-Fail Tests

Add or rehome trybuild coverage for:

- states cannot depend on transports, signer providers, app, binaries, storage
  implementations, or operations
- transient raw signed transaction types cannot implement/present as
  `MfmValue`, `MfmConfig`, public outputs, or semantic artifacts
- runtime signer-provider config cannot derive `MfmValue`/`MfmConfig`
- lifecycle typestate ordering still rejects validate-before-configure

### Integration Tests

Required integration coverage:

- contract lifecycle CLI and REST routes
- old DCV CLI and REST surfaces absent
- reth-backed deploy/configure/validate/lifecycle path
- replay from recorded evidence only
- replay rejects missing/mismatched artifacts
- local reth uses local/dev network id with explicit expected chain id
- runtime source and signer registries are process-local only
- prepared invocation artifacts exclude raw signed transactions, signatures,
  endpoints, auth, keystore paths, password paths, and provider internals

## Migration Risks And Sequencing Hazards

- Do not tighten namespace checks before adding exact temporary allowlists.
- Do not classify mixed crates in a way that hides violations. Expose the
  violation and shrink allowlists as the split lands.
- Do signing boundaries before deleting old signer config, or deploy/configure
  lose a replacement signing path.
- Do generic EVM transport before deleting `crates/transports/evm-dcv`, or
  portfolio and lifecycle will reimplement JSON-RPC under new names.
- Do artifact capability before moving replay verifier, or replay byte access
  will stay workflow-local.
- Rebaseline CLI/REST docs in the same change as public command/route
  replacement.
- Rebaseline Nixfied reth source names with parity fixtures; otherwise CI loses
  its configured source.
- Existing stored runs/specs will break. Do not add compatibility shims.

## Acceptance Criteria

The correction is complete when:

- no active public API exposes `dcv`
- no active schema namespace uses `mfm.evm.dcv`
- every workspace crate declares `package.metadata.mfm.category`
- category dependency checks pass without DCV allowlists
- EVM capability contracts live in `crates/evm-capabilities`
- live EVM JSON-RPC lives in `crates/transports/evm`
- endpoint/auth-bearing source structs live only in `crates/transports/evm`
- signer contracts live in `crates/signing`
- EVM signing bridge logic lives in `crates/evm-signing`
- keystore signer provider behavior lives in `crates/signers/keystore`
- lifecycle runner/replay behavior lives in `crates/adapters/evm-contracts`
- lifecycle state semantics live in `crates/states/evm-contracts`
- lifecycle topology lives in `crates/ops/evm-contract-lifecycle-op`
- artifact reads go through artifact-read capability contracts
- typed lifecycle config contains signer refs and semantic network/expected
  chain intent only
- EIP-1559 is the default EVM transaction style, with legacy supported
- local reth fixtures do not pretend to be Ethereum mainnet
- CLI and REST expose contract lifecycle language only
- `nix run .#ci -- --mode full --summary` passes
