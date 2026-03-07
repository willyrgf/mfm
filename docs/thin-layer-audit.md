# Thin-Layer Alignment Audit

Date: 2026-02-14
Status: Historical checkpoint after Option A implementation. The later state-layer relocation is complete, so current canonical shared-state paths are under `crates/states/*`; path references below remain useful as point-in-time audit evidence.

## 0. Implementation Update (Option A Checkpoint)

This document now reflects landed Option A work, not just planned work.

Closed findings in this checkpoint:

- `B1` keystore admin commands (`import/list/delete`) are now run-backed ops from CLI wrappers:
  - `bin/cli/src/commands/keystore/import.rs:103`
  - `bin/cli/src/commands/keystore/list.rs:83`
  - `bin/cli/src/commands/keystore/delete.rs:64`
  - `crates/ops/keystore-admin-op/src/lib.rs:101`
- `B2` deploy/configure/validate template semantics moved behind a composed op:
  - `crates/ops/evm-deploy-configure-validate-op/src/lib.rs:26`
  - `crates/app/src/lib.rs:975`
- `B2` portfolio snapshot normalization/orchestration moved into the op expand/state graph:
  - `crates/ops/portfolio-tracker-op/src/lib.rs:302`
  - `crates/ops/portfolio-tracker-op/src/lib.rs:370`
  - `crates/app/src/lib.rs:706`
- `B4/B5` state execution paths for keystore tx and signed deploy now route side effects through local IO namespaces:
  - `crates/ops/keystore-tx-op/src/lib.rs:275`
  - `crates/ops/keystore-tx-op/src/lib.rs:350`
  - `crates/ops/evm-write-op/src/lib.rs:962`
  - `crates/ops/common/src/local_io.rs:47`
  - `crates/app/src/lib.rs:431`
- `B6` reusable shared states were expanded for keystore admin flows:
  - `crates/ops/common/src/states/keystore_admin.rs`
  - `crates/ops/keystore-admin-op/src/lib.rs:12`

Remaining focus:

- Broader `B6` convergence for EVM deploy/configure/validate and portfolio state reuse.
- Full parity validation gates across `check/test/ci basic/ci parity`.
- Add an architecture guard to prevent new ambient IO in state handlers outside local IO transport.

## 1. Contract Baseline

The audit scored implementation against these explicit statements:

- `docs/redesign.md` section `4.9 Thin transport boundaries` (CLI/REST are start/resume/query adapters).
- `docs/redesign.md` section `4.7 No ambient IO in state logic` (execution side effects route through IO provider).
- `docs/redesign.md` section `11. Operations, Pipelines, and IDs` (ops expand to deterministic state graphs).
- `docs/architecture.md` section `7. Thin-Binary Request Flow` (transport-only binary behavior).
- `AGENTS.md` section `Binary Boundary Enforcement` (no domain workflow logic in binaries).

## 2. Audit Rubric

Boundary tags:

- `B1`: Business logic in binary layer (`bin/cli` or `bin/rest-api`).
- `B2`: Business workflow logic in app adapter layer (`crates/app`).
- `B3`: Operation packaging/ownership ambiguity (ops boundary unclear).
- `B4`: Ambient IO in state/business path (`std::fs`, direct env/stdin/password prompt) where IO abstraction should own execution effects.
- `B5`: Secret handling risk at boundaries.
- `B6`: Reusable state-library gap (shared state primitives exist but are not yet the dominant composition unit for ops).

Status labels:

- `Aligned`
- `Partially aligned`
- `Not aligned`
- `Contract gap`
- `Packaging gap`

## 3. Coverage

Audited surfaces:

- CLI command surfaces: 13
- REST routes: 9
- Feature IDs: 7
- Registered runtime ops: 14
- Shared reusable state implementations in `ops/common`: 7
- Op-local state implementations across op crates: 16

Primary entry/registry evidence:

- CLI command roots: `bin/cli/src/commands/mod.rs:42`, `bin/cli/src/commands/keystore/mod.rs:12`, `bin/cli/src/commands/run/mod.rs:12`, `bin/cli/src/commands/portfolio/mod.rs:7`
- REST route map: `bin/rest-api/src/lib.rs:146`
- Feature registry: `crates/app/src/lib.rs:1072`
- Runtime op registry: `crates/app/src/lib.rs:384`

## 4. Conformance Matrix

### 4.1 CLI Commands

| CLI Surface | Execution Path Today | Run-backed | State-composed | Status | Evidence |
|---|---|---:|---:|---|---|
| `mfm keystore import` | CLI wrapper -> single-op helper -> `keystore_import` op -> shared keystore admin state -> local IO | Yes | Yes | Aligned | `bin/cli/src/commands/keystore/import.rs:103`, `crates/ops/keystore-admin-op/src/lib.rs:101`, `crates/ops/common/src/states/keystore_admin.rs:138`, `crates/ops/common/src/local_io.rs:48` |
| `mfm keystore list` | CLI wrapper -> single-op helper -> `keystore_list` op -> shared keystore admin state -> local IO | Yes | Yes | Aligned | `bin/cli/src/commands/keystore/list.rs:83`, `crates/ops/keystore-admin-op/src/lib.rs:160`, `crates/ops/common/src/states/keystore_admin.rs:215`, `crates/ops/common/src/local_io.rs:49` |
| `mfm keystore delete` | CLI wrapper -> single-op helper -> `keystore_delete` op -> shared keystore admin state -> local IO | Yes | Yes | Aligned | `bin/cli/src/commands/keystore/delete.rs:64`, `crates/ops/keystore-admin-op/src/lib.rs:229`, `crates/ops/common/src/states/keystore_admin.rs:291`, `crates/ops/common/src/local_io.rs:50` |
| `mfm keystore tx-sign` | CLI wrapper -> single-op helper -> `keystore_tx_sign` op -> local IO signer/write path | Yes | Yes | Aligned | `bin/cli/src/commands/keystore/tx_sign.rs:117`, `crates/ops/keystore-tx-op/src/lib.rs:260`, `crates/ops/keystore-tx-op/src/lib.rs:275`, `crates/ops/common/src/local_io.rs:51` |
| `mfm keystore tx-send-raw` | CLI wrapper -> single-op helper -> `keystore_tx_send_raw` op -> local file read + RPC path via IO | Yes | Yes | Aligned | `bin/cli/src/commands/keystore/tx_send_raw.rs:56`, `crates/ops/keystore-tx-op/src/lib.rs:335`, `crates/ops/keystore-tx-op/src/lib.rs:350`, `crates/ops/common/src/local_io.rs:52` |
| `mfm portfolio snapshot` | CLI wrapper -> feature catalog -> app helper starts `portfolio_tracker` op -> typed report extraction | Yes | Yes | Aligned | `bin/cli/src/commands/portfolio/snapshot.rs:50`, `crates/app/src/lib.rs:706`, `crates/ops/portfolio-tracker-op/src/lib.rs:370`, `crates/ops/portfolio-tracker-op/src/lib.rs:603` |
| `mfm run start` | CLI wrapper -> `AppServices::start_run` | Yes | Depends on target op | Aligned | `bin/cli/src/commands/run/start.rs:47`, `crates/app/src/lib.rs:482` |
| `mfm run pipeline start` | CLI wrapper -> pipeline request -> `AppServices::start_run` | Yes | Yes (by selected ops) | Aligned | `bin/cli/src/commands/run/pipeline.rs:101`, `crates/app/src/lib.rs:482` |
| `mfm run pipeline deploy-configure-validate` | CLI wrapper -> app helper -> composed `evm_deploy_configure_validate` op | Yes | Yes | Aligned | `bin/cli/src/commands/run/pipeline.rs:143`, `crates/app/src/lib.rs:693`, `crates/app/src/lib.rs:975`, `crates/ops/evm-deploy-configure-validate-op/src/lib.rs:26` |
| `mfm run resume` | CLI wrapper -> `AppServices::resume_run` | Yes | N/A | Aligned | `bin/cli/src/commands/run/resume.rs:36`, `crates/app/src/lib.rs:541` |
| `mfm run status` | CLI wrapper -> `AppServices::run_status` | N/A | N/A | Aligned | `bin/cli/src/commands/run/status.rs:36`, `crates/app/src/lib.rs:570` |
| `mfm run events` | CLI wrapper -> `AppServices::run_events` | N/A | N/A | Aligned | `bin/cli/src/commands/run/events.rs:44`, `crates/app/src/lib.rs:649` |
| `mfm run artifacts get` | CLI wrapper -> app artifact helper | N/A | N/A | Aligned | `bin/cli/src/commands/run/artifacts.rs:42`, `crates/app/src/lib.rs:787` |

### 4.2 REST Routes

| Route | Execution Path Today | Status | Evidence |
|---|---|---|---|
| `GET /v1/health` | Infra probe only | Aligned | `bin/rest-api/src/lib.rs:146`, `bin/rest-api/src/lib.rs:196` |
| `GET /v1/ready` | Store readiness checks only | Aligned | `bin/rest-api/src/lib.rs:147`, `bin/rest-api/src/lib.rs:201` |
| `GET /v1/features` | Returns feature descriptors | Aligned | `bin/rest-api/src/lib.rs:148`, `bin/rest-api/src/lib.rs:340` |
| `POST /v1/features/:feature_id/execute` | Generic wrapper -> `FeatureCatalog::execute` | Mixed (depends on feature) | `bin/rest-api/src/lib.rs:149`, `bin/rest-api/src/lib.rs:348`, `crates/app/src/lib.rs:1279` |
| `POST /v1/runs/start` | Wrapper -> `AppServices::start_run` | Aligned | `bin/rest-api/src/lib.rs:150`, `bin/rest-api/src/lib.rs:246`, `crates/app/src/lib.rs:482` |
| `POST /v1/runs/:run_id/resume` | Wrapper -> `AppServices::resume_run` | Aligned | `bin/rest-api/src/lib.rs:151`, `bin/rest-api/src/lib.rs:259`, `crates/app/src/lib.rs:541` |
| `GET /v1/runs/:run_id/status` | Wrapper -> `AppServices::run_status` | Aligned | `bin/rest-api/src/lib.rs:152`, `bin/rest-api/src/lib.rs:271`, `crates/app/src/lib.rs:570` |
| `GET /v1/runs/:run_id/events` | Wrapper -> `AppServices::run_events` | Aligned | `bin/rest-api/src/lib.rs:153`, `bin/rest-api/src/lib.rs:287`, `crates/app/src/lib.rs:649` |
| `GET /v1/artifacts/:artifact_id` | Wrapper -> `AppServices::artifact_get` | Aligned | `bin/rest-api/src/lib.rs:154`, `bin/rest-api/src/lib.rs:300`, `crates/app/src/lib.rs:689` |

### 4.3 Feature IDs

| Feature ID | Implementation Owner Today | Run-backed | Status | Evidence |
|---|---|---:|---|---|
| `run.start` | Generic app run service | Yes | Aligned | `crates/app/src/lib.rs:1074`, `crates/app/src/lib.rs:1090`, `crates/app/src/lib.rs:1291` |
| `run.resume` | Generic app run service | Yes | Aligned | `crates/app/src/lib.rs:1075`, `crates/app/src/lib.rs:1127`, `crates/app/src/lib.rs:1297` |
| `run.status` | Generic app query service | N/A | Aligned | `crates/app/src/lib.rs:1076`, `crates/app/src/lib.rs:1147`, `crates/app/src/lib.rs:1301` |
| `run.events` | Generic app query service | N/A | Aligned | `crates/app/src/lib.rs:1077`, `crates/app/src/lib.rs:1170`, `crates/app/src/lib.rs:1306` |
| `artifact.get` | Generic app artifact service | N/A | Aligned | `crates/app/src/lib.rs:1078`, `crates/app/src/lib.rs:1194`, `crates/app/src/lib.rs:1315` |
| `pipeline.deploy_configure_validate.start` | App helper builds pipeline envelope that targets composed op | Yes | Aligned | `crates/app/src/lib.rs:693`, `crates/app/src/lib.rs:975`, `crates/app/src/lib.rs:1213`, `crates/app/src/lib.rs:1323` |
| `portfolio.snapshot` | App helper starts `portfolio_tracker` op and returns typed report fields | Yes | Aligned | `crates/app/src/lib.rs:706`, `crates/app/src/lib.rs:1241`, `crates/app/src/lib.rs:1328`, `crates/ops/portfolio-tracker-op/src/lib.rs:302` |

## 5. Registered Ops Matrix

| Registered Op | State-Graph Expansion | Boundary Status | Evidence |
|---|---|---|---|
| `proof` | Yes | Aligned | `crates/app/src/lib.rs:386`, `crates/ops/proof-op/src/lib.rs:116`, `crates/ops/proof-op/src/lib.rs:132` |
| `keystore_import` | Yes | Aligned | `crates/app/src/lib.rs:387`, `crates/ops/keystore-admin-op/src/lib.rs:101`, `crates/ops/common/src/states/keystore_admin.rs:138` |
| `keystore_list` | Yes | Aligned | `crates/app/src/lib.rs:388`, `crates/ops/keystore-admin-op/src/lib.rs:160`, `crates/ops/common/src/states/keystore_admin.rs:215` |
| `keystore_delete` | Yes | Aligned | `crates/app/src/lib.rs:389`, `crates/ops/keystore-admin-op/src/lib.rs:229`, `crates/ops/common/src/states/keystore_admin.rs:291` |
| `keystore_tx_sign` | Yes | Aligned | `crates/app/src/lib.rs:390`, `crates/ops/keystore-tx-op/src/lib.rs:113`, `crates/ops/keystore-tx-op/src/lib.rs:275`, `crates/ops/common/src/local_io.rs:51` |
| `keystore_tx_send_raw` | Yes | Aligned | `crates/app/src/lib.rs:391`, `crates/ops/keystore-tx-op/src/lib.rs:191`, `crates/ops/keystore-tx-op/src/lib.rs:350`, `crates/ops/common/src/local_io.rs:52` |
| `evm_read` | Yes | Aligned | `crates/app/src/lib.rs:392`, `crates/ops/evm-read-op/src/lib.rs:61`, `crates/ops/evm-read-op/src/lib.rs:80` |
| `evm_contract_from_nix` | Yes | Aligned | `crates/app/src/lib.rs:393`, `crates/ops/evm-write-op/src/lib.rs:1223`, `crates/ops/evm-write-op/src/lib.rs:1293` |
| `evm_deploy` | Yes | Aligned | `crates/app/src/lib.rs:394`, `crates/ops/evm-write-op/src/lib.rs:1345`, `crates/ops/evm-write-op/src/lib.rs:1448`, `crates/ops/evm-write-op/src/lib.rs:962`, `crates/ops/common/src/local_io.rs:53` |
| `evm_configure` | Yes | Aligned | `crates/app/src/lib.rs:395`, `crates/ops/evm-write-op/src/lib.rs:1525`, `crates/ops/evm-write-op/src/lib.rs:1659` |
| `evm_validate` | Yes | Aligned | `crates/app/src/lib.rs:396`, `crates/ops/evm-write-op/src/lib.rs:1741`, `crates/ops/evm-write-op/src/lib.rs:1855` |
| `evm_deploy_configure_validate` | Yes | Aligned | `crates/app/src/lib.rs:397`, `crates/ops/evm-deploy-configure-validate-op/src/lib.rs:26`, `crates/ops/evm-deploy-configure-validate-op/src/lib.rs:68` |
| `portfolio_tracker` | Yes | Aligned | `crates/app/src/lib.rs:398`, `crates/ops/portfolio-tracker-op/src/lib.rs:349`, `crates/ops/portfolio-tracker-op/src/lib.rs:476` |
| `nix_app` | Yes | Aligned | `crates/app/src/lib.rs:399`, `crates/ops/nix-app-op/src/lib.rs:84`, `crates/ops/nix-app-op/src/lib.rs:190` |

Additional packaging observation:

- `mfm-op-keystore` remains a re-export wrapper, not an operation implementation: `crates/ops/keystore-op/src/lib.rs:8`, `crates/ops/keystore-op/README.md:3`.

Reusable state-library observation:

- Shared state library adoption improved (`7` shared state impls), but op-local states still dominate (`16` impls):
  - shared: `crates/ops/common/src/states/keystore_admin.rs`, `crates/ops/common/src/states/evm.rs`
  - op-local examples: `crates/ops/evm-write-op/src/lib.rs:1448`, `crates/ops/portfolio-tracker-op/src/lib.rs:476`, `crates/ops/keystore-tx-op/src/lib.rs:260`

## 6. Findings (Ordered by Severity)

### High

None open in this checkpoint.

### Medium

1. `B6` reusable state-library convergence remains incomplete.
- Impact: shared states exist but most business flows still use op-local state implementations.
- Evidence: `crates/ops/common/src/states/keystore_admin.rs`, `crates/ops/common/src/states/evm.rs`, `crates/ops/evm-write-op/src/lib.rs:1448`, `crates/ops/portfolio-tracker-op/src/lib.rs:476`.
- Risk: duplicated semantics and slower evolution toward composition-first ops.

2. Guardrails for ambient IO regressions are not yet automated.
- Impact: no architecture test/check currently fails PRs when new state handlers reintroduce direct ambient IO outside intended transport.
- Evidence: missing dedicated check surface; explicit local transport boundary is in `crates/ops/common/src/local_io.rs:47`.
- Risk: regressions can land silently as code evolves.

3. Parity validation is still pending for this checkpoint.
- Impact: architecture-level changes are landed but not yet validated through full parity gates in one run.
- Evidence: expected gates are `nix run .#check`, `nix run .#test`, `nix run .#ci -- --basic --summary`, `nix run .#ci -- --parity --summary`.
- Risk: unnoticed behavior drift.

### Low

4. `B3` keystore ops packaging naming remains semantically ambiguous.
- Impact: `mfm-op-keystore` implies executable op behavior but only re-exports core keystore types.
- Evidence: `crates/ops/keystore-op/src/lib.rs:8`, `crates/ops/keystore-op/README.md:3`.
- Risk: boundary confusion in future additions.

## 7. What's Next (High-Value Finish Plan)

1. Establish parity baseline for this checkpoint.
- Run and archive: `nix run .#check`, `nix run .#test`, `nix run .#ci -- --basic --summary`, and `nix run .#ci -- --parity --summary` with live RPC configured.
- Treat this as the before/after safety net for all remaining Option A work.
- Exit criteria: all green, or a tracked failure list with owner, root-cause, and fix plan.

2. Complete `B6` convergence for deploy/configure/validate.
- Extract reusable EVM execution states (especially deploy/configure/validate pieces) from `crates/ops/evm-write-op/src/lib.rs` into `crates/ops/common/src/states`.
- Keep operation crates focused on config validation + graph composition only (`Operation::expand` assembly).
- Exit criteria: deploy/configure/validate flows are composed from shared state primitives, not bespoke op-local state handlers.

3. Complete `B6` convergence for portfolio snapshot.
- Move reusable portfolio pieces (balance/read/write states) from `crates/ops/portfolio-tracker-op/src/lib.rs` into shared state modules.
- Keep `portfolio_tracker` op as orchestration wiring + domain config normalization.
- Exit criteria: portfolio op mostly wires shared states and exports report/output contracts.

4. Add an architecture guard against ambient IO regressions.
- Add a test/check that fails when new state handlers use direct ambient IO (`std::fs`, direct env/password prompts) outside approved transport boundaries.
- Use `crates/ops/common/src/local_io.rs` as the explicit allowlisted local-side-effect boundary.
- Exit criteria: guard runs in normal quality gates and fails deterministically on boundary violations.

5. Re-run parity gates and close the audit.
- Re-run the same gate set from step 1 after steps 2-4 land.
- Update this audit and the CSV matrix with final counts/statuses.
- Exit criteria: Option A marked complete with no open High/Medium findings except explicitly deferred items.

## 8. Acceptance Criteria for Option A Completion

1. No business workflow logic in `bin/cli` or `bin/rest-api`.
2. Feature-specific execution behavior represented by ops/state graphs; app helpers remain transport/adaptation only.
3. Business execution paths run-backed and state-graph represented.
4. State execution side effects routed through IO abstraction boundaries.
5. Shared state-library reuse is the default for common workflow primitives.
6. CI parity gates (`check/test/basic/parity`) pass with no behavior regressions.
