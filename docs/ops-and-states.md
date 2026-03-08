# Operations and States Inventory

> Status: living inventory of built-in operations and production `State` implementations.
>
> Source of truth:
> - built-in op registry: `crates/app/src/lib.rs`
> - approved shared-state roots: `crates/tools/architecture-verify/src/main.rs`
> - concrete production states: `impl State for` definitions under the approved roots, plus the
>   intentional op-local exceptions documented below

## How To Use This Document

Use this document when you need the current catalog:

- what ops are built into the default app bundle
- which crate owns each op
- which shared state modules implement production behavior
- which op-local `State` implementations are still intentional exceptions

Use `docs/architecture.md` for placement and boundary rules.
Use `docs/redesign.md` for normative semantics and invariants.

Current snapshot:

- Built-in registered ops: `15`
- Shared production `State` impls: `29`
- Intentional op-local production `State` impls: `3`

## Built-In Ops

The built-in app bundle registers these ops in `DefaultOperationPlugin::register_operations`.

| Op ID | Version | Owner | Purpose | Primary states | Entry points |
|---|---|---|---|---|---|
| `proof` | `v1` | `crates/ops/proof-op` | Determinism and resume acceptance workflow | `NamespaceReadState`, `IdempotentSideEffectState`, `WriteOutputState` | `mfm run start`, feature `run.start` |
| `keystore_import` | `v1` | `crates/ops/keystore-admin-op` | Import a key into the keystore | `KeystoreImportState` | `mfm keystore import`, feature `run.start` |
| `keystore_list` | `v1` | `crates/ops/keystore-admin-op` | List keystore entries | `KeystoreListState` | `mfm keystore list`, feature `run.start` |
| `keystore_delete` | `v1` | `crates/ops/keystore-admin-op` | Delete a keystore entry | `KeystoreDeleteState` | `mfm keystore delete`, feature `run.start` |
| `keystore_tx_sign` | `v1` | `crates/ops/keystore-tx-op` | Sign an EIP-1559 transaction via the keystore | `KeystoreTxSignState` | `mfm keystore tx-sign`, feature `run.start` |
| `keystore_tx_send_raw` | `v1` | `crates/ops/keystore-tx-op` | Submit a signed raw transaction | `KeystoreTxSendRawState` | `mfm keystore tx-send-raw`, feature `run.start` |
| `evm_read` | `v1` | `crates/ops/evm-read-op` | Read chain data through reusable EVM read states | EVM read state family | `mfm run start`, feature `run.start` |
| `evm_contract_from_nix` | `v1` | `crates/ops/evm-write-op` | Adapt nix output into a contract artifact export | `NixArtifactToEvmContractState` | `mfm run start`, feature `run.start` |
| `evm_deploy` | `v1` | `crates/ops/evm-write-op` | Deploy a contract artifact to EVM | `EvmDeployState` | `mfm run start`, feature `run.start` |
| `evm_configure` | `v1` | `crates/ops/evm-write-op` | Execute post-deploy runtime calls | `EvmConfigureState` | `mfm run start`, feature `run.start` |
| `evm_validate` | `v1` | `crates/ops/evm-write-op` | Enforce read/event/client assertions | `EvmValidateState` | `mfm run start`, feature `run.start` |
| `evm_deploy_configure_validate` | `v1` | `crates/ops/evm-deploy-configure-validate-op` | Compose deploy/configure/validate into one op boundary | Child ops `evm_deploy`, `evm_configure`, `evm_validate` | `mfm run pipeline deploy-configure-validate`, feature `pipeline.deploy_configure_validate.start`, feature `run.start` |
| `portfolio_tracker` | `v1` | `crates/ops/portfolio-tracker-op` | Produce a replayable portfolio snapshot and typed report | `ReadU64HexState`, `NativeBalanceState`, `TokenBalanceState`, `WriteSnapshotState`, `WriteReportState` | `mfm portfolio snapshot`, feature `portfolio.snapshot`, feature `run.start` |
| `nix_app` | `v1` | `crates/ops/nix-app-op` | Run a nix-resolved program via the exec namespace | `NixExecState` | `mfm run start`, feature `run.start` |
| `aave_v3_origin_adapt_deploy` | `v1` | `crates/ops/aave-v3-origin-adapt-op` | Adapt Origin deploy output into an Aave V3 deploy manifest | `AdaptOriginDeployOutputState` | `mfm run start`, feature `run.start` |

Notes:

- `mfm-op-keystore-shim` is a helper wrapper crate, not a registered runtime op.
- Built-in feature entry points are owned by `FeatureCatalog` in `crates/app/src/lib.rs`.
- CLI/API transport layers stay thin; dedicated CLI commands exist only for a subset of ops.

## Shared Production States

These modules live under the approved shared-state roots and currently define the production
runtime behavior reused by thin ops.

| Module | State types | Purpose | Used by built-in ops |
|---|---|---|---|
| `crates/states/common/src/states/io.rs` | `NamespaceReadState` | Generic namespace-backed read step that records a fact and writes a context value | `proof` and other reusable read patterns |
| `crates/states/common/src/states/nix.rs` | `NixExecState` | Execute a nix-resolved or pre-resolved program through the exec namespace | `nix_app` |
| `crates/states/common/src/states/side_effect.rs` | `IdempotentSideEffectState` | Apply a side effect behind a stable idempotency key | `proof` |
| `crates/states/keystore/src/states/admin.rs` | `KeystoreImportState`, `KeystoreListState`, `KeystoreDeleteState` | Reusable keystore administration flows | `keystore_import`, `keystore_list`, `keystore_delete` |
| `crates/states/keystore/src/states/tx.rs` | `KeystoreTxSignState`, `KeystoreTxSendRawState` | Reusable signing and raw-transaction submission flows | `keystore_tx_sign`, `keystore_tx_send_raw` |
| `crates/evm-runtime/src/states/read.rs` | `ReadHexStringState`, `ReadU256HexState`, `EthCallState`, `ReadU64HexState`, `NativeBalanceState`, `TokenBalanceState` | Reusable chain read/query states | `evm_read`, `portfolio_tracker` |
| `crates/evm-runtime/src/states/write.rs` | `NixArtifactToEvmContractState`, `EvmDeployState`, `EvmConfigureState`, `EvmValidateState` | Reusable contract artifact adaptation and deploy/configure/validate states | `evm_contract_from_nix`, `evm_deploy`, `evm_configure`, `evm_validate` |
| `crates/states/aave-v3/src/states.rs` | `LoadCompileManifestState`, `DeployContractState`, `WaitForReceiptState`, `CollectDeployOutputsState`, `WriteDeployManifestState`, `AdaptOriginDeployOutputState`, `LoadDeployManifestState`, `ConfigureRuntimeCallState`, `WaitForConfigReceiptState`, `CollectConfigOutputsState`, `WriteConfigReportState` | Reusable Aave V3 deploy/configure/adaptation flow states | Direct built-in use today: `aave_v3_origin_adapt_deploy` for adaptation; the remaining states are reusable flow building blocks not yet registered as standalone built-in ops |

## Intentional Op-Local Production States

These are the current allowed exceptions to the shared-state rule. They stay op-local because they
assemble domain-specific outputs rather than reusable execution primitives.

| Owner | State type | Why still local |
|---|---|---|
| `crates/ops/proof-op` | `WriteOutputState` | Domain-specific output artifact for the proof acceptance workflow |
| `crates/ops/portfolio-tracker-op` | `WriteSnapshotState` | Portfolio-specific snapshot assembly and output writing |
| `crates/ops/portfolio-tracker-op` | `WriteReportState` | Portfolio-specific typed report assembly |

## Update Policy

Update this document in the same change whenever any of the following happen:

- a built-in op is added to or removed from `DefaultOperationPlugin::register_operations`
- an op ID or version changes
- a new production `impl State for` lands under the approved shared-state roots
- a new intentional op-local production state is introduced
- a CLI command or built-in feature becomes a first-class entry point for an op

Minimum code locations to check when updating:

- `crates/app/src/lib.rs` for built-in op registration and built-in features
- `crates/tools/architecture-verify/src/main.rs` for approved shared-state roots
- `crates/states/*` and `crates/evm-runtime/src/states/*` for shared production states
- `crates/ops/*/src/lib.rs` for intentional op-local `State` implementations
