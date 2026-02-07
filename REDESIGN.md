# MFM — Redesign

> **Purpose**: This document is the design contract for the next MFM architecture.  
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

### 1.1 Append-only + transactional
- All meaningful actions are recorded as **immutable events** in an **append-only log**.
- A “run” is an ordered event stream. No updates to past events.
- State transitions are **transactional** at the event-store layer:
  - either all events for a transition are appended, or none are.

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
- Nested machines are allowed (“state machines all the way down”):
  - a state handler may spawn sub-machines.

---

## 2. Design Decisions (Resolved)

### 2.1 Canonical serialization for hashing
- **Canonical JSON** is the standard for hashing structured data:
  - manifests, events, snapshots, typed payloads.
- Target semantics: **RFC 8785 (JCS)**-style canonicalization.

**Constraints**
- No NaN/Infinity.
- Avoid floats in hashed structures (prefer integer-scaled or decimal strings).

### 2.2 Context snapshots
- Snapshots are **full snapshots** (not deltas).
- Snapshots are stored as content-addressed artifacts.

### 2.3 Minimum standard events
- Runtime emits a **small required kernel** of events needed for correct recovery/resume.
- Beyond that, the **domain event set is configurable per op** (event profiles).

### 2.4 Replay mode semantics
- Replay mode does **not** hard-fail on network access.
- Missing facts/IO return a structured error.
- If the error is tagged **retryable**, the runtime retries according to run configuration.

### 2.5 Parallel execution
- Sequential execution is the default.
- Some ops may opt into parallelism, but it must remain auditable and replayable.
- Preferred expression: explicit fan-out / join (see §9).

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
- emits events
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
- e.g., RPC response, HTTP response, time reading, etc.
- stored as a content-addressed payload artifact and referenced from events

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
│   │   ├── eventstore/         # EventStore implementations (pg, local)
│   │   ├── artifactstore/      # ArtifactStore implementations (minio, fs)
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
- Directory/module names can drop `mfm_`.
- **Cargo package names should remain namespaced** to avoid collisions (`mfm-machine`, `mfm-core`, etc.).
  - Cargo translates `mfm-machine` → Rust crate import `mfm_machine`.

---

## 5. Dependency Graph & Boundary Rules

### 5.1 High-level dependency graph
```

{ cli, rest-api } -> ops -> { machine, core, collectors, storages }
machine -> machine-derive
storages -> { core, machine }   # for IDs/types and event model
collectors -> { core } (+ machine if needed for shared IO abstractions)
core -> (must not depend on collectors/storages/ops)

````

### 5.2 Boundary rules (non-negotiable)
- **Binaries** depend on ops and minimal shared crates only.
- **Ops** orchestrate only:
  - define expansion to state graphs
  - compose collectors + storages
  - no storage implementations in ops
- **Collectors** fetch/normalize data; must be usable under live or replay IO.
- **Storages** persist and query; no business logic.
- **Core** houses security-critical code; avoid heavy IO deps.
- **Machine** is generic; no chain-specific code.

---

## 6. Append-only Execution Model

### 6.1 Per-run event stream
A run is an ordered sequence of events stored in an append-only event store.

#### Kernel events (engine-level; always emitted)
Minimum required for recovery/resume/audit:

- `RunStarted { run_id, op_id, manifest_id }`
- `StateEntered { run_id, state_id, attempt, seq }`
- `StateCompleted { run_id, state_id, seq, context_snapshot_id? }`
- `StateFailed { run_id, state_id, seq, error }`
- `RunCompleted { run_id, status, final_snapshot_id? }`

Notes:
- `seq` is a strictly increasing per-run sequence number.
- `attempt` increments per state retry.

#### Domain events (operation-level; configurable per op)
Examples (not required by engine correctness):
- `FactRecorded { key, payload_id, meta }`
- `ArtifactWritten { artifact_id, kind, meta }`
- `OpBoundary { op_path, phase }`
- any other op-specific events

**Rule**
- Domain events must never be required for engine correctness.
- Domain events must never include secrets.

### 6.2 Transactionality
A “state transition” must be atomic from the event store’s perspective:
- append `StateEntered`
- append any domain events (facts/artifact refs)
- append `StateCompleted` or `StateFailed`

If a process dies mid-state:
- the run is resumed by reading the last durable kernel event boundary.

### 6.3 Snapshots & compaction
- Context snapshots are full snapshots stored as artifacts.
- A snapshot is referenced by kernel events (`context_snapshot_id`).
- Optional compaction is allowed only if auditable:
  - compaction emits events describing what was compacted and what snapshot replaces the range.

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
- `input_params` (canonical JSON)
- `config_refs` (content-addressed config files)
- `env_allowlist` + captured env values (only those allowed)
- `run_config` (retry policy, replay policy, event profile)
- `io_mode` (live / replay)

### 7.2 Canonical JSON hashing rules
- Structured data → canonical JSON bytes → hash → `ArtifactId`
- Binary blobs → raw bytes → hash → `ArtifactId`
- Hash function should be stable and widely available (e.g., SHA-256).

### 7.3 Facts as first-class citizens
Any external/non-deterministic input is a “fact”:
- RPC responses
- HTTP responses
- time readings
- randomness seeds (if used)

Facts are stored as payload artifacts and referenced via `FactRecorded` events.

---

## 8. Replay & IO Determinism

### 8.1 IO provider model
Handlers do not do ambient IO. They use an IO provider:

- `LiveIo`: performs real IO and may record facts/artifacts.
- `ReplayIo`: serves recorded facts/artifacts when available; otherwise returns a structured error.

### 8.2 Missing facts in replay mode
When replay needs a fact that does not exist:
- IO returns `IoError::MissingFact { key }`

This is not automatically fatal:
- if tagged retryable → retry per run policy
- otherwise → deterministic failure

### 8.3 “Retryable” errors are explicit
Errors must carry:
- category (e.g., network, rpc, parsing, storage)
- retryability (retryable / non-retryable)
- optional retry hints (backoff class, recommended delay, etc.)

---

## 9. State Machine Runtime Design

### 9.1 Requirements
The runtime must support:
- async handlers (no blocking runtime)
- recoverable/resumable execution
- deterministic replay mode
- nested machines
- side-effect tagging + idempotency strategy
- stable metadata + introspection

### 9.2 Handler shape (recommended)
A state handler is async and receives explicit dependencies:

```rust
#[async_trait]
pub trait State {
    fn id(&self) -> StateId;
    fn meta(&self) -> StateMeta;

    async fn handle(
        &self,
        ctx: &mut DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError>;
}
````

**Key properties**

* `IoProvider` switches live vs replay.
* `EventRecorder` is the only way to append events.
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

### 9.4 Sequential by default; parallel opt-in (fan-out / join)

Default executor is sequential.

Ops that need parallelism should use an explicit pattern:

1. `FanOut` state:

   * enumerates work items
   * spawns child runs (or child machines) per item
   * records child references

2. `Join` state:

   * waits/collects child results
   * merges deterministically (stable ordering)

This avoids concurrent writes into shared context and keeps auditability strong.

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

* include an `op_path` prefix:

  * `portfolio_tracker.fetch_balances`
  * `aave_tracker.index.logs`

If the same op appears multiple times in a pipeline, disambiguate:

* `pipeline[0].aave_tracker.fetch_blocks`
* `pipeline[1].aave_tracker.fetch_blocks`

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

Boundaries are domain events and may be enabled/disabled by the op’s event profile.

---

## 11. Nested Machines vs Flattened Composition

MFM supports two composition mechanisms:

### 11.1 Flattened composition (preferred for single-run pipelines)

* `ops1 -> ops2 -> ops3` becomes one plan, one run.
* Best when:

  * shared context is desired
  * sequencing is straightforward
  * a single audit trail is preferred

### 11.2 Nested machines spawned inside a state (preferred for isolation/parallelism)

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
pub trait EventStore {
    fn append(&self, run_id: RunId, expected_seq: u64, events: Vec<Event>) -> Result<u64>;
    fn read_range(&self, run_id: RunId, from: u64, to: Option<u64>) -> Result<Vec<Event>>;
    fn head(&self, run_id: RunId) -> Result<u64>;
}

pub trait ArtifactStore {
    fn put(&self, bytes: &[u8], kind: ArtifactKind) -> Result<ArtifactId>;
    fn get(&self, id: ArtifactId) -> Result<Vec<u8>>;
    fn exists(&self, id: ArtifactId) -> Result<bool>;
}
```

### 12.3 Backend strategy

* PostgreSQL: primary event store (strong transactions)
* MinIO/S3: artifact store for large blobs
* ClickHouse: optional projections/indexes derived from events

Local/dev:

* filesystem artifact store
* local event store (sqlite/postgres) as appropriate

### 12.4 Security rules for storage

* No secrets in events.
* Avoid secrets in artifacts; if unavoidable, artifacts must be encrypted and tightly controlled.
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

### 14.2 No secrets in events

* events must never contain:

  * passwords, mnemonics, private keys, raw decrypted buffers
* if needed, store references:

  * keystore entry IDs
  * encrypted artifact IDs

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
* nested-machine linkage tests
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

These are intentionally left open until forced by implementation:

* What is the minimal “standard library” of reusable state patterns (fetch/validate/store/join)?
* How do we version op schemas and ensure backward compatibility of stored manifests?
* Do we want a formal “event profile” spec (levels like minimal/normal/verbose) shared across ops?
* How do we represent and validate context schemas across ops (optional typed schema registry)?

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

* `minimal`: kernel events only + essential artifacts
* `normal`: facts + artifacts + boundaries
* `verbose`: detailed domain events for diagnostics

Kernel events are always emitted regardless of profile.

