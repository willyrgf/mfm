# Thin-Layer Alignment Audit

Date: 2026-02-14
Status: Audit + Option A implementation (active, major milestones completed)

## 0. Implementation Update (Option A)

This document started as a pure audit. Since then, Option A implementation has landed in multiple phases and several findings below are now fully addressed:

- `B1` keystore admin commands (`import/list/delete`) were moved to run-backed ops:
  - `bin/cli/src/commands/keystore/import.rs:103`
  - `bin/cli/src/commands/keystore/list.rs:83`
  - `bin/cli/src/commands/keystore/delete.rs:64`
  - `crates/ops/keystore-admin-op/src/lib.rs:113`
- `B6` reusable state-library adoption was advanced with shared keystore admin states in `ops/common`:
  - `crates/ops/common/src/states/mod.rs:2`
  - `crates/ops/common/src/states/keystore_admin.rs`
- `B4/B5` ambient IO was moved out of keystore admin/tx and signed deploy execution states into a dedicated local IO transport:
  - `crates/ops/common/src/local_io.rs`
  - `crates/app/src/lib.rs`
  - `crates/ops/keystore-tx-op/src/lib.rs`
  - `crates/ops/common/src/states/keystore_admin.rs`
  - `crates/ops/evm-write-op/src/lib.rs`
- `B2` portfolio snapshot orchestration moved into op layer; app now starts op and reads typed report:
  - `crates/ops/portfolio-tracker-op/src/lib.rs`
  - `crates/app/src/lib.rs`
- `B2` deploy/configure/validate template assembly moved behind a composed op (`evm_deploy_configure_validate`) registered in app:
  - `crates/ops/evm-deploy-configure-validate-op/src/lib.rs`
  - `crates/app/src/lib.rs`

Current residual work is mainly broader `B6` convergence: increasing reusable shared states beyond keystore flows so more operations compose common state primitives by default.

Note: sections 4-7 below capture the original audit snapshot. Use this section (`0`) as the source of truth for what has already been refactored.

## 1. Contract Baseline

The audit scored current implementation against these explicit contract statements:

- `REDESIGN.md:52` says binaries are thin wrappers that start/resume runs.
- `REDESIGN.md:59` says everything executes inside a state machine.
- `REDESIGN.md:220` says binaries are thin wrappers.
- `REDESIGN.md:527` says ops expand into state graphs.
- `REDESIGN.md:830` and `REDESIGN.md:836` move state handlers to `IoProvider` and move CLI/REST to start/resume surfaces.
- `ARCHITECTURE.md:195` says `bin/cli` and `bin/rest-api` are thin wrappers.
- `ENFORCE_BINS_THIN_LAYER.md:7` says binaries are thin wrappers.
- `ENFORCE_BINS_THIN_LAYER.md:8` says business execution must happen inside state-machine runs.

## 2. Audit Rubric

Boundary tags used in findings:

- `B1`: Business logic in binary layer (`bin/cli` or `bin/rest-api`).
- `B2`: Business workflow logic in app adapter layer (`crates/app`).
- `B3`: Operation packaging/ownership ambiguity (ops boundary unclear).
- `B4`: Ambient IO in state/business path (`std::fs`, direct env/stdin/password prompt) where IO abstraction should own execution effects.
- `B5`: Secret handling risk at boundaries (env/password prompt/file path handling outside intended abstraction).
- `B6`: Reusable state-library gap (shared state primitives exist but are not yet the dominant composition unit for ops).

Status labels:

- `Aligned`
- `Partially aligned`
- `Not aligned`
- `Contract gap` (flow is run-backed but still violates execution-contract details)

## 3. Coverage

Audited surfaces:

- CLI command surfaces: 13
- REST routes: 9
- Feature IDs: 7
- Registered runtime ops: 10
- Shared reusable state implementations in `ops/common`: 4
- Op-local state implementations across op crates: 16

Primary entry/registry evidence:

- CLI command roots: `bin/cli/src/commands/mod.rs:42`, `bin/cli/src/commands/keystore/mod.rs:12`, `bin/cli/src/commands/run/mod.rs:13`, `bin/cli/src/commands/portfolio/mod.rs:8`
- REST route map: `bin/rest-api/src/lib.rs:145`
- Feature registry: `crates/app/src/lib.rs:1163`
- Runtime op registry: `crates/app/src/lib.rs:378`

## 4. Conformance Matrix

### 4.1 CLI Commands

| CLI Surface | Execution Path Today | Run-backed | State-composed | Status | Evidence |
|---|---|---:|---:|---|---|
| `mfm keystore import` | CLI command reads secrets/input, unlocks keystore, imports key directly | No | No | Not aligned (`B1`) | `bin/cli/src/commands/keystore/import.rs:78`, `bin/cli/src/commands/keystore/import.rs:114`, `bin/cli/src/commands/keystore/import.rs:127`, `bin/cli/src/support/keystore_manager.rs:31`, `bin/cli/src/support/keystore_manager.rs:71` |
| `mfm keystore list` | CLI directly unlocks keystore and calls `list_keys` | No | No | Not aligned (`B1`) | `bin/cli/src/commands/keystore/list.rs:62`, `bin/cli/src/commands/keystore/list.rs:68`, `bin/cli/src/support/keystore_manager.rs:31` |
| `mfm keystore delete` | CLI directly resolves key and calls `delete_key` | No | No | Not aligned (`B1`) | `bin/cli/src/commands/keystore/delete.rs:50`, `bin/cli/src/commands/keystore/delete.rs:59`, `bin/cli/src/commands/keystore/delete.rs:117` |
| `mfm keystore tx-sign` | CLI wrapper -> SDK single-op helper -> `keystore_tx_sign` op | Yes | Yes | Contract gap (`B4`,`B5`) | `bin/cli/src/commands/keystore/tx_sign.rs:117`, `crates/sdk/src/unstable.rs:885`, `crates/ops/keystore-tx-op/src/lib.rs:113`, `crates/ops/keystore-tx-op/src/lib.rs:260`, `crates/ops/keystore-tx-op/src/lib.rs:516` |
| `mfm keystore tx-send-raw` | CLI wrapper -> SDK single-op helper -> `keystore_tx_send_raw` op | Yes | Yes | Contract gap (`B4`) | `bin/cli/src/commands/keystore/tx_send_raw.rs:56`, `crates/sdk/src/unstable.rs:885`, `crates/ops/keystore-tx-op/src/lib.rs:191`, `crates/ops/keystore-tx-op/src/lib.rs:341`, `crates/ops/keystore-tx-op/src/lib.rs:356` |
| `mfm portfolio snapshot` | CLI wrapper -> feature catalog -> app service feature helper -> run start | Yes | Partial | Partially aligned (`B2`) | `bin/cli/src/commands/portfolio/snapshot.rs:50`, `crates/app/src/lib.rs:1371`, `crates/app/src/lib.rs:695`, `crates/app/src/lib.rs:785` |
| `mfm run start` | CLI wrapper -> `AppServices::start_run` | Yes | Depends on target op | Aligned | `bin/cli/src/commands/run/start.rs:46`, `crates/app/src/lib.rs:471` |
| `mfm run pipeline start` | CLI wrapper -> pipeline request -> `AppServices::start_run` | Yes | Yes (by selected ops) | Aligned | `bin/cli/src/commands/run/pipeline.rs:94`, `bin/cli/src/commands/run/pipeline.rs:100`, `crates/app/src/lib.rs:471` |
| `mfm run pipeline deploy-configure-validate` | CLI wrapper -> app template helper -> pipeline generated in app | Yes | Partial | Partially aligned (`B2`) | `bin/cli/src/commands/run/pipeline.rs:141`, `crates/app/src/lib.rs:682`, `crates/app/src/lib.rs:1057` |
| `mfm run resume` | CLI wrapper -> `AppServices::resume_run` | Yes | N/A | Aligned | `bin/cli/src/commands/run/resume.rs:35`, `crates/app/src/lib.rs:528` |
| `mfm run status` | CLI wrapper -> `AppServices::run_status` | N/A | N/A | Aligned | `bin/cli/src/commands/run/status.rs:35`, `crates/app/src/lib.rs:557` |
| `mfm run events` | CLI wrapper -> `AppServices::run_events` | N/A | N/A | Aligned | `bin/cli/src/commands/run/events.rs:43`, `crates/app/src/lib.rs:638` |
| `mfm run artifacts get` | CLI wrapper -> artifact store read helper | N/A | N/A | Aligned | `bin/cli/src/commands/run/artifacts.rs:40`, `crates/app/src/lib.rs:869` |

### 4.2 REST Routes

| Route | Execution Path Today | Status | Evidence |
|---|---|---|---|
| `GET /v1/health` | Infra probe only | Aligned | `bin/rest-api/src/lib.rs:196` |
| `GET /v1/ready` | Store readiness checks only | Aligned | `bin/rest-api/src/lib.rs:200` |
| `GET /v1/features` | Returns feature descriptors | Aligned | `bin/rest-api/src/lib.rs:339` |
| `POST /v1/features/:feature_id/execute` | Generic wrapper -> `FeatureCatalog::execute` | Mixed (depends on feature) | `bin/rest-api/src/lib.rs:347`, `crates/app/src/lib.rs:1371` |
| `POST /v1/runs/start` | Wrapper -> `AppServices::start_run` | Aligned | `bin/rest-api/src/lib.rs:245`, `bin/rest-api/src/lib.rs:251`, `crates/app/src/lib.rs:471` |
| `POST /v1/runs/:run_id/resume` | Wrapper -> `AppServices::resume_run` | Aligned | `bin/rest-api/src/lib.rs:258`, `bin/rest-api/src/lib.rs:263`, `crates/app/src/lib.rs:528` |
| `GET /v1/runs/:run_id/status` | Wrapper -> `AppServices::run_status` | Aligned | `bin/rest-api/src/lib.rs:270`, `bin/rest-api/src/lib.rs:275`, `crates/app/src/lib.rs:557` |
| `GET /v1/runs/:run_id/events` | Wrapper -> `AppServices::run_events` | Aligned | `bin/rest-api/src/lib.rs:282`, `bin/rest-api/src/lib.rs:292`, `crates/app/src/lib.rs:638` |
| `GET /v1/artifacts/:artifact_id` | Wrapper -> `AppServices::artifact_get` | Aligned | `bin/rest-api/src/lib.rs:299`, `bin/rest-api/src/lib.rs:304`, `crates/app/src/lib.rs:677` |

### 4.3 Feature IDs

| Feature ID | Implementation Owner Today | Run-backed | Status | Evidence |
|---|---|---:|---|---|
| `run.start` | Generic app run service | Yes | Aligned | `crates/app/src/lib.rs:1166`, `crates/app/src/lib.rs:1383`, `crates/app/src/lib.rs:471` |
| `run.resume` | Generic app run service | Yes | Aligned | `crates/app/src/lib.rs:1167`, `crates/app/src/lib.rs:1388`, `crates/app/src/lib.rs:528` |
| `run.status` | Generic app query service | N/A | Aligned | `crates/app/src/lib.rs:1168`, `crates/app/src/lib.rs:1393`, `crates/app/src/lib.rs:557` |
| `run.events` | Generic app query service | N/A | Aligned | `crates/app/src/lib.rs:1169`, `crates/app/src/lib.rs:1398`, `crates/app/src/lib.rs:638` |
| `artifact.get` | Generic app artifact service | N/A | Aligned | `crates/app/src/lib.rs:1170`, `crates/app/src/lib.rs:1407`, `crates/app/src/lib.rs:677` |
| `pipeline.deploy_configure_validate.start` | App helper builds pipeline template in adapter layer | Yes | Partially aligned (`B2`) | `crates/app/src/lib.rs:1171`, `crates/app/src/lib.rs:1412`, `crates/app/src/lib.rs:682`, `crates/app/src/lib.rs:1057` |
| `portfolio.snapshot` | App helper performs domain validation/normalization/aggregation before and after run | Yes | Partially aligned (`B2`) | `crates/app/src/lib.rs:1176`, `crates/app/src/lib.rs:1417`, `crates/app/src/lib.rs:695`, `crates/app/src/lib.rs:843` |

## 5. Registered Ops Matrix

| Registered Op | State-Graph Expansion | Boundary Status | Evidence |
|---|---|---|---|
| `proof` | Yes | Aligned | `crates/app/src/lib.rs:380`, `crates/ops/proof-op/src/lib.rs:116`, `crates/ops/proof-op/src/lib.rs:132` |
| `keystore_tx_sign` | Yes | Contract gap (`B4`,`B5`) | `crates/app/src/lib.rs:381`, `crates/ops/keystore-tx-op/src/lib.rs:113`, `crates/ops/keystore-tx-op/src/lib.rs:276`, `crates/ops/keystore-tx-op/src/lib.rs:516`, `crates/ops/keystore-tx-op/src/lib.rs:548` |
| `keystore_tx_send_raw` | Yes | Contract gap (`B4`) | `crates/app/src/lib.rs:382`, `crates/ops/keystore-tx-op/src/lib.rs:191`, `crates/ops/keystore-tx-op/src/lib.rs:356` |
| `evm_read` | Yes | Aligned | `crates/app/src/lib.rs:383`, `crates/ops/evm-read-op/src/lib.rs:61`, `crates/ops/evm-read-op/src/lib.rs:80` |
| `evm_contract_from_nix` | Yes | Aligned | `crates/app/src/lib.rs:384`, `crates/ops/evm-write-op/src/lib.rs:1294`, `crates/ops/evm-write-op/src/lib.rs:1324` |
| `evm_deploy` | Yes | Contract gap (`B4`,`B5`) | `crates/app/src/lib.rs:385`, `crates/ops/evm-write-op/src/lib.rs:1416`, `crates/ops/evm-write-op/src/lib.rs:1454`, `crates/ops/evm-write-op/src/lib.rs:1553`, `crates/ops/evm-write-op/src/lib.rs:965` |
| `evm_configure` | Yes | Aligned | `crates/app/src/lib.rs:386`, `crates/ops/evm-write-op/src/lib.rs:1596`, `crates/ops/evm-write-op/src/lib.rs:1630` |
| `evm_validate` | Yes | Aligned | `crates/app/src/lib.rs:387`, `crates/ops/evm-write-op/src/lib.rs:1812`, `crates/ops/evm-write-op/src/lib.rs:1847` |
| `portfolio_tracker` | Yes | Aligned (op itself) | `crates/app/src/lib.rs:388`, `crates/ops/portfolio-tracker-op/src/lib.rs:191`, `crates/ops/portfolio-tracker-op/src/lib.rs:211` |
| `nix_app` | Yes | Aligned | `crates/app/src/lib.rs:389`, `crates/ops/nix-app-op/src/lib.rs:84`, `crates/ops/nix-app-op/src/lib.rs:100` |

Additional packaging observation:

- `mfm-op-keystore` is a re-export wrapper, not an operation implementation: `crates/ops/keystore-op/src/lib.rs:8`, `crates/ops/keystore-op/README.md:3`.

Reusable state-library observation:

- A shared state library location exists (`crates/ops/common/src/states`) and is documented as reusable (`crates/ops/common/src/lib.rs:1`, `crates/ops/common/README.md:3`, `REDESIGN.md:857`).
- Current reuse is still narrow (4 shared state impls) relative to op-local state implementations (16), so ops are not yet predominantly composed by reusable state primitives.

## 6. Findings (Ordered by Severity)

### High

1. `B1` Keystore admin business logic is still in CLI binaries.
- Impact: `keystore import`, `keystore list`, `keystore delete` bypass run/event/state-machine execution, violating thin-wrapper + run-backed model.
- Evidence: `bin/cli/src/commands/keystore/import.rs:114`, `bin/cli/src/commands/keystore/list.rs:62`, `bin/cli/src/commands/keystore/delete.rs:50`, `bin/cli/src/support/keystore_manager.rs:31`.
- Risk: inconsistent observability/replay story and duplicated policy enforcement at transport layer.

2. `B2` Portfolio feature orchestration is implemented in app adapter layer.
- Impact: domain behavior (RPC precondition, address normalization, token merge/sort, response extraction from snapshots) exists outside ops/state graph.
- Evidence: `crates/app/src/lib.rs:695`, `crates/app/src/lib.rs:703`, `crates/app/src/lib.rs:719`, `crates/app/src/lib.rs:767`, `crates/app/src/lib.rs:797`.
- Risk: split ownership and higher chance of logic drift between feature and operation contracts.

### Medium

3. `B2` Deploy/configure/validate template construction lives in app layer.
- Impact: template semantics are not represented as a first-class op contract.
- Evidence: `crates/app/src/lib.rs:682`, `crates/app/src/lib.rs:686`, `crates/app/src/lib.rs:1057`, `bin/cli/src/commands/run/pipeline.rs:143`.
- Risk: composition rules duplicated at adapter surface instead of ops layer.

4. `B4`,`B5` Keystore tx ops are run-backed but still use ambient FS/env/stdin in execution path.
- Impact: state handlers/helpers perform direct file reads/writes and prompt/password env access instead of IO abstraction.
- Evidence: `crates/ops/keystore-tx-op/src/lib.rs:356`, `crates/ops/keystore-tx-op/src/lib.rs:427`, `crates/ops/keystore-tx-op/src/lib.rs:537`, `crates/ops/keystore-tx-op/src/lib.rs:548`, `crates/ops/common/src/keystore_tx.rs:263`.
- Risk: replay/portability/testing complexity and secret-boundary coupling to process environment.

5. `B4`,`B5` `evm_deploy` signed path reads signing key directly from environment.
- Impact: side-effect path depends on ambient env secret retrieval.
- Evidence: `crates/ops/evm-write-op/src/lib.rs:1553`, `crates/ops/evm-write-op/src/lib.rs:965`.
- Risk: operational inconsistency across execution environments and weaker abstraction boundary.

6. `B6` Reusable state-library pattern is under-enforced and under-adopted.
- Impact: many workflows are still implemented with op-local bespoke states instead of primarily composing shared state primitives.
- Evidence: shared reusable state implementations are concentrated in `crates/ops/common/src/states/evm.rs` (4 `impl State for` blocks), while op crates define 16 op-local `impl State for` blocks (`crates/ops/evm-write-op/src/lib.rs:1364`, `crates/ops/keystore-tx-op/src/lib.rs:260`, `crates/ops/portfolio-tracker-op/src/lib.rs:323`, `crates/ops/proof-op/src/lib.rs:192`, etc.).
- Risk: duplicated state patterns, inconsistent semantics, and slower evolution toward “operations only reuse states.”

### Low

7. `B3` Keystore ops packaging is semantically ambiguous.
- Impact: `mfm-op-keystore` name implies operation behavior but currently only re-exports core keystore types.
- Evidence: `crates/ops/keystore-op/src/lib.rs:8`, `crates/ops/keystore-op/README.md:3`.
- Risk: encourages future direct usage from wrappers instead of run-backed ops.

## 7. Proposal Options (No Code Changes in This Audit)

### Option A: Strict Ops-Only Target

- Description: all business workflows move to op/state layers; binaries and `crates/app` remain transport/run-control only.
- Scope includes: keystore admin flows, portfolio snapshot orchestration, pipeline template orchestration, ambient IO cleanup in state execution paths, and formalization/adoption of a reusable state library so operations primarily compose shared states.
- Benefits: strongest alignment with redesign contract; single ownership of business behavior.
- Tradeoffs: highest migration size and coordination cost.
- Estimated effort/risk: `XL` effort, `Medium-High` migration risk, best long-term maintainability.

### Option B: Wrapper-Only Strictness

- Description: enforce thinness only in `bin/cli` and `bin/rest-api`; keep app feature orchestration helpers.
- Scope includes: move only CLI direct keystore admin logic to run-backed path.
- Benefits: smaller blast radius, faster delivery.
- Tradeoffs: leaves `B2` architecture debt in `crates/app`.
- Estimated effort/risk: `M-L` effort, `Low-Medium` risk, moderate long-term debt.

### Option C: Hybrid Phased

- Description: close highest-severity wrapper leaks first, then move app feature orchestration and ambient IO boundary gaps.
- Suggested phases:
  1. Keystore admin wrappers -> run-backed path.
  2. Portfolio and deploy/configure/validate feature orchestration -> ops.
  3. Ambient IO normalization inside affected ops.
- Benefits: incremental risk reduction with clear milestones.
- Tradeoffs: temporary mixed architecture during transition.
- Estimated effort/risk: `L-XL` cumulative effort, `Medium` risk, best balance for staged delivery.

## 8. Decision Checklist for Implementation Phase

Required decisions before coding:

1. Choose target option (`A`, `B`, or `C`).
2. Decide if app feature IDs remain stable while internal ownership moves to ops.
3. Decide whether keystore admin commands keep current CLI flags/output schema exactly or permit additive fields.
4. Decide IO abstraction policy for secrets/files in state execution paths.
5. Choose migration order for test suites (op tests first vs CLI parity first).
6. Define acceptance gate for “thin-layer complete” (which `B*` tags must be zero).

## 9. Suggested Acceptance Criteria (for Later Refactor PRs)

1. No business workflow logic under `bin/cli` or `bin/rest-api`.
2. Feature-specific business orchestration removed from `crates/app` or explicitly justified as transport-only.
3. All business execution paths run-backed and state-graph represented.
4. State execution paths avoid ambient IO except where explicitly modeled/approved.
5. Existing CLI/REST output contracts remain backward compatible unless explicitly versioned.
