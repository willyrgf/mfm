# Aave v3 Origin Parity Pipeline

Status: active (updated 2026-02-17).

## Purpose
Define the canonical parity pipeline for local Aave v3 deploy/lend/borrow on reth.

## Pipeline
1. `fetch_origin` (`nix_app`, tool `mfm-aave-v3-origin-fetch`)
2. `compile_origin` (`nix_app`, tool `mfm-aave-v3-origin-compile`)
3. `deploy_origin_stack` (`nix_app`, tool `mfm-aave-v3-origin-deploy`)
4. `adapt_origin_deploy` (`aave_v3_origin_adapt_deploy`)
5. `run_scenario` (`aave_v3_scenario`)

## Source of Truth
1. Upstream repository: `https://github.com/aave-dao/aave-v3-origin`
2. Pinned commit: `1e3d70c4151a94166ebc59e2eaa4aff6e6ba6978`
3. Source packaging is Nix-based (`fetchFromGitHub`) in `nixfied/project/conf.nix`.
4. Foundry runs only inside Nix apps; tests/ops never call forge directly.

## Pipeline Contracts
1. `aave_v3_origin_source_v1`: source identity metadata.
2. `aave_v3_origin_compile_manifest_v1`: compiled artifact manifest for traceability.
3. `aave_v3_origin_deploy_output_v1`: raw deployed origin stack output.
4. `aave_v3_deploy_manifest_v1`: normalized deploy manifest after adapter.
5. `aave_v3_reth_scenario_report_v1`: final scenario report.

## Runtime Rules
1. Actor names are neutral (`supplier`, `borrower`).
2. Scenario values are explicit and required:
   `funder_account_index`, `supplier_account_index`, `borrower_account_index`,
   `fund_wei`, `usdc_supply_amount`, `wbtc_collateral_amount`,
   `usdc_borrow_amount`, `borrow_rate_mode`.
3. No test-only defaults in shared Aave state logic.

## Validation
1. `nix run .#check`
2. `nix run .#test`
3. `nix run .#ci -- --parity --summary`

Current expected parity result includes:
1. `[PASS] parity-aave-v3-reth`
2. `[PASS] parity-portfolio-tracker-reth`

## Wiring References
1. Integration test: `tests/integration/tests/parity_aave_v3_reth_scenario.rs`
2. CI step: `nixfied/project/ci/scripts/steps/parity-aave-v3-reth.nix`
3. CI mode registration: `nixfied/project/ci.nix`
