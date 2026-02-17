# Aave v3 Origin Parity Pipeline

## Summary
The parity flow now uses `aave-dao/aave-v3-origin` as the source of truth with a pinned commit and keeps Foundry execution inside Nix apps.

Pipeline shape:
1. `fetch_origin` (`nix_app`)
2. `compile_origin` (`nix_app`)
3. `deploy_origin_stack` (`nix_app`)
4. `adapt_origin_deploy` (`aave_v3_origin_adapt_deploy`)
5. `run_scenario` (`aave_v3_scenario`)

## Contracts
1. `aave_v3_origin_source_v1`
   Source identity metadata (repo + pinned commit).
2. `aave_v3_origin_compile_manifest_v1`
   Origin compile artifact metadata used for parity traceability.
3. `aave_v3_origin_deploy_output_v1`
   Raw origin deploy output with contract addresses + artifacts.
4. `aave_v3_deploy_manifest_v1`
   Normalized deploy manifest produced by adapter op.
5. `aave_v3_reth_scenario_report_v1`
   Final parity scenario report.

## Runtime Rules
1. Scenario actors use neutral naming:
   - `supplier`
   - `borrower`
2. Scenario config requires explicit values:
   - `funder_account_index`
   - `supplier_account_index`
   - `borrower_account_index`
   - `fund_wei`
   - `usdc_supply_amount`
   - `wbtc_collateral_amount`
   - `usdc_borrow_amount`
   - `borrow_rate_mode`
3. Shared state logic contains no test-only actor naming or implicit scenario amount/index defaults.

## CI and Test Wiring
1. Integration test: `tests/integration/tests/parity_aave_v3_reth_scenario.rs`.
2. Parity CI step: `nixfied/project/ci/scripts/steps/parity-aave-v3-reth.nix`.
3. Parity mode registration remains in `nixfied/project/ci.nix`.
