# Parity RPC Control Plane Problems

This document captures the current problems that prevent the parity lane from reliably using the repository's RPC control plane.

## Summary

The parity suite does not consistently route RPC calls through the control-plane-managed source model.

Today the repo has:

- A Postgres-backed control-plane storage crate for `rpc_source:*` stream families and `mfm_rpc_source_state` projections in `crates/storages/control-plane-postgres`.
- An EVM HTTP transport that is configured from environment variables and maintains source health in process memory in `crates/collectors/evm-jsonrpc-http`.
- Parity tests and parity CI wiring that still use raw `MFM_EVM_RPC_URL` access and ad hoc transport construction.

Because of that split, parity RPC behavior is not guaranteed to use the control plane in a correctness-critical sense.

## Intended Model

The intended model should be:

- `MFM_EVM_RPC_URL` and `MFM_EVM_RPC_SOURCES_JSON` are source-registration inputs only.
- Application code, CLI code, REST code, ops, and parity tests should not select raw URLs directly.
- All canonical EVM RPC calls should go through `rpc.control`.
- This includes reads, writes, setup, source probes, and source ranking updates.
- The routed EVM transport should be an internal executor selected by `rpc.control`, not the public canonical ingress.
- The routed transport should consult the RPC control plane for source health, probe capability, cooldown, and source selection.
- The transport should then send the HTTP request directly to the selected endpoint.
- The transport should append durable `rpc_source:*` observation and probe records back into the control plane after calls.

In that model, the control plane does not need to be a separate HTTP proxy. It can remain an in-process routing decision layer backed by correctness-critical Postgres state.

The important consequence is that `MFM_EVM_RPC_URL` is a bootstrap input for registering one available source, not a caller-facing escape hatch.

Setting `MFM_EVM_RPC_URL` or `MFM_EVM_RPC_SOURCES_JSON` in a harness is therefore not itself a bug. The bug is letting canonical callers read those raw values directly, or letting env-derived config remain the routing authority instead of a control-plane setup/sync step.

Likewise, `rpc_source_id` is best treated as a migration bridge, not a final canonical request contract. During cutover it is useful for replacing raw URLs with stable named sources such as `reth_local` and `helios_local`. After cutover, canonical request models should stop asking callers to choose source ids directly and let `rpc.control` select sources internally.

This is also consistent with the existing transport contract that already rejects per-request `rpc_url` overrides in `crates/collectors/evm-jsonrpc-http/src/lib.rs`.

## Problems

### 1. The control-plane store exists, but the transport stack does not use it

The Postgres control-plane crate provides durable `rpc_source:*` records and a rebuildable `mfm_rpc_source_state` projection:

- `ControlPlanePostgresStore::connect` in `crates/storages/control-plane-postgres/src/lib.rs`
- `ControlPlanePostgresStore::append_rpc_source_records` in `crates/storages/control-plane-postgres/src/lib.rs`
- `ControlPlanePostgresStore::rpc_source_state` in `crates/storages/control-plane-postgres/src/lib.rs`

But the application transport stack is still bootstrapped from env-derived transport config:

- `EvmJsonRpcHttpTransportFactory::from_env` in `crates/collectors/evm-jsonrpc-http/src/lib.rs`
- transport registration in `crates/app/src/lib.rs`

There is no code path in the parity runtime where the EVM transport consults `ControlPlanePostgresStore` before selecting a source, or records observations/probes into `rpc_source:*` streams after calls.

## 2. The EVM transport is env-driven, not control-plane-driven

The EVM transport resolves sources from:

- `MFM_EVM_RPC_SOURCES_JSON`
- falling back to `MFM_EVM_RPC_URL`

This happens in `resolve_evm_rpc_sources_from_env` in `crates/collectors/evm-jsonrpc-http/src/lib.rs`.

The legacy fallback synthesizes a single source with id `user_primary` from `MFM_EVM_RPC_URL`.

That means the transport can function entirely without any control-plane state. Source registration, source ordering, and endpoint metadata are runtime env inputs rather than control-plane-managed state.

## 3. Health and cooldown logic live inside the process, not in the control plane

The EVM transport currently tracks source health and routing behavior in memory inside the transport implementation in `crates/collectors/evm-jsonrpc-http/src/lib.rs`.

Examples include:

- source ordering
- unhealthy source handling
- failover / hedged-light behavior
- per-process probe state

This makes parity behavior process-local rather than control-plane-coordinated. Even if the control-plane store exists, parity runs do not depend on it for source health decisions.

## 4. Shared parity CI wiring still treats a raw RPC URL as the operative Reth contract

The shared parity service environment in `nixfied/project/module.nix` exports:

- `MFM_EVM_RPC_URL="http://127.0.0.1:$RETH_HTTP_PORT"`

Exporting that bootstrap input is not inherently wrong.

The problem is that the parity lane still treats the raw URL as the operative contract:

- there is no required setup/sync step that turns that bootstrap input into control-plane-managed source state
- parity code still reads the raw URL directly instead of going through named control-plane-managed sources

The `ci-parity-evm-reth` task in `nixfied/project/module.nix` runs the parity integration tests under that environment.

## 5. Reth parity tests read `MFM_EVM_RPC_URL` directly

Several parity tests directly read `MFM_EVM_RPC_URL` and use it as an endpoint:

- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs`
- `tests/integration/tests/parity_aave_v3_reth_scenario.rs`
- `bin/cli/tests/parity_keystore_reth_tx_send.rs`

This is a direct bypass of a control-plane-first routing contract.

## 6. Several parity helpers build ad hoc one-off transports from literal URLs

Some parity tests do not even rely on the normal app transport bootstrap. Instead they construct their own `EvmJsonRpcHttpConfig` with a single literal URL source inside test helper functions.

Examples:

- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs`

These helpers create explicit source lists such as `helper_primary` bound directly to the raw `rpc_url` test variable.

That bypasses:

- app-level transport construction
- shared source registry behavior
- any future control-plane-backed resolution logic unless those helpers are removed

## 7. During migration, several parity request payloads do not specify `rpc_source_id`

Parity fixtures for portfolio and Aave requests often set:

- `"rpc_source_id": null`

Examples:

- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs`
- `tests/integration/tests/parity_aave_v3_reth_scenario.rs`

The state logic does support routing hints via `rpc_source_id`:

- portfolio state routing in `crates/states/portfolio/src/states.rs`
- Aave portfolio routing in `crates/states/aave-v3/src/portfolio/states.rs`

During migration, this matters because named source ids are the safest bridge away from raw URL selection.

This is not the final-state contract. In the final canonical design, request-level `rpc_source_id` disappears from normal read APIs and source selection moves into `rpc.control`.

But parity fixtures are not even consistently using the transitional source-id surface today, so the parity lane is not yet asserting named source identity at the request boundary during cutover.

## 8. The legacy `user_primary` fallback weakens source identity guarantees

When only `MFM_EVM_RPC_URL` is set, the transport synthesizes a source id named `user_primary`.

That has two problems:

- it is not a canonical environment-specific source id like `reth_local` or `helios_local`
- it allows tests and commands to succeed without explicit source registration

This weakens the guarantee that parity traffic is routed through stable, named, control-plane-managed sources.

## 9. The keystore parity path still succeeds through the legacy env fallback

The CLI keystore send flow resolves a source id from CLI input or `MFM_EVM_RPC_SOURCE_ID` in `bin/cli/src/support/command_defaults.rs`.

However, the parity keystore test uses:

- direct `MFM_EVM_RPC_URL` reads for chain queries
- `user_primary` for raw transaction submission

See `bin/cli/tests/parity_keystore_reth_tx_send.rs`.

So even this path, which is closer to source-id routing than other parity tests, still depends on the legacy env fallback rather than an explicit control-plane-owned source identity.

## 10. Helios parity wiring is better, but still bootstrap-plus-route-hint rather than control-plane-backed

The packaged Helios snapshot path in `nixfied/project/module.nix` sets:

- `MFM_EVM_RPC_URL`
- `MFM_EVM_RPC_SOURCES_JSON`
- `MFM_EVM_RPC_PREFERRED_ORDER`
- `MFM_EVM_RPC_SOURCE_ID`

and uses `helios_local` in the request payload.

This is better than the Reth parity path because it uses an explicit source id.

The remaining issue is not that bootstrap env is present. The issue is that parity still stops at env bootstrap plus a caller-selected source id.

It does not prove that the transport is consulting `ControlPlanePostgresStore` or writing `rpc_source:*` observations/probes to Postgres during parity execution.

## 11. There is no enforcement that parity traffic updates control-plane state

There is currently no parity assertion that:

- `mfm_rpc_source_state` rows exist for the sources used by parity
- `rpc_source:*` stream records were appended during parity execution
- control-plane health state influenced routing decisions

Without those assertions, parity can continue to pass while bypassing the control plane.

## 12. There is no enforcement against direct raw-URL bypasses in parity code

There is no CI or test-level guard preventing parity code from:

- reading `MFM_EVM_RPC_URL` directly
- constructing explicit literal-URL `EvmJsonRpcHttpConfig` instances
- leaving `rpc_source_id` unset in parity request fixtures while source-id bridging remains in place

That makes regressions easy even if some parts of the parity lane are later moved closer to the control plane.

## 13. The current top-level architecture docs still leave the ownership boundary ambiguous

The architecture doc says correctness-critical control-plane coordination belongs in the sibling Postgres storage crate and must not widen `StreamStore` in v1:

- `docs/architecture.md`

That boundary is reasonable, but the current top-level docs still do not make the intended ownership split explicit enough:

- endpoint URLs and auth remain runtime bootstrap inputs only
- durable control-plane state owns sanitized source identity, health/probe state, cooldown, and pool/ranking state
- canonical callers should not treat bootstrap env as the routing authority

Today endpoint URLs are still env-owned via `EvmJsonRpcSource` in `crates/collectors/evm-jsonrpc-http/src/lib.rs`, while health/probe persistence is isolated in `crates/storages/control-plane-postgres`.

The intended split is already described more clearly in `RPC_CONTROL_PLANE_WIRE_UP.md`, but `docs/architecture.md` has not yet been updated to make that boundary normative.

Until that architecture contract is updated in the top-level docs and then implemented in code, parity cannot be said to be fully using the RPC control plane.

## Current Non-Control-Plane EVM RPC Usages

This section inventories current EVM RPC usages that still bypass the intended control-plane-first model, or that preserve the legacy raw-URL fallback contract.

Not all of these callsites are equally problematic:

- runtime and parity callsites are the main blockers
- transport-level test fixtures are acceptable in limited cases, but should remain explicitly scoped to transport testing and not parity behavior
- documentation and examples matter because they encode the operating model contributors will copy

### A. Runtime and transport bootstrap still depend on env-driven source resolution

- `crates/collectors/evm-jsonrpc-http/src/lib.rs`
  - `resolve_evm_rpc_sources_from_env`
  - `resolve_evm_rpc_config_from_env`
  - legacy single-source fallback to `user_primary`
- `crates/app/src/lib.rs`
  - `start_portfolio_snapshot` fails fast based on `resolve_evm_rpc_sources_from_env()` rather than control-plane source state

These are not parity-only issues. They are the core reason the runtime can function without consulting the control plane.

### B. Nix tasks and local tooling still use raw RPC URLs directly

- `nixfied/project/module.nix`
  - shared parity env exports `MFM_EVM_RPC_URL` directly from `RETH_HTTP_PORT`
  - the Helios smoke task curls `MFM_EVM_RPC_URL` directly
  - the packaged Helios snapshot path still reconstructs `MFM_EVM_RPC_URL` even when source-id routing env is also present
- `nixfied/project/aave-origin-tools.nix`
  - requires `MFM_EVM_RPC_URL`
  - passes `--rpc-url "$MFM_EVM_RPC_URL"` to `forge script`

These paths bypass the control-plane-first contract at the harness and tool boundary.

### C. Parity integration tests that read raw RPC URLs directly

- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
  - reads `MFM_EVM_RPC_URL`
  - performs setup RPC calls through a helper that accepts a literal `rpc_url`
- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
  - reads `MFM_EVM_RPC_URL`
  - performs setup RPC calls through a helper that accepts a literal `rpc_url`
- `tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs`
  - reads `MFM_EVM_RPC_URL`
  - performs setup RPC calls through a helper that accepts a literal `rpc_url`
- `tests/integration/tests/parity_aave_v3_reth_scenario.rs`
  - reads `MFM_EVM_RPC_URL`
  - performs setup RPC calls through a helper that accepts a literal `rpc_url`
- `bin/cli/tests/parity_keystore_reth_tx_send.rs`
  - reads `MFM_EVM_RPC_URL`
  - performs setup RPC calls against the raw URL

These are the highest-signal parity bypasses and should be removed first.

### D. Parity helpers that construct one-off direct source registries

The following parity tests build `EvmJsonRpcHttpConfig` directly from literal URLs using helper source ids like `helper_primary`:

- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs`
- `tests/integration/tests/parity_aave_v3_reth_scenario.rs`

These callsites are especially important because they bypass:

- app-level transport registration
- any future control-plane-backed source lookup
- any shared parity enforcement around canonical source ids such as `reth_local` or `helios_local`

### E. Parity callsites still using the legacy `user_primary` identity

- `bin/cli/tests/parity_keystore_reth_tx_send.rs`
  - uses `run_tx_send_raw(..., "user_primary")`
  - asserts returned source identity is `user_primary`

This preserves the legacy single-source fallback model instead of exercising a canonical named source such as `reth_local`.

### F. Direct raw HTTP JSON-RPC helper usage in parity

- `bin/cli/tests/parity_keystore_reth_tx_send.rs`
  - `rpc_request` uses `reqwest::Client::post(rpc_url)` directly

Even if the final send path uses routed IO, this helper still bypasses the routed transport for setup and verification calls.

### G. Parity and example request payloads that leave `rpc_source_id` unset

Current source-less request payloads include:

- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs`
- `tests/integration/tests/parity_aave_v3_reth_scenario.rs`
- `tests/integration/tests/rest_api_run_control.rs`
- `bin/cli/src/commands/portfolio/snapshot.rs`
- `crates/states/aave-v3/src/portfolio/model.rs`

Some of these are examples or non-parity fixtures, but they still normalize the idea that `rpc_source_id` may be omitted in EVM-facing requests.

During migration that weakens the cutover away from raw URLs, because the bridge contract is supposed to move callers from direct URLs to stable named sources first.

It should not be read as a claim that `rpc_source_id` is part of the final canonical API. In the final canonical design, request-level `rpc_source_id` disappears and `rpc.control` owns source choice.

### H. Transport-level integration tests that intentionally build direct source configs

These tests build direct source registries from stub URLs:

- `tests/integration/tests/evm_rpc_pool_failover.rs`
- `tests/integration/tests/evm_rpc_getlogs_chunking.rs`
- `crates/collectors/evm-jsonrpc-http/src/tests/evm_jsonrpc_http_tests.rs`

These are reasonable as transport-scoped tests, but they should be treated as controlled exceptions rather than evidence that parity or runtime behavior is using the control plane correctly.

### I. Legacy docs and examples that still teach the fallback model

- `docs/evm-rpc-routing.md`
  - documents `MFM_EVM_RPC_URL` mapping to `user_primary`
- `bin/rest-api/README.md`
  - documents `MFM_EVM_RPC_URL` as a single-source fallback endpoint
- `bin/cli/README.md`
  - uses `--source-id user_primary` in examples
  - documents `MFM_EVM_RPC_SOURCE_ID="user_primary"`
  - documents legacy `MFM_EVM_RPC_URL` fallback
- `docs/helios.md`
  - shows reconstructed runtime env including `MFM_EVM_RPC_URL`

Even when some of these documents mention source-id routing, they still preserve the operator mental model that raw URLs are a normal first-class calling surface.

## Operational Consequence

Right now parity success does not prove any of the following:

- that source routing decisions were control-plane-backed
- that health / cooldown state was durable across processes
- that parity exercised `rpc_source:*` append-only coordination
- that parity requests were pinned to stable named sources instead of raw URLs

In practice, parity currently proves RPC functionality against local Helios/Reth endpoints, but not full RPC control-plane integration.

## Appendix A: Refactor Plan To Eliminate The Bypasses

This appendix turns the problem inventory into a concrete refactor sequence.

The plan assumes the intended model above:

- env-driven RPC URLs remain bootstrap source-registration inputs only
- canonical callers use `rpc.control` and routed IO only
- request-level `rpc_source_id` and route hints are transitional migration surfaces only
- the control plane owns source health, probe capability, cooldown, and durable route state
- the transport consults the control plane and then executes the HTTP request directly against the selected endpoint
- setup, probes, ranking, reads, and writes are all part of the control-plane-owned path

### Phase 0. Lock The Contract First

Objective:

- freeze the architectural boundary before code moves

Work:

- treat `MFM_EVM_RPC_URL` as bootstrap-only, not a caller-facing contract
- declare canonical local source ids for setup/bootstrap only:
  - `reth_local`
  - `helios_local`
- update the top-level architecture contract so:
  - `rpc.control` is canonical ingress
  - `evm` is executor-only
- document that raw URL selection is not allowed in canonical runtime or parity code
- document that transport-level stub tests may remain direct, but only as explicitly scoped test fixtures
- document that `rpc_source_id` and route hints are migration-only and disappear from final canonical read APIs
- document that source setup and ranking happen through `rpc.control` setup/probe/rank ops and workflows

Files to update:

- `RPC_CONTROL_PLANE_WIRE_UP.md`
- `PROBLEMS_PARITY_RPC_CONTROL_PLANE.md`
- `docs/architecture.md`
- `docs/evm-rpc-routing.md`
- `bin/cli/README.md`
- `bin/rest-api/README.md`
- `docs/helios.md`

Exit criteria:

- one written contract says which surfaces are canonical
- one written contract says which remaining direct surfaces are explicitly non-canonical

### Phase 1. Add A Control-Plane Bridge To The EVM Transport

Objective:

- make the existing routed EVM transport depend on control-plane state instead of only process-local memory

Work:

- add a control-plane-backed adapter around `EvmJsonRpcHttpTransportFactory`
- add an idempotent startup setup step that syncs runtime source bootstrap config into durable
  control-plane pool state
- model that setup step as reusable states plus thin `rpc.control` setup ops/workflows, not as ad hoc
  application boot code
- add reusable probe and ranking ops/workflows that record response time per source and publish pool
  ranking snapshots per network
- before source selection:
  - read durable source state from the control plane
  - apply health, cooldown, and capability constraints from `mfm_rpc_source_state`
- after calls and probes:
  - append `RpcSourceObservedRecord`
  - append `RpcSourceProbedRecord`
- keep endpoint URL and authorization runtime-only
  - do not persist raw URLs or auth headers into control-plane storage
- keep `evm-jsonrpc-http` as the HTTP executor, not the durable routing authority
- keep any source-id-based bridging explicitly transitional

Primary code areas:

- `crates/collectors/evm-jsonrpc-http/src/lib.rs`
- `crates/storages/control-plane-postgres/src/lib.rs`
- possibly a new adapter module/crate if a clean bridge layer is needed

Design note:

- this phase does not require the control plane to become a separate proxy service
- it only requires the routed transport to consult and update durable control-plane state

Exit criteria:

- EVM source selection and probe state are no longer purely process-local
- successful and failed live calls update `rpc_source:*` durable records
- startup source setup is idempotent and required before canonical request handling
- ranking snapshots are durable and reusable across later canonical RPC calls

### Phase 2. Rebind App Bootstrap And Canonical Runtime Surfaces

Objective:

- ensure canonical runtime entrypoints instantiate the control-plane-backed transport rather than the env-only transport

Work:

- replace env-only transport bootstrap in `mfm-app`
- make canonical app startup run control-plane source setup before request handling
- make canonical app startup run control-plane probe and ranking workflows before request handling
- remove env-only assumptions from higher-level fail-fast checks
- begin migrating callers away from route hints/source ids entirely

Primary code areas:

- `crates/app/src/lib.rs`
- `bin/rest-api/src/lib.rs`
- `bin/rest-api/src/main.rs`
- `bin/cli/src/support/command_defaults.rs`
- any CLI command wiring that still normalizes legacy `user_primary` as a canonical identity

Exit criteria:

- canonical app/CLI/REST entrypoints no longer treat env-only source resolution as the authority
- canonical write/read surfaces use `rpc.control` or the transitional bridge toward it

### Phase 3. Migrate Parity Harness And Local Task Wiring

Objective:

- stop parity from booting and probing RPC nodes through raw URLs

Work:

- update parity env wiring to export canonical source ids and source registries, not raw-URL-only contracts
- add parity bootstrap workflow that seeds the control-plane source rows required for:
  - `reth_local`
  - `helios_local`
- replace raw curl-based parity probes with `rpc.control` setup/probe/rank workflows and
  control-plane-aware checks
- decide whether direct forge-based deploy tooling stays:
  - if it remains temporarily, keep it outside the canonical managed path
  - final milestone should migrate it to control-plane-backed submission or remove it from the
    canonical workflow

Primary code areas:

- `nixfied/project/module.nix`
- `nixfied/project/aave-origin-tools.nix`

Exit criteria:

- parity services start with canonical named source ids
- parity setup no longer depends on `MFM_EVM_RPC_URL` as the main contract
- parity preflight publishes durable ranking state before tests execute

### Phase 4. Remove Parity Test Bypasses

Objective:

- make parity tests exercise the same routed/control-plane-backed path used by canonical runtime code

Work:

- replace direct `std::env::var("MFM_EVM_RPC_URL")` setup patterns with a shared routed helper
- remove parity helpers that construct one-off `EvmJsonRpcHttpConfig` registries from literal URLs
- remove direct raw HTTP JSON-RPC helpers from parity tests
- during migration, use setup-defined canonical source ids instead of `user_primary`
- by final cutover, remove request-level `rpc_source_id` from canonical parity request models too

Primary code areas:

- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
- `tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs`
- `tests/integration/tests/parity_aave_v3_reth_scenario.rs`
- `bin/cli/tests/parity_keystore_reth_tx_send.rs`

Recommended implementation detail:

- add one shared parity RPC helper in `tests/integration/src/` that accepts source ids, not raw URLs

Exit criteria:

- no parity test constructs an ad hoc literal-URL source registry
- no parity test reads `MFM_EVM_RPC_URL` directly
- parity uses the same control-plane-backed path as canonical runtime flows

### Phase 5. Clean Up Canonical Examples And Default Fixtures

Objective:

- stop teaching the old fallback model in docs, examples, and default request fixtures

Work:

- replace `user_primary` in canonical docs/examples with explicit named sources
- remove or downgrade docs that present `MFM_EVM_RPC_URL` as a normal first-class calling contract
- update transitional examples to use setup-defined source ids only where required during migration
- remove `rpc_source_id` from final canonical request examples

Primary code areas:

- `docs/evm-rpc-routing.md`
- `bin/cli/README.md`
- `bin/rest-api/README.md`
- `docs/helios.md`
- `bin/cli/src/commands/portfolio/snapshot.rs`
- `crates/states/aave-v3/src/portfolio/model.rs`

Exit criteria:

- contributor-facing docs teach source-id routing as the normal path
- example payloads no longer normalize source-less canonical EVM requests

### Phase 6. Add Enforcement And Regression Tests

Objective:

- make it hard to reintroduce direct raw-URL bypasses after the refactor lands

Work:

- add CI checks that fail on:
  - direct `MFM_EVM_RPC_URL` reads in parity/canonical runtime code
  - ad hoc `EvmJsonRpcHttpConfig` construction in parity tests
  - canonical parity fixtures with `rpc_source_id: null`
- add integration assertions that parity execution appends `rpc_source:*` records
- add integration assertions that setup/probe/rank workflows append `source_pool:*` ranking records
- add integration assertions that `mfm_rpc_source_state` rows exist and change during parity execution
- add restart/replay checks proving source health state is durable across process boundaries

Primary validation areas:

- parity CI workflow
- integration test suite
- control-plane Postgres state assertions

Exit criteria:

- parity passing proves the control plane was exercised
- obvious raw-URL regressions fail CI immediately

### Phase 7. Remove Or Rename Remaining Direct Surfaces

Objective:

- remove public canonical bypasses completely and leave only tightly scoped internal test fixtures

Work:

- keep transport-unit tests that need direct stub URLs, but clearly treat them as scoped exceptions
- remove public operator/dev bypass tools from canonical surfaces
- remove compatibility language that makes direct/raw paths look canonical

Exit criteria:

- there is no ambiguity about which RPC paths are canonical and which are explicit bypass tools
- there are no surviving public canonical bypass tools

## Appendix A Acceptance Criteria

The problem is considered solved only when all of the following are true:

- canonical runtime EVM traffic does not select raw URLs directly
- canonical runtime EVM traffic enters through `rpc.control`
- canonical setup, probe, ranking, reads, and writes all enter through `rpc.control`
- parity tests do not read `MFM_EVM_RPC_URL` directly
- parity tests do not build literal-URL one-off source registries
- startup source setup is required and idempotent
- source ranking is produced by reusable control-plane probe/rank workflows and reused by later calls
- transitional source ids do not leak into final canonical request APIs
- control-plane durable state participates in source selection
- live probes and call outcomes append `rpc_source:*` records
- parity success demonstrates control-plane-backed routing, not just node reachability
- no current public direct/raw EVM surfaces remain canonical after cutover
