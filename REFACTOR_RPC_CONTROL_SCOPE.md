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

- Long-term model:
  - `rpc.control` gets a first-class `control_scope` concept as a production surface.
  - `control_scope` is request-visible and participates in durable fact identity.
- Rollout/migration:
  - use an explicit control-plane reset
  - do not migrate or rewrite old `__default__`-keyed control-plane streams
- Catalog safety:
  - same-scope different-catalog usage is a hard error
  - fingerprint validation is per `(control_scope, network_id)`, not just per scope

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
- `control_scope` must be explicit in every effective `rpc.control` request shape:
  - `RpcControlRequest::EvmCall`
  - `RpcControlRequest::PrepareSources`
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
  - compute/store a catalog fingerprint per `(control_scope, network_id)`
  - reject writes or initialization when a process tries to use the same `(control_scope, network_id)` with a different effective catalog
- Fingerprint input should be the effective non-secret routing catalog for that `(control_scope, network_id)`:
  - fingerprint schema version
  - `control_scope`
  - `network_id`
  - effective candidate sources for that network
  - per-source non-secret routing fields:
    - `id`
    - `kind`
    - `require_get_proof_probe`
  - effective preferred order projected onto those candidate source ids
- Do not persist raw endpoint credentials in the fingerprint:
  - do not include `authorization`
  - do not include raw `rpc_url`
- If future safety requires distinguishing two sources with the same public routing metadata but different backends:
  - add an explicit non-secret source identity/revision field
  - do not persist raw URLs as a shortcut

## Explicit Reset Strategy

- This refactor assumes an explicit control-plane reset before rollout.
- Existing persisted `rpc_source:*` and `source_pool:*` state is disposable for this change.
- Do not attempt in-place migration from old identities such as:
  - `rpc_source:<network_id>:<source_id>`
  - `source_pool:<network_id>:<pool_kind>`
  - `__default__`
- Reset scope:
  - clear control-plane stream families `rpc_source:*` and `source_pool:*`
  - clear matching projection rows/tables derived from those families
- After reset, the new scope-aware identities become the only supported durable shape.

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
- Add coverage for both control-plane persistence backends:
  - dedicated Postgres control-plane store
  - `StreamStore` mode over the shared stream substrate
- Add a cross-network shared-scope test:
  - same `control_scope`
  - different `network_id`
  - different effective catalogs
  - must not trip catalog-fingerprint mismatch
- Add a same `(control_scope, network_id)` different-catalog rejection test.

## What not to do

- Do not overload `network_id` with test IDs or session IDs.
  - `network_id` already has domain meaning in request models and runtime states.
- Do not hide `control_scope` only in transport-local configuration.
  - effective scope must participate in serialized request identity and replay facts
- Do not rely on Cargo target isolation for control-plane races.
- Do not assume fresh process/object instances imply fresh control-plane state.
- Do not persist raw RPC URLs or authorization material in scope/catalog fingerprints.

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
- The exact shape should preserve crate boundaries and avoid smuggling ambient globals into state logic.

### Storage layer

- Update control-plane identity types so stream keys include scope.
- Current stream families already separate `rpc_source:*` and `source_pool:*`.
- Refactor identities so those become effectively scope-aware.
- Update both persistence paths together:
  - dedicated Postgres control-plane store
  - `StreamStore`-backed control-plane store
- Add durable catalog-fingerprint validation per `(control_scope, network_id)`.

### Integration helper

- Update the integration helper in:
  - [tests/integration/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/src/lib.rs)
- Add a way to pass explicit scope.
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
- [tests/integration/src/lib.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/src/lib.rs)
- [docs/evm-rpc-routing.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/docs/evm-rpc-routing.md)
- [bin/rest-api/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/rest-api/README.md)
- [bin/cli/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/bin/cli/README.md)
- [crates/collectors/rpc-control/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/collectors/rpc-control/README.md)
- [crates/transports/rpc-control/README.md](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/crates/transports/rpc-control/README.md)
- [tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_portfolio_tracker_reth_mock_erc20.rs)
- [tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs)
- [tests/integration/tests/parity_aave_v3_reth_scenario.rs](/Users/willyrgf/dev/rust/src/github.com/willyrgf/2/mfm/tests/integration/tests/parity_aave_v3_reth_scenario.rs)

## Suggested implementation order

1. Add first-class request-visible `control_scope` to `rpc.control` request/response/client surfaces.
2. Make `control_scope` part of durable fact identity by normalizing defaults into serialized requests before hashing.
3. Key persisted control-plane identities by `control_scope` plus existing network/source dimensions.
4. Update both control-plane persistence backends to the new identity shape.
5. Add catalog fingerprint validation per `(control_scope, network_id)` using non-secret effective catalog fields only.
6. Plumb explicit scope through integration helpers and any app-facing configuration surfaces.
7. Define and execute the explicit control-plane reset for old `rpc_source:*` and `source_pool:*` state.
8. Give each shared parity test/session a unique scope where isolation is required.
9. Add:
   - one intentional shared-scope integration test
   - one cross-network shared-scope test
   - one replay/fact-key isolation regression test
10. Consider whether same-scope retry/backoff should be improved.

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
  - hard error at `(control_scope, network_id)`

## Recommended next step

- Implement `control_scope` as a first-class production concept with request-visible identity, reset old control-plane state, and then use that same mechanism to isolate parity tests.
