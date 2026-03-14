# RPC Redesign: Central Ordered and Reliable Chain Access

Date: 2026-03-14
Status: proposal

References:
- `PROBLEM_HELIOS.md`
- `docs/redesign.md`
- `docs/architecture.md`
- `docs/evm-rpc-routing.md`

## 1. Why This Exists

`PROBLEM_HELIOS.md` exposes the immediate operational issue: public mainnet RPCs are too unstable to
be the foundation for Helios-backed portfolio snapshots.

That problem is broader than Helios.

Right now we have point solutions for:
- per-call EVM routing
- per-call read hedging
- per-run replay capture
- thin raw-transaction submission

What we do not have is a central, durable authority for:
- wallet ordering
- nonce allocation
- read and write rate limiting
- cross-run source health memory
- write delivery guarantees
- network-specific source quality
- consensus and execution routing as one system

If MFM is going to manage multiple wallets on multiple networks concurrently, this cannot stay
distributed across CLI flags, per-request `source_id` choices, or attempt-local transport state.

The definite solution is a central RPC control plane.

## 2. Problem Statement

The current design is insufficient for production-grade ordered chain access.

Current gaps:
- `crates/collectors/evm-jsonrpc-http` keeps health score and cooldown in transport-local memory.
  The transport is created per state attempt, so health memory resets frequently and does not become
  a cross-run source of truth.
- `keystore_tx_sign` requires the caller to provide `nonce`. That is unsafe once multiple runs can
  transact from the same wallet on the same network.
- `keystore_tx_send_raw` requires an explicit `source_id` and submits to one source only.
- EVM write helpers can fetch `eth_getTransactionCount("pending")`, but there is no durable nonce
  journal or wallet lane serialization across runs.
- `mfm-machine` is sequential per run, but nothing today serializes external side effects across
  different runs targeting the same `(network, wallet)` resource.
- Helios readiness depends on both consensus and execution sources, but we do not model those source
  families centrally or score them together.

The result is predictable:
- nonce races
- duplicate or out-of-order sends
- read storms against weak RPCs
- repeated use of degraded endpoints after process restart
- no principled way to decide when to prefer Helios, when to bypass it, and when to fan out writes

## 3. Design Goals

The new design must provide all of the following:

1. Ordered writes per wallet and network.
2. Durable nonce allocation across concurrent runs and processes.
3. Central rate limiting for reads and writes.
4. Persistent source quality tracking per network and capability.
5. Policy-based routing that selects the best source automatically.
6. Multi-endpoint transaction broadcast when configured.
7. Replay-safe and resume-safe execution within `mfm-machine`.
8. Separation of control-plane decisions from raw transport execution.
9. No persistence of secrets, private keys, auth headers, or full secret-bearing URLs.

## 4. Non-Goals

This redesign does not require:
- pushing business logic into `bin/cli` or `bin/rest-api`
- making raw low-level RPC calls disappear entirely
- making Helios the only execution path
- introducing a second unrelated append-only storage engine just for RPC coordination

Low-level direct send/sign flows may remain as escape hatches, but they must stop being the
canonical production path.

## 5. Core Decision

Split RPC into two layers:

```text
shared states / ops
        |
        v
RPC control plane         <- ordered decisions, nonce journal, rate budgets, source scoring
        |
        v
RPC data plane            <- actual JSON-RPC / consensus HTTP execution
```

The current `evm-jsonrpc-http` transport is a useful data-plane executor.
It should stop being the source of truth for source health and write policy.

The control plane becomes authoritative for:
- source registry
- source capabilities
- source scoring and cooldown
- per-source and per-network rate budgets
- wallet write lanes
- nonce reservations
- transaction broadcast plans
- receipt/replacement reconciliation

Storage-wise, the cleanest direction is not "machine event store plus another coordination store."

It is:
- one generalized append-only stream store
- one Postgres implementation
- machine run streams as one stream family
- wallet lanes / tx intents / source health as other stream families
- projections derived from those streams

So the redesign should generalize the current run-scoped event store contract, not work around it
with a second system.

## 6. Architectural Model

### 6.1 Generalized Stream Store

The current store API is run-specific.

Conceptually today it is:
- `head_seq(run_id)`
- `append(run_id, expected_seq, events)`
- `read_range(run_id, ...)`

That is really a specialized form of a more general primitive:
- `head_seq(stream_id)`
- `append(stream_id, expected_seq, records)`
- `read_range(stream_id, ...)`

The redesign should extract that lower-level primitive.

Examples of stream families:
- `run:<run_id>`
- `wallet_lane:<network_id>:<sender_address>`
- `tx_intent:<intent_id>`
- `rpc_source:<network_id>:<source_id>`
- `source_pool:<network_id>:<pool_kind>`

The important point is:
- ordering and optimistic concurrency are defined by canonical `stream_id`
- secondary semantic indexes are still useful, but they are not the ordering primitive

### 6.2 Control Plane

The control plane is a durable coordination subsystem shared by all runs.

It owns four resource families:

1. `SourcePool`
- Keyed by network and source family.
- Examples:
  - `eth-mainnet/execution`
  - `eth-mainnet/consensus`
  - `eth-mainnet/light-client-local`
  - `arb-mainnet/execution`

2. `Source`
- A concrete endpoint or local service.
- Examples:
  - `eth-mainnet.helios_local`
  - `eth-mainnet.publicnode`
  - `eth-mainnet.lodestar_consensus`

3. `WalletLane`
- Keyed by `(network_id, sender_address)`.
- This is the ordering boundary for nonces and transaction submission.

4. `TxIntent`
- A durable write intent identified by a stable idempotency key.
- This is the unit that moves through reserve -> sign -> broadcast -> mine -> finalize.

The control plane should use the generalized stream store, not a bespoke storage mechanism.

### 6.3 Data Plane

The data plane executes concrete network calls:
- EVM JSON-RPC over HTTP
- consensus HTTP for Helios bootstrap/update sources
- local Helios RPC
- optional private relay or specialized write transports

The data plane does not decide ordering or wallet ownership.
It executes plans issued by the control plane.

## 7. Durable State Model

The right model is:
- one generalized append-only stream store as the durable substrate
- append-only ledgers per semantic stream family
- materialized heads / projections derived from those ledgers

That mirrors the repo's design style:
- immutable event history
- explicit current state derived from history
- no mutation of past records

This is not a second storage system.
It is a generalization of the current one.

### 7.1 Why stream families matter

The current machine run stream is keyed by `run_id`.

That is correct for:
- `RunStarted`
- `StateEntered`
- `StateCompleted`
- `StateFailed`

It is not sufficient for wallet coordination because nonce ordering is not a run resource.
It is a `(network_id, sender_address)` resource.

So the fix is not "stop using append-only streams."
The fix is "stop assuming every important stream is a run stream."

### 7.2 Stream-level atomicity requirements

If we unify on one generalized stream store, we should support atomic multi-stream append.

Why:
- reserving a nonce touches the wallet lane stream
- binding that nonce touches the tx intent stream
- projections for lane head and intent state must advance consistently

So the generalized store should eventually support an atomic operation like:
- append to `wallet_lane:*`
- append to `tx_intent:*`
- commit derived projection updates

all in one database transaction.

Recommended durable records:

### 7.3 Source Records

`rpc_source_events` append-only examples:
- `source_registered`
- `probe_succeeded`
- `probe_failed`
- `read_succeeded`
- `read_failed`
- `rate_limited`
- `stale_head_detected`
- `divergence_detected`
- `tx_accepted`
- `tx_rejected`
- `receipt_observed`

`rpc_source_state` materialized fields:
- `network_id`
- `source_id`
- `capabilities`
- `score`
- `disabled_until`
- `last_head_number`
- `last_head_at`
- `success_ewma`
- `latency_ewma_ms`
- `rate_limit_ewma`
- `write_accept_ewma`
- `receipt_visibility_ewma`

### 7.4 Wallet Lane Records

`wallet_lane_events` append-only examples:
- `lane_initialized`
- `nonce_reserved`
- `tx_broadcast_planned`
- `tx_accepted`
- `tx_replaced`
- `tx_mined`
- `tx_finalized`
- `lane_gap_detected`
- `lane_reconciled`

`wallet_lane_state` materialized fields:
- `network_id`
- `sender_address`
- `next_allocatable_nonce`
- `highest_confirmed_nonce`
- `highest_pending_nonce`
- `max_inflight`
- `last_reconcile_at`

### 7.5 Transaction Records

`tx_intents` durable fields:
- `intent_id`
- `wallet_lane_id`
- `sequence`
- `nonce`
- `replacement_generation`
- `tx_hash`
- `state`
- `broadcast_policy`
- `first_accepted_at`
- `mined_at`
- `finalized_at`

`tx_broadcast_attempts` append-only fields:
- `intent_id`
- `source_id`
- `attempt_no`
- `outcome`
- `observed_tx_hash`
- `error_class`
- `observed_at`

## 8. Routing and Quality Policy

### 8.1 Sources Need Capability Tags

A source is not just "an RPC URL".

Each source must declare capabilities such as:
- `read_light`
- `read_heavy`
- `write_send_raw`
- `receipt_lookup`
- `trace`
- `proof`
- `consensus_bootstrap`
- `consensus_updates`
- `light_client_local`

This matters because:
- Helios local is a good read source when healthy, but it is not the canonical write source.
- Some endpoints are acceptable for reads but not for writes.
- Some write endpoints may be private relays and should not be used for generic reads.

### 8.2 Source Scoring Must Be Persistent

Source quality should be ranked from durable observations, not per-attempt memory.

Score inputs should include:
- success rate
- latency
- head freshness lag
- head divergence from peer set
- rate-limit frequency
- write acceptance rate
- receipt visibility speed
- consensus bootstrap/update success for Helios upstreams

Scores must decay over time so a source can recover, but the memory must survive process restart.

### 8.3 Routing Must Be Policy-Based

Ops and states should ask for a routing policy, not a concrete endpoint.

Examples:
- `snapshot_read`
- `latency_read`
- `consistency_read`
- `write_primary`
- `write_fanout`
- `consensus_bootstrap`

Direct `source_id` routing should remain only for:
- debugging
- explicit operator override
- controlled tests

It should not be the normal application contract.

## 9. Read Path

### 9.1 Central Read Admission

Every read request should first pass through control-plane admission:
- classify method and capability requirement
- acquire source or pool budget
- select source order
- return a routing plan to the data plane

This replaces purely transport-local best-effort selection.

### 9.2 Rate Limiting

Rate limiting should exist at three levels:
- per source
- per network pool
- per method class

At minimum, distinct budgets are needed for:
- `read_light`
- `read_heavy`
- `eth_getLogs`
- `receipt_poll`
- `write_send_raw`
- `consensus_bootstrap`

Rate-limit signals must update both:
- immediate admission control
- longer-term source quality score

### 9.3 Read Consistency Profiles

Not every read needs the same policy.

We should support at least two profiles:

1. `best_head`
- Prefer the freshest healthy source.
- Allow hedging for light methods.
- Good for operational health checks and UI-like queries.

2. `anchored_snapshot`
- Pin a network read session to a chosen block anchor.
- Require all snapshot reads to use that block tag or a compatible source.
- Good for deterministic portfolio or protocol snapshots.

This is the correct place to make Helios useful without making it mandatory.

## 10. Write Path

### 10.1 Canonical Write Contract

The canonical write API must accept a transaction intent, not a final nonce-picked raw transaction.

The managed production path should standardize on:
- local or delegated signing under MFM control
- `eth_sendRawTransaction` for network submission

It should not standardize on `eth_sendTransaction`, because that pushes nonce and signing behavior
back into individual nodes and makes multi-endpoint broadcast semantics much weaker.

Inputs should include:
- `network_id`
- wallet reference
- destination/data/value
- gas or fee policy
- broadcast policy
- optional ordering group metadata

Inputs should not require:
- caller-supplied nonce
- caller-chosen `source_id`

### 10.2 Wallet Lane Serialization

The ordering boundary is `(network_id, sender_address)`.

All writes for the same lane must be serialized through one durable lane head.

The control-plane algorithm is:

1. Resolve wallet to canonical sender address.
2. Derive `WalletLaneId = (network_id, sender_address)`.
3. Acquire the durable lane lock for that wallet, preferably with `SELECT ... FOR UPDATE` on the
   lane row or a stable Postgres advisory lock keyed by `(network_id, sender_address)`.
4. Atomically reserve the next nonce for `intent_id`.
5. If `intent_id` already exists, return the same reservation.
6. Sign using the reserved nonce.
7. Broadcast according to policy.
8. Persist outcomes.
9. Reconcile until terminal state.

The important property is that concurrent runs can call step 3 safely and get ordered nonces.

### 10.3 Safety-First Inflight Policy

My view is that the default should be:

- `max_inflight_per_lane = 1`

That is the safest baseline.

It keeps semantics simple:
- one nonce in flight per wallet per network
- no queue of speculative future nonces
- straightforward replacement policy

Higher throughput can be introduced later with a configured lane pipeline depth, but that should be
an explicit policy choice, not the default.

### 10.4 Replacement Semantics

Replacements must reuse the same nonce.

The model should track:
- `intent_id`
- `nonce`
- `replacement_generation`
- current fee policy
- current tx hash

Replacing a transaction must not consume a new lane sequence or a new nonce.

### 10.5 Broadcast Policy

Write broadcast should support:
- `single`
- `primary_plus_fanout`
- `quorum`

The recommended default is:
- `primary_plus_fanout`
- success threshold: at least one healthy write-capable source accepts the tx
- fanout count: small top-N, not "spray to everything"

The point is delivery confidence, not pointless duplication.

### 10.6 Multi-Node Broadcast Outcome Rules

When broadcasting the same raw transaction to multiple nodes:
- a returned tx hash matching the signed payload is success
- `already known` after one success is neutral, not a failure
- `nonce too low` after one success may be neutral if chain state confirms propagation or mining
- transport timeout on one source should not cancel success observed on another source

Every source result still needs to be persisted because it updates quality scoring.

## 11. Helios Must Be Modeled as a Composite Source

Helios is not a single source in practice.

For mainnet it depends on:
- local Helios process health
- upstream consensus endpoint quality
- upstream execution endpoint quality

So the design must track three things separately:

1. `helios_local`
- readiness
- head lag
- consistency faults

2. consensus source pool
- bootstrap availability
- finalized header freshness
- light-client update availability

3. execution source pool
- head freshness
- consistency
- reliability under Helios sync pressure

Policy for using Helios should be:
- prefer it for snapshot reads when it is fresh and healthy
- bypass it automatically when it drifts past an allowed lag threshold
- never let a bad consensus bootstrap source poison the score of unrelated execution sources

This is the direct lesson from `PROBLEM_HELIOS.md`.

## 12. How This Fits `mfm-machine`

### 12.1 Keep State Logic Clean

State handlers still must use `IoProvider`.

The control plane should be exposed through a typed adapter over `IoProvider`, for example a new
namespace group such as:
- `rpc.control`

That keeps the existing architecture intact:
- ops remain planners
- shared states remain executable units
- binaries remain thin

### 12.2 New Shared-State Surface

If we implement this, the reusable executable pieces should live in a shared-state crate, not in
the CLI.

Likely reusable states:
- `OpenReadSessionState`
- `ReserveWalletNonceState`
- `SignTransactionIntentState`
- `BroadcastTransactionState`
- `TrackTransactionReceiptState`
- `ReconcileWalletLaneState`
- `ProbeRpcSourcesState`

### 12.3 Per-Run Event Stream vs Cross-Run Coordination

This is the key architectural point:

The current run-scoped event stream is not enough for wallet ordering across runs.

Why:
- `mfm-machine` event streams are keyed by run
- event-store concurrency only protects append order inside one run
- nonce ordering is a cross-run resource problem

The correct redesign is not "add a totally separate append-only storage system."

The correct redesign is:
- generalize the event store into a stream store keyed by semantic `stream_id`
- keep run history as the `run:*` stream family
- add `wallet_lane:*`, `tx_intent:*`, and `rpc_source:*` stream families on the same substrate

So machine and RPC control share one append-only substrate, while keeping separate typed semantics.

### 12.4 Resume and Replay

Replay correctness requires stable idempotent decisions.

For writes:
- nonce reservation must be idempotent by `intent_id`
- broadcast planning must be idempotent by `intent_id` and `replacement_generation`
- a resumed run must re-obtain the same reservation, not allocate a new nonce

For reads:
- selected routing plan or observed response should be recorded as replayable facts where required

For both:
- no auth headers, secret URLs, or keys may leak into facts, events, or artifacts

## 13. Recommended Implementation Shape

### 13.1 Generalize The Current Event Store

Instead of adding a second storage implementation, refactor the current event store into a more
general stream store.

Suggested direction:
- extract a lower-level generic stream-store trait from the current run-scoped `EventStore`
- evolve the Postgres backend so one implementation can serve all stream families

Preferred end state:
- `mfm-machine` exposes one generic `StreamStore` contract
- run history becomes the `run:*` stream family
- machine runtime code uses `StreamId::run(run_id)` directly
- we do not keep a permanent dual public surface if it only adds indirection

If we use a temporary `EventStore` facade during migration, it should be a short-lived compatibility
step, not the final architecture.

Suggested ownership:
- refactor `crates/storages/event-store-postgres`
- or rename it to something like `crates/storages/stream-store-postgres` if we want the name to
  match the generalized contract

It should manage:
- run streams
- source ledgers and materialized state
- wallet lane ledgers and materialized heads
- transaction journals
- rate-budget accounting

For wallet ordering, this backend must provide one transaction that can:
- lock the wallet lane
- read or reconcile the current lane head
- reserve a nonce for `intent_id`
- commit the updated lane head and intent record atomically

Ideally it also grows an atomic multi-stream append primitive rather than forcing that logic into
ad hoc SQL outside the store contract.

### 13.2 Concrete API Sketch

The goal is to generalize the current run-scoped store, not replace append-only semantics.

One reasonable first cut looks like this:

```rust
pub struct StreamId(pub String);

impl StreamId {
    pub fn run(run_id: RunId) -> Self {
        Self(format!("run:{}", run_id.0))
    }

    pub fn wallet_lane(network_id: &str, sender_address: &str) -> Self {
        Self(format!("wallet_lane:{network_id}:{sender_address}"))
    }

    pub fn tx_intent(intent_id: &str) -> Self {
        Self(format!("tx_intent:{intent_id}"))
    }

    pub fn rpc_source(network_id: &str, source_id: &str) -> Self {
        Self(format!("rpc_source:{network_id}:{source_id}"))
    }
}

pub struct StreamRecord {
    pub stream_id: StreamId,
    pub seq: u64,
    pub ts_millis: Option<u64>,
    pub kind: String,
    pub payload: serde_json::Value,
}

pub struct NewStreamRecord {
    pub kind: String,
    pub payload: serde_json::Value,
    pub ts_millis: Option<u64>,
}

pub struct StreamAppend {
    pub stream_id: StreamId,
    pub expected_seq: u64,
    pub records: Vec<NewStreamRecord>,
}

pub struct AppendBatchResult {
    pub new_heads: std::collections::BTreeMap<StreamId, u64>,
}

#[async_trait]
pub trait StreamStore: Send + Sync {
    async fn head_seq(&self, stream_id: &StreamId) -> Result<u64, StorageError>;

    async fn append(
        &self,
        append: StreamAppend,
    ) -> Result<u64, StorageError>;

    async fn append_batch(
        &self,
        appends: Vec<StreamAppend>,
    ) -> Result<AppendBatchResult, StorageError>;

    async fn read_range(
        &self,
        stream_id: &StreamId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<StreamRecord>, StorageError>;
}
```

The important semantic rules should be:
- `stream_id` is the canonical ordering key
- `append(...)` is compare-and-swap on one stream
- `append_batch(...)` is atomic across all participating streams
- projections may be updated in the same database transaction as append/append_batch

For migration only, a run-oriented facade could look like:

```rust
pub struct RunEventStore<S> {
    inner: S,
}

#[async_trait]
impl<S: StreamStore> EventStore for RunEventStore<S> {
    async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError> {
        self.inner.head_seq(&StreamId::run(run_id)).await
    }

    async fn append(
        &self,
        run_id: RunId,
        expected_seq: u64,
        events: Vec<EventEnvelope>,
    ) -> Result<u64, StorageError> {
        let records = events
            .into_iter()
            .map(|env| NewStreamRecord {
                kind: "machine_event".to_string(),
                payload: serde_json::to_value(env.event)
                    .expect("machine event must serialize"),
                ts_millis: env.ts_millis,
            })
            .collect();

        self.inner
            .append(StreamAppend {
                stream_id: StreamId::run(run_id),
                expected_seq,
                records,
            })
            .await
    }

    async fn read_range(
        &self,
        run_id: RunId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        let records = self
            .inner
            .read_range(&StreamId::run(run_id), from_seq, to_seq)
            .await?;

        records
            .into_iter()
            .map(|record| {
                let event = serde_json::from_value(record.payload)
                    .map_err(|_| StorageError::Other(/* ... */))?;
                Ok(EventEnvelope {
                    run_id,
                    seq: record.seq,
                    ts_millis: record.ts_millis,
                    event,
                })
            })
            .collect()
    }
}
```

This gives us:
- one storage substrate
- one concurrency model
- one place for append atomicity
- a migration path if we want one

Preferred final state after the breaking change:
- machine runtime reads and writes `run:*` streams directly
- `EventEnvelope` becomes a machine-domain payload, not the storage substrate type
- there is only one store trait to understand

### 13.3 Concrete Refactor Plan

If we actually do this now, I would refactor in this order.

#### Step 1: Introduce generic stream types in `crates/machine`

Primary file:
- `crates/machine/src/lib.rs`

Changes:
- add `StreamId`
- add generic `StreamRecord`
- add `NewStreamRecord`
- add `StreamAppend`
- add `AppendBatchResult`
- replace or deprecate `stores::EventStore` with `stores::StreamStore`

Target outcome:
- storage API no longer mentions `RunId`
- append semantics are keyed by `StreamId`

#### Step 2: Move run-specific logic out of the storage contract

Primary files:
- `crates/machine/src/runtime/writer.rs`
- `crates/machine/src/runtime.rs`
- `crates/machine/src/runtime/child_runs.rs`
- `crates/machine/src/attempt_envelope.rs`

Changes:
- convert runtime writer to use `StreamId::run(run_id)`
- encode machine events as stream payloads under `kind = "machine_event"` or equivalent typed codec
- keep attempt-envelope analysis on decoded machine events, not on raw storage records

Target outcome:
- machine runtime remains run-scoped semantically
- storage substrate is no longer run-specific

#### Step 3: Generalize in-memory storage

Primary file:
- `crates/storages/event-store-mem/src/lib.rs`

Changes:
- replace `HashMap<RunId, Vec<EventEnvelope>>` with `HashMap<StreamId, Vec<StreamRecord>>`
- implement `StreamStore`
- add atomic `append_batch` behavior in-memory

Target outcome:
- tests and local development continue working on the new substrate first

#### Step 4: Generalize Postgres storage

Primary file:
- `crates/storages/event-store-postgres/src/lib.rs`

Schema direction:
- replace `mfm_runs` and `mfm_events` as the only abstraction with generic stream tables such as:
  - `mfm_streams(stream_id, head_seq, domain, kind, ...)`
  - `mfm_stream_records(stream_id, seq, ts_millis, record_kind, payload)`
- add projection tables for:
  - wallet lanes
  - tx intents
  - source state
  - rate budgets

Behavior:
- `append` is single-stream CAS
- `append_batch` is atomic multi-stream CAS in one SQL transaction
- projection updates happen in the same SQL transaction where required

Target outcome:
- one durable backend for machine runs and RPC coordination

#### Step 5: Migrate machine tests

Primary files:
- `crates/machine/src/tests/runtime_tests.rs`
- `crates/machine/src/tests/exec_transport_tests.rs`
- `crates/machine/src/live_io_router.rs` test helpers

Changes:
- update no-op stores and test doubles to implement `StreamStore`
- keep run-history assertions by reading `run:*` streams and decoding machine events

Target outcome:
- machine semantics remain unchanged even though storage is generalized

#### Step 6: Add RPC-control stream families

Primary new ownership:
- stream families on the same backend, not a new store crate
- IO adapter in `crates/collectors/rpc-control`

New stream families:
- `wallet_lane:*`
- `tx_intent:*`
- `rpc_source:*`
- `source_pool:*`

Target outcome:
- control-plane state lands on the same substrate the machine already uses

#### Step 7: Add first projection-backed control flows

First flows to implement:
- source quality persistence
- source cooldown persistence
- `reserve_nonce`
- `reconcile_lane`

Target outcome:
- highest-risk correctness problem, nonce allocation, moves first

#### Step 8: Migrate shared write states

Primary files:
- `crates/evm-runtime/src/rpc.rs`
- `crates/evm-runtime/src/states/write.rs`
- `crates/states/aave-v3/src/states.rs`
- `crates/states/keystore-submit/src/tx.rs`

Changes:
- stop using direct pending nonce allocation as the canonical write path
- route managed writes through wallet lanes and tx intents

Target outcome:
- one canonical ordered write path

#### Step 9: Remove transitional compatibility surface

If we introduced a temporary `EventStore` facade:
- delete it once the machine runtime is fully on `StreamStore`
- keep only one public storage contract

Target outcome:
- reduced surface area, not expanded surface area

### 13.4 Control-Plane IO Adapter

Add a typed adapter over `IoProvider`.

Suggested ownership:
- `crates/collectors/rpc-control`

This adapter should expose operations like:
- `open_read_session`
- `plan_read_route`
- `reserve_nonce`
- `plan_broadcast`
- `record_broadcast_outcome`
- `record_receipt`
- `reconcile_lane`

### 13.5 Shared States

Suggested ownership:
- `crates/states/rpc`

This keeps executable logic reusable and aligned with `docs/architecture.md`.

### 13.6 EVM Data Plane

Keep `crates/collectors/evm-jsonrpc-http`, but demote it to data-plane execution.

It should still do transport-safe tasks:
- HTTP execution
- sanitized diagnostics
- capability probes

But persistent routing memory and wallet policy should move out of it.

## 14. Migration Plan

### Phase 1: Persist Source Quality

Ship first:
- generalized stream-store contract
- migrate current run event store onto `run:*` stream family
- persistent source registry
- persistent score and cooldown
- central rate budgets
- policy-based route selection

Keep current write flow temporarily, but stop relying on attempt-local score memory.

### Phase 2: Wallet Lanes and Nonce Reservation

Add:
- durable wallet lanes
- canonical `reserve_nonce`
- transaction intent ids

At this point, caller-supplied nonce should stop being the normal path.

### Phase 3: Broadcast Fanout and Receipt Journal

Add:
- `primary_plus_fanout`
- per-source broadcast outcome recording
- receipt tracking and replacement lineage

### Phase 4: Read Sessions and Helios Composite Supervision

Add:
- `anchored_snapshot` read sessions
- consensus pool scoring
- Helios local freshness gating
- automatic Helios bypass when stale

## 15. Opinionated Conclusions

These are my concrete recommendations:

1. Do not keep nonce selection in CLI input or low-level ops for the canonical path.
2. Do not keep RPC health only inside `evm-jsonrpc-http` transport-local memory.
3. Do not solve this by adding a second unrelated append-only store if we can generalize the current one cleanly.
4. Keep one physical stream store, many typed stream families, and projections derived from them.
5. Do not make Helios the only source. Make it a preferred local source inside a broader pool.
6. Do not blindly broadcast to every endpoint. Use a small healthy fanout set and persist results.
7. Start with strict per-wallet lane depth `1`. Add higher throughput only after reconciliation is
   proven correct.
8. Treat consensus RPC quality and execution RPC quality as separate first-class signals.
9. Build the coordination layer as a durable shared subsystem that `mfm-machine` states call
   through typed IO, not as ad hoc global mutable process memory.

That gives us one place to reason about:
- ordering
- reliability
- rate budgets
- source quality
- Helios readiness
- replay-safe on-chain execution

Without that central layer, we will keep solving the same RPC failures one endpoint, one command,
and one retry loop at a time.
