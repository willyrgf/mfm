# Aave v3 on Reth Parity Scenario Plan (5-Step Decomposition, Composable States)

## Summary
Build a parity integration flow that cleanly separates external build tooling from internal MFM execution:
1. fetch Aave contracts via a pinned Nix app,
2. compile contracts via Forge in a second pinned Nix app,
3. deploy runtime with internal MFM EVM primitives,
4. configure runtime with internal MFM EVM primitives,
5. execute lend/borrow scenario with strict assertions and a final report artifact.

This keeps `nix_app` focused on deterministic external tooling and keeps on-chain execution logic in reusable state-machine states under `crates/ops/common/src/states/*`.

## Ownership and Boundary
1. `nix_app` owns step 1 and step 2 only.
2. Internal MFM ops own step 3, step 4, and step 5.
3. `aave_v3_scenario` remains the scenario test op, but it is composed from smaller reusable generic states.

## Operations to Add
1. Reuse `nix_app` (`op_id = "nix_app"`, `v1`) twice in pipeline:
   - `fetch_contracts`
   - `compile_contracts`
2. Add `aave_v3_deploy_runtime` (`v1`) op:
   - Thin op crate that wires deploy states.
3. Add `aave_v3_configure_runtime` (`v1`) op:
   - Thin op crate that wires configure states.
4. Add `aave_v3_scenario` (`v1`) op:
   - Thin op crate that wires scenario states and emits final report artifact.

## Pipeline Shape (Decision Complete)
1. `fetch_contracts` (`nix_app`)
   - Input: pinned `app` flake reference for contract source fetch tool.
   - Output key: `result`.
   - Output schema kind: `aave_v3_contract_source_v1`.

2. `compile_contracts` (`nix_app`)
   - Input: pinned `app` flake reference for Forge compile tool.
   - Reads `fetch_contracts.result` as needed.
   - Output key: `result`.
   - Output schema kind: `aave_v3_compile_manifest_v1`.
   - Contract for step 3 input: typed deployment manifest (not raw Foundry `out/`).

3. `deploy_runtime` (`aave_v3_deploy_runtime`)
   - Reads compile manifest.
   - Deploys required contracts using internal EVM write primitives.
   - Output schema kind: `aave_v3_deploy_manifest_v1`.

4. `configure_runtime` (`aave_v3_configure_runtime`)
   - Reads deploy manifest.
   - Applies protocol/reserve/runtime configuration.
   - Output schema kind: `aave_v3_config_report_v1`.

5. `run_scenario` (`aave_v3_scenario`)
   - Reads deploy + config outputs.
   - Runs funding/collateral/supply/borrow flow.
   - Performs strict invariant checks.
   - Writes final summary schema kind: `aave_v3_reth_scenario_report_v1`.

## Reusable State Decomposition
Implement shared reusable states in `crates/ops/common/src/states/aave_v3.rs` and keep ops thin.

### Deploy-reusable states
1. `load_compile_manifest`
2. `deploy_contract` (generic per manifest entry)
3. `wait_for_receipt`
4. `collect_deploy_outputs`
5. `write_deploy_manifest`

### Configure-reusable states
1. `load_deploy_manifest`
2. `configure_runtime_call` (generic ABI call)
3. `wait_for_config_receipt`
4. `collect_config_outputs`
5. `write_config_report`

### Scenario-reusable states
1. `resolve_chain`
2. `resolve_accounts`
3. `fund_wallet`
4. `approve_erc20`
5. `supply_asset`
6. `borrow_asset`
7. `read_positions`
8. `assert_invariants`
9. `write_summary`

## Scenario Defaults (Locked)
1. Accounts: deterministic reth account indices `merican=1`, `saylor=2`.
2. Funding/supply/borrow amounts:
   - USDC supply: `1_000_000e6`
   - WBTC collateral: `10e8`
   - USDC borrow: `400_000e6`
3. Borrow mode: variable (`rateMode = 2`).
4. Assertion profile: strict and deterministic.

## Typed Inter-Step Contracts
1. `aave_v3_contract_source_v1`
   - Source identity metadata from fetch step.
2. `aave_v3_compile_manifest_v1`
   - Canonical deployment plan + ABI/bytecode data needed for internal deploy.
3. `aave_v3_deploy_manifest_v1`
   - Deployed addresses + tx hashes + block references.
4. `aave_v3_config_report_v1`
   - Configuration tx hashes + applied parameters.
5. `aave_v3_reth_scenario_report_v1`
   - Final balances, positions, assertions, run metadata.

## Integration and CI Wiring
1. Add parity integration test:
   - `tests/integration/tests/parity_aave_v3_reth_scenario.rs`
2. Pipeline used in test:
   - `fetch_contracts -> compile_contracts -> deploy_runtime -> configure_runtime -> run_scenario`
3. Add parity CI step script:
   - `nixfied/project/ci/scripts/steps/parity-aave-v3-reth.nix`
4. Register step in parity mode:
   - `nixfied/project/ci.nix`
5. Add two Nix apps (no raw `.sh` in repo):
   - contracts fetch app
   - contracts compile app
   - place in `nixfied/local/default.nix` with typed app contracts.

## Test Cases
1. Happy path parity run:
   - all 5 steps complete,
   - final report matches strict expected balances/debt/collateral.
2. Op/unit validation failures:
   - missing compile/deploy manifest fields,
   - malformed address/ABI/amount values,
   - invalid account indices.
3. Replay determinism tests:
   - scenario read/assert states produce stable results with replay facts.
4. Negative scenario:
   - over-borrow fails with stable error code and failure surfaced in report.

## Assumptions and Defaults Locked
1. Aave scope: local core+periphery runtime needed for lend/borrow scenario; governance stack out of scope.
2. Architecture rule: thin ops, reusable execution states, thin binaries.
3. External tooling split: fetch and compile are explicit independent `nix_app` steps.
4. Internal execution split: deploy/configure/scenario are internal MFM ops.
