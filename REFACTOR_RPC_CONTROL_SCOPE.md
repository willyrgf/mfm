# rpc-control scope refactor handoff

## Summary

- Root cause: parity tests that use shared persisted `rpc-control` state can race on shared control-plane streams when they fall back to the global default scope `__default__`.
  - This includes the dedicated Postgres control-plane store.
  - It also includes `StreamStore` mode when the underlying stream store is shared Postgres.
- Symptom seen in GHA:
  - `code=control_plane_concurrency`
  - `source_pool head seq did not match expected seq for source_pool:__default__:default`
- Prior commit `7e93e614cf155f0beee93229e98aa998504825e7` was the wrong fix for this issue.
  - That commit isolated `CARGO_TARGET_DIR` by task.
  - This failure is in persisted `rpc-control` state, not Cargo build outputs.
- `7e93e61` has been reverted in the current worktree with `git revert --no-commit`.

## Decisions Locked In

- Replay/cutover:
  - no backward compatibility for pre-refactor `rpc.control` fact identity
  - this is a dev-branch hard cutover
  - old runs that depend on pre-scope `rpc.control` facts are not expected to resume/replay after rollout
- Long-term model:
  - `rpc.control` gets a first-class `control_scope` concept as a production surface.
  - `control_scope` is request-visible and participates in durable fact identity.
- Network identity:
  - canonical managed `rpc.control` calls must provide explicit `network_id`
  - canonical managed `rpc.control` calls must carry explicit effective `control_scope`
  - no canonical managed fallback to a synthetic global/networkless scope such as `__default__`
  - no caller-controlled `rpc_source_id` / `route.source_id` in the canonical managed request surface
  - bootstrap sources that omit `network_id` are invalid in this cutover
  - states/ops that currently omit `network_id` must be updated in the same change
- Surface cleanup:
  - remove public/direct EVM write ingress that bypasses or competes with `rpc.control`
  - keep legitimate deploy/configure/send workflow behavior only as callers of canonical `rpc.control`
  - remove raw-send compatibility surfaces instead of preserving them
  - remove `keystore_tx_send_raw`
  - remove `mfm keystore tx-send-raw`
  - remove `MFM_EVM_RPC_SOURCE_ID`
  - remove legacy single-source bootstrap fallback `MFM_EVM_RPC_URL` / `MFM_EVM_RPC_AUTHORIZATION`
- Rollout/migration:
  - use an explicit control-plane reset
  - do a targeted delete of `rpc_source:*` and `source_pool:*` records/heads from the shared stream tables
  - drop and recreate control-plane projection tables so the new schema is applied cleanly
  - do not migrate or rewrite old `__default__`-keyed control-plane streams
- Catalog safety:
  - same-scope different-catalog usage is a hard error
  - catalog identity is durably declared append-only in `source_pool:*`
  - fingerprint validation is per `(control_scope, network_id, pool_kind)`
  - `rpc.control` owns source selection internally; execution must only use sources inside the effective declared catalog
  - current slice uses `pool_kind = default`, so operationally this is still one catalog per scope/network today

## What failed

- Failing test in GHA:
  - [tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs)
  - test name: `parity_portfolio_tracker_snapshot_with_mock_erc20_mint`
- Failure surfaced during setup pipeline deploy:
  - `setup pipeline failed: run_id=... state_failed=evm_mock_erc20_setup.deploy.deploy ... code=control_plane_concurrency ... source_pool head seq did not match expected seq for source_pool:__default__:default`
- GHA context:
  - macOS full CI hit the real failure first
  - Ubuntu full CI was later canceled
  - `peak_workers=3`, so parity work overlapped against shared services/state

## Why it happens

### Current fallback behavior

- `rpc-control` request shape already has a real domain field:
  - [crates/collectors/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/collectors/rpc-control/src/lib.rs)
  - `JsonRpcCall.network_id: Option<String>`
- Transport resolves scope from:
  - requested `network_id`, or
  - routed source `network_id`, or
  - global fallback `__default__`
- Fallback logic lives in:
  - [crates/transports/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/transports/rpc-control/src/lib.rs)
- Current public/request bootstrap surface still includes transitional compatibility that this refactor removes:
  - caller-controlled `route.source_id`
  - networkless/bootstrap-global sources with `network_id: None`
  - legacy single-source env fallback `MFM_EVM_RPC_URL` / `MFM_EVM_RPC_AUTHORIZATION`
- Some current production states/ops still issue canonical managed calls without `network_id`:
  - [crates/evm-runtime/src/states/read.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/states/read.rs)
  - [crates/evm-runtime/src/states/write.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/states/write.rs)
  - [crates/evm-runtime/src/rpc.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/rpc.rs)
  - [crates/ops/evm-read-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/evm-read-op/src/lib.rs)
  - [crates/ops/evm-write-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/evm-write-op/src/lib.rs)
  - [crates/states/keystore-submit/src/tx.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/states/keystore-submit/src/tx.rs)
  - [crates/ops/keystore-tx-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/keystore-tx-op/src/lib.rs)
- This refactor intentionally removes that canonical managed fallback behavior.

### Test helper behavior

- Integration helper builds sources with `network_id: None`:
  - [tests/integration/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/src/lib.rs)
- That means helper-driven managed RPC calls collapse into `__default__`.
- Multiple tests can then write the same persisted streams:
  - `source_pool:__default__:default`
  - `rpc_source:__default__:*`

### Persisted shared state

- Postgres control-plane append path:
  - [crates/storages/control-plane-postgres/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/storages/control-plane-postgres/src/lib.rs)
- Shared-stream append path used by integration helpers:
  - [tests/integration/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/src/lib.rs)
- The conflict is an optimistic concurrency failure on append.
- This is not a contract deployment bug.
- This is not a portfolio assertion bug.
- This is shared control-plane state collision.

## Why GHA reproduces and local usually does not

- GHA runs overlapping parity work with shared Postgres/reth/minio.
- Local runs are usually:
  - one test
  - one task
  - or otherwise less concurrent
- That makes the window between:
  - read current head seq
  - append with expected seq
  much more likely to collide in GHA.
- Not reproducing locally is consistent with a timing-sensitive shared-state race.

## Important design conclusion

- The right fix is not "each test has its own Rust `rpc-control` instance".
- The helper already creates fresh transport/client objects per call.
- The shared thing is persisted control-plane state.
- The right fix is "each isolated session gets its own `rpc-control` scope".

## Proposed production-safe model

### Add a first-class scope/namespace

- Add a new first-class field for `rpc-control` isolation:
  - canonical name: `control_scope`
- Keep `network_id` with its existing meaning:
  - which blockchain/network the request targets
- Require `network_id` for canonical managed `rpc.control` requests:
  - `RpcControlRequest::EvmCall`
  - `RpcControlRequest::PrepareSources`
  - if a canonical managed request omits `network_id`, return a structured error instead of resolving a synthetic fallback scope
- `control_scope` must be explicit in every effective `rpc.control` request shape:
  - `RpcControlRequest::EvmCall`
  - `RpcControlRequest::PrepareSources`
- Canonical managed request models must not expose caller-controlled source selection:
  - remove `route.source_id` / `rpc_source_id` from the public `rpc.control` request contract
  - `rpc.control` remains responsible for choosing the concrete executor source internally
- Canonical send flows stay inside `rpc.control`:
  - if `eth_sendRawTransaction` is still needed internally, it remains an internal `rpc.control` execution detail
  - do not preserve dedicated raw-send compatibility ops/CLI flows as a public surface
- Client/session defaults are allowed only as a convenience:
  - they must materialize the effective `control_scope` into the serialized request before fact-key hashing and `IoCall` emission
- Any response surface that echoes identity must keep the fields separate:
  - `network_id` continues to mean blockchain/network
  - `control_scope` continues to mean control-plane isolation namespace
- Persist control-plane state keyed by:
  - `(control_scope, network_id, source_id)` for source state
  - `(control_scope, network_id, pool_kind)` for pool state

### Default behavior

- Default production scope:
  - stable shared value such as `shared`
- If a caller omits scope and the system chooses `shared`, that effective value must still be stamped into the request before hashing/fact recording.
- Allow explicit per-session or per-workflow scopes when desired.
- `network_id` does not get a default:
  - canonical managed callers must provide it
  - remove canonical managed dependence on `__default__`
- Bootstrap sources do not get a wildcard/default network:
  - every configured source must declare explicit `network_id`
  - remove networkless/bootstrap-global source support in this cutover
- Legacy single-source bootstrap fallback is removed:
  - do not keep `MFM_EVM_RPC_URL`
  - do not keep `MFM_EVM_RPC_AUTHORIZATION`
  - runtime bootstrap comes from explicit catalog entries only

### Why this matters in production

- MFM will eventually run multiple independent sessions:
  - market data
  - portfolio snapshot
  - transaction execution
- Some sessions should share health/ranking state.
- Some sessions should not.
- Example:
  - portfolio snapshot and market data may intentionally share a scope if they use the same source catalog and benefit from shared health/cooldown knowledge
  - tx execution may use a distinct scope if it has stricter routing requirements or different source sets

### Catalog safety

- Shared scope across incompatible source catalogs is dangerous even without visible failures.
- Two sessions with different source sets can overwrite membership/ranking for the same pool.
- Recommended guard:
  - compute/store a catalog fingerprint per `(control_scope, network_id, pool_kind)`
  - reject writes or initialization when a process tries to use the same `(control_scope, network_id, pool_kind)` with a different effective catalog
- Store catalog identity durably as a new append-only `source_pool:*` record kind:
  - canonical record name: `pool_catalog_declared`
  - do not store catalog identity only in a mutable projection row or side table
  - the record should live on the same `source_pool:<control_scope>:<network_id>:<pool_kind>` stream family that already carries membership/ranking
- Fingerprint input should be the effective non-secret routing catalog for that `(control_scope, network_id, pool_kind)`:
  - fingerprint schema version
  - `control_scope`
  - `network_id`
  - `pool_kind`
  - effective candidate sources for that network
  - per-source non-secret routing fields:
    - `id`
    - `kind`
    - `require_get_proof_probe`
  - effective preferred order projected onto those candidate source ids
- Persist not only the digest but also the normalized non-secret catalog snapshot inside the append-only record payload:
  - this keeps the catalog identity auditable/retrievable without consulting ambient runtime config
  - it also preserves rebuildability from stream history alone
- Do not persist raw endpoint credentials in the fingerprint:
  - do not include `authorization`
  - do not include raw `rpc_url`
- If future safety requires distinguishing two sources with the same public routing metadata but different backends:
  - add an explicit non-secret source identity/revision field
  - do not persist raw URLs as a shortcut
- Validation flow should be:
  - derive the effective normalized catalog snapshot for `(control_scope, network_id, pool_kind)`
  - load the current `source_pool` projection
  - if no catalog declaration exists yet, append `pool_catalog_declared`
  - if the existing declared fingerprint matches, continue
  - if the existing declared fingerprint differs, fail before any membership/ranking/source writes
  - managed source selection/execution must only use sources that belong to the effective declared catalog
  - there is no public route-pinned compatibility path after this cutover
- If other states/ops need to consume this metadata later:
  - expose it through typed `rpc.control` request/response surfaces
  - do not let states/ops read storage crates directly

## Explicit Reset Strategy

- This refactor assumes an explicit control-plane reset before rollout.
- This reset is operationally mandatory:
  - it must complete before any refactor-era process starts against the shared database/stream store
- Repo-owned execution path:
  - inspect SQL only:
    - `nix run .#rpc-control-scope-reset -- --print-sql`
  - execute once against the target database:
    - `DATABASE_URL=postgresql://... nix run .#rpc-control-scope-reset -- --yes`
- Existing persisted `rpc_source:*` and `source_pool:*` state is disposable for this change.
- Do not attempt in-place migration from old identities such as:
  - `rpc_source:<network_id>:<source_id>`
  - `source_pool:<network_id>:<pool_kind>`
  - `__default__`
- Reset scope:
  - targeted delete only the control-plane stream families `rpc_source:*` and `source_pool:*` from the shared stream tables
  - do not wipe unrelated stream families
  - drop and recreate `mfm_rpc_source_state` and `mfm_source_pool_state`
  - recreate the projection schema with `control_scope`-aware identities and any new catalog-declaration fields
- Repo-owned reset task:
  - `DATABASE_URL=postgresql://... nix run .#rpc-control-scope-reset -- --yes`
  - review-only mode: `nix run .#rpc-control-scope-reset -- --print-sql`
- After reset, the new scope-aware identities become the only supported durable shape.
- This reset also defines the replay cutover:
  - pre-refactor `rpc.control` fact identity is not preserved
  - pre-cutover runs that depend on old `rpc.control` facts are not expected to resume/replay

## Proposed test model

- Use the same scope abstraction in tests that production will use.
- Do not add a test-only hack.

### Test isolation

- Most parity tests should set a unique scope per test.
- Good candidates:
  - test name
  - test name + random suffix
  - UUID

### Shared-scope verification

- Add at least one dedicated integration test that intentionally shares a scope across multiple sessions.
- That test should verify the desired production behavior for shared-session reuse.

### Replay and backend verification

- Add a replay/fact-key regression test:
  - same `state_id + method + params + network_id`
  - different `control_scope`
  - must not alias to the same durable fact binding
- Add an explicit cutover regression test for the new canonical behavior:
  - canonical managed calls without `network_id` must fail with a structured error
- Add an explicit regression test for custom/manual fact-key paths:
  - receipt polling and any remaining send helpers must not alias across different `control_scope`
- Add coverage for both control-plane persistence backends:
  - dedicated Postgres control-plane store
  - `StreamStore` mode over the shared stream substrate
- Add a cross-network shared-scope test:
  - same `control_scope`
  - different `network_id`
  - different effective catalogs
  - must not trip catalog-fingerprint mismatch
- Add a same `(control_scope, network_id)` different-catalog rejection test.
- Add a same `(control_scope, network_id, pool_kind)` same-catalog idempotence test:
  - repeated declaration of the same normalized catalog must be accepted
- Add a config-validation test:
  - bootstrap sources that omit `network_id` must be rejected
- Add a surface-removal test:
  - canonical `rpc.control` requests do not expose caller-controlled `route.source_id`
- Add a cutover/removal test:
  - raw-send compatibility op/CLI surfaces are removed rather than preserved

## What not to do

- Do not overload `network_id` with test IDs or session IDs.
  - `network_id` already has domain meaning in request models and runtime states.
- Do not hide `control_scope` only in transport-local configuration.
  - effective scope must participate in serialized request identity and replay facts
- Do not keep canonical managed support for synthetic networkless/global routing.
  - canonical managed callers must supply `network_id`
- Do not keep networkless/bootstrap-global source config.
  - every configured source must declare explicit `network_id`
- Do not keep caller-controlled source selection on canonical `rpc.control`.
  - remove `route.source_id` / `rpc_source_id` from the public request surface
- Do not keep raw-send compatibility surfaces.
  - remove the dedicated raw-send op/CLI/docs/tests instead of carrying them forward
- Do not interpret this refactor as deleting legitimate deploy/configure/send workflow behavior.
  - the change is to collapse public EVM network ingress onto `rpc.control`, not to remove write workflows entirely
- Do not keep `MFM_EVM_RPC_URL`, `MFM_EVM_RPC_AUTHORIZATION`, or `MFM_EVM_RPC_SOURCE_ID`.
- Do not rely on Cargo target isolation for control-plane races.
- Do not assume fresh process/object instances imply fresh control-plane state.
- Do not persist raw RPC URLs or authorization material in scope/catalog fingerprints.
- Do not store catalog fingerprint state only in mutable SQL projections/side tables.
- Do not let states/ops bypass `rpc.control` and read control-plane storage crates directly.

## Likely implementation shape

### Request/transport surface

- Extend `rpc-control` request/config path with an explicit `control_scope` value.
- Required places:
  - `JsonRpcCall`
  - `PrepareSources`
- Optional convenience places:
  - typed client default/sugar
  - transport factory configuration / env-derived defaulting
- Hard requirement:
  - if defaults are used, they must be normalized into the request before fact-key derivation
- Hard requirement:
  - canonical managed requests must provide `network_id`
  - update current read/write states/ops that omit `network_id` in the same change
- Hard requirement:
  - remove `JsonRpcRoute` / `route.source_id` from the public `rpc.control` request model
- Hard requirement:
  - remove transport logic that derives effective scope/network from caller-selected source ids
- Hard requirement:
  - reject bootstrap sources that omit `network_id`
- Hard requirement:
  - remove legacy single-source env fallback `MFM_EVM_RPC_URL` / `MFM_EVM_RPC_AUTHORIZATION`
- Hard requirement:
  - there is no dedicated raw-send compatibility request/CLI/op after this cutover
- The exact shape should preserve crate boundaries and avoid smuggling ambient globals into state logic.

### Caller and fact-key updates

- Update reusable EVM read/write state configs and helper functions so canonical managed calls carry explicit `network_id`.
- Update public send surfaces so any surviving EVM send path is routed through canonical `rpc.control` with explicit `network_id` and `control_scope`.
- Remove raw-send compatibility surfaces instead of porting them:
  - `keystore_tx_send_raw`
  - `mfm keystore tx-send-raw`
  - related CLI/default-env helpers and compatibility docs/tests
- Update custom fact-key paths that currently bypass `fact_key_for_request`:
  - receipt polling
  - any remaining send helpers
  - no scope/network aliasing is allowed after cutover

### Storage layer

- Update control-plane identity types so stream keys include scope.
- Current stream families already separate `rpc_source:*` and `source_pool:*`.
- Refactor identities so those become effectively scope-aware.
- Update both persistence paths together:
  - dedicated Postgres control-plane store
  - `StreamStore`-backed control-plane store
- Extend `source_pool:*` with append-only catalog declaration:
  - add `pool_catalog_declared`
  - persist both the fingerprint and the normalized non-secret catalog snapshot
  - rebuild projection state from stream history only
- Add durable catalog-fingerprint validation per `(control_scope, network_id, pool_kind)`.
- Apply the rollout as a reset plus schema recreation:
  - targeted delete control-plane stream families
  - drop/recreate projection tables
  - no in-place migration of old disposable state

### Integration helper

- Update the integration helper in:
  - [tests/integration/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/src/lib.rs)
- Add a way to pass explicit scope.
- Make helper-driven canonical managed calls require explicit `network_id`.
- Use unique per-test scope in Postgres-backed parity tests.

### Retry behavior

- Scope isolation is the real fix.
- After that, consider whether append retry logic should still be hardened:
  - current retry loop in `rpc-control` is small
  - additional backoff/retry may still help same-scope legitimate concurrent writers
- This is secondary, not primary.

## Concrete files to revisit

- [crates/collectors/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/collectors/rpc-control/src/lib.rs)
- [crates/transports/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/transports/rpc-control/src/lib.rs)
- [crates/storages/control-plane-postgres/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/storages/control-plane-postgres/src/lib.rs)
- [crates/evm-runtime/src/rpc.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/rpc.rs)
- [tests/integration/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/src/lib.rs)
- [docs/evm-rpc-routing.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/evm-rpc-routing.md)
- [bin/rest-api/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/rest-api/README.md)
- [bin/cli/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/cli/README.md)
- [docs/ops-and-states.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/ops-and-states.md)
- [crates/collectors/rpc-control/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/collectors/rpc-control/README.md)
- [crates/transports/rpc-control/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/transports/rpc-control/README.md)
- [tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs)
- [tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs)
- [tests/integration/tests/parity_aave_v3_reth_scenario.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_aave_v3_reth_scenario.rs)
- [tests/integration/tests/evm_rpc_pool_failover.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/evm_rpc_pool_failover.rs)
- [tests/integration/tests/evm_rpc_getlogs_chunking.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/evm_rpc_getlogs_chunking.rs)
- [crates/evm-runtime/src/states/read.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/states/read.rs)
- [crates/evm-runtime/src/states/write.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/states/write.rs)
- [crates/ops/evm-read-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/evm-read-op/src/lib.rs)
- [crates/ops/evm-write-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/evm-write-op/src/lib.rs)
- [crates/ops/keystore-tx-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/keystore-tx-op/src/lib.rs)
- [crates/states/keystore-submit/src/tx.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/states/keystore-submit/src/tx.rs)
- [bin/cli/src/commands/keystore/tx_send_raw.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/cli/src/commands/keystore/tx_send_raw.rs)
- [bin/cli/src/support/command_defaults.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/cli/src/support/command_defaults.rs)
- [docs/architecture.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/architecture.md)
- [docs/redesign.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/redesign.md)

## Suggested implementation order

1. Remove caller-controlled source selection from the public `rpc.control` surface:
   - remove `route.source_id` / `rpc_source_id`
   - remove dedicated raw-send compatibility surfaces
2. Add first-class request-visible `control_scope` to `rpc.control` request/response/client surfaces.
3. Require explicit `network_id` for canonical managed calls and update current read/write states/ops that omit it.
4. Remove networkless/bootstrap-global source support and legacy single-source env fallback:
   - every configured source must declare `network_id`
   - remove `MFM_EVM_RPC_URL` / `MFM_EVM_RPC_AUTHORIZATION`
5. Make `control_scope` part of durable fact identity by normalizing defaults into serialized requests before hashing.
6. Update custom/manual fact-key paths so they do not alias across different `control_scope` / `network_id`.
7. Treat this as a hard cutover:
   - no backward compatibility for pre-refactor `rpc.control` fact identity
8. Key persisted control-plane identities by `control_scope` plus existing network/source dimensions.
9. Extend `source_pool:*` with append-only `pool_catalog_declared` and projection support.
10. Add catalog validation per `(control_scope, network_id, pool_kind)`.
11. Update both control-plane persistence backends to the new identity shape.
12. Define and execute the explicit control-plane reset:
   - targeted delete of `rpc_source:*` / `source_pool:*`
   - drop/recreate projection tables
13. Plumb explicit scope through integration helpers and any app-facing configuration surfaces.
14. Give each shared parity test/session a unique scope where isolation is required.
15. Add:
   - one intentional shared-scope integration test
   - one cross-network shared-scope test
   - one replay/fact-key isolation regression test
   - one no-`network_id` canonical rejection test
   - one networkless-source config rejection test
   - one same-catalog idempotence test
   - one removed-surface regression for raw-send/source-pin compatibility removal
16. Consider whether same-scope retry/backoff should be improved.

## Commit-by-commit checklist

The sequence below is the recommended implementation plan for landing this refactor on `dev`.
Each commit is intended to be reviewable and to leave the tree in a coherent state.
Commit subjects below follow the repo convention and are written in lower case.

### Commit 1

- Subject:
  - `remove raw-send and route-pinned rpc-control compatibility surfaces`
- Goal:
  - cut the non-canonical public ingress first so every surviving caller is aligned around managed `rpc.control`
- Required changes:
  - remove `JsonRpcRoute` / `route.source_id` from the public `rpc.control` request model
  - remove `keystore_tx_send_raw`
  - remove `mfm keystore tx-send-raw`
  - remove `MFM_EVM_RPC_SOURCE_ID`
  - unregister the raw-send op from the default app bundle
  - delete compatibility docs/tests that only exist for route-pinned/raw-send behavior
- Primary files:
  - [crates/collectors/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/collectors/rpc-control/src/lib.rs)
  - [crates/ops/keystore-tx-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/keystore-tx-op/src/lib.rs)
  - [crates/states/keystore-submit/src/tx.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/states/keystore-submit/src/tx.rs)
  - [bin/cli/src/commands/keystore/tx_send_raw.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/cli/src/commands/keystore/tx_send_raw.rs)
  - [bin/cli/src/support/command_defaults.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/cli/src/support/command_defaults.rs)
  - [crates/app/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/app/src/lib.rs)
  - [docs/ops-and-states.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/ops-and-states.md)
- Verification:
  - `nix run .#check`
  - targeted grep confirms no remaining public `tx-send-raw`, `keystore_tx_send_raw`, or `MFM_EVM_RPC_SOURCE_ID` surfaces in app-facing code/docs

### Commit 2

- Subject:
  - `add control_scope to rpc-control request identity`
- Goal:
  - make `control_scope` a first-class caller-visible field before storage changes
- Required changes:
  - add `control_scope` to `JsonRpcCall`
  - add `control_scope` to `RpcControlRequest::PrepareSources`
  - add `control_scope` to any response surface that echoes control-plane identity
  - add typed client helpers/defaulting that stamp the effective scope into the serialized request before hashing
  - default production scope remains `shared`
- Primary files:
  - [crates/collectors/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/collectors/rpc-control/src/lib.rs)
  - [crates/collectors/rpc-control/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/collectors/rpc-control/README.md)
- Verification:
  - unit tests for `fact_key_for_request` show same request with different `control_scope` hashes differently
  - `nix run .#check`

### Commit 3

- Subject:
  - `require network_id on canonical evm read and write ops`
- Goal:
  - hard-cut built-in managed callers to always carry explicit blockchain identity
- Required changes:
  - add `network_id` to `evm_read` op config
  - add `network_id` to deploy/configure/validate op/state configs
  - plumb `network_id` through reusable read/write states and helper functions
  - update validation/reporting code so managed selection happens against declared network instead of probing first and checking chain id later
- Primary files:
  - [crates/ops/evm-read-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/evm-read-op/src/lib.rs)
  - [crates/ops/evm-write-op/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/ops/evm-write-op/src/lib.rs)
  - [crates/evm-runtime/src/states/read.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/states/read.rs)
  - [crates/evm-runtime/src/states/write.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/states/write.rs)
  - [crates/evm-runtime/src/states/price.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/states/price.rs)
- Verification:
  - op/unit tests updated for new required config shape
  - one regression test proves canonical managed calls without `network_id` now fail
  - `nix run .#check`

### Commit 4

- Subject:
  - `fix rpc-control fact keys for scope-aware replay`
- Goal:
  - eliminate replay aliasing across `control_scope` / `network_id`
- Required changes:
  - update custom/manual fact-key paths in `crates/evm-runtime/src/rpc.rs`
  - ensure receipt polling keys include effective `network_id` and `control_scope`
  - ensure any remaining send/helper keys include effective `network_id` and `control_scope`
  - remove any ambient defaulting that happens after fact-key derivation
- Primary files:
  - [crates/evm-runtime/src/rpc.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/evm-runtime/src/rpc.rs)
  - [crates/collectors/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/collectors/rpc-control/src/lib.rs)
- Verification:
  - regression test: same `state_id + method + params + network_id` with different `control_scope` does not alias
  - regression test: receipt polling fact keys do not alias across scope/network
  - `nix run .#check`

### Commit 5

- Subject:
  - `remove rpc-control default network fallback and legacy bootstrap env`
- Goal:
  - remove `__default__` and all networkless/bootstrap-global routing behavior from the transport layer
- Required changes:
  - reject canonical managed requests without `network_id`
  - reject bootstrap sources that omit `network_id`
  - remove `MFM_EVM_RPC_URL` / `MFM_EVM_RPC_AUTHORIZATION`
  - remove transport logic that derives effective scope/network from global fallback behavior
  - keep `shared` scope defaulting only as stamped request identity, not ambient routing state
- Primary files:
  - [crates/transports/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/transports/rpc-control/src/lib.rs)
  - [crates/app/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/app/src/lib.rs)
  - [tests/integration/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/src/lib.rs)
- Verification:
  - regression test: networkless bootstrap source config is rejected
  - regression test: canonical managed call without `network_id` fails with structured error
  - `nix run .#check`

### Commit 6

- Subject:
  - `key rpc-control state by control_scope`
- Goal:
  - cut over both persistence backends to scope-aware durable identity
- Required changes:
  - add `control_scope` to `RpcSourceRef` and `SourcePoolRef`
  - change stream ids to `rpc_source:<control_scope>:<network_id>:<source_id>`
  - change stream ids to `source_pool:<control_scope>:<network_id>:<pool_kind>`
  - update Postgres projection PKs and load/upsert queries to include `control_scope`
  - update stream-backed rebuild paths to the same identity shape
- Primary files:
  - [crates/storages/control-plane-postgres/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/storages/control-plane-postgres/src/lib.rs)
  - [crates/storages/control-plane-postgres/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/storages/control-plane-postgres/README.md)
  - [crates/transports/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/transports/rpc-control/src/lib.rs)
- Verification:
  - unit tests for stream-id parsing/building
  - regression coverage for both Postgres control-plane store and `StreamStore` mode
  - `nix run .#check`

### Commit 7

- Subject:
  - `declare rpc-control catalogs per scope and network`
- Goal:
  - prevent same-scope different-catalog corruption
- Required changes:
  - add append-only `pool_catalog_declared`
  - persist both fingerprint and normalized non-secret catalog snapshot
  - extend source-pool projections with catalog declaration fields
  - reject mismatched catalog reuse for the same `(control_scope, network_id, pool_kind)` before membership/ranking/source writes
  - ensure managed execution only uses sources inside the declared catalog
- Primary files:
  - [crates/storages/control-plane-postgres/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/storages/control-plane-postgres/src/lib.rs)
  - [crates/transports/rpc-control/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/transports/rpc-control/src/lib.rs)
- Verification:
  - regression test: same `(control_scope, network_id)` with different catalogs fails
  - regression test: same catalog declaration is idempotent
  - regression test: same `control_scope` across different `network_id` values does not collide
  - `nix run .#check`

### Commit 8

- Subject:
  - `add rpc-control reset task for scope cutover`
- Goal:
  - provide the explicit one-shot rollout mechanism required by this refactor
- Required changes:
  - add a repo-owned reset entrypoint outside app startup
  - recommended shape: a Nixfied task wrapping repo-owned SQL that:
    - deletes `rpc_source:%` and `source_pool:%` rows from `mfm_stream_records`
    - deletes matching heads from `mfm_streams`
    - drops and recreates `mfm_rpc_source_state`
    - drops and recreates `mfm_source_pool_state`
  - document that this reset must run exactly once after the final refactor commit lands and before any refactor-era process starts
- Implemented task:
  - `nix run .#rpc-control-scope-reset -- --print-sql`
  - `DATABASE_URL=postgresql://... nix run .#rpc-control-scope-reset -- --yes`
- Primary files:
  - [nixfied/project/module.nix](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/nixfied/project/module.nix)
  - [nixfied/project/default.nix](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/nixfied/project/default.nix)
  - repo-owned SQL/script file to be added alongside the task wiring
- Verification:
  - dry-run or documented operator command path is reviewed
  - reset instructions are explicit about execution timing

### Commit 9

- Subject:
  - `update rpc-control integration helpers parity tests and docs`
- Goal:
  - align test fixtures, parity flows, and docs with the hard cutover
- Required changes:
  - update integration helper to require explicit `network_id` and support explicit `control_scope`
  - give shared parity tests unique scopes where isolation is required
  - convert parity/bootstrap users away from `MFM_EVM_RPC_URL`
  - update CLI/REST/architecture/routing docs to describe the final surface only
  - update inventories and READMEs for removed raw-send op and new scope-aware behavior
- Primary files:
  - [tests/integration/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/src/lib.rs)
  - [tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs)
  - [tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs)
  - [tests/integration/tests/parity_aave_v3_reth_scenario.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_aave_v3_reth_scenario.rs)
  - [docs/evm-rpc-routing.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/evm-rpc-routing.md)
  - [bin/cli/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/cli/README.md)
  - [bin/rest-api/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/rest-api/README.md)
  - [docs/ops-and-states.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/ops-and-states.md)
  - [docs/helios.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/helios.md)
- Verification:
  - intentional shared-scope integration test passes
  - isolated parity tests use unique scopes
  - targeted parity/bootstrap tests no longer depend on removed env/CLI surfaces

### Final validation commit or pre-push gate

- Subject:
  - no extra code changes unless the earlier commits uncover fallout
- Required full verification before pushing to `dev`:
  - `nix run .#check`
  - `nix run .#test`
  - `nix run .#ci -- --mode basic --summary`
  - `nix run .#ci -- --mode full --summary`
- Rollout order:
  - merge/push the final code commits
  - run the explicit control-plane reset once
  - only then allow parity/CI/services to start against the shared database

## Useful commands

- Inspect the reverted commit:
  - `git show 7e93e614cf155f0beee93229e98aa998504825e7`
- Reproduce CI entrypoint:
  - `nix run .#ci -- --mode full --summary`
- Relevant parity task wiring:
  - `nixfied/project/module.nix`

## Resolved questions

- Should scope be attached to each request, to the transport/session, or both?
  - each request
  - transport/session defaults are allowed only as convenience that stamps the effective scope into the request before hashing
- Should the default production scope be implicit (`shared`) or require explicit config in multi-session services?
  - default to `shared` when callers do not ask for isolation
  - services that need independent control-plane behavior should still set explicit scopes
- Should same-scope different-catalog usage be a hard error or a separate scope derivation rule?
  - hard error at `(control_scope, network_id, pool_kind)`
- Should caller-controlled source pinning remain on the public `rpc.control` surface?
  - no
  - remove `route.source_id` / `rpc_source_id` from the canonical managed contract
- Should raw-send compatibility surfaces be preserved?
  - no
  - all surviving send behavior must route through canonical `rpc.control`
- Do pre-cutover runs need to replay/resume after this lands?
  - no
  - this is a dev-branch hard cutover with no backward compatibility for old `rpc.control` fact identity
- Are networkless canonical managed calls still supported?
  - no
  - canonical managed callers must provide `network_id`
- Are networkless bootstrap sources still supported?
  - no
  - every configured bootstrap source must declare explicit `network_id`
- Is legacy single-source bootstrap fallback retained?
  - no
  - remove `MFM_EVM_RPC_URL` / `MFM_EVM_RPC_AUTHORIZATION`
- Where should catalog identity live durably?
  - as an append-only `pool_catalog_declared` record on `source_pool:*`
  - persist both the fingerprint and the normalized non-secret catalog snapshot
  - rebuild projection state from stream history rather than relying on mutable side tables only

## Recommended next step

- Implement `control_scope` as a first-class production concept with request-visible identity, reset old control-plane state, and then use that same mechanism to isolate parity tests.
