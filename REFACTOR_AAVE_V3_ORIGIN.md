# Refactor Plan: Aave V3 Origin Pipeline

## Objective
Refactor the Aave V3 parity flow to use `aave-dao/aave-v3-origin` as source-of-truth, keep all Foundry build/deploy behavior inside nix-packaged apps, and remove test-specific identities/defaults from reusable runtime implementation.

## Locked Decisions
1. Source pinning uses a fixed Git commit SHA of `https://github.com/aave-dao/aave-v3-origin`.
2. Parity target is a full local origin stack deployment (not mock/local minimal contracts).
3. Forge config + execution stays inside nix-packaged apps only.
4. Scenario runtime config uses explicit required fields (no test-oriented defaults).
5. `merican`/`saylor` naming is removed from shared ops/state code.

## Problems to Fix
1. Current compile path still uses mock contracts (`contracts/src/MockAaveV3Pool.sol`, `MockERC20.sol`) in Aave pipeline tooling.
2. Aave pipeline metadata currently reports local mock source identity.
3. Shared state code in `crates/ops/common/src/states/aave_v3.rs` includes test-specific actor naming and scenario defaults.

## Scope
### In Scope
1. Replace Aave pipeline source/compile/deploy inputs with pinned origin flow.
2. Add/adjust nix apps for origin fetch, compile, and deploy outputs.
3. Introduce/adjust adapter logic from origin deployment output into MFM deploy manifest contract.
4. Refactor Aave scenario runtime types/config to remove test-specific naming/defaults.
5. Update parity Aave integration test and CI step wiring to the new flow.

### Out of Scope
1. Mainnet deployment support.
2. Governance flows beyond parity scenario needs.
3. General redesign of unrelated ops.

## Implementation Plan

### Workstream 1: Nix-Packaged Origin Tooling
1. Update `nixfied/project/conf.nix`:
   - Add pinned-origin fetch tool (`mfm-aave-v3-origin-fetch`) returning `aave_v3_origin_source_v1`.
   - Add pinned-origin compile tool (`mfm-aave-v3-origin-compile`) running Foundry compile in nix app context.
   - Add pinned-origin deploy tool (`mfm-aave-v3-origin-deploy`) producing deterministic deploy output artifact/JSON.
2. Update `nixfied/local/default.nix`:
   - Register corresponding apps (`aave-v3-origin-fetch`, `aave-v3-origin-compile`, `aave-v3-origin-deploy`) using current app API contract.
3. Ensure no direct forge calls from tests/ops; only `nix_app` invokes these tools.

### Workstream 2: Runtime Contract and Adapter Layer
1. Add a dedicated adapter op/state path to normalize origin deploy output into:
   - `aave_v3_deploy_manifest_v1`.
2. Keep existing deploy manifest contract stable for downstream scenario op.
3. Validate required contract addresses/ABIs/ids and fail with typed parsing errors on malformed origin outputs.

### Workstream 3: Remove Test Leakage from Shared Aave States
1. Refactor `crates/ops/common/src/states/aave_v3.rs`:
   - Rename actor fields to neutral names:
     - `merican` -> `supplier`
     - `saylor` -> `borrower`
   - Remove scenario-specific default helper functions for account indexes and amounts.
   - Make scenario config fields explicit/required:
     - `funder_account_index`
     - `supplier_account_index`
     - `borrower_account_index`
     - `fund_wei`
     - `usdc_supply_amount`
     - `wbtc_collateral_amount`
     - `usdc_borrow_amount`
     - `borrow_rate_mode`
2. Update validation/error messages to neutral actor naming.
3. Keep generic runtime-safe defaults only where not test-profile specific (for example port key names).

### Workstream 4: Integration and CI Wiring
1. Update `tests/integration/tests/parity_aave_v3_reth_scenario.rs`:
   - Pipeline steps use origin apps:
     - `fetch_origin` -> `compile_origin` -> `deploy_origin_stack` -> `adapt_origin_deploy` -> `run_scenario`.
   - Provide all scenario values explicitly in test config (no reliance on runtime defaults).
   - Keep workspace flake ref resolution for stable `path:<abs>#app` references.
2. Update `nixfied/project/ci/scripts/steps/parity-aave-v3-reth.nix` only as needed for step naming/log clarity.
3. Keep parity mode entry in `nixfied/project/ci.nix`.

### Workstream 5: Mock Contract Decommissioning for Aave Path
1. Remove Aave-pipeline dependency on:
   - `contracts/src/MockAaveV3Pool.sol`
   - mock-based Aave manifest generation.
2. If mock contracts are unused globally, remove them and related tool registrations.
3. If still needed by non-Aave tests, isolate them under explicit mock-only paths and keep origin flow separate.

## Interfaces and Contracts
1. New/updated source contract: `aave_v3_origin_source_v1`.
2. New/updated compile contract: `aave_v3_origin_compile_manifest_v1`.
3. Existing deploy handoff remains: `aave_v3_deploy_manifest_v1` (after adapter).
4. Scenario report remains: `aave_v3_reth_scenario_report_v1`.

## Testing Plan
1. Unit tests:
   - Origin adapter: success path + malformed/missing field failures.
   - Scenario config validation: required-field enforcement and actor-index rules.
2. Integration tests:
   - Updated `parity_aave_v3_reth_scenario` full pipeline.
3. CI parity verification:
   - `nix run .#ci -- --parity --summary` with `parity-aave-v3-reth` passing.
4. Quality gates:
   - `nix run .#check`
   - `nix run .#test`

## Acceptance Criteria
1. Aave parity pipeline no longer compiles or deploys mock Aave contracts.
2. Aave fetch/compile/deploy are fully nix-packaged and invoked via `nix_app`.
3. Shared Aave runtime code contains no `merican`/`saylor` naming and no test-specific scenario defaults.
4. Parity Aave integration test passes with explicit scenario inputs.
5. Parity CI summary reports Aave step as passed.

## Execution Status (2026-02-17)
Status: Complete.

Implemented outcomes:
1. Origin-source pipeline is wired as `fetch_origin -> compile_origin -> deploy_origin_stack -> adapt_origin_deploy -> run_scenario`.
2. Origin outputs are normalized through a dedicated adapter op into `aave_v3_deploy_manifest_v1`.
3. Shared Aave runtime state uses neutral actor naming (`supplier`, `borrower`) with explicit required scenario config values.
4. Parity integration test uses explicit scenario values and validates origin contract kinds.
5. Nix app registrations/configuration for origin fetch/compile/deploy are active and use a nix-packaged pinned source derivation (not workspace cache paths).
6. Scenario config validation now hard-requires `fund_wei` to be parseable and `> 0`, with focused unit coverage for all numeric guard rails.
7. Legacy mock-runtime Aave crates and `contracts/src/MockAaveV3Pool.sol` were removed from the branch.

Validation executed:
1. `nix run .#check` (pass)
2. `nix run .#test` (pass, 284/284)
3. `nix run .#ci -- --parity --summary` (pass)
   - Includes `parity-aave-v3-reth` passing.

Documentation updated:
1. `AAVE_V3_DEPLOY_LEND_BORROW_TEST_PIPELINE.md`
2. `REFACTOR_AAVE_V3_ORIGIN.md` (this status section)
