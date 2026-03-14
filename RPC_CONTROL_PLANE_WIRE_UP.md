# RPC Control Plane Wire-Up

Date: 2026-03-14
Status: planning handoff
Scope: implementation-ready plan for making the control plane the canonical routing and write-coordination path

## Purpose

- Capture the current RPC/control-plane gaps verified in the repository.
- Lock the boundary between:
  - generic control-plane coordination
  - EVM-specific planning and execution
  - raw transport execution
- Define the public namespace, signer model, and migration shape before code lands.
- Provide one implementation plan that updates code and docs together.

## Final Design Decisions For This Milestone

- `rpc.control` is the only new canonical state-facing runtime namespace.
- A typed client layer must sit on top of `rpc.control`.
  - States and reusable runtime helpers should use the typed client, not hand-roll `IoCall`s.
- `namespace = "evm"` remains in the repo, but only as an internal EVM data-plane executor surface.
  - It must stop being the canonical state-facing routing authority.
- If `evm_read` is kept, it stays public only as a control-plane-backed read tool.
  - It must not remain source-pinned or bypass the control plane.
  - It must not be renamed to `*_direct`.
- Offline/direct signer tools may still exist explicitly.
  - `keystore tx-sign` may remain as an offline/direct signing surface.
  - It is not the canonical managed write path.
- Raw-send bypass tools are not canonical.
  - `keystore tx-send-raw` does not remain a normal managed surface.
  - If a raw-send bypass is retained, it must be renamed to an explicit direct name such as
    `keystore tx-send-raw-direct` and documented as bypassing control-plane guarantees.
- The control plane must own write ordering, source choice, admission, durability, and replay-safe planning.
- The control plane accepts signer references and signer capabilities.
  - It must not persist secrets, private keys, auth headers, or full private URLs.
- Canonical managed submit must always execute with a stable internal intent key.
  - Callers may provide an explicit idempotency key.
  - If they do not, the typed client must derive one from the canonical immutable write input.
- Canonical control-plane behavior requires persistent storage.
  - `MemStreamStore` remains test-only and may only back explicitly offline/direct tools.
- Anchored read-session opening is correctness-side-effect-free in v1.
  - If future read admission requires correctness-critical durable mutation, add a dedicated durable
    read-session or read-admission family first.
- Documentation updates are part of the milestone, not follow-up work.

## Important Framing

- The filename says `RPC`, but this must not become "EVM JSON-RPC control plane only".
- EVM is the first network family to migrate.
- The generic control-plane core must remain extensible to:
  - Bitcoin
  - Zcash
  - Starknet
  - Near
  - future networks with different write-ordering models
- Practical consequence:
  - EVM can use wallet-lane plus nonce reservation now.
  - Future account-based networks may need different sequence models.
  - UTXO networks will need spend reservation instead of nonce lanes.

## Executive Summary

- The append-only stream substrate is ready.
- The canonical control plane is not implemented.
- Today, `evm-jsonrpc-http` still owns transport-local routing memory:
  - source score
  - cooldown
  - probe state
  - route ordering
  - write dispatch choice
- Canonical writes still depend on:
  - caller nonce
  - caller source choice
  - live `eth_getTransactionCount("pending")`
- Canonical reads still thread `rpc_source_id` through state/runtime models.
- Helios is still modeled as a singleton source instead of a supervised composite.
- The correct milestone is not "more transport logic".
- The correct milestone is:
  - land a real control-plane backend on the shared stream-store substrate
  - make `rpc.control` the canonical ingress
  - demote `evm-jsonrpc-http` to an internal EVM executor

## Source Documents To Merge Or Replace

- `REDESIGN_RPC.md`
- `rpc_control_plane_integraiton.md`
- `docs/redesign.md`
- `docs/architecture.md`
- `docs/evm-rpc-routing.md`

## Non-Negotiable Invariants

- The control plane is the source of truth for route selection.
- Data planes execute plans; they do not own durable routing decisions.
- Append-only stream families remain the durable substrate.
- Correctness-critical multi-stream updates must commit atomically.
- No secrets, auth headers, or private URLs may be persisted in:
  - events
  - facts
  - artifacts
  - snapshots
  - CLI/API outputs
  - error details
- Replay and resume must reproduce prior control-plane decisions or fact-captured plans.
- Canonical public APIs must not require:
  - caller-supplied nonce
  - caller-selected source id
  - request-level transport routing hints
- Offline/direct tools may exist, but they must be explicitly named and non-canonical.

## Current-State Audit

### 1. Transport Still Owns Routing Memory

- Default app wiring still registers `evm-jsonrpc-http` directly as the live `evm` transport:
  - `crates/app/src/lib.rs`
- Live transports are still instantiated per attempt:
  - `crates/machine/src/runtime/attempt.rs`
  - `crates/machine/src/live_io_router.rs`
- `crates/collectors/evm-jsonrpc-http/src/lib.rs` still initializes mutable in-memory:
  - score
  - cooldown
  - probe state
  - source ordering
  - write dispatch policy

### 2. Canonical Write Flows Still Bypass Durable Coordination

- `mfm keystore tx-sign` still requires caller nonce:
  - `bin/cli/src/commands/keystore/tx_sign.rs`
- `mfm keystore tx-send-raw` still requires caller or env source id:
  - `bin/cli/src/commands/keystore/tx_send_raw.rs`
- Both keystore tx CLI flows still use ephemeral in-memory stream storage:
  - `bin/cli/src/support/run_stores.rs`
- `crates/ops/keystore-tx-op/src/lib.rs` still encodes the old contract.
- Shared EVM runtime still exposes canonical helpers around:
  - `eth_getTransactionCount("pending")`
  - `eth_sendTransaction`
  - single-source `eth_sendRawTransaction`
  - files:
    - `crates/evm-runtime/src/rpc.rs`
    - `crates/evm-runtime/src/states/write.rs`
    - `crates/states/aave-v3/src/states.rs`

### 3. Canonical Read Flows Still Accept Transport-Level Routing

- `rpc_source_id` remains a normal input in portfolio models and runtime states:
  - `crates/states/portfolio/src/model.rs`
  - `crates/states/portfolio/src/states.rs`
- Symbol runtime still threads route ids through network config:
  - `crates/states/symbol/src/states.rs`
- Aave portfolio reads do the same:
  - `crates/states/aave-v3/src/portfolio/states.rs`
- The EVM collector still exposes explicit route hints:
  - `crates/collectors/evm/src/lib.rs`

### 4. Helios Is Still Modeled Too Narrowly

- Snapshot runtime still exports only one EVM source:
  - `nixfied/project/module.nix`
- Mainnet snapshot CI still pins `rpc_source_id = "helios_local"`:
  - `nixfied/project/module.nix`
- That prevents independent modeling of:
  - Helios readiness
  - execution pool health
  - consensus pool health
  - automatic stale bypass

### 5. Docs And Public Surfaces Still Describe The Old Contract

- `docs/evm-rpc-routing.md` still documents source-id-based EVM routing as the operator contract.
- `bin/cli/README.md` still documents:
  - `--nonce`
  - `--source-id`
  - ephemeral keystore tx stores
- `bin/rest-api/README.md` still advertises direct keystore tx ops as current run-start surfaces.

## Architecture Shape

```text
shared states / ops
        |
        v
typed rpc.control client
        |
        v
rpc.control namespace
        |
        v
generic control-plane core + network-specific adapters
        |
        v
network-specific data-plane executors
        |
        v
raw transports / protocols
```

## Layer Responsibilities

- Shared states and ops:
  - ask for reads or writes in network or business terms
  - do not choose transport source ids
  - do not allocate nonces directly
  - do not call internal `evm` executor paths directly in canonical flows
- Typed `rpc.control` client:
  - exposes stable request and response types over `IoProvider`
  - shields states from transport payload details
  - owns fact-capture contract for control-plane decisions
- Generic control-plane core:
  - source and pool coordination
  - read admission
  - idempotency
  - source quality and cooldown
  - shared storage transaction helpers
- EVM control-plane adapter:
  - anchored read-session planning
  - wallet-lane reservation
  - tx-intent lifecycle
  - broadcast planning
  - receipt reconciliation
  - signer selection for EVM wallets
- EVM data plane:
  - executes concrete JSON-RPC calls against explicit sources
  - performs probes
  - returns sanitized results
  - does not own durable ranking memory

## Namespace And Client Contract

### Canonical Namespace

- `rpc.control` is the canonical runtime namespace for managed network reads and writes.
- The state-facing client should be a typed wrapper over `IoProvider` that emits `namespace = "rpc.control"` requests.
- The typed client should live in a crate/module shaped like a normal Layer 1 domain adapter.
- States should depend on the typed client, not on raw `IoCall` assembly.

### Internal Namespace

- `namespace = "evm"` remains an internal EVM executor surface.
- It is still useful for:
  - EVM HTTP execution
  - probes
  - executor-focused tests
  - explicit internal control-plane implementation details
- It is not the canonical routing surface anymore.

### Architecture Contract Update Required

- `docs/architecture.md` currently says runtime EVM network calls route through `evm-jsonrpc-http`.
- This milestone changes that contract.
- The architecture doc must be updated in the same change so the new boundary is normative:
  - `rpc.control` is canonical ingress
  - `evm` is executor-only

## Generalization Boundary

### Must Be Generic Now

- source registry
- source capabilities
- source quality scoring
- source cooldown
- rate budgets
- pool ranking
- read admission
- read-session planning interfaces
- idempotency and replay rules
- per-source outcome journal
- common storage transaction helpers

### Can Be Network-Specific In The First Milestone

- write resource model
- fee policy details
- transaction serialization
- receipt shape
- finality semantics
- signer implementation details
- concrete transport execution

## Signer Model

### Core Rule

- Wallet identity and signer identity are different resources.
- The control plane manages wallet ordering and intent lifecycle.
- The signer is a capability used during managed submit.

### Required Abstractions

- `wallet_ref`
  - stable wallet identity used by ops and reports
- `signer_ref`
  - stable signer identity used by the control plane
- `signer_kind`
  - local keystore
  - future remote or external signer classes
- `signer_capabilities`
  - local-only vs remote
  - interactive vs non-interactive
  - supported networks
  - supported tx types

### Runtime Resolution Rule

- Canonical managed submit resolves `signer_ref` through a runtime signer registry or resolver.
- That resolver may use local runtime config such as:
  - process env
  - app wiring
  - local config files
- Canonical managed requests must not require a raw keystore filesystem path.
- In v1:
  - `local_keystore` is resolved from runtime-only configuration
  - keystore locator details stay out of manifests, events, facts, snapshots, outputs, and error
    details
- Explicit offline/direct tools may still accept concrete local filesystem paths because they are not
  the canonical managed path.

### Persistence Rules

- Persist signer references and capabilities only.
- Do not persist:
  - private keys
  - passwords
  - mnemonics
  - raw secret-bearing signer configs

### V1 Scope

- Support `local_keystore` as the first managed signer.
- Keep offline/direct `keystore tx-sign` as a non-canonical signer tool.
- Do not design v1 around interactive browser signers.
  - MetaMask-style signers may be added later behind the same abstraction.
  - They should not drive the core contract for this milestone.

## Public Contract Target

### Canonical Read Contract

- Inputs should include:
  - `network_id`
  - read profile or read purpose
  - optional anchor/session inputs where needed
- Inputs should not include:
  - `rpc_source_id`
  - `route_source_id`
  - transport-specific source hints

### Canonical Write Contract

- Inputs should include:
  - `network_id`
  - `wallet_ref`
  - `signer_ref` or wallet-to-signer resolution inputs
  - destination
  - data/value
  - fee policy
  - broadcast policy
  - optional caller-supplied idempotency key
- Inputs should not include:
  - caller-supplied nonce
  - caller-selected source id
- The typed client must always produce a stable internal intent key.
  - If the caller supplies an idempotency key, use it after normalization.
  - Otherwise derive the key from canonical immutable intent input before any reservation happens.

### Low-Level Read Tool Contract

- If `evm_read` remains a public tool, it must route through `rpc.control`.
- It may remain useful for:
  - low-level diagnostics
  - operator troubleshooting
  - control-plane-backed raw method reads
- It must not remain a source-pinned bypass.

### Explicit Offline Or Direct Tools

- Offline/direct tools may remain only when they are explicitly named and documented as non-canonical.
- First example:
  - `keystore tx-sign`
- Existing `keystore tx-sign` is grandfathered if docs and help text explicitly describe it as an
  offline/direct escape hatch.
- If a raw-send bypass remains, it must use an explicitly direct name.
- These tools:
  - do not define the canonical write contract
  - do not choose canonical routing policy
  - do not replace managed submit

## Recommended New Ownership

### Generic Control-Plane Core

- New crate, suggested names:
  - `crates/control-plane`
  - `crates/network-control`
- Responsibilities:
  - generic source and pool projections
  - idempotent intent interfaces
  - generic read admission/session interfaces
  - common storage transaction helpers

### EVM Control-Plane Adapter

- New crate, suggested names:
  - `crates/control-plane-evm`
  - `crates/network-control-evm`
- Responsibilities:
  - EVM read-session planning
  - EVM wallet-lane reservation
  - EVM broadcast planning
  - EVM receipt reconciliation
  - EVM signer integration

### State-Facing Typed Client And Live Transport

- The Layer 1 typed `rpc.control` client must live in a domain-adapter crate, not in shared state
  code.
- Recommended shape:
  - `crates/collectors/rpc-control` or `crates/collectors/network-control`
- Responsibilities:
  - typed request and response models
  - fact-key and `record_value` policy
  - replay-safe wrapper behavior over `IoProvider`
- The live `namespace = "rpc.control"` transport is an internal runtime transport, not an external
  collector.
- Recommended shape:
  - `crates/transports/rpc-control`
- Responsibilities:
  - register the `rpc.control` namespace
  - bridge typed calls into the durable control-plane backend
  - delegate concrete EVM execution to internal executor surfaces

### Shared State Placement Rule

- New executable `State::handle` implementations that consume the typed client remain in shared-state
  crates such as:
  - `crates/states/common`
  - `crates/evm-runtime`
  - other domain shared-state crates as needed
- Do not move reusable execution logic into the transport or control-plane crates.
- Do not put the typed client inside a shared-state crate.

### Reusable State Layer

- Reuse existing shared-state crates where practical.
- Only add a new shared-state crate if it meaningfully reduces duplication.
- Do not force generic coordination logic into an `evm-*` crate.

### Existing Data Plane To Keep

- Keep `crates/collectors/evm-jsonrpc-http` as the EVM executor.
- Remove from it:
  - durable source score ownership
  - cooldown authority
  - route ordering authority
  - canonical write policy

## Required Stream Families And Projections

### Generic

- `rpc_source:<network_id>:<source_id>`
- `source_pool:<network_id>:<pool_kind>`

### EVM First Cut

- `wallet_lane:<network_id>:<sender_address>`
- `tx_intent:<intent_id>`

### Projection Tables

- `rpc_source_state`
  - source quality
  - cooldown
  - last observed head
  - latency and reliability rollups
- `source_pool_state`
  - ranked sources per capability or pool
  - budget counters
- `wallet_lane_state`
  - next allocatable nonce
  - highest pending nonce
  - highest confirmed nonce
  - inflight depth
- `tx_intent_state`
  - reserved nonce
  - current state
  - current tx hash
  - replacement generation
  - per-source broadcast outcomes summary

### Correctness Rule

- Stream append and projection updates must commit in one DB transaction.
- This must not be built on `crates/storages/indexer` as-is.
  - That crate is explicitly derived-only and not suitable for correctness-critical state.
- This milestone uses the shared Postgres stream-store substrate plus control-plane projection tables
  in the same database and transaction boundary.
- This is not a second durable coordination system.
  - it is an extension or wrapper around the shared stream-store substrate
  - it may live in a sibling Postgres-backed control-plane storage crate
  - it may extend the existing Postgres stream-store implementation
- The storage boundary must be explicit before implementation starts:
  - either expose transactional helpers around the shared Postgres stream store
  - or provide a control-plane Postgres storage layer that shares the same DB transaction boundary
- Do not bury control-plane semantics inside `crates/machine`.

## Read-Session Contract

### V1 Rule

- Anchored read sessions are fact-captured and reused within the run.
- V1 does not require a dedicated durable read-session stream family.
- `open_anchored_read_session` is correctness-side-effect-free in v1.
  - it may read durable source and pool state
  - it must not commit correctness-critical control-plane mutations before the session fact is
    durably captured
- If fact capture fails, the session is treated as not opened and may be recomputed on retry.
- Durable read admission and durable read-budget reservation are deferred until there is a dedicated
  durable session or admission family.

### Required Session Output

- pinned block number
- primary read source
- ordered fallback sources
- Helios status
- any policy metadata needed for replay-safe downstream execution

### Replay Rule

- Session planning must be captured as facts in a way replay can reproduce exactly.
- Downstream canonical read states must consume pinned session outputs, not calculate route choice again.
- The typed client must use a stable session fact key derived from:
  - state identity
  - network identity
  - session purpose or profile
  - any explicit anchor inputs
- Replay mode must fail with a stable missing-fact error if a canonical downstream read asks for a
  session that was not durably captured.

## Write-Intent Contract

### Required Identity

- Every managed write must resolve to one stable `intent_key`.
- `intent_key` is the uniqueness boundary for:
  - reservation
  - resume
  - replay-safe submit behavior
  - `tx_intent:<intent_key>`
- Public API may accept an explicit idempotency key.
- If the caller omits it, the typed client must derive `intent_key` from canonical immutable intent
  input before any control-plane mutation happens.

### Required Lifecycle

- reserve
- sign
- broadcast
- observe
- reconcile
- terminal success or terminal failure

### Signed Payload Rule

- After the `sign` step succeeds, v1 stores the raw signed transaction as an immutable artifact and
  records only its artifact reference in intent state.
- Do not inline the raw signed payload into:
  - events
  - error details
  - CLI/API outputs
- `tx_intent_state` must include at least:
  - unsigned intent identity
  - reserved nonce
  - `signed_payload_id` when signing succeeded
  - current tx hash when broadcast succeeded
- Resume after crash-before-broadcast must reuse `signed_payload_id` when present.
  - It must not allocate a new nonce.
  - It must not re-sign unless the intent is still in a pre-sign state.

### Failure Semantics Must Be Explicit

- Define behavior for:
  - crash after reservation but before signing
  - crash after signing but before broadcast
  - partial broadcast fanout
  - resume of an already-broadcast intent
  - abandoned or orphaned reservations
- Do not leave reclaim or reuse behavior to implementation intuition.
- Required v1 decisions:
  - reservation without `signed_payload_id` may be resumed in-place or reclaimed only by explicit
    documented rule
  - reservation with `signed_payload_id` but no broadcast must resume by reusing the same signed
    payload
  - already-broadcast intents must poll or reconcile existing tx hashes before any replacement logic
  - automatic fee-bump replacement is out of scope for this milestone

## Helios As Composite Supervision

- Model separately:
  - `helios_local`
  - execution pool
  - consensus pool
- Prefer Helios for snapshot reads only when:
  - readiness is true
  - freshness lag is within threshold
  - consensus side is healthy
- Otherwise bypass automatically to the best healthy execution source.

## Migration Plan

### Phase 0: Lock The Contract

- Update this handoff first.
- Update `docs/architecture.md` for the new namespace boundary.
- Record the final contract:
  - `rpc.control` is canonical
  - `evm` is internal executor-only
  - `evm_read` remains public only if it uses the control plane
  - signer model is wallet plus signer, not wallet equals signer

### Phase 1: Land The Control-Plane Backend

- Implement correctness-critical projection tables and transactional update path.
- Add typed stream-family helpers for:
  - `rpc_source`
  - `source_pool`
  - `wallet_lane`
  - `tx_intent`
- Implement durable source quality first.
- Implement source cooldown first.
- Implement pool ranking and budgets first.
- Add a temporary bridge so current `namespace = "evm"` execution can consult and update durable
  source quality and cooldown while `rpc.control` is not yet canonical ingress.
  - This bridge is transitional only.
  - Delete it once canonical ingress is rebound.

### Phase 2: Land `rpc.control` Namespace And Typed Client

- Add the `rpc.control` transport and typed client layer.
- Keep `evm` executor wiring internal for now.
- Ensure the typed client owns fact-capture and replay contract for:
  - anchored read sessions
  - write-intent planning
  - intent observation and resume
- The typed client contract must explicitly define:
  - fact keys
  - when `IoProvider::call(...)` is used
  - when `IoProvider::record_value(...)` is used
  - replay behavior for every canonical `rpc.control` operation

### Phase 3: Rebind Production Routing

- Default app wiring must stop exposing `evm-jsonrpc-http` as the canonical state-facing authority.
- Register `rpc.control` as the canonical ingress in `crates/app/src/lib.rs`.
- `evm-jsonrpc-http` becomes an internal executor used by the EVM control plane.
- Canonical shared states must enter through the typed `rpc.control` client.

### Phase 4: Migrate Canonical Writes

- Implement managed EVM submit flow:
  - reserve nonce
  - resolve signer
  - sign
  - broadcast
  - persist outcomes
  - reconcile to terminal state
- Replace canonical write surfaces with one managed intent API.
- Keep offline/direct `keystore tx-sign` as non-canonical.
- Remove canonical use of:
  - `eth_getTransactionCount("pending")`
  - `eth_sendTransaction`
  - caller source-id broadcast selection

### Phase 5: Migrate Canonical Reads

- Introduce anchored read-session planning.
- `PinPortfolioNetworksState` should open one anchored session per network.
- Downstream states should consume pinned session data, not choose route ids.
- Remove `rpc_source_id` from canonical portfolio, symbol, and Aave runtime models.
- If `evm_read` remains, migrate it to the control-plane-backed contract in the same phase.

### Phase 6: Rework Helios As Composite Supervision

- Export separate runtime sources for:
  - Helios local
  - execution fallbacks
  - consensus fallbacks
- Add Helios freshness gate and automatic bypass policy.

### Phase 7: Remove Transitional Compatibility Surface

- Stop exposing `rpc_source_id` and `source_id` in canonical request models.
- Stop documenting direct routed EVM transport as the normal surface.
- Keep only explicitly named offline/direct tools that remain intentional.
- Remove the temporary Phase 1 bridge from direct `evm` execution into durable source-quality state.

## Legacy Surfaces To Delete Or Demote

- request-level `rpc_source_id` in canonical portfolio, symbol, and Aave flows
- `route_source_id` in canonical state configs
- `source_id` in canonical managed write flows
- caller-supplied `nonce` in canonical managed write flows
- direct `eth_sendTransaction` in canonical write paths
- direct `eth_getTransactionCount("pending")` for canonical nonce allocation
- default `MFM_EVM_RPC_SOURCE_ID` contract for canonical routing
- ephemeral `MemStreamStore` for canonical tx commands

## Enforcement Plan

- Add CI checks that fail on new canonical uses of:
  - `rpc_source_id`
  - `route_source_id`
  - `with_route_source_id`
  - `eth_getTransactionCount` with `"pending"`
  - `eth_sendTransaction`
- Do not use `EvmIoClient::new` itself as the enforcement boundary.
  - `EvmIoClient` is a typed adapter over `IoProvider`, not a live transport bypass by itself.
- Allowlist only:
  - internal EVM executor implementation
  - explicit offline/direct modules
  - tests that intentionally exercise direct transport behavior
- Add a small audit script or CI grep task rather than relying on convention.

## Documentation Plan

### Must Update In The Same Change

- `docs/redesign.md`
- `docs/architecture.md`
- `docs/ops-and-states.md`
- `bin/cli/README.md`
- `bin/rest-api/README.md`
- relevant crate READMEs and rustdoc examples that still publish the old canonical contract,
  especially any public example that mentions:
  - `namespace = "evm"` as canonical ingress
  - `route.source_id`
  - `rpc_source_id`
  - direct raw-send as the normal managed path

### Rewrite Or Remove

- `docs/evm-rpc-routing.md`
  - either rewrite as an internal EVM executor runbook
  - or remove it if that surface is no longer worth documenting separately

### Merge Then Delete

- `REDESIGN_RPC.md`
- `rpc_control_plane_integraiton.md`

## Acceptance Criteria

- Canonical shared states and canonical ops enter managed network behavior through `rpc.control`.
- `namespace = "evm"` is no longer the canonical state-facing routing authority.
- Route choice survives restart because quality and cooldown are durable.
- Two concurrent writes from the same EVM wallet reserve ordered nonces without duplication.
- Canonical write APIs no longer require caller nonce or caller source id.
- Canonical managed submit always uses a stable internal intent key.
- Canonical read APIs no longer accept `rpc_source_id`.
- Portfolio snapshot reads run through anchored read sessions.
- Helios is preferred only when fresh and healthy, and automatically bypassed when stale.
- Default CLI, REST, and app flows use persistent store-backed control-plane behavior.
- Canonical managed submit fails fast when persistent control-plane storage is unavailable.
- If `evm_read` remains public, it uses the control plane.
- Offline/direct signing tools are explicitly documented as non-canonical.
- If a raw-send bypass remains, it is explicitly named as direct and is not documented as the normal
  managed path.

## Test Plan

### Unit

- source-quality projection updates
- cooldown persistence
- lane reservation idempotency
- `max_inflight_per_lane = 1`
- multi-stream CAS conflict handling
- Helios freshness and bypass policy
- signer resolution rules
- signer-capability validation

### Integration

- two concurrent writes from the same wallet serialize correctly
- managed submit broadcasts to multiple sources and records outcomes
- restart preserves score and cooldown
- anchored snapshot session pins block and reuses it across reads
- stale Helios is bypassed automatically
- canonical CLI and REST flows require persistent store-backed control-plane wiring
- retained `evm_read` path uses the control plane and not direct source hints

### Replay And Resume

- resuming a managed write reuses the same reservation
- resuming an in-flight intent polls existing hash rather than allocating a new nonce
- replayed snapshot reads use recorded session facts rather than recalculating routing
- orphaned pre-broadcast reservations follow the explicit reclaim or reuse rule

## Risks

- The biggest risk is still accidentally implementing an EVM-only control plane and treating it as generic later.
- The second biggest risk is leaving direct canonical bypasses available in production state code and assuming convention will stop them.
- The third biggest risk is treating correctness-critical projections as derived-only rather than part of the transactional contract.
- A fourth risk is designing around future interactive signers too early and compromising the v1 replay and execution contract.

## Recommended First Implementation Slice

- Land generic source and pool durability first.
- Land `rpc.control` namespace and typed client second.
- Land EVM wallet-lane plus tx-intent next.
- Rebind app routing so canonical production state code enters through `rpc.control`.
- Migrate canonical writes before canonical reads.
- Migrate Helios freshness and bypass after anchored read sessions exist.
- Add CI enforcement before broad follow-on refactors so regressions fail fast.
- Merge and remove stale docs as part of the same series.

## Handoff Summary

- The stream-store foundation is good enough.
- The missing work is the control plane itself plus the enforcement boundary around it.
- `rpc.control` is the canonical ingress.
- `evm` remains useful, but only as an internal executor surface.
- EVM is the first implementation, not the shape of the whole design.
- If `evm_read` remains, it must route through the control plane.
- Signing must be modeled as wallet plus signer, not a one-off keystore shortcut.
- Managed writes always resolve to one stable intent key and one durable reservation lifecycle.
- V1 read-session opening is fact-backed and correctness-side-effect-free until a dedicated durable
  read-admission family exists.
- The milestone is complete only when canonical reads and writes cannot bypass the control plane.

## Validation Notes

- Static review only.
- No tests were run for this handoff.
