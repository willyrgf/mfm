# Aave-Origin Promotion Plan

Status: implemented wrapper-heavy adopter for the shared authored-config framework

Last updated: 2026-04-01

## Summary

This document records the investigation, implementation decisions, and landed migration shape for promoting the
Aave-origin wrapper flow into the shared authored-config framework described in
`RFC_REFACTOR_EDGES_N_CONFIG.md`.

Primary conclusion:

- Aave-origin now uses explicit authored, canonical, built, and execute boundaries.
- Unlike `publish-docs`, Aave-origin has a real deterministic post-canonicalization build stage:
  the current wrapper already enforces a fixed fetch/compile/deploy/adapt topology with stable
  artifact contracts.
- The landed backend still preserves the existing external Origin tools and moves orchestration
  behind internal MFM ops instead of rewriting Origin compile or deploy semantics in Rust.
- The current compatibility backend still supports exactly one packaged source pin; canonical source
  config is therefore explicit and validated rather than silently implied by wrapper packaging.

## Current State

The primary parity flow now runs through `aave_v3_origin_stack` in
`tests/integration/tests/parity_aave_v3_reth_scenario.rs`.

That promoted root op still expands into the current compatibility backend:

1. `nix_app(fetch_origin)` runs `mfm-aave-v3-origin-fetch`
2. `nix_app(compile_origin)` runs `mfm-aave-v3-origin-compile`
3. `nix_app(deploy_origin_stack)` runs `mfm-aave-v3-origin-deploy`
4. `aave_v3_origin_adapt_deploy` converts Origin deploy output into the standard deploy manifest
5. later scenario steps use generic `evm_configure` and `evm_validate`

A raw wrapper-oriented parity path is still retained as separate compatibility coverage in the same
test file so the legacy backend remains observable during migration.

The shell tools live in `nixfied/project/aave-origin-tools.nix` and are exposed to developers and
tasks through `nixfied/project/conf.nix` and `nixfied/local/default.nix`.

Current stable wrapper contracts:

| Step | Program | Output kind | Notes |
|---|---|---|---|
| Fetch | `mfm-aave-v3-origin-fetch` | `aave_v3_origin_source_v1` | Emits repo URL, pinned commit, and local path |
| Compile | `mfm-aave-v3-origin-compile` | `aave_v3_origin_compile_manifest_v1` | Copies pinned source, rewrites `foundry.toml`, builds, and shapes a compile manifest with `jq` |
| Deploy | `mfm-aave-v3-origin-deploy` | `aave_v3_origin_deploy_output_v1` | Copies pinned source, rewrites `foundry.toml`, injects patched Solidity and Foundry script, deploys via RPC, then shapes output with `jq` |
| Adapt | `aave_v3_origin_adapt_deploy` | `aave_v3_deploy_manifest_v1` | Already implemented as a thin op over shared Aave state logic |

Current secret/runtime env contract:

- `MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY`
- `MFM_EVM_RPC_URL`
- optional actor and amount env vars:
  - `MFM_AAVE_V3_ORIGIN_SUPPLIER`
  - `MFM_AAVE_V3_ORIGIN_BORROWER`
  - `MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT`
  - `MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT`

## What Is Already Well Placed

These pieces should remain reusable and should not move into binaries or wrappers:

- `crates/states/aave-v3/src/manifest.rs`
  - compile-manifest, origin-output, deploy-manifest, and config-report types
  - structural validation for `aave_v3_origin_compile_manifest_v1`
  - structural validation for `aave_v3_origin_deploy_output_v1`
- `crates/states/aave-v3/src/states.rs`
  - deploy/configure runtime states
  - manifest loading, deployment, receipt waiting, output collection, and report emission
  - deploy-output adaptation state
- `crates/ops/aave-v3-origin-adapt-op`
  - thin adaptation op over the shared Aave state implementation
- `crates/ops/nix-app-op`
  - generic exec transport op; keep this as reusable plumbing rather than embedding Aave semantics

## Fat Edges Still Present

The remaining thick edge is wrapper-owned orchestration, not missing runtime substrate.

Current wrapper-owned responsibilities that should not remain shell-only:

| Concern | Current owner | Problem | Target owner |
|---|---|---|---|
| Source pin/provenance | `aave-origin-tools.nix` | Repo pin is only implicit in tool packaging and output JSON | Typed authored/canonical config plus build report |
| Compile/deploy input defaults | deploy shell script env defaults | Semantic defaults live in shell instead of typed config | Workflow-family canonicalization |
| Execution topology | raw parity pipeline using `nix_app` steps | Callers must know fetch/compile/deploy/adapt wiring | Internal execute op |
| Result shaping | `jq` in compile/deploy tools | Artifact contracts are stable but assembly is shell-local | Typed library/build helpers and stable reports |
| Wrapper promotion path | `nixfied/local/default.nix` task apps | Exposes tools directly instead of an internal MFM family boundary | Keep as compatibility surface only |

Responsibilities that should remain external in the first migration slice:

- running Foundry against the upstream Origin repo
- patching upstream source files needed by the current Foundry deployment path
- upstream-specific Solidity deployment script behavior

Those can stay behind `nix_app` in Phase 1 and Phase 2.

## BuiltConfig Decision

Aave-origin should use a `BuiltConfig` boundary.

Rationale:

- The workflow has a deterministic fixed topology after canonicalization:
  `fetch -> compile -> deploy -> adapt`
- The current wrapper tools already emit stable typed artifacts at each stage.
- The execution boundary is meaningful even while the backend is still external Foundry execution.
- This is different from `publish-docs`, where the useful downstream artifact is a live planner
  result derived from current remote observations rather than a reusable compiled execution spec.

Built config for Aave-origin should represent an execution-ready phase-A plan, not a live
observation-driven planner snapshot.

## Target Architecture

### Workflow Family Name

Use `aave_v3_origin_stack` as the workflow-family name for the promoted wrapper path.

Planned public/internal root ops:

- `aave_v3_origin_stack_config_build`
- `aave_v3_origin_stack_execute`
- `aave_v3_origin_stack`

`aave_v3_origin_stack` is the compatibility root:

- canonical input composes `aave_v3_origin_stack_config_build -> aave_v3_origin_stack_execute`
- built input goes straight to `aave_v3_origin_stack_execute`

Current implementation note:

- the compatibility root now wires `aave_v3_origin_stack_execute` from the build child's
  `/built_config` planner payload
- the build child is authoritative for publication/report artifacts and for execute-child input
  materialization

Keep `aave_v3_origin_adapt_deploy` as a narrow helper op during migration.

### AuthoredConfig

Authoring should accept TOML or JSON and describe the semantic market-deploy intent, not raw shell
commands.

Required authored fields:

- `source.repo_url`
- `source.commit_sha`
- `network_id`
- `deploy_signing_key_env`
- `rpc_url_env`
- `supplier`
- `borrower`
- `usdc_supply_amount`
- `wbtc_collateral_amount`

Optional authored fields with fixed defaults:

- `control_scope = "shared"`
- `fetch_timeout_ms = 300000`
- `compile_timeout_ms = 600000`
- `deploy_timeout_ms = 600000`

Authoring rules:

- EVM addresses canonicalize through shared address normalization.
- Amounts are typed integers in base units.
- Env-bearing fields carry env variable names only, never secret values.

### CanonicalConfig

Canonical config must contain only deterministic typed values:

- pinned source identity
- normalized network and control-scope selection
- normalized addresses
- integer amount values
- non-secret env variable names needed at execution time
- timeout policy

Canonical config must not contain:

- local source paths
- discovered binary paths
- secret values
- ad hoc shell snippets

### BuiltConfig

Built config should be an execution-ready phase-A plan backed by the current external tools.

Required built fields:

- canonical source identity and scenario settings
- fixed step graph:
  - `fetch_origin`
  - `compile_origin`
  - `deploy_origin_stack`
  - `adapt_origin_deploy`
- per-step timeout values
- stable import/export port wiring
- stable external tool references for the compatibility backend
- stable env contract for the deploy step:
  - RPC URL env name
  - deploy signing key env name
  - actor/amount env names or inlined non-secret values where safe

Built config must not contain:

- actual env values
- dynamic RPC observations
- deploy receipts or runtime outputs

### Execute Boundary

`aave_v3_origin_stack_execute` should consume built config and, in the first backend, expand into:

1. `nix_app(fetch_origin)`
2. `nix_app(compile_origin)`
3. `nix_app(deploy_origin_stack)`
4. `aave_v3_origin_adapt_deploy`

The execute op should emit:

- raw fetch result artifact
- compile manifest artifact
- origin deploy output artifact
- final deploy manifest export
- stable execution report with artifact ids and exported context keys

## Migration Phases

### Phase 1

Create the workflow-family config boundary without changing the current runtime backend.

Changes:

- add `crates/aave-v3-origin-config`
- define authored, canonical, and built config types
- implement pure TOML/JSON parsing plus canonicalization
- implement pure lowering from canonical config to an external-tool-backed built config
- add equivalence tests for JSON/TOML authored inputs
- add determinism tests for built-config lowering

Do not:

- rewrite the Foundry compile or deploy behavior
- remove current shell tools
- remove raw compatibility parity coverage yet

### Phase 2

Introduce explicit internal op boundaries over the existing backend.

Changes:

- add `crates/ops/aave-v3-origin-op`
- implement `aave_v3_origin_stack_config_build`
- implement `aave_v3_origin_stack_execute`
- implement `aave_v3_origin_stack` compatibility root
- reuse shared publication states for canonical/built artifacts and stable build reports

Execution backend in this phase:

- keep `nix_app` for fetch/compile/deploy
- keep `aave_v3_origin_adapt_deploy` for final adaptation

### Phase 3

Migrate callers onto the internal family boundary.

Changes:

- update the primary parity tests to use `aave_v3_origin_stack` instead of raw `nix_app`
  orchestration
- add app feature entrypoints if needed
- keep existing Nix task apps as compatibility wrappers over the same backend

Compatibility rule:

- current Nix apps continue to work during migration
- the internal MFM family becomes the preferred API for new compositions

### Phase 4

Reduce shell ownership only where it creates real leverage.

Candidates:

- move compile-manifest shaping out of `jq` and into typed Rust helpers or a narrower exec wrapper
- replace deploy-output reshaping with typed runtime-side extraction
- evaluate whether parts of Origin deploy can transition from external Foundry orchestration to
  shared Aave runtime states or typed lowerings

Do not force this phase until earlier phases are stable.

## Tests and Acceptance Criteria

Required tests for eventual implementation:

- authored TOML and JSON equivalence for Aave-origin config
- canonicalization tests for address normalization and timeout defaults
- built-config determinism tests
- validation tests that secret env names are accepted but secret values are never persisted
- regression tests for compile-manifest and origin-output contract compatibility
- integration parity coverage proving the promoted op preserves the current `fetch -> compile -> deploy -> adapt` semantics

Minimum acceptance criteria:

- the current Aave parity scenario remains green
- `aave_v3_origin_compile_manifest_v1` output stays compatible with existing validators
- `aave_v3_origin_deploy_output_v1` output stays compatible with existing validators
- final deploy manifest matches the current adapted result contract

## Non-Negotiable Constraints

- Preserve `docs/design.md`
- Keep append-only and per-append atomicity guarantees intact
- Keep canonical JSON authoritative for hashed typed structures
- Keep secrets out of persisted artifacts, manifests, context snapshots, and user-facing surfaces
- Keep binaries thin
- Keep ops planning-only and states execution-only
- Prefer shared reusable Aave states and common publication helpers over new op-local execution code
