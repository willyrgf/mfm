# Refactor Report: Aave V3 Origin Pipeline

Status: complete (2026-02-17).

## Objective
Move Aave v3 parity to `aave-dao/aave-v3-origin` as source of truth, keep all Foundry execution inside Nix apps, and remove test-only leakage from shared runtime states.

## Final Decisions
1. Source is pinned to `https://github.com/aave-dao/aave-v3-origin` commit `1e3d70c4151a94166ebc59e2eaa4aff6e6ba6978`.
2. Parity target is full local origin stack deployment on reth.
3. Fetch/compile/deploy run only through Nix apps (`nix_app` op).
4. Scenario runtime config is explicit and required (no test-default amounts/indexes).
5. Shared actor naming is neutral (`supplier`, `borrower`).

## Implemented Changes
1. Nix-packaged origin source in `nixfied/project/conf.nix` via `fetchFromGitHub` with fixed hash and submodules.
2. Origin toolchain apps are active:
   - `mfm-aave-v3-origin-fetch`
   - `mfm-aave-v3-origin-compile`
   - `mfm-aave-v3-origin-deploy`
3. Compile/deploy tool temp copy is made writable (`chmod -R u+w`) before patching files.
4. Origin deploy output is adapted through `aave_v3_origin_adapt_deploy` into `aave_v3_deploy_manifest_v1`.
5. Parity scenario pipeline is:
   `fetch_origin -> compile_origin -> deploy_origin_stack -> adapt_origin_deploy -> run_scenario`.
6. Shared Aave scenario state enforces explicit required numeric configuration.
7. Legacy Aave runtime mock crates were removed:
   - `crates/ops/aave-v3-deploy-runtime-op`
   - `crates/ops/aave-v3-configure-runtime-op`
8. Removed obsolete Aave mock pool contract:
   - `contracts/src/MockAaveV3Pool.sol`

## Contract Kinds
1. `aave_v3_origin_source_v1`
2. `aave_v3_origin_compile_manifest_v1`
3. `aave_v3_origin_deploy_output_v1`
4. `aave_v3_deploy_manifest_v1`
5. `aave_v3_reth_scenario_report_v1`

## Validation
1. `nix run .#check` passed.
2. `nix run .#test` passed (`284/284`).
3. `nix run .#ci -- --parity --summary` passed.
4. Parity summary includes:
   - `[PASS] parity-aave-v3-reth`
   - `[PASS] parity-portfolio-tracker-reth`

## Documentation Status
1. `AAVE_V3_DEPLOY_LEND_BORROW_TEST_PIPELINE.md` updated as canonical pipeline reference.
2. No additional architecture/doc contract updates are required for this refactor.
