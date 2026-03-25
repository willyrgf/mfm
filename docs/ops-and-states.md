# Operations and States Inventory

> Status: living inventory of built-in operations and production `State` implementations.
>
> Source of truth:
> - built-in op registry: `crates/app/src/lib.rs`
> - shared production state modules documented below under `crates/states/*` and `crates/evm-runtime/src/states/*`
> - concrete production states: `impl State for` definitions under those shared modules, plus the
>   intentional op-local exceptions documented below

## How To Use This Document

Use this document when you need the current catalog:

- what ops are built into the default app bundle
- which crate owns each op
- which shared state modules implement production behavior
- which op-local `State` implementations are still intentional exceptions

Use `docs/architecture.md` for placement and boundary rules.
Use `docs/design.md` for normative semantics and invariants.

Current snapshot:

- Built-in public root ops: `14`
- Planner-internal registered child ops: `8` (`portfolio_tracker` semantic lowering only)
- Shared production `State` impls: `33`
- Intentional op-local production `State` impls: `3`

Planning model note:

- This inventory is intentionally focused on built-in/public root ops registered in the default app
  bundle and on production runtime states.
- The default app registry also keeps planner-internal portfolio semantic child ops available so
  recursive flattening can resolve them, but public `run.start` entrypoints reject those internal
  ids.
- Runtime execution units remain states only; internal recursive op expansion must flatten before
  runtime starts.
- This document does not attempt to enumerate every future internal composite planning boundary.

## Built-In Ops

The built-in app bundle registers these public root ops in
`DefaultOperationPlugin::register_operations`. Public single-op entrypoints such as CLI `run start`
and REST `/v1/runs/start` accept only these root ops; planner-internal semantic ids are not public
API.

| Op ID | Version | Owner | Purpose | Primary states | Entry points |
|---|---|---|---|---|---|
| `proof` | `v1` | `crates/ops/proof-op` | Determinism and resume acceptance workflow | `ProofReadState`, `ProofApplySideEffectState`, `WriteOutputState` | `mfm run start`, feature `run.start` |
| `keystore_import` | `v1` | `crates/ops/keystore-admin-op` | Import a key into the keystore | `KeystoreImportState` | `mfm keystore import`, feature `run.start` |
| `keystore_list` | `v1` | `crates/ops/keystore-admin-op` | List keystore entries | `KeystoreListState` | `mfm keystore list`, feature `run.start` |
| `keystore_delete` | `v1` | `crates/ops/keystore-admin-op` | Delete a keystore entry | `KeystoreDeleteState` | `mfm keystore delete`, feature `run.start` |
| `keystore_tx_sign` | `v1` | `crates/ops/keystore-tx-op` | Sign an EIP-1559 transaction via the keystore | `KeystoreTxSignState` | `mfm keystore tx-sign`, feature `run.start` |
| `evm_read` | `v1` | `crates/ops/evm-read-op` | Low-level chain read op backed by the reusable `rpc.control` read states | EVM read state family | `mfm run start`, feature `run.start` |
| `evm_contract_from_nix` | `v1` | `crates/ops/evm-write-op` | Adapt nix output into a contract artifact export | `NixArtifactToEvmContractState` | `mfm run start`, feature `run.start` |
| `evm_deploy` | `v1` | `crates/ops/evm-write-op` | Deploy a contract artifact to EVM | `EvmDeployState` | `mfm run start`, feature `run.start` |
| `evm_configure` | `v1` | `crates/ops/evm-write-op` | Execute post-deploy runtime calls | `EvmConfigureState` | `mfm run start`, feature `run.start` |
| `evm_validate` | `v1` | `crates/ops/evm-write-op` | Enforce read/event/client assertions | `EvmValidateState` | `mfm run start`, feature `run.start` |
| `evm_deploy_configure_validate` | `v1` | `crates/ops/evm-deploy-configure-validate-op` | Compose deploy/configure/validate into one op boundary | Child ops `evm_deploy`, `evm_configure`, `evm_validate` | `mfm run pipeline deploy-configure-validate`, feature `pipeline.deploy_configure_validate.start`, feature `run.start` |
| `portfolio_tracker` | `v1` | `crates/ops/portfolio-tracker-op` | Produce a canonical replayable multi-network portfolio snapshot and typed report through the fixed semantic runtime | `PrepareExecutionSourcesState`, `ResolveSubjectsState`, `PinExecutionViewsState`, `ResolveValuationInputsState`, `ObserveCompiledBatchState`, `MergeObservationsState`, `AssembleSnapshotState`, `ProjectReportState` | `mfm portfolio snapshot`, feature `portfolio.snapshot`, feature `run.start` |
| `nix_app` | `v1` | `crates/ops/nix-app-op` | Run a nix-resolved program via the exec namespace | `NixExecState` | `mfm run start`, feature `run.start` |
| `aave_v3_origin_adapt_deploy` | `v1` | `crates/ops/aave-v3-origin-adapt-op` | Adapt Origin deploy output into an Aave V3 deploy manifest | `AdaptOriginDeployOutputState` | `mfm run start`, feature `run.start` |

Notes:

- `mfm-op-keystore-shim` is a helper wrapper crate, not a registered runtime op.
- Built-in feature entry points are owned by `FeatureCatalog` in `crates/app/src/lib.rs`.
- The default app transport bundle exposes `rpc.control` as the canonical state-facing EVM ingress;
  the raw `evm` executor is kept for internal/direct use only.
- Built-in `evm_*` ops and canonical `rpc.control` requests require explicit `network_id`;
  `control_scope` defaults to `shared` unless the caller opts into isolation.
- CLI/API transport layers stay thin; dedicated CLI commands exist only for a subset of ops.

## Shared Production States

These modules live under the documented shared-state roots and currently define the production
runtime behavior reused by thin ops.

Even under recursive op planning, these remain the runtime execution units after planner
flattening.

| Module | State types | Purpose | Used by built-in ops |
|---|---|---|---|
| `crates/states/common/src/states/nix.rs` | `NixExecState` | Execute a nix-resolved or pre-resolved program through the exec namespace | `nix_app` |
| `crates/states/common/src/states/proof.rs` | `ProofReadState`, `ProofApplySideEffectState` | Typed proof read and side-effect states that avoid raw proof namespace strings | `proof` |
| `crates/states/keystore/src/states/admin.rs` | `KeystoreImportState`, `KeystoreListState`, `KeystoreDeleteState` | Reusable keystore administration flows | `keystore_import`, `keystore_list`, `keystore_delete` |
| `crates/states/keystore/src/states/tx.rs` | `KeystoreTxSignState` | Reusable local keystore signing flow | `keystore_tx_sign` |
| `crates/evm-runtime/src/states/read.rs` | `ReadHexStringState`, `ReadU256HexState`, `EthCallState`, `ReadU64HexState`, `NativeBalanceState`, `TokenBalanceState` | Reusable control-plane-backed chain read/query states | `evm_read`, `portfolio_tracker` |
| `crates/evm-runtime/src/states/rpc_control.rs` | `PrepareSourcesState` | Reusable `rpc.control` source preparation and responsiveness preflight | none directly; shared contract used by semantic and EVM write runtimes |
| `crates/evm-runtime/src/states/price.rs` | `read_evm_oracle_unit_price` | Reusable control-plane-backed EVM oracle price reads for valuation source execution | `portfolio_tracker` |
| `crates/states/portfolio/src/semantic_states.rs` | `PrepareExecutionSourcesState`, `ResolveSubjectsState`, `PinExecutionViewsState`, `ResolveValuationInputsState`, `ObserveCompiledBatchState`, `MergeObservationsState`, `AssembleSnapshotState`, `ProjectReportState` | Fixed semantic runtime states for source preparation, subject/view resolution, valuation resolution, batch observation, merge, and snapshot/report projection | `portfolio_tracker` |
| `crates/evm-runtime/src/states/write.rs` | `NixArtifactToEvmContractState`, `EvmDeployState`, `EvmConfigureState`, `EvmValidateState` | Reusable contract artifact adaptation and deploy/configure/validate states routed through `rpc.control`, with managed-source preflight before write/validate calls | `evm_contract_from_nix`, `evm_deploy`, `evm_configure`, `evm_validate` |
| `crates/states/aave-v3/src/states.rs` | `LoadCompileManifestState`, `DeployContractState`, `WaitForReceiptState`, `CollectDeployOutputsState`, `WriteDeployManifestState`, `AdaptOriginDeployOutputState`, `LoadDeployManifestState`, `ConfigureRuntimeCallState`, `WaitForConfigReceiptState`, `CollectConfigOutputsState`, `WriteConfigReportState` | Reusable Aave V3 deploy/configure/adaptation flow states | Direct built-in use today: `aave_v3_origin_adapt_deploy` for adaptation; the remaining states are reusable flow building blocks not yet registered as standalone built-in ops |

## Intentional Op-Local Production States

These are the current allowed exceptions to the shared-state rule. They stay op-local because they
assemble domain-specific outputs rather than reusable execution primitives.

| Owner | State type | Why still local |
|---|---|---|
| `crates/ops/proof-op` | `WriteOutputState` | Domain-specific output artifact for the proof acceptance workflow |

## Update Policy

Update this document in the same change whenever any of the following happen:

- a built-in op is added to or removed from `DefaultOperationPlugin::register_operations`
- an op ID or version changes
- a new production `impl State for` lands under the shared-state modules documented above
- a new intentional op-local production state is introduced
- a CLI command or built-in feature becomes a first-class entry point for an op

For future recursive planner cutovers:

- if an internal composite sub-op becomes a built-in/public root op, add it to the built-in ops
  table
- if a recursive planning change does not alter the public built-in root op set or production state
  inventory, this document does not need to enumerate the internal sub-op tree

Minimum code locations to check when updating:

- `crates/app/src/lib.rs` for built-in op registration and built-in features
- the shared-state module list in this document plus `crates/states/*` and `crates/evm-runtime/src/states/*`
- `crates/states/*` and `crates/evm-runtime/src/states/*` for shared production states
- `crates/ops/*/src/lib.rs` for intentional op-local `State` implementations
