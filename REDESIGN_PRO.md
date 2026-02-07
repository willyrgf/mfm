Below is a “ready-to-paste” **REDESIGN.md** that takes your sketch and fills in the missing architectural decisions, invariants, crate boundaries, and the append-only + reproducibility model.

You can copy this as-is, then trim or reorder sections later.

---

# MFM — Redesign

> **Intent**: This document is the design contract for the next MFM architecture.
> It prioritizes *reproducibility, auditability, and simplicity* over “feature velocity”.

---

## System Design Requirements

### 1) Append-only + transactional by default

MFM must behave like an **event-sourced system**:

* All meaningful actions produce **immutable events**.
* A “run” (an execution of an operation) is an **append-only event stream**.
* State is derived from events (plus optional snapshots).
* Any persistent write that changes system-visible state is either:

  * appended as an event inside a transaction, or
  * written as an immutable artifact referenced by an event.

**Why**:

* Enables robust recovery and replay.
* Enables strong audit trails.
* Makes reproducibility a first-class property.

**Non-goal**: “Everything is forever.”
We can support compaction/snapshots and retention policies, but those must be explicit and auditable.

---

### 2) Reproducibility (Nix philosophy, system-wide)

MFM must treat *executions* like Nix treats *builds*:

* A run has a **manifest** describing *exactly* what it depends on.
* Artifacts are **content-addressed** (hash-based IDs).
* Re-running the same operation with the same inputs should produce:

  * identical artifacts for pure steps, and
  * identical *recorded facts* for impure steps if replayed in “offline/replay mode”.

**Key principle**: External inputs (RPC calls, HTTP responses, wall-clock time) are *facts* that must be captured if reproducibility is required.

---

### 3) Simplicity (KISS)

Prefer “boring” primitives that are easy to reason about:

* Event log + artifact store + state machine runtime
* Minimal number of core traits
* Minimal cross-crate coupling
* Small, reviewable changes

---

### 4) Composability & reusability

MFM is an “operations toolkit”, not a monolith:

* Operations are composable state machines
* Collectors, storage backends, and primitives are reusable libraries
* Binaries (CLI/API) are thin wrappers over operations

---

### 5) Auditability: everything is trackable

Every run must have:

* unique run ID
* immutable event history
* referenced inputs (config snapshots, parameters)
* referenced outputs (artifacts)
* provenance (code version, build environment)
* error details (structured, stable)

**Important**: track *events*, not logs. Logs are optional; events are required.

---

### 6) Everything executes inside a state machine

* Every operation is a state machine.
* “CLI commands” and “API endpoints” are just ways to start/resume a machine.
* State machines can call sub-state-machines:

  * a handler may spawn a nested machine, and must record linkage in events.

---

## Core Concepts & Terminology

### Operation

A reusable definition of *what to do*:

* inputs schema
* state graph (states + dependencies)
* declared side effects
* output schema

### Run

A concrete execution of an Operation:

* immutable run manifest
* event stream of transitions and facts
* produces artifacts
* can be resumed/replayed

### State

A step inside a machine:

* deterministic transformation or an impure boundary
* reads/writes context
* emits events
* may produce artifacts

### Context

The evolving data available to the machine. Context must support:

* typed reads/writes
* snapshotting
* deterministic serialization (for hashing and replay)

### Event

Immutable record appended to a run:

* “state entered”, “state completed”, “fact recorded”, “artifact produced”, etc.

### Artifact

Immutable blob or document produced by a run:

* content-addressed ID (hash)
* stored in artifact store (filesystem/S3/MinIO/etc.)
* referenced by events (not embedded in events)

---

## Append-only Execution Model

### Event Stream (per run)

Each run is an ordered sequence:

* `RunStarted { manifest_hash, op_id, run_id, parent_run_id? }`
* `StateEntered { state_id, attempt, timestamp? }`
* `FactRecorded { fact_type, payload_ref }` *(optional but required for reproducible replays of impure steps)*
* `ArtifactWritten { artifact_id, kind, metadata }`
* `StateCompleted { state_id, duration_ms?, context_snapshot_id? }`
* `StateFailed { state_id, error_code, error_details }`
* `RunCompleted { status, final_context_snapshot_id? }`

**Rule**: Events must be append-only. Never update past events.

---

### Transactionality

A “state transition” should be committed atomically:

* append `StateEntered`
* append facts / artifact references (not blobs)
* append `StateCompleted` or `StateFailed`

If the process crashes mid-state, recovery reads the log and determines:

* the last completed state
* whether the current state is safe to retry
* which facts/artifacts exist

---

### Snapshots & Compaction

Event sourcing can be heavy; snapshots are allowed:

* Periodic `ContextSnapshotWritten { snapshot_id, hash }`
* Snapshot is an artifact (content-addressed)
* Replay can start from the latest snapshot + remaining events

**Compaction** is allowed only if:

* original events are retained (or archived immutably)
* compaction produces an auditable record:

  * `CompactionPerformed { old_range, new_snapshot_id, policy_id }`

---

## Reproducibility Model

### Run Manifest

Each run has a manifest artifact, hashed into `manifest_hash`:

Recommended fields:

* `op_id` + `op_version`
* `git_commit` (or workspace revision)
* `cargo_lock_hash`
* `nix_derivation` / `nix_flake_lock_hash` (when available)
* `rustc_version`
* `platform` (arch, OS)
* `input_params` (canonical-serialized)
* `config_refs` (content-addressed config files)
* `environment_allowlist` (only explicitly allowed env vars)
* `time_source` mode:

  * `live` (records time facts)
  * `replay` (reuses time facts)
* `external_io` mode:

  * `live` (records RPC/HTTP facts)
  * `replay` (must not fetch network)

**Canonical serialization requirement**:

* Manifests and typed context snapshots must be serialized deterministically
* Hashing must be stable across machines

---

### Facts: external inputs must be capturable

Anything non-deterministic must be either:

* forbidden in reproducible mode, or
* recorded as “facts” and replayed

Examples of facts:

* HTTP response body + headers (or a normalized representation)
* RPC response data
* block headers / logs retrieved
* current time
* random seed (if randomness is used)

---

### Content addressing

Use content hashes for:

* manifest
* context snapshots
* artifacts
* recorded facts payloads

This yields Nix-like guarantees:

* If two artifacts have the same ID, they are byte-identical.
* If a run references artifact IDs, it is reconstructible.

---

## State Machine Runtime Design

### Requirements

The runtime must support:

* async handlers (no blocking runtime)
* recoverability and resume
* deterministic replay mode
* nested state machines
* explicit tagging of side effects
* stable metadata and introspection

---

### State handler shape (recommended)

Move towards an async interface that makes IO explicit:

```rust
#[async_trait]
pub trait State {
    fn id(&self) -> StateId;
    fn metadata(&self) -> StateMeta; // tags, dependencies, etc.

    async fn handle(
        &self,
        ctx: &mut DynContext,
        io: &mut dyn IoProvider,
        recorder: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError>;
}
```

**Key points**:

* `IoProvider` can be swapped between `LiveIo` and `ReplayIo`
* `EventRecorder` is the only way to mutate the run history
* `StateOutcome` can reference artifacts produced

---

### Nested state machines

A state may run a sub-machine:

* Parent emits `SubRunSpawned { child_run_id, child_manifest_hash }`
* Child run has `parent_run_id`
* Parent can:

  * wait for child completion
  * or treat it as asynchronous work (later)

**Rule**: linkage must be recorded in events (no hidden control flow).

---

### Side effects, idempotency, and safety

States must declare side-effect behavior:

* `PURE`: deterministic, no IO, safe to replay freely
* `READ_ONLY_IO`: reads external data; must record facts for reproducibility
* `APPLY_SIDE_EFFECT`: submits transactions / writes external systems

**Idempotency contract**:

* Side-effecting states must provide a dedupe key strategy:

  * e.g. “transaction intent hash”
* Runtime should support “at-least-once execution” safely:

  * it may retry after crash

---

### Fixing known limitations in the current runtime

The redesign should explicitly address:

* **Sync-only handlers** → make handlers async.
* **Tracker stores shared Arc not snapshot** → track snapshots or snapshot IDs.
* **Unbounded history** → switch to delta/event-based + periodic snapshots.
* **DependencyStrategy unused** → implement it, or remove it until needed.
* **Filter uses println** → use proper logging (and never print from library).
* **`&'static str` Tag/Label limitation** → allow owned strings with validation.

---

## Storage Architecture

### Two primary storage responsibilities

1. **Event Store** (append-only)
2. **Artifact Store** (content-addressed blobs)

Optional supporting services:

* Locking/leases for concurrency control
* Index/query store for analytics (ClickHouse)

---

### Storage Traits (recommended)

Define minimal, composable interfaces:

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

**Important**:

* `append` should be optimistic-concurrency safe (`expected_seq`)
* artifacts are immutable; never overwrite by ID

---

### Backend strategy

You listed storages like ClickHouse, Postgres, MinIO. A clean split is:

* **PostgreSQL**:

  * primary event store (strong transactions)
  * run registry / indexes
* **MinIO/S3**:

  * artifact store for large blobs (snapshots, payloads)
* **ClickHouse**:

  * derived analytics tables (optional, fed from events)

Local/dev:

* filesystem artifact store
* sqlite/postgres for events (sqlite acceptable for dev; postgres preferred for parity)

---

### Security boundaries for storage

* No secrets in events or artifacts unless explicitly encrypted.
* If secrets must exist, store them only in:

  * keystore (encrypted)
  * or encrypted artifacts with strict handling
* Events should reference secret IDs, never secret contents.

---

## Workspace Structure

Proposed structure (expanded with roles and boundaries):

```
mfm/
├── Cargo.toml
├── crates/
│   ├── machine/                # state machine runtime + context + tags/labels
│   ├── machine-derive/         # proc macros
│   ├── core/                   # primitives: keystore, config models, crypto helpers
│   ├── collectors/
│   │   ├── evm/                # EVM chain collectors (RPC, logs, traces)
│   │   └── coingecko/          # price/facts collectors (HTTP)
│   ├── storages/
│   │   ├── eventstore/         # EventStore implementations (pg, local)
│   │   ├── artifactstore/      # ArtifactStore implementations (minio, fs)
│   │   └── indexer/            # optional: clickhouse projections
│   ├── ops/                    # operation definitions (state machines)
│   │   ├── aave-tracker/
│   │   ├── portfolio-tracker/
│   │   └── portfolio-management/
│   └── sdk/                    # optional: client SDK for calling ops from external apps
├── bin/
│   ├── cli/
│   └── rest-api/
├── flake.nix
└── flake.lock
```

### Naming: removing `mfm_` prefixes safely

Be careful with Rust crate name collisions:

* Avoid naming a crate simply `core` (conflicts with Rust `core`).
* Prefer a clear namespace strategy:

  * internal path: `crates/core/`
  * cargo package: `mfm-core` (hyphen) or `mfm_core` (underscore)
* Same for `machine` vs “state-machine”.

**Recommendation**:

* Keep package names namespaced (`mfm-*`) for clarity and to avoid collisions.
* Remove the prefix from module paths and directory names if you want, but keep cargo package names safe.

Example compromise:

* directory: `crates/machine/`
* package name: `mfm-machine`
* rust import: `mfm_machine` (Cargo converts `-` to `_`)

---

## Dependency Graph and Boundary Rules

### High-level dependency graph

```
{ cli, rest-api } -> ops -> { machine, core, collectors, storages }
machine -> machine-derive
collectors -> { core } (types/utilities) + (optional) machine (for shared IO abstractions)
storages -> { core } (types) + machine (run/event types)
core -> (should NOT depend on collectors/storages/ops)
```

### Boundary rules (non-negotiable)

* **Binaries** (`bin/*`) depend only on `ops` + minimal shared crates.
* **ops** is orchestration-only:

  * defines state machines
  * composes collectors + storages
  * should not contain storage implementations
* **collectors**:

  * fetch/normalize external data
  * must support replay mode by accepting an IO provider / fact store
* **storages**:

  * no business logic
  * only persistence + querying
* **core**:

  * security-critical code (keystore/crypto) must not depend on heavy IO stacks
  * must be usable without CLI/API
* **machine**:

  * generic runtime, no blockchain-specific code

---

## Operations Model

### What lives in an op crate

Each `ops/<name>` crate should include:

* `OperationId` + version
* input/output schemas
* state graph definition
* tests:

  * deterministic replay test
  * recovery/resume test
  * artifact emission test

### Operation lifecycle

1. validate inputs
2. build run manifest
3. create run in event store
4. execute machine:

   * append events as it goes
   * store artifacts/facts
5. finalize with `RunCompleted`

---

## CLI and REST API Responsibilities

### CLI

* Starts/resumes runs
* Queries run history
* Prints stable JSON/text formats
* Never owns business logic (calls ops)

### REST API

* Same responsibilities as CLI, exposed over HTTP
* Must support:

  * start run
  * resume run
  * fetch run events
  * fetch artifacts (or signed URLs)
  * observe progress (polling or SSE/websocket later)

---

## Security & Secret Handling

### Keystore

* Keep current keystore threat model: tamper detection, DoS guards, constant-time compares, zeroization
* Add:

  * atomic write (temp file + fsync + rename) or transactional store
  * optional audit events (no secrets): import/delete/lock/unlock metadata events

### Events and artifacts

* Never store private keys, mnemonics, raw decrypted data.
* If an operation needs secret material:

  * retrieve from keystore at runtime
  * keep in memory in zeroizing buffers
  * emit only references/metadata

---

## Testing Requirements (Design-level)

Every operation crate should have:

* **Replay determinism test**:

  * run in live mode (record facts)
  * rerun in replay mode (no network)
  * assert same outputs/artifacts hashes
* **Recovery test**:

  * crash simulation after N events
  * resume
  * assert run completes and is consistent
* **Schema stability test** (for CLI/API outputs):

  * stable JSON contract checks

Runtime crates should have:

* snapshot correctness tests
* event append concurrency tests (`expected_seq`)
* nested machine linkage tests

---

## Migration Plan (incremental, reviewable)

A suggested sequence that keeps PRs small:

1. Introduce `EventStore` + `ArtifactStore` traits (with local implementations).
2. Add run manifest + run event stream types (no behavior change yet).
3. Make state machine runtime able to emit events (still synchronous if needed).
4. Convert handlers to async (introduce `IoProvider` abstraction).
5. Introduce replay mode (fact recording + fact playback).
6. Move CLI to “run ops” model rather than direct keystore-only commands (keystore commands remain, but become just another op or stay as a separate surface).
7. Add Postgres + MinIO backends.
8. Add projections/indexing (ClickHouse) only after event model is stable.

---

## Open Questions (to resolve explicitly)

* What canonical serialization format do we standardize on for hashing?

  * JSON canonicalization, CBOR canonical, or something else
* How do we want to represent typed context in snapshots?

  * full snapshot vs deltas
* What is the minimum set of standard events needed for all runs?
* How strict should “replay mode” be (hard fail on network access)?
* Do we want parallel state execution, or keep it sequential for simplicity?

---

## Summary

This redesign makes MFM:

* append-only and recoverable by construction
* reproducible via manifests + content-addressed artifacts + recorded facts
* composable: ops built from collectors + storages + primitives
* auditable: runs are event streams, not just logs
* “state machines all the way down”, including nested machines

---

If you want, I can also provide a **concrete “minimum viable event schema”** (Rust structs + serde shape) and a **crates rename plan** that avoids Rust name collisions while still removing the `mfm_` feel in directory/module layout.

