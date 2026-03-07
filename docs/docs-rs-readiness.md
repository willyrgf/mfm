# docs.rs Publishing Readiness Guide

> Generated: 2026-03-07
> Purpose: Actionable checklist for the engineer agent adding code documentation and examples across the workspace.

## Executive Summary

**Current state: NOT READY for docs.rs publishing.**

- 0/36 crates enforce `#![warn(missing_docs)]` or `#![deny(missing_docs)]`
- ~500+ public items lack doc comments across the workspace
- 1 crate has a single doc example (crates/core keystore); all others have zero
- 7+ crates are missing crate-level `//!` documentation entirely

## Global Requirements (Apply to Every Crate)

1. Add `#![warn(missing_docs)]` to every `lib.rs` (upgrade to `#![deny(missing_docs)]` once coverage is complete).
2. Every `lib.rs` must have a crate-level `//!` doc block explaining purpose, relationship to the architecture, and a minimal usage example.
3. Every public item (`pub fn`, `pub struct`, `pub enum`, `pub trait`, `pub type`, `pub const`, `pub mod`) must have a `///` doc comment.
4. Every public struct/enum field must have a `///` doc comment.
5. Every trait method must have a `///` doc comment.
6. Add at least one ```` ```rust ```` doc example per crate (ideally on the main public type or entry-point function).
7. Security-sensitive items (keystore, secret store, crypto) must document threat model considerations.

## Priority Tiers

### Tier 1 — Core Engine (document first)

These crates define the execution model and are the foundation everything else depends on.

#### crates/machine/ — State machine runtime
- **Crate doc:** Present but minimal (3 lines)
- **Missing docs lint:** No
- **Doc examples:** None
- **Undocumented public items (~30):**
  - `lib.rs:15` — `IdValidationError`
  - `lib.rs:97` — `OpId::new()`
  - `lib.rs:105` — `OpId::must_new()`
  - `lib.rs:109` — `OpId::as_str()`
  - `lib.rs:178` — `StateId::new()`
  - `lib.rs:186` — `StateId::must_new()`
  - `lib.rs:190` — `StateId::as_str()`
  - `lib.rs:379` — `default_nix_flake_allowlist()`
  - `lib.rs:416-427` — All `standard_tags` constants (`CONFIG`, `FETCH_DATA`, `COMPUTE`, `EXECUTE`, `REPORT`, `APPLY_SIDE_EFFECT`, `IMPURE`)
  - `lib.rs:658-661` — Domain event name constants
  - `lib.rs:716` — `Event` enum
  - `lib.rs:742` — `ArtifactWritten`
  - `lib.rs:749` — `OpBoundary`
  - `lib.rs:763` — `ChildRunCompleted`
  - `lib.rs:886` — `StateOutcome`
  - `lib.rs:907` — `DynState` type alias
  - `lib.rs:923` — `StateNode`
  - `lib.rs:936` — `ExecutionPlan`
  - `lib.rs:962` — `PlanValidator` trait
  - `lib.rs:974` — `ArtifactKind` enum
  - `lib.rs:1035` — `RunResult`
- **Example ideas:** Show how to define a simple `State`, build a `StateNode`, create an `ExecutionPlan`, and execute a run.

#### crates/machine-derive/ — Proc-macro crate
- **Crate doc:** Present (3 lines)
- **Undocumented items:** 0 (minimal public surface)
- **Action:** Expand crate doc with usage example showing `#[derive(...)]` on a state struct.

#### crates/machine-test-support/ — Test utilities
- **Crate doc:** Present (4 lines)
- **Undocumented items (3):**
  - `lib.rs:41` — `init_test_observability()`
  - `lib.rs:58` — `artifact_store_contract_tests()`
  - `lib.rs:64` — `event_store_contract_tests()`
- **Action:** Document each function and add a doc example showing test setup.

---

### Tier 2 — Core Primitives & SDK

#### crates/core/ — Primitives + keystore
- **Crate doc:** MISSING
- **Doc examples:** Partial (keystore module only, at `keystore/mod.rs:10-39`)
- **Undocumented items (~40+):**
  - `lib.rs` — No crate doc, undocumented module declarations
  - `config/mod.rs:18` — `Config`, `NetworkConfig`, `WalletConfig`, `DexConfig`, `SecureWallet` and all their methods (`load()`, `validate()`, `load_wallet()`)
  - `config/network.rs:8` — `Kind` enum, `Network` struct, `Networks` wrapper, `NetworkValueError`
  - `config/token.rs:8` — `Kind` enum, `TokenNetwork`, `TokenNetworks`, `Token`, `Tokens`, `SlippageParseError`
  - `config/dexes.rs:7` — `Kind` enum, `Dex`, `Dexes`
  - `config/authentication/mod.rs:9` — `Method` enum, `Methods`
  - `config/authentication/wallet.rs:5` — `Wallet`, `read_private_key()`
  - `keystore/mod.rs:211` — `AuditEvent`, `AuditLogEntry`
  - Note: Keystore core types (`Keystore`, `KeyEntry`, `KeyInfo`, `SecureKey`, `KeystoreConfig`, `KeyType`, `KeystoreError`) ARE documented
- **Action:** Add crate doc. Document all config types. Expand keystore doc example.

#### crates/evm-core/ — EVM utilities
- **Crate doc:** MISSING
- **Doc examples:** None
- **Undocumented items (~45, every public item):**
  - `lib.rs:1-5` — All module declarations (`abi`, `encoding`, `hex`, `rlp`, `util_error`)
  - `abi.rs` — `AbiFunction`, `AbiEvent`, `ParsedAbi`, `parse_abi()`, `parse_bytecode()`, `function_selector()`, `encode_params()`, `resolve_function_call()`, `constructor_data()`, `parse_value_wei_to_hex()`, `decode_single_output_to_json()`
  - `encoding.rs` — `ERC20_SELECTOR_BALANCE_OF`, `ERC20_SELECTOR_DECIMALS`, `address_hex_lower()`, `encode_erc20_balance_of()`, `u64_hex_quantity()`, `format_u256_units()`, `parse_u256_hex()`, `parse_u256_hex_value()`, `parse_u8_u256()`, `parse_hex_string_response()`, `parse_u256_hex_response()`, `normalize_address()`, `address_to_hex_prefixed()`, and ~10 more
  - `hex.rs` — `normalize_hex_str()`, `hex_to_bytes()`, `bytes_to_hex_prefixed()`, `hex_encode_utf8()`, `hex_nibble()`
  - `rlp.rs` — All 6 public RLP functions
  - `util_error.rs` — `UtilError`
- **Action:** This is the worst-documented crate. Add crate doc explaining it's the low-level EVM encoding/decoding layer. Document every function with parameter/return descriptions and examples for hex/ABI utilities.

#### crates/sdk/ — Orchestration SDK
- **Crate doc:** Present (3 lines)
- **Doc examples:** None
- **Undocumented items (~20):**
  - `lib.rs:44` — `mod errors` (module doc)
  - `lib.rs:54` — `mod op` (module doc)
  - `lib.rs:86` — `DynOperation` type alias
  - `lib.rs:94` — `mod pipeline` (module doc)
  - `lib.rs:150` — `mod launcher` (module doc)
  - All trait method docs on `Operation`, `OperationRegistry`, `PipelinePlanner`, `RunLauncher`
  - All struct field docs on `OpIo`, `PipelineStep`, `Pipeline`, `PipelineManifestInput`, `LaunchPipeline`
  - `unstable.rs` — `HashMapOperationRegistry::register()`, `SdkPlanResolver::new()`, `SpawnChildRunV1`, `SpawnChildRunResult`, `AwaitChildRunV1`, `AwaitChildRunResult`, `spawn_child_run_v1()`, `await_child_run_v1()`
- **Action:** Add doc examples showing how to define an Operation, register it, build a pipeline, and launch a run.

#### crates/app/ — Application orchestration bridge
- **Crate doc:** MISSING
- **Doc examples:** None
- **Undocumented items (~80+, every public item):**
  - `lib.rs` — `ErrorClass`, `AppError`, `MapContext`, all `default_*()` helpers, `EngineBundle`, `OperationPlugin` trait, `TransportPlugin` trait, `DefaultOperationPlugin`, `DefaultTransportPlugin`, `AppBuilder`, `AppServices` and all its methods (`start_run()`, `resume_run()`, `run_status()`, `run_events()`, `artifact_get()`, `start_portfolio_snapshot()`, etc.)
  - All request/response types: `RunsStartRequest`, `SingleOpStartRequest`, `PipelineStartRequest`, `RunStartResponse`, `RunResumeResponse`, `RunStatusResponse`, `RunsEventsQuery`, `RunsEventsResponse`, `ArtifactGetResponse`, `ArtifactBody`
  - `DeployConfigureValidateSpec`, `FeatureKind`, `FeatureDescriptor`, `FeatureRequest`, `FeatureExecutionResult`, `FeatureCatalog`
  - `PortfolioSnapshotRequest`, `PortfolioSnapshotResponse`, `PortfolioTokenSpec`, `parse_portfolio_tokens_json()`
  - `observability.rs` — 7 env-var constants, `LogFormat`, `ObservabilityConfig`, `observability_from_env()`, `init_observability()`
- **Action:** This is the second-worst crate. Add crate doc explaining it's the bridge between binaries and the engine. Document all public types. Add example showing `AppBuilder` -> `AppServices` -> `start_run()`.

---

### Tier 3 — States Layer

#### crates/states/common/ — Cross-domain reusable states
- **Crate doc:** Present (4 lines)
- **Doc examples:** None
- **Undocumented items (~50+):**
  - `ctx.rs` — `read_json()`, `read_u64_required()`, `read_json_required()`, `read_string_required()`, `read_array_required()`, `read_typed()`, `write_json()`
  - `errors.rs` — `info()`, `sdk_error()`, `sdk_parse_error()`, `sdk_unknown_error()`, `state_error()`, `state_error_with_state()`, `state_unknown()`, `state_unknown_msg()`, `state_from_io()`, `keystore_error_category()`
  - `idempotency.rs` — `state_scope()`, `op_scope()`, `canonical()`, `state_purpose()`, `op_purpose()`, `idempotency_key_for_value()`
  - `local_io_helpers.rs` — `emit_report_event()`, `local_call()`, `local_fact_key()`, `attach_state_id()`
  - `output.rs` — `write_output_artifact()`
  - `rpc.rs` — `validation_assertion_error_message()`, `expect_string()`, `expect_array()`, `assert_condition()`
  - `test_support.rs` — ~20 public items (test helpers, `MapContext`, `MemEventStore`, `MemArtifactStore`, etc.)
  - `states/io.rs` — `NamespaceReadState`
  - `states/meta.rs` — All `tags` module functions and metadata builder functions
  - `states/nix.rs` — `NixExecStateConfig`, `validate_nix_exec_config()`, `NixExecState`
  - `states/side_effect.rs` — `TriggerOnce`, `IdempotentSideEffectState`
- **Action:** Document all context helpers, error builders, and state types. Add examples for `NamespaceReadState` and `IdempotentSideEffectState`.

#### crates/states/keystore/ — Keystore domain states
- **Crate doc:** MISSING
- **Doc examples:** None
- **Undocumented items (~30+):**
  - `lib.rs:1-2` — Module declarations
  - `tx.rs` — `KeystoreTxError`, `Eip1559TxToSign`, `SignedEip1559Tx`, `parse_address()`, `parse_u128_quantity()`, `parse_data_hex()`, `parse_rpc_url()`, `validate_raw_transaction_hex()`, `resolve_key_id()`, `sign_eip1559_transaction()`, `write_raw_transaction_file()`, `output_context_key()`
  - `states/admin.rs` — `KeystoreAdminError`, `KeystoreImportType`, `KeystoreImportReport`, `KeystoreListSortBy`, `KeystoreListKey`, `KeystoreListReport`, `KeystoreDeleteReport`, config structs, state structs and constructors, `decode_optional_hex_string()`, `resolve_keystore_path()`, `sdk_error_from_helper()`
  - `states/tx.rs` — `KeystoreTxSignStateConfig`, `KeystoreTxSendRawStateConfig`, `KeystoreTxSignState`, `KeystoreTxSendRawState`

#### crates/states/aave-v3/ — Aave V3 domain states
- **Crate doc:** MISSING
- **Doc examples:** None
- **Undocumented items (~40+):**
  - `manifest.rs` — `ContractArtifactJson`, `AaveCompileManifestContract`, `AaveCompileManifest`, `AaveDeployManifestContract`, `AaveDeployManifest`, `AaveOriginDeployOutputContract`, `AaveOriginDeployOutput`, `AaveConfigCallRecord`, `AaveConfigReport`, `AaveDeployRuntimeConfig`, `AaveConfigureRuntimeConfig`, all `validate_*()` and `decode_*()` functions
  - `states.rs` — All 11 state structs (`LoadCompileManifestState`, `DeployContractState`, `WaitForReceiptState`, etc.)

#### crates/evm-runtime/ — EVM execution states
- **Crate doc:** MISSING
- **Doc examples:** None
- **Undocumented items (~60+, every public item):**
  - `dcv.rs` — `ContractArtifactConfig`, `ConfigureCallConfig`, `BlockTag`, `ReadAssertionConfig`, `EventAssertionConfig`, `PreparedReadAssertion`, `PreparedEventAssertion`, and 15+ functions
  - `rpc.rs` — `RpcRawTxSubmission`, `LegacyCreateTxSigningRequest`, and 20+ public async functions (`send_transaction()`, `estimate_gas_hex()`, `gas_price_hex()`, `wait_for_receipt()`, etc.)
  - `states/read.rs` — `U64Expectation`, `ReadHexStringState`, `ReadU256HexState`, `EthCallDecode`, `EthCallState`, `ReadU64HexState`, `NativeBalanceState`, `TokenBalanceState`
  - `states/write.rs` — `EvmDeployStateConfig`, `EvmConfigureRuntimeCall`, `EvmConfigureStateConfig`, `EvmValidateStateConfig`, `NixArtifactToEvmContractState`, `EvmDeployState`, `EvmConfigureState`, `EvmValidateState`

---

### Tier 4 — Ops Layer

All 10 op crates follow the same pattern. None have `#![warn(missing_docs)]`. Only 6/10 have crate-level docs.

#### Missing crate-level docs (4 crates):
- `crates/ops/keystore-tx-op/`
- `crates/ops/keystore-admin-op/`
- `crates/ops/evm-deploy-configure-validate-op/`
- `crates/ops/aave-v3-origin-adapt-op/`

#### Have crate-level docs (6 crates):
- `crates/ops/keystore-op/` — minimal (re-export wrapper)
- `crates/ops/evm-read-op/`
- `crates/ops/evm-write-op/`
- `crates/ops/portfolio-tracker-op/`
- `crates/ops/nix-app-op/`
- `crates/ops/proof-op/`

#### Common undocumented items across ops:
- Operation structs and their `OpConfig` types
- All struct fields (especially config fields that users must populate)
- Op-local state structs (in portfolio-tracker-op and proof-op)
- Report/output types and their fields
- Zero doc examples in all 10 crates

---

### Tier 5 — Storage Layer

All 6 crates have good crate-level docs but lack item-level documentation.

| Crate | Crate Doc | Undocumented Items |
|-------|-----------|-------------------|
| `event-store-mem` | Good | `MemEventStore`, `new()` |
| `event-store-postgres` | Good | `PostgresEventStore`, `connect()`, `connect_env()` |
| `artifact-store-fs` | Good | `FsArtifactStore`, `new()` |
| `artifact-store-s3` | Good | `S3ArtifactStore`, `new()`, `from_env()` |
| `artifact-store-secret` | Good | `SecretKey` (all methods), `SecretArtifactStore` (all methods) |
| `indexer` | Good | `ProjectionIndexer`, `new()` |

---

### Tier 6 — Collectors & Transports

#### Collectors with crate-level docs (3/5):
- `collectors/evm` — Present
- `collectors/evm-jsonrpc-http` — Present (detailed)
- `collectors/nix-exec` — Present

#### Collectors MISSING crate-level docs (2/5):
- `collectors/nix`
- `collectors/exec`

#### Transports — ALL 4 MISSING crate-level docs:
- `transports/local-evm`
- `transports/local-fs`
- `transports/local-keystore`
- `transports/proof`

#### Common undocumented items:
- All struct types and fields across all 9 crates
- All constants (namespace identifiers)
- All factory constructors
- All client methods
- Zero doc examples

---

## Documentation Gaps from Prior Review

These items were identified during the architecture review and should also be addressed:

1. **`crates/app/` lacks a README.md** — Add one explaining the orchestration bridge role.
2. **`docs/state-layer-migration-plan.md`** — Mark as completed (migration is done).
3. **`docs/three-tier-audit.md`** and **`docs/thin-layer-audit.md`** — Update status sections.

## Suggested Workflow for the Engineer Agent

1. **Start with Tier 1** (machine, machine-derive, machine-test-support) — these define the core types everything depends on.
2. **Move to Tier 2** (core, evm-core, sdk, app) — focus on evm-core and app first since they're the worst.
3. **Then Tier 3** (states) — document the shared state primitives.
4. **Then Tier 4-6** (ops, storages, collectors, transports) — these follow patterns established in earlier tiers.
5. After each crate is documented, add `#![warn(missing_docs)]` to prevent regressions.
6. Run `nix run .#check` after each crate to verify compilation.
7. Run `cargo doc --no-deps --document-private-items` to preview docs.rs output.

## Style Guidelines for Doc Comments

Follow existing patterns from the best-documented code (crates/core/src/keystore/):

```rust
/// Brief one-line description.
///
/// Longer explanation if needed, covering:
/// - what this does
/// - when to use it
/// - important invariants or constraints
///
/// # Examples
///
/// ```rust
/// use mfm_machine::StateId;
///
/// let id = StateId::new("my_state").unwrap();
/// assert_eq!(id.as_str(), "my_state");
/// ```
///
/// # Errors
///
/// Returns `IdValidationError` if the identifier doesn't match `^[a-z][a-z0-9_]{0,62}$`.
///
/// # Panics
///
/// (Document if applicable)
```

For security-sensitive items, include a `# Security` section:

```rust
/// # Security
///
/// This type holds secret material. The inner buffer is zeroized on drop.
/// Do not log, serialize, or expose this value in error messages.
```
