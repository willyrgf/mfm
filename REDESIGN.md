# MFM — Redesign (v4)

> **Purpose**: This document is the design contract for the next MFM architecture.
> Last updated: 2026-02-09
> Version: v4 (consolidated contract; resolves v3 internal inconsistencies)
>
> It prioritizes **reproducibility, auditability, simplicity, and security**.
> If code disagrees with this document, the code is wrong (until the doc is explicitly updated).

---

## 0. Guiding Principles

- **Small, reviewable changes**: evolve this architecture incrementally.
- **Correctness + security first**: especially around keystore/crypto and persistence.
- **Stable boundaries**: libraries remain usable without binaries.
- **Auditability is a feature**: prefer explicit events and deterministic replay over hidden control flow.
- **Reproducibility is an architectural property**: not a “mode” bolted on later.

---

## 1. System Design Requirements

### 1.1 Append-only + transactional (per-append atomicity; attempt envelopes)
- All meaningful actions are recorded as **immutable events** in an **append-only log**.
- A “run” is an ordered event stream. No updates to past events.
- Event-store appends are **transactional** at the storage layer:
  - each `EventStore::append([...])` is atomic and never partially visible to readers.
- A **state attempt** is delimited by a kernel envelope in the event stream (§6.2/§6.4) and MAY span multiple
  appends:
  - `StateEntered … [domain…] … (StateCompleted|StateFailed)`

### 1.2 Reproducibility (Nix philosophy applied system-wide)
- Every run is defined by a **run manifest** (content-addressed).
- Any derived artifact/snapshot is **content-addressed**.
- External/non-deterministic inputs are treated as **facts**:
  - they may be recorded and replayed,
  - and their absence in replay produces a structured error.

### 1.3 Simplicity (KISS)
Prefer a small set of “boring primitives”:
- state machine runtime
- event store
- artifact store
- IO abstraction (live vs replay)
- operation planning (expand ops → state graphs)
- (optional later) typed schema registry (see §17)

### 1.4 Composability & reusability
- Ops are state graphs. State graphs compose.
- Collectors, storage backends, and primitives are reusable libraries.
- Binaries (CLI/API) are thin wrappers that start/resume runs.

### 1.5 Auditability
- Everything must be trackable via events + artifacts.
- A run must be inspectable end-to-end:
  - inputs, configuration, environment, transitions, outputs, errors, retries.

### 1.6 Everything executes inside a state machine
- Every execution is a state machine.
- “Ops” are sequences/graphs of states plus configuration.
- Nested machines are a supported *concept*, but **engine-managed child runs are deferred for Milestone 1**
  (Milestone 1 uses flattened composition only; see §11).

---

## 2. Design Decisions (Resolved)

### 2.1 Canonical serialization for hashing
- **Canonical JSON** is the standard for hashing structured data:
  - manifests, events, snapshots, typed payloads.
- Target semantics: **RFC 8785 (JCS)**-style canonicalization.

**Constraints**
- No NaN/Infinity.
- Hashed structures MUST NOT contain floats (JSON numbers with fractional parts). Use integer-scaled values or
  decimal strings.

### 2.2 Context snapshots
- Snapshots are **full snapshots** (not deltas).
- Snapshots are stored as content-addressed artifacts.

### 2.3 Minimum standard events
- Runtime emits a **small required kernel** of events needed for correct recovery/resume.
- Beyond that, the **domain event set is configurable per op** (event profiles).

### 2.4 Replay mode semantics
- Replay mode does **not** hard-crash the process on attempted IO.
- Replay mode returns **structured IO errors** when deterministic facts are missing or when replayable IO is invoked
  without a `fact_key` (see §8).

### 2.5 Parallel execution
- Sequential execution is the default.
- Parallel execution is deferred for Milestone 1. Any future parallelism MUST remain auditable and replayable.

### 2.6 Cargo package naming
- Cargo package names SHOULD use hyphens (e.g. `mfm-machine`, `mfm-core`).
- Rust import paths use underscores (Cargo mapping), e.g. `use mfm_machine::...`.

### 2.7 Milestone 1 security policy: no secrets persisted
- **Milestone 1 hard rule:** secrets must not appear in persisted surfaces:
  - manifests, events, artifacts (including fact payloads and context snapshots), CLI/API outputs, and error details.
- Encrypted secret-bearing artifacts are explicitly deferred until after Milestone 1.

---

## 3. Core Concepts & Terminology

### 3.1 Operation (Op)
A reusable definition of “what to do”:
- input schema
- run configuration schema
- expansion rules (op → state graph)
- output schema (what it produces/exports)

### 3.2 Run
A concrete execution of an Op:
- has a run ID
- has a manifest artifact (hashed)
- has an append-only event stream
- produces artifacts (snapshots, outputs, recorded facts payloads)

### 3.3 State
A step in a machine:
- reads/writes context (namespaced)
- uses IO provider (live/replay)
- emits domain events
- may store artifacts and reference them via events

### 3.4 Context
The evolving key/value data for a run:
- typed reads/writes
- deterministic serialization
- full snapshot support

### 3.5 Event
An immutable record appended to a run’s event store:
- kernel events: required for engine correctness
- domain events: configurable per op

### 3.6 Artifact
An immutable blob/document stored in an artifact store:
- content-addressed ID (hash)
- referenced by events (events do not embed large blobs)

### 3.7 Fact
A recorded external input:
- e.g., RPC response, HTTP response, time reading, randomness bytes
- stored as a content-addressed payload artifact and referenced from events

### 3.8 What does NOT belong in context
Context is for *derived working state* needed by downstream states, not for:
- IO provider internal runtime state (connection pools, caches, backoff counters).
- Large raw external payloads (store as fact payload artifacts instead; see §7 and §8).
- Secrets (never store).

If something is needed for reproducibility/debuggability:
- **Configuration/provenance** belongs in the **manifest** and referenced config artifacts.
- **External inputs/outputs** belong in the **fact/artifact stores**, referenced by events.
- **Context snapshots** are stored as artifacts and referenced from kernel events.

---

## 4. Workspace Structure

Proposed layout:

```

mfm/
├── Cargo.toml
├── crates/
│   ├── machine/                # state machine runtime + context + tags/labels + planning types
│   ├── machine-derive/         # proc macros
│   ├── core/                   # primitives: keystore, config models, crypto utilities, typed IDs
│   ├── collectors/
│   │   ├── evm/                # EVM collectors (RPC, logs, traces)
│   │   └── coingecko/          # HTTP collectors for price/facts
│   ├── storages/
│   │   ├── event-store/         # event-store implementations (pg, local)
│   │   ├── artifact-store/      # artifact-store implementations (minio, fs)
│   │   └── indexer/            # optional: projections (clickhouse)
│   ├── ops/                    # operation definitions (expand into state graphs)
│   │   ├── aave-tracker/
│   │   ├── portfolio-tracker/
│   │   └── portfolio-management/
│   └── sdk/                    # optional: client SDK
├── bin/
│   ├── cli/
│   └── rest-api/
├── flake.nix
└── flake.lock

```

### 4.1 Naming: removing `mfm_` prefixes
- Directory/module names can drop `mfm_` (e.g., `crates/machine/`, `crates/core/`, `bin/cli/`).
- **Cargo package names should remain namespaced** to avoid collisions (`mfm-machine`, `mfm-core`, `mfm-sdk`, etc.).
  - Cargo translates `mfm-machine` → Rust crate import `mfm_machine`.
- Cargo package naming policy is in §2.6.

---

## 5. Dependency Graph & Boundary Rules

### 5.1 High-level dependency graph
```

{ cli, rest-api } -> { ops, sdk }       # thin wrappers
sdk -> machine                          # orchestration helpers; generic over store traits
ops -> { machine, core, collectors, storages } (+ sdk for Operation/Pipeline traits)
machine -> machine-derive
storages -> { core, machine }   # for IDs/types and event model
collectors -> { core } (+ machine if needed for shared IO abstractions)
core -> (must not depend on collectors/storages/ops/sdk)

````

### 5.2 Boundary rules (non-negotiable)
- **Binaries** are thin wrappers; they should depend on ops and (recommended) sdk only.
- **Ops** orchestrate only:
  - define expansion to state graphs
  - compose collectors + storages
  - no storage implementations in ops
- **SDK** provides orchestration ergonomics (planning helpers, run launcher/resume); it must stay thin.
- **Collectors** fetch/normalize data; must be usable under live or replay IO.
- **Storages** persist and query; no business logic.
- **Core** houses security-critical code; avoid heavy IO deps.
- **Machine** is generic; no chain-specific code.

### 5.3 Machine vs SDK ownership (planning vs orchestration)

Recommended split (avoid “god crates” while keeping correctness centralized):

Put these in `crates/machine/` (runtime-owned; correctness-critical):
- `StateId`, `StateMeta`, dependency edge model
- `StateGraph`, `ExecutionPlan`
- kernel event types (`RunStarted`, `StateEntered`, ...)
- executor + context snapshot mechanics

Put these in `crates/sdk/` (planning convenience + integration; ergonomics):
- `Operation` (or `Op`) trait + versioning conventions
- op registry / discovery helpers
- `Pipeline` builder (`then()`, `build()`)
- run launcher / resume helpers (thin wrapper around machine + stores)

Ops crates (`crates/ops/*`) typically:
- implement the `Operation` trait (from sdk)
- produce `StateGraph`s made of machine states

---

## 6. Append-only Execution Model

### 6.1 Per-run event stream
A run is an ordered sequence of events stored in an append-only event store.

#### Kernel events (engine-level; always emitted)
Minimum required for recovery/resume/audit:

- `RunStarted { op_id, manifest_id, initial_snapshot_id }`
- `StateEntered { state_id, attempt, base_snapshot_id }`
- `StateCompleted { state_id, context_snapshot_id }`
- `StateFailed { state_id, error, failure_snapshot_id? }`
- `RunCompleted { status, final_snapshot_id? }`

**Notes**
- `seq` is a strictly increasing per-run sequence number.
- **Milestone 1 seq convention:** `seq` is 1-indexed; empty run has `head_seq = 0`.
- `attempt` increments per state retry.
- `initial_snapshot_id` is the snapshot of the initial context for the run.
- `base_snapshot_id` is the snapshot the attempt starts from.
- `failure_snapshot_id` is diagnostic-only and MUST NOT be used as a resume checkpoint.

#### Domain events (operation-level; configurable per op)
Examples (not required by engine correctness):
- `FactRecorded { key, payload_id, meta }`
- `ArtifactWritten { artifact_id, kind, meta }`
- `OpBoundary { op_path, phase }`

**Recommended standard `DomainEvent.name` values (stable)**
- `fact_recorded` — payload matches `FactRecorded`
- `artifact_written` — payload matches `ArtifactWritten`
- `op_boundary` — payload matches `OpBoundary`

**Rules**
- Domain events MUST NOT be required for **plan progression** or **resume checkpoint selection**
  (kernel events remain sufficient for recovery/resume correctness).
- Domain events MUST NOT include secrets.
- If the runtime records a fact payload (because a handler invoked replayable IO with a `fact_key`,
  or because `now_millis()` / `random_bytes()` is used), the runtime MUST durably persist the
  `FactKey -> payload_id` binding as a `fact_recorded` domain event **regardless of `EventProfile`**.
  Without this binding, `ReplayIo` cannot be correct (and orphan-attempt facts could be lost).

### 6.2 Transactionality (per-append atomicity) + Attempt Envelopes

The event store is transactional at the **append** boundary:
- each `EventStore::append([...])` is atomic (all events written or none; never partially visible).

A state attempt is represented by a kernel envelope in the event stream:
- `StateEntered { ... }`
- zero or more domain events
- exactly one terminal kernel event: `StateCompleted` or `StateFailed`

A single attempt MAY span multiple appends.

#### Crash/Resume Semantics (Orphan Handling)
- If the stream ends with `StateEntered` and no terminal kernel event, the attempt is **in-flight**.
  On resume, the engine retries from `base_snapshot_id` (discarding staged context).
- Domain events emitted during an in-flight attempt remain in the stream. On resume:
  - facts are run-scoped and remain valid for dedupe and replay
  - other domain events MUST NOT advance plan progression or checkpoint selection

### 6.3 Snapshots & checkpointing
- Context snapshots are full snapshots stored as artifacts.
- A snapshot is referenced by kernel events (`context_snapshot_id`).
- **Milestone 1 policy:** checkpoint after every successful state transition.
  - If a state produces no logical changes, the engine MAY reuse the same snapshot ID (content-addressed dedupe),
    but `context_snapshot_id` is still emitted explicitly.

### 6.4 Transactional state semantics (context + events)
- A state runs against a staged view of context derived from `base_snapshot_id`.
- On success: staged writes commit → full snapshot is produced → `StateCompleted` references it.
- On failure: staged writes are discarded; run remains at `base_snapshot_id`.

Kernel event semantics:
- `StateEntered` MUST be appended at the start of every attempt, before calling the handler.
- Only `StateCompleted { context_snapshot_id }` advances the run’s effective checkpoint.
- `StateFailed` MUST NOT advance the checkpoint.

IO rule:
- IO results that affect determinism MUST be recorded as facts (payload artifacts) and referenced via events.
  Handlers must not depend on IO provider internal state for correctness.

---

## 7. Reproducibility Model

### 7.1 Run manifest
Each run has a manifest artifact whose ID is included in `RunStarted`.

Recommended manifest fields:
- `op_id` + `op_version`
- `git_commit` (or workspace revision)
- `cargo_lock_hash`
- `flake_lock_hash` (or equivalent)
- `rustc_version`, `target_triple`
- `input_params` (canonical JSON; no secrets)
- `config_refs` (content-addressed config files; no secrets)
- `env_allowlist` + captured env values (only those allowed)
- `run_config` (retry policy, replay policy, event profile)
- `io_mode` (live / replay)

Additional guidance:
- IO provider *configuration/provenance* SHOULD be represented in `config_refs` and/or manifest fields,
  but MUST NOT include secrets (API keys, passwords, headers, decrypted buffers).
- Large or structured config documents SHOULD be stored as artifacts and referenced (not embedded).

### 7.2 Canonical JSON hashing rules
- Structured data → canonical JSON bytes → hash → `ArtifactId`
- Binary blobs → raw bytes → hash → `ArtifactId`
- Hash function (Milestone 1): SHA-256.

### 7.3 Facts as first-class citizens (single-assignment)
Any external/non-deterministic input used by deterministic logic is a “fact”.

Facts are stored as fact payload artifacts and referenced via `FactRecorded` domain events.

Within a run:
- A `FactKey` is **single-assignment**: the first durable `FactRecorded { key, payload_id }` binds that key
  permanently to that payload.
- Facts recorded during in-flight (orphaned) attempts remain valid for replay and dedupe.

---

## 8. Replay & IO Determinism

### 8.1 IO provider model
Handlers do not do ambient IO; they use an IO provider.

- `LiveIo`: performs real IO and may record facts.
- `ReplayIo`: serves recorded facts/artifacts when available; otherwise returns a structured error.

Rules:
- In Live mode, replayable IO SHOULD provide `fact_key`. Calls without `fact_key` are non-replayable by default.
- In Replay mode, any deterministic IO MUST provide `fact_key`.
  - If `fact_key` is absent: return an `IoError` with stable code `missing_fact_key` (non-retryable by default).
- Raw external payloads SHOULD be stored as fact payload artifacts (or referenced artifacts) and referenced by
  events; context should store derived/normalized results or references.

### 8.2 Missing facts in replay mode
When replay needs a fact that does not exist:
- If `fact_key` is present but not recorded: return `IoError::MissingFact { key, ... }`.

Retryability:
- `MissingFact` retryability is controlled by run config:
  - `RunConfig.replay_missing_fact_retryable` (default false).

### 8.3 Time and randomness
`now_millis()` and `random_bytes()` are nondeterministic and therefore are treated as facts when used.

Milestone 1 rule:
- `LiveIo` MUST record time/random reads as fact payloads with deterministic, attempt-scoped keys derived from:
  - `(run_id, state_id, attempt, call_ordinal, kind)` (exact encoding is implementation-defined but MUST be stable)
  - `call_ordinal` is per-`(state_id, attempt)` and increments on each call (starts at 0).
- `ReplayIo` MUST serve those recorded values or return `MissingFact`.

### 8.4 “Retryable” errors are explicit
Errors must carry:
- category (e.g., network, rpc, parsing, storage)
- retryability (retryable / non-retryable)
- optional retry hints (backoff class, recommended delay, etc.)

---

## 9. State Machine Runtime Design

### 9.1 Requirements
The runtime must support:
- async handlers
- recoverable/resumable execution
- deterministic replay mode
- flattened composition for Milestone 1 (nested machines deferred)
- side-effect tagging + idempotency strategy
- stable metadata + introspection

### 9.2 Handler shape (aligned with Appendix C)
A state handler is async and receives explicit dependencies:

```rust
#[async_trait]
pub trait State: Send + Sync {
    fn meta(&self) -> StateMeta;

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError>;
}
````

**Key properties**

* `IoProvider` switches live vs replay.
* `EventRecorder` is the only way for handlers to emit domain events.
* `StateId` is assigned by the execution plan; states do not own their IDs.
* `StateOutcome` references produced artifacts/snapshots.

### 9.3 Side effects & idempotency

States declare behavior:

* `PURE`: deterministic, no external IO
* `READ_ONLY_IO`: reads external IO, should record facts
* `APPLY_SIDE_EFFECT`: writes external systems (tx submission, writes)

Side-effecting states must provide an idempotency strategy:

* a dedupe key (e.g., “intent hash”)
* recorded in events
* executor may retry safely after crash

### 9.4 Sequential by default; fan-out/join deferred for Milestone 1

Milestone 1 executor is sequential.
Milestone 1 support rule:
- `RunConfig.execution_mode` MUST be `Sequential`.
- If `FanOutJoin { .. }` is requested, the engine MUST return a structured error
  (e.g., `RunError::InvalidPlan` with a stable `ErrorCode` like `unsupported_execution_mode`).

Future (post Milestone 1):
- Parallelism, if introduced, MUST remain auditable and replayable.
- Preferred expression: explicit fan-out/join with deterministic merge in a `Join` state.
- Engine-managed child runs (nested machines) are a separate feature and require explicit linkage events.

---

## 10. Operations as Expandable State Graphs

### 10.1 Core idea

An op expands into a **state graph** (states + dependency edges).
The engine executes states; “ops” are a planning abstraction.

### 10.2 Expansion

Each op must be able to resolve itself into a concrete graph given config:

* `OpConfig`: domain parameters
* `RunConfig`: execution policy (retry policy, replay policy, event profile)

Expansion produces:

* set of states
* dependency edges (DAG)
* optional op boundaries (informational)
* namespacing strategy

### 10.3 Flattening ops into one machine (K = N + M + …)

If:

* `ops1` expands into N states
* `ops2` expands into M states

Then a pipeline op can expand into a single graph of:

* `K = N + M` states

The engine sees:

* one execution plan
* one run
* one event stream
* one shared context (namespaced)

This enables composition at both levels:

* compose states into ops
* compose ops into larger ops (flattening)

### 10.4 Namespacing & uniqueness

Flattening requires stable unique state identifiers:

* include an `op_path` prefix and a stable `state_local_id`:

  * `portfolio_tracker.main.fetch_balances`
  * `aave_tracker.index.fetch_logs`

If the same op appears multiple times in a pipeline/machine, disambiguate using **named steps**
with stable identifiers (Option A):

* `portfolio_management.prices.fetch_blocks`
* `portfolio_management.balances.fetch_blocks`

**Rule**

* `StateId` must be stable across machines and environments (no random IDs in identifiers).

### 10.5 Context collision rules

Shared context implies collision risk.

Default rule:

* context keys are namespaced by op path automatically.

Cross-op wiring is explicit:

* ops declare exports and imports
* planner validates that all imports are satisfiable

Example:

* `ops1` exports `prices.latest_eth`
* `ops2` imports `prices.latest_eth`

### 10.6 Optional op boundaries

Even though the engine executes K states, boundaries are useful for humans and APIs:

* `OpBoundary { op_path, phase = Started }`
* `OpBoundary { op_path, phase = Completed }`

Boundaries are domain events and may be enabled/disabled by the op's event profile.

### 10.7 Machines as named pipelines (Milestone 1 enforced ID shape)

A machine is a pipeline op with named steps.

Milestone 1 enforced conventions:

* `OpPath = <machine_id>.<step_id>` (2 segments)
* `StateId = <machine_id>.<step_id>.<state_local_id>` (3 segments)
* Each segment MUST match: `^[a-z][a-z0-9_]{0,62}$`
* Dots are reserved separators and forbidden inside segments.

Single-op runs:

* If the CLI runs a single op, the SDK MUST wrap it as a 1-step machine with:

  * `machine_id = <op_id>`
  * `step_id = main`

---

## 11. Nested Machines vs Flattened Composition

Milestone 1:

* flattened composition only (one plan, one run)
* engine-managed child runs deferred

### 11.1 Flattened composition (preferred for single-run pipelines)

* `ops1 -> ops2 -> ops3` becomes one plan, one run.
* Best when:

  * shared context is desired
  * sequencing is straightforward
  * a single audit trail is preferred

### 11.2 Nested machines spawned inside a state (deferred until after Milestone 1)

This is a supported *concept* but not part of the Milestone 1 execution model.

* A handler may spawn a sub-machine.
* Parent does not need to understand child internals.

Audit rule:

* linkage must be recorded:

  * at minimum: `ChildRunSpawned { child_run_id, child_manifest_id }`

Best when:

* parallel fan-out workloads
* isolate failures/retry policies per subtask
* long-running child workflows that can resume independently

---

## 12. Storage Architecture

### 12.1 Two primary storage roles

1. **Event Store** (append-only)
2. **Artifact Store** (content-addressed)

Optional:

* index/projection store for analytics (ClickHouse)

### 12.2 Storage traits (sketch)

```rust
#[async_trait]
pub trait EventStore: Send + Sync {
    async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError>;

    async fn append(
        &self,
        run_id: RunId,
        expected_seq: u64,
        events: Vec<EventEnvelope>,
    ) -> Result<u64, StorageError>;

    async fn read_range(
        &self,
        run_id: RunId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<EventEnvelope>, StorageError>;
}

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>) -> Result<ArtifactId, StorageError>;
    async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError>;
    async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError>;
}
```

### 12.3 Backend strategy

* PostgreSQL: primary event store (strong transactions)
* MinIO/S3: artifact store for large blobs
* ClickHouse: optional projections/indexes derived from events

Local/dev:

- A Nix-first approach SHOULD make production-like dependencies easy to run locally:
  - PostgreSQL + MinIO can be used for dev and integration testing, pinned by flake inputs.
- However, we SHOULD keep fast local implementations for tight iteration and unit tests:
  - filesystem artifact store
  - in-memory or sqlite event store

Recommended test lanes:
1) **Unit / fast lane** (default):
   - in-memory/sqlite event store + filesystem artifacts
2) **Integration / parity lane** (opt-in):
   - PostgreSQL event store + MinIO artifact store

Both lanes MUST satisfy the same correctness contract (append-only events, content-addressed artifacts,
deterministic replay semantics).

### 12.4 Security rules for storage

* **Milestone 1 hard rule:** secrets must not be persisted (manifests, events, artifacts, outputs, error details).
* Events must never contain passwords, mnemonics, private keys, raw decrypted buffers, or secret-bearing headers.
* Post-Milestone 1 (deferred): if secret-bearing artifacts are introduced, they MUST be encrypted and access-controlled.
* Events reference secret IDs, never secret contents.

---

## 13. CLI and REST API Responsibilities

### 13.1 CLI

* starts/resumes runs
* queries run history
* prints stable outputs (text/json)
* does not own business logic

### 13.2 REST API

* same as CLI over HTTP
* should support:

  * start run
  * resume run
  * fetch events
  * fetch artifacts (or signed URLs)
  * progress observation (polling; SSE/websocket later if needed)

---

## 14. Security & Secret Handling

### 14.1 Keystore

The keystore remains security-critical:

* constant-time comparisons
* tamper detection and DoS guards
* zeroization
* strict parsing/validation

Enhancements aligned with append-only architecture:

* atomic writes for keystore persistence (temp + fsync + rename) or transactional persistence
* optional audit events (no secrets): import/delete/lock/unlock metadata events

### 14.2 No secrets persisted (Milestone 1)

Milestone 1 hard rule: secrets must not appear in persisted surfaces (manifests, events, artifacts, outputs, error
details).

* events must never contain:

  * passwords, mnemonics, private keys, raw decrypted buffers
* if something needs to refer to secrets, store only references/handles:

  * keystore entry IDs
  * (deferred) encrypted secret-bearing artifact IDs (post-Milestone 1 only)

---

## 15. Testing Requirements (Design-level)

### 15.1 For every op crate

* **Replay determinism**

  * run in live mode (record facts)
  * rerun in replay mode (no external IO required)
  * assert same output artifact IDs / hashes

* **Recovery/resume**

  * simulate crash after N events
  * resume
  * assert run completes consistently

* **Output stability**

  * JSON schema stability tests for CLI/API responses

### 15.2 For runtime/storage crates

* optimistic concurrency tests (`expected_seq`)
* snapshot correctness tests (full snapshot hashing)
* nested-machine linkage tests (reserved for later milestones)
* idempotency/dedupe tests for side-effect patterns

---

## 16. Migration Plan (Incremental)

1. Introduce `EventStore` + `ArtifactStore` traits and local implementations.
2. Introduce run manifest type + hashing + artifact storage.
3. Add kernel event emission to the runtime (even before async conversion if needed).
4. Convert state handlers to async and introduce `IoProvider`.
5. Implement replay mode with `ReplayIo` and missing-fact error semantics.
6. Introduce op planning:

   * `Operation::expand(...) -> StateGraph`
   * pipeline flattening to one execution plan
7. Move CLI and REST API to “start/resume run” surfaces.
8. Add PostgreSQL + MinIO backends; keep local backends for tests.
9. Add projections (ClickHouse) only after event model stabilizes.

---

## 17. Open Questions (Remaining)

These are intentionally left open until forced by implementation.
Current decisions (subject to change via explicit doc update):

### 17.1 Minimal standard library of reusable state patterns
Initial set:
- `fetch` (READ_ONLY_IO; records facts)
- `validate` (PURE; schema/invariants)
- `execute` (APPLY_SIDE_EFFECT; requires idempotency)
- `store` (writes artifacts + references)
- `join` (deterministic merge; supports fan-out/join)
- `report` (outputs as artifacts)

### 17.2 Op schema versioning and backward compatibility
- Every operation MUST have an explicit `op_version`.
- Stored manifests/events MUST include `op_id` + `op_version`.
- Version changes MUST preserve reproducibility:
  - old runs remain replayable under their recorded manifest/build provenance
  - migrations create new runs/manifests rather than mutating historical data

### 17.3 Event profiles
- No formal shared spec for now.
- The baseline in Appendix B is sufficient; kernel events are always emitted.

### 17.4 Typed schema registry
- A typed schema registry is desirable.
- Schemas SHOULD be stored as artifacts (content-addressed) and referenced by manifests.
- Planner/runtime MAY validate producer/consumer compatibility across ops.

---

## Appendix A — Planning API (Sketch)

The engine executes states, not ops.

* `Operation::expand(op_config, run_config) -> StateGraph`
* `Pipeline::then(op) -> Pipeline`
* `Pipeline::build(...) -> ExecutionPlan`
* `ExecutionPlan` is what the engine executes.

---

## Appendix B — Event Profiles (Concept)

Ops choose an event profile that controls domain event verbosity:

* `minimal`: kernel events + determinism-critical runtime domain events (e.g., `fact_recorded`) + essential artifacts
* `normal`: `minimal` + recommended audit domain events (e.g., `artifact_written`, `op_boundary`)
* `verbose`: `normal` + detailed domain events for diagnostics

Kernel events are always emitted regardless of profile.

## Appendix C — Public API Contract (v0.1)

This section defines the **minimal stable public API surface** for the `machine` and `sdk`
crates.

**Scope**
- Only **types + traits** (no implementations).
- Anything not listed here is **internal/unstable**, even if it is temporarily `pub`.

**Intent**
- `machine` defines **what can be executed** and **how it is represented**
- `sdk` defines **what an operation is**, how ops compose (pipelines), and how runs are launched

---

# C.1 `mfm-machine` — Public API Contract

```rust
//! crates/machine/src/lib.rs — public API contract (types + traits only)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

pub mod ids {
    use super::*;

    /// Stable identifier for an operation (human meaningful).
    /// Invariant: stable across environments; should not be random.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct OpId(pub String);

    /// Milestone 1 enforced: "<machine_id>.<step_id>"
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct OpPath(pub String);

    /// Milestone 1 enforced: "<machine_id>.<step_id>.<state_local_id>"
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct StateId(pub String);

    /// Unique run identifier (can be random).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct RunId(pub uuid::Uuid);

    /// Content-addressed identifier (hash) for an artifact.
    /// Invariant: lowercase hex digest string (algorithm defined by policy; default SHA-256).
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ArtifactId(pub String);

    /// Namespaced key for recorded facts (external inputs).
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct FactKey(pub String);

    /// Namespaced key for context entries.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ContextKey(pub String);

    /// Stable machine-readable error code.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ErrorCode(pub String);
}

pub mod canonical {
    /// Canonical JSON policy marker.
    ///
    /// Design contract:
    /// - Structured data that participates in hashing MUST be serialized as canonical JSON.
    /// - Target semantics: RFC 8785 (JCS).
    ///
    /// Implementations belong in `machine` internals; this module only reserves the concept.
    pub trait CanonicalJsonPolicy: Send + Sync {}
}

pub mod config {
    use super::*;
    use crate::ids::OpId;
    use crate::meta::Tag;

    /// Whether a run is allowed to perform live IO or must replay from recorded facts/artifacts.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum IoMode {
        Live,
        Replay,
    }

    /// Controls domain event verbosity. Kernel events are always emitted.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum EventProfile {
        Minimal,
        Normal,
        Verbose,
        Custom(String),
    }

    /// Backoff policy for retryable errors.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum BackoffPolicy {
        Fixed { delay: Duration },
        Exponential {
            base_delay: Duration,
            max_delay: Duration,
        },
    }

    /// Retry policy for retryable errors (including replay missing-fact errors if configured retryable).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RetryPolicy {
        pub max_attempts: u32,
        pub backoff: BackoffPolicy,
    }

    /// Execution mode.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ExecutionMode {
        Sequential,
        /// Explicit fan-out/join model. Avoids concurrent writes to shared context.
        FanOutJoin { max_concurrency: u32 },
    }

    /// Run-level context checkpointing policy.
    ///
    /// Default is `AfterEveryState` to keep resume semantics simple (no replay required on resume).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ContextCheckpointing {
        AfterEveryState,
        /// Reserved for future: periodic/tag-based checkpointing policies.
        Custom(String),
    }

    /// Run-level execution configuration (policy).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunConfig {
        pub io_mode: IoMode,
        pub retry_policy: RetryPolicy,
        pub event_profile: EventProfile,
        pub execution_mode: ExecutionMode,
        pub context_checkpointing: ContextCheckpointing,

        /// If true, ReplayIo MissingFact errors are retryable (default false).
        pub replay_missing_fact_retryable: bool,

        /// States with any of these tags may be skipped by the executor.
        /// Common use: skip APPLY_SIDE_EFFECT for dry runs.
        pub skip_tags: Vec<Tag>,
    }

    /// Minimal run manifest shape (stored as an artifact; hashed via canonical JSON).
    /// Note: `input_params` MUST be canonical-JSON hashable and MUST NOT contain secrets.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunManifest {
        pub op_id: OpId,
        pub op_version: String,
        pub input_params: serde_json::Value,
        pub run_config: RunConfig,
        pub build: BuildProvenance,
    }

    /// Build provenance (reproducibility metadata).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct BuildProvenance {
        pub git_commit: Option<String>,
        pub cargo_lock_hash: Option<String>,
        pub flake_lock_hash: Option<String>,
        pub rustc_version: Option<String>,
        pub target_triple: Option<String>,
        pub env_allowlist: Vec<String>,
    }
}

pub mod meta {
    use super::*;

    /// Tags are used for classification, filtering, and policy decisions.
    /// Recommended format: lowercase; allow separators for namespacing if needed.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Tag(pub String);

    /// Standard tags (stable identifiers).
    /// Implementations may provide helpers, but these string constants are the contract.
    pub mod standard_tags {
        // kind tags
        pub const CONFIG: &str = "config";
        pub const FETCH_DATA: &str = "fetch_data";
        pub const COMPUTE: &str = "compute";
        pub const EXECUTE: &str = "execute";
        pub const REPORT: &str = "report";

        // behavior tags
        pub const APPLY_SIDE_EFFECT: &str = "apply_side_effect";
        pub const IMPURE: &str = "impure";
    }

    /// Side-effect classification (affects replay and retry semantics).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum SideEffectKind {
        Pure,
        ReadOnlyIo,
        ApplySideEffect,
    }

    /// Optional idempotency declaration for side-effecting states.
    /// Key semantics: stable value used for dedupe (e.g., tx intent hash).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Idempotency {
        None,
        Key(String),
    }

    /// Strategy for choosing a recovery point among dependency candidates.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DependencyStrategy {
        Latest,
        Earliest,
        LatestSuccessful,
    }

    /// State metadata used for policy decisions and validation.
    ///
    /// Notes:
    /// - `depends_on` is an authoring-time *hint* (often used by planners).
    /// - Execution correctness is governed by explicit plan edges.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StateMeta {
        pub tags: Vec<Tag>,

        /// Optional authoring-time dependency hints expressed as tags.
        pub depends_on: Vec<Tag>,
        pub depends_on_strategy: DependencyStrategy,

        pub side_effects: SideEffectKind,
        pub idempotency: Idempotency,
    }
}

pub mod errors {
    use super::*;
    use crate::ids::{ErrorCode, StateId};

    /// Error category used for stable handling and policies.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ErrorCategory {
        ParsingInput,
        OnChain,
        OffChain,
        Rpc,
        Storage,
        Context,
        Unknown,
    }

    /// Structured error info (canonical-JSON compatible; MUST NOT contain secrets).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ErrorInfo {
        pub code: ErrorCode,
        pub category: ErrorCategory,
        pub retryable: bool,
        pub message: String,
        pub details: Option<serde_json::Value>,
    }

    /// Errors returned by state handlers (no secrets).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StateError {
        pub state_id: Option<StateId>,
        pub info: ErrorInfo,
    }

    /// IO errors (live or replay).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum IoError {
        /// Replay was asked to perform deterministic IO without a fact key.
        MissingFactKey(ErrorInfo),

        MissingFact { key: crate::ids::FactKey, info: ErrorInfo },
        Transport(ErrorInfo),
        RateLimited(ErrorInfo),
        Other(ErrorInfo),
    }

    /// Context errors.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ContextError {
        MissingKey { key: crate::ids::ContextKey, info: ErrorInfo },
        Serialization(ErrorInfo),
        Other(ErrorInfo),
    }

    /// Storage errors (event store / artifact store).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum StorageError {
        Concurrency(ErrorInfo),
        NotFound(ErrorInfo),
        Corruption(ErrorInfo),
        Other(ErrorInfo),
    }

    /// Run-level errors from the engine.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum RunError {
        InvalidPlan(ErrorInfo),
        Storage(StorageError),
        Context(ContextError),
        Io(IoError),
        State(StateError),
        Other(ErrorInfo),
    }
}

pub mod context {
    use super::*;
    use crate::errors::ContextError;
    use crate::ids::ContextKey;

    /// Dynamic context interface.
    ///
    /// Contract:
    /// - `dump()` returns a **full snapshot** of current state (canonical JSON object recommended).
    /// - Implementations MUST ensure deterministic serialization of snapshot artifacts.
    pub trait DynContext: Send {
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError>;
        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError>;
        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError>;

        /// Full snapshot of current context state.
        fn dump(&self) -> Result<serde_json::Value, ContextError>;
    }

    /// Typed convenience extension (no default bodies; implementations may blanket-impl internally).
    pub trait TypedContextExt {
        fn read_typed<T: serde::de::DeserializeOwned>(
            &self,
            key: &ContextKey,
        ) -> Result<Option<T>, ContextError>;

        fn write_typed<T: Serialize>(
            &mut self,
            key: ContextKey,
            value: &T,
        ) -> Result<(), ContextError>;
    }
}

pub mod events {
    use super::*;
    use crate::errors::StateError;
    use crate::ids::{ArtifactId, OpId, OpPath, RunId, StateId};

    /// Run completion status.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum RunStatus {
        Completed,
        Failed,
        Cancelled,
    }

    /// Kernel event variants required for recovery/resume correctness.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum KernelEvent {
        RunStarted {
            op_id: OpId,
            manifest_id: ArtifactId,
            /// Snapshot of initial context at run start.
            initial_snapshot_id: ArtifactId,
        },
        StateEntered {
            state_id: StateId,
            attempt: u32,
            /// Snapshot the attempt starts from (resume/retry boundary).
            base_snapshot_id: ArtifactId,
        },
        StateCompleted {
            state_id: StateId,
            context_snapshot_id: ArtifactId,
        },
        StateFailed {
            state_id: StateId,
            error: StateError,
            /// Diagnostic-only snapshot (must not be used as a resume boundary).
            failure_snapshot_id: Option<ArtifactId>,
        },
        RunCompleted {
            status: RunStatus,
            final_snapshot_id: Option<ArtifactId>,
        },
    }

    /// Optional domain event (operation-defined; verbosity is controlled by event profile).
    ///
    /// Rules:
    /// - payload MUST be canonical-JSON compatible
    /// - payload MUST NOT contain secrets
    /// - large payloads SHOULD be stored as artifacts and referenced via `payload_ref`
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct DomainEvent {
        pub name: String,
        pub payload: serde_json::Value,
        pub payload_ref: Option<ArtifactId>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Event {
        Kernel(KernelEvent),
        Domain(DomainEvent),
    }

    /// Envelope stored in the event store.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct EventEnvelope {
        pub run_id: RunId,
        pub seq: u64,

        /// Informational timestamp; must not be required for deterministic replay semantics.
        pub ts_millis: Option<u64>,

        pub event: Event,
    }

    /// Recommended standard domain event payloads (not required by engine).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct FactRecorded {
        pub key: crate::ids::FactKey,
        pub payload_id: ArtifactId,
        pub meta: serde_json::Value,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ArtifactWritten {
        pub artifact_id: ArtifactId,
        pub kind: crate::stores::ArtifactKind,
        pub meta: serde_json::Value,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct OpBoundary {
        pub op_path: OpPath,
        pub phase: String, // e.g. "started" | "completed"
    }

    /// Reserved for later milestones (nested machines).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ChildRunSpawned {
        pub parent_run_id: RunId,
        pub child_run_id: RunId,
        pub child_manifest_id: ArtifactId,
    }
}

pub mod io {
    use super::*;
    use crate::errors::IoError;
    use crate::ids::{ArtifactId, FactKey};

    /// Opaque IO call surface; collectors define typed adapters on top.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct IoCall {
        /// Namespace like "http", "jsonrpc", "coingecko", etc.
        pub namespace: String,
        /// Canonical JSON request payload (typed by the caller/collector).
        pub request: serde_json::Value,
        /// Fact key for recording/replay.
        ///
        /// Contract:
        /// - In Live mode, callers SHOULD provide this for replayable IO.
        /// - In Replay mode, deterministic IO MUST provide this (otherwise `IoError::MissingFactKey`).
        pub fact_key: Option<FactKey>,
    }

    /// Opaque IO result surface; collectors define typed adapters on top.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct IoResult {
        /// Canonical JSON response payload.
        pub response: serde_json::Value,
        /// If recorded, points to the stored payload artifact.
        pub recorded_payload_id: Option<ArtifactId>,
    }

    /// IO provider (LiveIo/ReplayIo are implementations).
    #[async_trait]
    pub trait IoProvider: Send {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError>;

        /// Lookup recorded fact payload by key.
        async fn get_recorded_fact(&mut self, key: &FactKey) -> Result<Option<ArtifactId>, IoError>;

        /// Current time. If used in deterministic logic, implementations MUST record as facts.
        async fn now_millis(&mut self) -> Result<u64, IoError>;

        /// Random bytes. If used in reproducible paths, implementations MUST record as facts.
        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError>;
    }
}

pub mod recorder {
    use super::*;
    use crate::errors::RunError;
    use crate::events::DomainEvent;

    /// Domain event recorder used by state handlers.
    ///
    /// Engine contract:
    /// - Domain events are associated with the current state attempt (bounded by `StateEntered` and a terminal
    ///   event).
    /// - A state attempt MAY span multiple transactional appends; each append is atomic.
    #[async_trait]
    pub trait EventRecorder: Send {
        async fn emit(&mut self, event: DomainEvent) -> Result<(), RunError>;
        async fn emit_many(&mut self, events: Vec<DomainEvent>) -> Result<(), RunError>;
    }
}

pub mod state {
    use super::*;
    use crate::context::DynContext;
    use crate::errors::StateError;
    use crate::io::IoProvider;
    use crate::meta::StateMeta;
    use crate::recorder::EventRecorder;

    /// Indicates whether the engine should snapshot context after the state.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum SnapshotPolicy {
        Never,
        OnSuccess,
        Always,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StateOutcome {
        pub snapshot: SnapshotPolicy,
    }

    /// Note:
    /// - The engine MAY still checkpoint context according to `RunConfig.context_checkpointing`
    ///   regardless of `StateOutcome.snapshot`. This hint controls additional snapshot behavior
    ///   and/or diagnostic snapshots, not permission to bypass required checkpoints.

    /// State behavior. States do NOT own their `StateId` — IDs are assigned by the plan.
    #[async_trait]
    pub trait State: Send + Sync {
        fn meta(&self) -> StateMeta;

        async fn handle(
            &self,
            ctx: &mut dyn DynContext,
            io: &mut dyn IoProvider,
            rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError>;
    }

    pub type DynState = Arc<dyn State>;
}

pub mod plan {
    use super::*;
    use crate::ids::{OpId, StateId};
    use crate::state::DynState;

    /// An edge `from -> to` means `from` must complete before `to` can run.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct DependencyEdge {
        pub from: StateId,
        pub to: StateId,
    }

    #[derive(Clone)]
    pub struct StateNode {
        pub id: StateId,
        pub state: DynState,
    }

    /// A state graph is the executable structure derived from ops/pipelines.
    #[derive(Clone)]
    pub struct StateGraph {
        pub states: Vec<StateNode>,
        pub edges: Vec<DependencyEdge>,
    }

    #[derive(Clone)]
    pub struct ExecutionPlan {
        pub op_id: OpId,
        pub graph: StateGraph,
    }

    /// Plan validation errors (fail-fast).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum PlanValidationError {
        EmptyPlan,
        DuplicateStateId { state_id: StateId },
        MissingStateForEdge { missing: StateId },
        CircularDependency { cycle: Vec<StateId> },

        /// Optional: if planners derive edges from tag dependencies, they may validate those too.
        DanglingDependencyTag { state_id: StateId, missing_tag: crate::meta::Tag },
    }

    pub trait PlanValidator: Send + Sync {
        fn validate(&self, plan: &ExecutionPlan) -> Result<(), PlanValidationError>;
    }
}

pub mod stores {
    use super::*;
    use crate::errors::StorageError;
    use crate::events::EventEnvelope;
    use crate::ids::{ArtifactId, RunId};

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ArtifactKind {
        Manifest,
        ContextSnapshot,
        FactPayload,
        Output,
        Other(String),
    }

    /// Append-only event store with optimistic concurrency.
    #[async_trait]
    pub trait EventStore: Send + Sync {
        async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError>;

        async fn append(
            &self,
            run_id: RunId,
            expected_seq: u64,
            events: Vec<EventEnvelope>,
        ) -> Result<u64, StorageError>;

        async fn read_range(
            &self,
            run_id: RunId,
            from_seq: u64,
            to_seq: Option<u64>,
        ) -> Result<Vec<EventEnvelope>, StorageError>;
    }

    /// Immutable, content-addressed artifact store.
    #[async_trait]
    pub trait ArtifactStore: Send + Sync {
        async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>) -> Result<ArtifactId, StorageError>;
        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError>;
        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError>;
    }
}

pub mod engine {
    use super::*;
    use crate::config::{RunConfig, RunManifest};
    use crate::context::DynContext;
    use crate::errors::RunError;
    use crate::ids::{ArtifactId, RunId};
    use crate::plan::ExecutionPlan;
    use crate::stores::{ArtifactStore, EventStore};

    /// Current run phase (observability).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum RunPhase {
        Running,
        Completed,
        Failed,
        Cancelled,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunResult {
        pub run_id: RunId,
        pub phase: RunPhase,
        pub final_snapshot_id: Option<ArtifactId>,
    }

    /// Inputs required to start a run.
    pub struct StartRun {
        pub manifest: RunManifest,
        pub manifest_id: ArtifactId,
        pub plan: ExecutionPlan,
        pub run_config: RunConfig,
        pub initial_context: Box<dyn DynContext>,
    }

    /// Store bundle passed to the engine.
    pub struct Stores {
        pub events: Arc<dyn EventStore>,
        pub artifacts: Arc<dyn ArtifactStore>,
    }

    /// Execution engine interface.
    #[async_trait]
    pub trait ExecutionEngine: Send + Sync {
        async fn start(&self, stores: Stores, run: StartRun) -> Result<RunResult, RunError>;
        async fn resume(&self, stores: Stores, run_id: RunId) -> Result<RunResult, RunError>;
    }
}
```

# C.2 `mfm-sdk` — Public API Contract

```rust
//! crates/sdk/src/lib.rs — public API contract (types + traits only)
//
// Notes:
// - `mfm-sdk` depends on `mfm-machine` and provides orchestration ergonomics only.
// - Anything not listed here is internal/unstable, even if temporarily `pub`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use mfm_machine::config::{BuildProvenance, RunConfig};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunResult, Stores};
use mfm_machine::errors::{ErrorInfo, RunError};
use mfm_machine::ids::{OpId, OpPath, RunId};
use mfm_machine::plan::{ExecutionPlan, StateGraph};

pub mod ids {
    use super::*;

    /// Milestone 1 enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct MachineId(pub String);

    /// Milestone 1 enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct StepId(pub String);

    /// Local state id within an operation.
    ///
    /// Milestone 1 enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct StateLocalId(pub String);

    /// Logical import/export key for cross-op wiring (not a `ContextKey`).
    ///
    /// Recommended: dot-separated segments like `prices.latest_eth`.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct PortKey(pub String);
}

pub mod errors {
    use super::*;

    /// SDK planning/launch errors (no secrets).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct SdkError {
        pub info: ErrorInfo,
    }
}

pub mod op {
    use super::*;
    use crate::errors::SdkError;
    use crate::ids::PortKey;

    /// Declared op IO surface for pipeline validation.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct OpIo {
        pub imports: Vec<PortKey>,
        pub exports: Vec<PortKey>,
    }

    /// A reusable operation definition.
    ///
    /// Contract:
    /// - `op_id` + `op_version` MUST be stable across environments.
    /// - `expand()` MUST be deterministic and MUST NOT perform IO.
    /// - `expand()` MUST assign StateIds of the form: "<op_path>.<state_local_id>" (3 segments).
    pub trait Operation: Send + Sync {
        fn op_id(&self) -> OpId;
        fn op_version(&self) -> String;

        fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError>;

        fn expand(
            &self,
            op_path: OpPath,
            op_config: &serde_json::Value,
            run_config: &RunConfig,
        ) -> Result<StateGraph, SdkError>;
    }

    pub type DynOperation = Arc<dyn Operation>;

    /// Registry used to resolve operations (by id + version) at plan/launch/resume time.
    pub trait OperationRegistry: Send + Sync {
        fn resolve(&self, op_id: &OpId, op_version: &str) -> Result<DynOperation, SdkError>;
    }
}

pub mod pipeline {
    use super::*;
    use crate::errors::SdkError;
    use crate::ids::{MachineId, StepId};
    use crate::op::OperationRegistry;

    /// One pipeline step.
    ///
    /// Contract:
    /// - `step_id` MUST be unique within the pipeline.
    /// - `op_config` MUST be canonical-JSON hashable and MUST NOT contain secrets.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct PipelineStep {
        pub step_id: StepId,
        pub op_id: OpId,
        pub op_version: String,
        pub op_config: serde_json::Value,
    }

    /// A Milestone 1 flattened machine definition (ordered steps).
    ///
    /// Contract:
    /// - `machine_id` + `pipeline_version` map to `RunManifest.{op_id, op_version}`.
    /// - Step `OpPath` is "<machine_id>.<step_id>".
    /// - Milestone 1 single-op convention: wrap a single op as a 1-step pipeline with:
    ///   - `machine_id = <op_id>`
    ///   - `steps[0].step_id = "main"`
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Pipeline {
        pub machine_id: MachineId,
        pub pipeline_version: String,
        pub steps: Vec<PipelineStep>,
    }

    /// Recommended `RunManifest.input_params` shape for pipeline runs (canonical JSON; no secrets).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct PipelineManifestInput {
        pub pipeline: Pipeline,
        pub input: serde_json::Value,
    }

    /// Pipeline planning contract (flattened composition).
    pub trait PipelinePlanner: Send + Sync {
        /// Implementations MUST:
        /// - resolve ops via `OperationRegistry`
        /// - ensure all `StateId`s are unique and match "<machine_id>.<step_id>.<state_local_id>"
        /// - enforce step order by adding dependency edges between step graphs (flattened composition)
        fn build_execution_plan(
            &self,
            registry: Arc<dyn OperationRegistry>,
            pipeline: &Pipeline,
            run_config: &RunConfig,
        ) -> Result<ExecutionPlan, SdkError>;
    }
}

pub mod launcher {
    use super::*;
    use crate::op::OperationRegistry;
    use crate::pipeline::{Pipeline, PipelinePlanner};

    /// Start inputs for launching a pipeline run.
    pub struct LaunchPipeline {
        pub pipeline: Pipeline,
        pub input: serde_json::Value,
        pub run_config: RunConfig,
        pub build: BuildProvenance,
        pub initial_context: Box<dyn DynContext>,
    }

    /// Run launcher contract:
    /// - plan the pipeline via `PipelinePlanner`
    /// - compute + store the `RunManifest` artifact (content-addressed)
    ///   - `RunManifest.input_params` SHOULD embed `PipelineManifestInput { pipeline, input }` (no secrets)
    /// - call `ExecutionEngine::{start,resume}`
    #[async_trait]
    pub trait RunLauncher: Send + Sync {
        async fn start_pipeline(
            &self,
            engine: Arc<dyn ExecutionEngine>,
            stores: Stores,
            registry: Arc<dyn OperationRegistry>,
            planner: Arc<dyn PipelinePlanner>,
            req: LaunchPipeline,
        ) -> Result<RunResult, RunError>;

        async fn resume(
            &self,
            engine: Arc<dyn ExecutionEngine>,
            stores: Stores,
            registry: Arc<dyn OperationRegistry>,
            planner: Arc<dyn PipelinePlanner>,
            run_id: RunId,
        ) -> Result<RunResult, RunError>;
    }
}
```
