# Operations and States Inventory

> Status: living inventory of built-in operations and production `State` implementations.
>
> Source of truth:
> - legacy dynamic op registry: `crates/app-legacy/src/lib.rs`
> - certified typed runner registry: `crates/app/src/lib.rs`
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

- Built-in public dynamic root ops: `14`
- Planner-internal registered dynamic child ops: `0`
- Shared production legacy `State` impls: `20`
- Certified typed portfolio state contracts: `9`
- Intentional op-local production `State` impls: `1`

Planning model note:

- This inventory is intentionally focused on built-in/public root ops registered in the default app
  bundle, certified typed workflows registered in the typed app bundle, and production runtime
  states.
- Runtime execution units remain states only; internal recursive op expansion must flatten before
  runtime starts.
- Current canonical-input compatibility roots for deploy/configure/validate compose build then
  execute with the execute child sourcing its config from the build child's planner payload.
  The build child is authoritative for artifact/report publication, ordering, and execute-child
  input materialization.
- This document does not attempt to enumerate every future internal composite planning boundary.

## Built-In Ops

The built-in app bundle registers these public root ops in
`DefaultOperationPlugin::register_operations`. Public single-op entrypoints such as CLI `run start`
and REST `/v1/runs/start` accept only these root ops.
REST `/v1/runs/start` and feature `run.start` require tagged request envelopes
(`single_op_start_v1` or `pipeline_start_v1`) and reject unknown top-level fields.

| Op ID | Version | Owner | Purpose | Primary states | Entry points |
|---|---|---|---|---|---|
| `keystore_import` | `v1` | `crates/ops/keystore-op` | Import a key into the keystore | `KeystoreImportState` | `mfm keystore import`, feature `run.start` |
| `keystore_list` | `v1` | `crates/ops/keystore-op` | List keystore entries | `KeystoreListState` | `mfm keystore list`, feature `run.start` |
| `keystore_delete` | `v1` | `crates/ops/keystore-op` | Delete a keystore entry | `KeystoreDeleteState` | `mfm keystore delete`, feature `run.start` |
| `keystore_tx_sign` | `v1` | `crates/ops/keystore-op` | Sign an EIP-1559 transaction via the keystore | `KeystoreTxSignState` | `mfm keystore tx-sign`, feature `run.start` |
| `evm_read` | `v1` | `crates/ops/evm-read-op` | Low-level chain read op backed by the reusable `rpc.control` read states | EVM read state family | `mfm run start`, feature `run.start` |
| `evm_contract_from_nix` | `v1` | `crates/ops/evm-write-op` | Adapt nix output into a contract artifact export | `NixArtifactToEvmContractState` | `mfm run start`, feature `run.start` |
| `evm_deploy_contract_set` | `v1` | `crates/ops/evm-write-op` | Deploy a compiled contract-set manifest through signed raw transaction intents and emit a deployed manifest | `LoadCompiledContractSetState`, `DeployContractSetState`, `WaitForContractSetReceiptsState`, `CollectDeployedContractSetState`, `WriteDeployedContractSetState` | `mfm run start`, feature `run.start` |
| `evm_deploy` | `v1` | `crates/ops/evm-write-op` | Deploy a contract artifact through a signed raw transaction intent | `EvmDeployState` | `mfm run start`, feature `run.start` |
| `evm_configure` | `v1` | `crates/ops/evm-write-op` | Execute post-deploy signed raw transaction intents | `EvmConfigureState` | `mfm run start`, feature `run.start` |
| `evm_validate` | `v1` | `crates/ops/evm-write-op` | Enforce read/event/client assertions | `EvmValidateState` | `mfm run start`, feature `run.start` |
| `evm_deploy_configure_validate_config_build` | `v1` | `crates/ops/evm-deploy-configure-validate-op` | Build canonical deploy/configure/validate config into built execution config, publish canonical/built config artifacts, and emit a stable build report | `WriteJsonValueState`, `WriteContextValueArtifactState` | feature `run.start` |
| `evm_deploy_configure_validate_execute` | `v1` | `crates/ops/evm-deploy-configure-validate-op` | Execute pre-built deploy/configure/validate config through the fixed deploy/configure/validate runtime graph | Child ops `evm_deploy`, `evm_configure`, `evm_validate` | feature `run.start` |
| `evm_deploy_configure_validate` | `v1` | `crates/ops/evm-deploy-configure-validate-op` | Canonical public root that composes `evm_deploy_configure_validate_config_build` before `evm_deploy_configure_validate_execute` | Child build workflow plus `evm_deploy`, `evm_configure`, `evm_validate` | `mfm run pipeline deploy-configure-validate`, feature `pipeline.deploy_configure_validate.start`, feature `run.start` |
| `nix_app` | `v1` | `crates/ops/nix-app-op` | Run a nix-resolved program via the exec namespace | `NixExecState` | `mfm run start`, feature `run.start` |

Notes:

- Built-in dynamic feature entry points are owned by `FeatureCatalog` in
  `crates/app-legacy/src/lib.rs`.
- `pipeline.deploy_configure_validate.start` now routes through the public
  `evm_deploy_configure_validate/v1` root op family instead of assembling a one-step pipeline
  template in app glue.
- The default app transport bundle exposes `rpc.control` as the canonical state-facing EVM ingress;
  the raw `evm` executor is kept for internal/direct use only.
- Built-in `evm_*` ops and canonical `rpc.control` requests require explicit `network_id`;
  `control_scope` defaults to `shared` unless the caller opts into isolation.
- `proof` is no longer registered as a legacy dynamic `run.start` op. Its executable surface is the
  certified typed program in `crates/ops/proof-op`, with typed fact, side-effect, receipt,
  confirmation, public-output, and replay-verifier contracts in `crates/collectors/proof` and the
  deterministic typed runner implementation in `crates/transports/proof`.
- Portfolio snapshots are no longer registered as legacy dynamic `portfolio_tracker`,
  `portfolio_execute`, or `portfolio_config_build` ops. `mfm portfolio snapshot` compiles a
  certified typed spec through `crates/ops/portfolio-tracker-op` and executes it with runners from
  `crates/transports/portfolio`.
- CLI/API transport layers stay thin; dedicated CLI commands exist only for a subset of ops.

## Certified Typed Workflows

| Workflow | Owner | Typed state contracts | Entry points |
|---|---|---|---|
| portfolio tracker | `crates/ops/portfolio-tracker-op` | `PrepareSourcesState`, `ResolveSubjectsState`, `PinViewsState`, `ResolveValuationsState`, `ObserveBatchState`, `MergeObservationsState`, `AssembleSnapshotState`, `PublishSnapshotState`, `ProjectReportState` | `mfm portfolio snapshot` |

## Shared Production States

These modules live under the documented shared-state roots and currently define the production
runtime behavior reused by thin ops.

Even under recursive op planning, these remain the runtime execution units after planner
flattening.

For EVM flows, pure primitives, transaction models, signing hashes, and raw transaction encoding
live in `crates/evm-core`, deploy/configure/validate schema and calldata preparation live in
`crates/evm-dcv-model`, typed local signer IO lives in `crates/collectors/local-evm`, and live
local signing remains in `crates/transports/local-evm`.
The shared states below keep reusable EVM execution behavior in `crates/evm-runtime`.

| Module | State types | Purpose | Used by built-in ops |
|---|---|---|---|
| `crates/states/common/src/states/nix.rs` | `NixExecState` | Execute a nix-resolved or pre-resolved program through the exec namespace, with optional runtime stdin handoff and non-secret environment overrides for compatibility backends | `nix_app` |
| `crates/states/common/src/states/publish.rs` | `WriteJsonValueState`, `WriteContextValueArtifactState` | Deterministic JSON publication and context-to-artifact emission for thin workflow boundaries | `evm_deploy_configure_validate_config_build` |
| `crates/states/keystore/src/states/admin.rs` | `KeystoreImportState`, `KeystoreListState`, `KeystoreDeleteState` | Reusable keystore administration flows that route typed `local.keystore.*` requests and persist non-secret reports; live filesystem/prompt/password handling stays in `crates/transports/local-keystore` | `keystore_import`, `keystore_list`, `keystore_delete` |
| `crates/states/keystore/src/states/tx.rs` | `KeystoreTxSignState` | Reusable transaction-signing state that routes typed local-keystore signing requests and persists only the non-secret signing report; EIP-1559 transaction modeling/encoding lives in `crates/evm-core`, key signing in `crates/core`, and raw transaction file writes stay in `crates/transports/local-keystore` | `keystore_tx_sign` |
| `crates/evm-runtime/src/states/read.rs` | `ReadHexStringState`, `ReadU256HexState`, `EthCallState`, `ReadU64HexState`, `NativeBalanceState`, `TokenBalanceState` | Reusable control-plane-backed chain read/query states | `evm_read` |
| `crates/evm-runtime/src/states/rpc_control.rs` | `PrepareSourcesState` | Reusable `rpc.control` source preparation and responsiveness preflight | none directly; shared contract used by semantic and EVM write runtimes |
| `crates/evm-runtime/src/states/price.rs` | `read_evm_oracle_unit_price` | Reusable control-plane-backed EVM oracle price reads for valuation source execution | none directly |
| `crates/evm-runtime/src/states/contract_set.rs` | `LoadCompiledContractSetState`, `DeployContractSetState`, `WaitForContractSetReceiptsState`, `CollectDeployedContractSetState`, `WriteDeployedContractSetState` | Reusable generic contract-set deployment states that record public signed transaction intent metadata, keep raw transaction capabilities in protected artifacts, and broadcast idempotently through `rpc.control` | `evm_deploy_contract_set` |
| `crates/evm-runtime/src/states/write.rs` | `NixArtifactToEvmContractState`, `EvmDeployState`, `EvmConfigureState`, `EvmValidateState` | Reusable contract artifact adaptation and deploy/configure/validate states routed through `rpc.control`; write states require `signing_key_env`, record public signed transaction intent metadata, and keep raw transaction capabilities in protected artifacts before broadcast | `evm_contract_from_nix`, `evm_deploy`, `evm_configure`, `evm_validate` |

## Intentional Op-Local Production States

These are the current allowed exceptions to the shared-state rule. They stay op-local because they
assemble domain-specific outputs rather than reusable execution primitives.

| Owner | State type | Why still local |
|---|---|---|
| `crates/ops/proof-op` | `ProofWorkflowOperation` | Typed proof program assembly; executable proof state contracts live in `crates/collectors/proof` and are certified before runtime |

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
