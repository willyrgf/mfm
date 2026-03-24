# RFC: Scattered States To Semantics For `mfm::portfolio::snapshot`

Status: draft notes

Last updated: 2026-03-24

## Summary

This note captures the current code path for:

```bash
nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>
```

The goal is to turn the currently scattered implementation details into one semantic map that is
easy to edit into a more formal RFC later.

The most important finding is that the current live code path is:

1. flake app launcher
2. `task.mfm.portfolio.snapshot` shell wrapper
3. packaged `mfm_cli --output-format json portfolio snapshot --request-file ...`
4. `AppServices::start_portfolio_snapshot`
5. single-op run for `portfolio_tracker v1`
6. namespaced state graph
7. final context snapshot extraction
8. raw JSON envelope emitted unchanged to stdout

It is not currently:

1. app
2. `workflow.mfm.portfolio.snapshot`
3. hidden `services-start` / `exec` / `services-stop` tasks

That workflow/task split still appears in docs, but not in the current Nix code or generated
launcher.

## Verified Entrypoint Chain

### Flake and app resolution

- `flake.nix` builds outputs through `nixfiedLib.mkFlakeOutputs` with `nixfied/project/module.nix`.
- `nixfied/project/module.nix` defines `mkTaskApp`.
- `apps."mfm::portfolio::snapshot"` is a `taskRef` pointing at `task.mfm.portfolio.snapshot`.
- The generated launcher for this build resolves the app name `mfm::portfolio::snapshot`,
  selects the concrete launcher via `run-selected-app.nix`, and `exec`s the selected binary.

Relevant files:

- `flake.nix`
- `nixfied/project/module.nix`
- generated launcher path returned by:
  - `nix eval --json '.#apps.<system>."mfm::portfolio::snapshot"'`

### Important correction: no current workflow hop

What the current code shows:

- the public app points directly to `task.mfm.portfolio.snapshot`
- the generated launcher records no workflow id for `task.mfm.portfolio.snapshot`

What stale docs still say:

- `README.md` says the app delegates sequencing to `workflow.mfm.portfolio.snapshot`
- `docs/helios.md` says the public task delegates to:
  - `workflow.mfm.portfolio.snapshot`
  - `task.mfm.portfolio.snapshot.services-start`
  - `task.mfm.portfolio.snapshot.exec`
  - `task.mfm.portfolio.snapshot.services-stop`

That stale split is important because any redesign work should start from the current shell task,
not from the older workflow description.

## End-To-End Call Path

### 1. `nix run` resolves the app

- `nix run .#mfm::portfolio::snapshot` evaluates the flake.
- The app surface comes from `frameworkOutputs.apps`.
- The selected app is the generated launcher for `mfm::portfolio::snapshot`.

### 2. The app resolves to `task.mfm.portfolio.snapshot`

- `apps."mfm::portfolio::snapshot"` is a `taskRef`.
- The referenced task is `task.mfm.portfolio.snapshot`.
- This is the real public wrapper for the portfolio snapshot flow.

### 3. The task enforces the shell contract

The shell wrapper:

- accepts exactly one positional request file
- rejects unknown arguments
- normalizes relative paths to absolute paths
- checks that the file exists and is readable
- requires `POSTGRES_PORT`
- rejects `MFM_KEEP_SERVICES`
- requires Postgres lifecycle hooks to be present

This wrapper owns stdout for the public app.

### 4. The task derives service lifecycle policy

The wrapper infers service policy from:

- `SERVICE_REUSE_POLICY`
- `SERVICE_OWNER_SCOPE`
- `SERVICE_DISCOVERY_SCOPE`

It then:

- sets cleanup traps
- decides whether owned services should be torn down on exit

### 5. The task bootstraps Postgres

The wrapper:

- checks `SVC_POSTGRES_READY`
- reuses Postgres if already ready
- otherwise starts Postgres via `SVC_POSTGRES_FULL_START`
- then re-checks readiness

### 6. The task optionally bootstraps Helios

Helios is currently opportunistic, not strictly required by the task model.

Behavior:

- if Helios hooks exist and Helios is not skipped, the wrapper uses Helios
- it requires `HELIOSRPC_PORT`
- it defaults `HELIOS_NETWORK` to `mainnet`
- it rejects any non-mainnet `HELIOS_NETWORK`
- it reuses or starts Helios
- if the caller did not provide managed RPC bootstrap env, it seeds:
  - `MFM_EVM_RPC_SOURCES_JSON`
  - `MFM_EVM_RPC_PREFERRED_ORDER`

If Helios is unavailable or skipped:

- the wrapper requires the caller to already provide `MFM_EVM_RPC_SOURCES_JSON`

This is a useful semantic distinction:

- Postgres is model-required
- Helios is convenience-managed when hooks exist
- direct managed RPC input is the actual runtime requirement

### 7. The task launches packaged `mfm_cli`

After service bootstrap, the wrapper:

- exports `DATABASE_URL`
- resolves the packaged `mfm_cli` binary
- runs:

```bash
mfm_cli --output-format json portfolio snapshot --request-file "$request_file"
```

This path uses the packaged CLI binary, not `cargo run`.

### 8. The CLI parses the request and builds stores

`bin/cli/src/commands/portfolio/snapshot.rs` does the following:

- parses request input via `mfm_app::parse_portfolio_snapshot_request_input`
- builds stores from `RunStoresArgs`
- creates `AppServices`
- calls `services.start_portfolio_snapshot(request)`

Store semantics:

- stream store:
  - Postgres
  - from `--database-url` or `DATABASE_URL`
- artifact store:
  - filesystem
  - from `--artifact-root`, `MFM_ARTIFACT_ROOT`, or `~/.mfm/run_artifacts`

### 9. `AppServices` starts a single-op run

`AppServices::start_portfolio_snapshot`:

- validates the portfolio bundle again
- serializes the full request as op config
- starts `RunsStartRequest::Single`
- uses:
  - `op_id = "portfolio_tracker"`
  - `op_version = "v1"`

Default run config semantics:

- live I/O
- sequential execution
- no retry backoff
- context checkpointing after every state

### 10. The app bundle provides runtime wiring

The app bundle registers:

- `PortfolioTrackerOp`
- transport factories including:
  - local fs
  - local evm
  - `rpc.control`
  - exec transport
  - nix flake transport

So the portfolio snapshot flow is not a CLI-local special case. It is a normal op execution path
through the engine.

### 11. `PortfolioTrackerOp` expands into the state graph

The op planner:

- validates and decodes `portfolio` plus `valuation_source_registry`
- splits symbols into:
  - base symbols
  - Aave protocol-position symbols
- filters wallet symbol assignments accordingly
- builds this DAG:

1. `PrepareSourcesState`
2. `PinPortfolioNetworksState`
3. `ResolveWalletsState`
4. `ReadDirectPricesState`
5. `CollectObservationsState`
6. `CollectAaveObservationsState`
7. `MergeObservationsState`
8. `WritePortfolioSnapshotState`
9. `WritePortfolioReportState`

## State Semantics Map

| State | Semantic role | Main inputs | Main outputs | Notes |
|---|---|---|---|---|
| `PrepareSourcesState` | Preflight managed RPC source pools | routed networks and `control_scope` | none in context | Fails fast if a network/scope has no healthy `rpc.control` sources |
| `PinPortfolioNetworksState` | Freeze the execution view to concrete chain/block pins | portfolio config, valuation source registry, live RPC | `network_pins` | Validates runtime inputs again, checks `eth_chainId`, reads `eth_blockNumber` |
| `ResolveWalletsState` | Convert configured wallets into runtime wallet identities and capabilities | wallet configs | `resolved_wallets` | Base runtime currently supports only `address_only` wallets |
| `ReadDirectPricesState` | Read all unique direct valuation sources once | symbol configs, valuation source registry, pinned networks | `direct_prices` | Reads oracle-backed direct prices at pinned blocks; derived prices are not computed here |
| `CollectObservationsState` | Build canonical observations for base symbols | base wallets, base symbols, resolved wallets, direct prices, network pins | `base_observations` | Reads native and ERC-20 balances, then computes values from fixed/direct/derived prices |
| `CollectAaveObservationsState` | Build canonical observations for Aave protocol positions | Aave wallets, Aave symbols, resolved wallets, direct prices, network pins | `aave_observations` | Reads reserve/debt positions and normalizes them into the same observation shape |
| `MergeObservationsState` | Canonical merge and ordering boundary | `base_observations`, `aave_observations` | `observations` | Concatenates and sorts deterministically |
| `WritePortfolioSnapshotState` | Assemble the canonical snapshot and persist the snapshot artifact | resolved wallets, network pins, merged observations, portfolio config | `snapshot`, `snapshot_artifact_id` plus output artifact | Uses `io.now_millis()` for `generated_at_ms` |
| `WritePortfolioReportState` | Derive user-facing totals/report from the snapshot | `snapshot` | `report` | Produces wallet totals and portfolio totals by quote and emits a completion event |

## More Detailed State Notes

### `PrepareSourcesState`

Semantic meaning:

- "Before any reads happen, prove that each required routed network has at least one responsive
  managed source."

Operational detail:

- deduplicates `(network_id, control_scope)`
- calls `prepare_sources_in_scope`
- fails if every returned source is unhealthy

### `PinPortfolioNetworksState`

Semantic meaning:

- "Turn an open-ended live portfolio query into a replayable, block-pinned view."

Operational detail:

- computes required networks from:
  - wallet network assignments
  - valuation readers
- validates configured `chain_id` against live RPC `eth_chainId`
- reads `eth_blockNumber`
- writes deterministic sorted `network_pins`

This is the main replay boundary for the rest of the portfolio flow.

### `ResolveWalletsState`

Semantic meaning:

- "Resolve the portfolio's wallet declarations into concrete runtime wallet identities."

Current limitation:

- only `address_only` is supported in this runtime slice
- `keystore_entry`
- `node_managed_account`
- `external_signer`

all currently fail here

### `ReadDirectPricesState`

Semantic meaning:

- "Resolve every direct pricing source needed by this portfolio at the already pinned view."

Operational detail:

- scans symbol valuation configs
- collects unique source ids
- resolves valuation sources from the registry
- reads each direct source once
- stores direct price values plus source refs pinned to a block

Derived prices are not materialized here. They are computed later from the cached direct prices.

### `CollectObservationsState`

Semantic meaning:

- "For each base symbol assigned to each wallet, read the quantity and compute its values."

Operational detail:

- native balances use `eth_getBalance`
- ERC-20 balances use `balanceOf`
- decimals come from config or on-chain `decimals()`
- valuation values are built from:
  - fixed unit price
  - direct price lookup
  - derived unit price from numerator / denominator direct prices

This is the base-token semantic layer.

### `CollectAaveObservationsState`

Semantic meaning:

- "For each Aave protocol position, read the protocol-specific quantity and normalize it into the
  same canonical observation model."

Operational detail:

- reads reserve and debt token positions
- handles collateral-enabled checks where configured
- emits standard `Observation` records so downstream states do not care whether the source was a
  base token or an Aave position

### `MergeObservationsState`

Semantic meaning:

- "Collapse multiple observation producers into one canonical observation set."

It is a pure ordering and merge boundary.

### `WritePortfolioSnapshotState`

Semantic meaning:

- "Produce the replayable canonical portfolio snapshot artifact."

Operational detail:

- groups observations by wallet
- copies and normalizes symbol configs
- constructs `PortfolioSnapshot`
- writes snapshot JSON into context
- writes snapshot JSON as an output artifact
- records the snapshot artifact id in context

### `WritePortfolioReportState`

Semantic meaning:

- "Turn the canonical snapshot into the user-facing totals/report projection."

Operational detail:

- reads the full snapshot
- computes wallet totals by quote
- merges into portfolio totals by quote
- writes report into context
- emits `portfolio_tracker.completed`

## Context Namespacing

One subtle but important runtime detail:

- the op planner uses local keys like:
  - `resolved_wallets`
  - `network_pins`
  - `report`
- but `AppServices` later extracts:
  - `portfolio_tracker.main.snapshot_artifact_id`
  - `portfolio_tracker.main.report`

The reason is `NamespacedContext` in `crates/sdk/src/unstable.rs`.

What it does:

- local writes are qualified as `<op_path>.<key>`
- local reads default to that same qualified namespace
- imported keys can read from another op path if wired through exports/imports

So the semantic model is:

- planner code talks in local, op-relative keys
- the runtime persists those keys under a fully qualified op path

For this op, the effective path is `portfolio_tracker.main`.

## Persistence And Returned Data

### Stream store vs artifact store

There are two persisted surfaces involved:

- stream store:
  - Postgres
  - run stream and kernel/domain event history
- artifact store:
  - filesystem by default
  - context snapshots and output artifacts

### `final_snapshot_id` vs `snapshot_artifact_id`

These are different things:

- `final_snapshot_id`
  - the engine's final context checkpoint artifact id
  - used by `AppServices` to load the final context JSON
- `snapshot_artifact_id`
  - the canonical portfolio snapshot artifact written by `WritePortfolioSnapshotState`

The app response exposes both when available.

### Where the final report comes from

`AppServices::start_portfolio_snapshot` does not rebuild the report itself.

It:

- loads the final context snapshot artifact
- decodes the snapshot JSON
- extracts the namespaced keys for:
  - `snapshot_artifact_id`
  - `report`

If the run failed:

- it scans the run stream backwards
- finds the last `StateFailed`
- maps that error into the app error contract

## Public Output Contract

The public app stdout contract is owned in two layers:

### CLI layer

The CLI wraps the portfolio response as:

- `FeatureExecutionResult`
- `feature_id = "portfolio.snapshot"`
- `result = <PortfolioSnapshotResponse>`

Then the JSON output renderer wraps that as:

```json
{
  "status": "success",
  "data": {
    "feature_id": "portfolio.snapshot",
    "result": { "...": "..." }
  }
}
```

### Shell wrapper layer

The shell task then validates with `jq` that:

- `.status == "success"`
- `.data.feature_id == "portfolio.snapshot"`
- `.data.result` exists

Only after that does it replay the raw CLI JSON to stdout unchanged.

So the public app is intentionally preserving `mfm_cli` as the final output-contract authority.

## Mismatches And Gotchas Discovered While Tracing

### 1. Docs are stale about workflow composition

Current code path:

- app -> task -> packaged CLI -> app services -> op

Stale docs still describe:

- app -> workflow -> hidden lifecycle tasks

Files to update eventually:

- `README.md`
- `docs/helios.md`

### 2. The task model explicitly requires Postgres, not Helios

The task's `requirements.services` includes:

- `postgres`

The shell code then opportunistically uses Helios when hooks are available.

That means there is a semantic mismatch between:

- model-declared service requirements
- shell-level optional Helios bootstrap behavior

### 3. Wallet support is narrower than the config surface

The wallet config model can express multiple implementations, but the current portfolio runtime
slice only supports:

- `address_only`

Any semantics work should call this out clearly.

### 4. Base and Aave flows deliberately converge on one observation model

This is a useful design invariant:

- base balances and Aave positions are different acquisition paths
- both are normalized into the same canonical `Observation`
- downstream snapshot/report code stays generic

### 5. The replay boundary is the pinned network view plus canonical observations

This is the important semantic center of the portfolio flow:

- freeze network/block pins
- read direct prices at that pinned view
- produce canonical observations
- persist canonical snapshot artifact
- derive report projection from snapshot

## File Anchors

Primary files touched during tracing:

- `flake.nix`
- `nixfied/project/module.nix`
- `bin/cli/src/commands/portfolio/snapshot.rs`
- `bin/cli/src/support/run_stores.rs`
- `bin/cli/src/support/app_services.rs`
- `bin/cli/src/presentation/output.rs`
- `crates/app/src/lib.rs`
- `crates/ops/portfolio-tracker-op/src/lib.rs`
- `crates/evm-runtime/src/states/rpc_control.rs`
- `crates/states/wallet/src/states.rs`
- `crates/states/symbol/src/states.rs`
- `crates/states/aave-v3/src/portfolio/states.rs`
- `crates/states/portfolio/src/states.rs`
- `crates/sdk/src/unstable.rs`
- `docs/ops-and-states.md`
- `README.md`
- `docs/helios.md`

## High-Value Code Anchors

These were the fastest jump points during tracing. The line numbers are from the current checkout
and will drift over time.

- flake output wiring:
  - `flake.nix:9-37`
- app/task wiring:
  - `nixfied/project/module.nix:543-562`
  - `nixfied/project/module.nix:829-1072`
  - `nixfied/project/module.nix:1971-1979`
- CLI entrypoint:
  - `bin/cli/src/commands/portfolio/snapshot.rs:37-88`
  - `bin/cli/src/support/run_stores.rs:26-67`
  - `bin/cli/src/support/app_services.rs:13-33`
  - `bin/cli/src/presentation/output.rs:72-107`
  - `bin/cli/src/presentation/output.rs:224-247`
- app-layer run and extraction:
  - `crates/app/src/lib.rs:321-336`
  - `crates/app/src/lib.rs:456-580`
  - `crates/app/src/lib.rs:624-710`
  - `crates/app/src/lib.rs:913-1000`
- op planner:
  - `crates/ops/portfolio-tracker-op/src/lib.rs:71-83`
  - `crates/ops/portfolio-tracker-op/src/lib.rs:206-390`
- state implementations:
  - `crates/evm-runtime/src/states/rpc_control.rs:40-110`
  - `crates/states/wallet/src/states.rs:15-79`
  - `crates/states/symbol/src/states.rs:73-207`
  - `crates/states/symbol/src/states.rs:225-405`
  - `crates/states/symbol/src/states.rs:428-605`
  - `crates/states/aave-v3/src/portfolio/states.rs:33-160`
  - `crates/states/portfolio/src/states.rs:29-140`
  - `crates/states/portfolio/src/states.rs:149-315`
- context namespacing:
  - `crates/sdk/src/unstable.rs:467-500`
- inventory docs:
  - `docs/ops-and-states.md:46-77`
- stale docs to reconcile later:
  - `README.md:107-121`
  - `docs/helios.md:16-34`

## Commands Used To Trace This

Repo-local code search:

```bash
rg -n "mfm::portfolio::snapshot|portfolio.snapshot" .
```

Key file reads:

```bash
sed -n '780,1105p' nixfied/project/module.nix
sed -n '1,220p' bin/cli/src/commands/portfolio/snapshot.rs
sed -n '913,1008p' crates/app/src/lib.rs
sed -n '206,390p' crates/ops/portfolio-tracker-op/src/lib.rs
sed -n '68,605p' crates/states/symbol/src/states.rs
sed -n '29,315p' crates/states/portfolio/src/states.rs
sed -n '462,515p' crates/sdk/src/unstable.rs
```

App-surface verification:

```bash
nix eval --json '.#apps.<system>."mfm::portfolio::snapshot"'
```

Then inspect the launcher path returned by that command.

## Open Questions For The Real RFC

1. Should the public app go back to an explicit workflow model, or should the direct-task shape be
   treated as canonical?
2. Should Helios become an explicit model requirement, or should the app remain "Postgres required,
   Helios opportunistic, direct RPC bootstrap authoritative"?
3. Should the state semantics be documented as:
   - a portfolio-specific RFC
   - an expansion of `docs/ops-and-states.md`
   - or both?
4. Should the replay boundary be described primarily around:
   - pinned networks
   - canonical observations
   - canonical snapshot artifact
   rather than around the task wrapper?
5. Do we want a more explicit semantic distinction between:
   - snapshot production
   - report projection
   - stdout envelope ownership

## Working Conclusion

If this RFC gets turned into a more formal design document, the cleanest story appears to be:

- public app shell owns process/bootstrap/stdout semantics
- `mfm_cli` owns the public JSON payload contract
- `AppServices` owns run-launch and final extraction semantics
- `portfolio_tracker` owns domain planning semantics
- the shared states own the real runtime semantics

That decomposition matches the current code better than the stale workflow-centric docs.
