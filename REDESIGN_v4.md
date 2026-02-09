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
- (optional later) typed schema registry

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
- Large raw external payloads (store as fact payload artifacts instead).
- Secrets (never store).

---

## 4. Workspace Structure

(unchanged from prior drafts; omitted here for brevity—same as v3 layout and boundary intent.)

---

## 5. Dependency Graph & Boundary Rules

(unchanged; same as v3.)

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

**Rule**
- Domain events must never be required for engine correctness.
- Domain events must never include secrets.

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

---

## 7. Reproducibility Model

### 7.1 Run manifest
Each run has a manifest artifact whose ID is included in `RunStarted`.

Recommended fields:
- `op_id` + `op_version`
- build provenance (git/cargo/flake/rustc/target)
- `input_params` (canonical JSON; no secrets)
- config refs (as artifacts; no secrets)
- env allowlist + captured values (allowlisted only)
- run config (retry, replay policy, event profile)
- io mode (live/replay)

### 7.2 Canonical JSON hashing rules
- Structured data → canonical JSON bytes → hash → `ArtifactId`
- Binary blobs → raw bytes → hash → `ArtifactId`
- Hash function (Milestone 1): SHA-256.

### 7.3 Facts as first-class citizens (single-assignment)
Any external/non-deterministic input used by deterministic logic is a “fact”.

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
  - `(run_id, state_id, call_ordinal, kind)` (exact encoding is implementation-defined but MUST be stable)
- `ReplayIo` MUST serve those recorded values or return `MissingFact`.

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

---

## 10. Operations as Expandable State Graphs

(unchanged conceptually; tightened ID rules below.)

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

---

## 12–17 Remaining Sections

Same intent as v3; Milestone 1 security remains “no secrets persisted”; encrypted secret-bearing artifacts deferred.

---

## Appendix C — Public API Contract (v0.1)

This is the minimal stable public API surface for `mfm-machine` and `mfm-sdk` (types + traits only).

```rust
//! crates/machine/src/lib.rs — public API contract (types + traits only)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

pub mod ids {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct OpId(pub String);

    /// Milestone 1 enforced: "<machine_id>.<step_id>"
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct OpPath(pub String);

    /// Milestone 1 enforced: "<machine_id>.<step_id>.<state_local_id>"
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct StateId(pub String);

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct RunId(pub uuid::Uuid);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ArtifactId(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct FactKey(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ContextKey(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ErrorCode(pub String);
}

pub mod canonical {
    pub trait CanonicalJsonPolicy: Send + Sync {}
}

pub mod config {
    use super::*;
    use crate::meta::Tag;
    use crate::ids::OpId;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum IoMode {
        Live,
        Replay,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum EventProfile {
        Minimal,
        Normal,
        Verbose,
        Custom(String),
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum BackoffPolicy {
        Fixed { delay: Duration },
        Exponential { base_delay: Duration, max_delay: Duration },
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RetryPolicy {
        pub max_attempts: u32,
        pub backoff: BackoffPolicy,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ExecutionMode {
        Sequential,
        FanOutJoin { max_concurrency: u32 },
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ContextCheckpointing {
        AfterEveryState,
        Custom(String),
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunConfig {
        pub io_mode: IoMode,
        pub retry_policy: RetryPolicy,
        pub event_profile: EventProfile,
        pub execution_mode: ExecutionMode,
        pub context_checkpointing: ContextCheckpointing,

        /// If true, ReplayIo MissingFact errors are retryable (default false).
        pub replay_missing_fact_retryable: bool,

        pub skip_tags: Vec<Tag>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunManifest {
        pub op_id: OpId,
        pub op_version: String,
        pub input_params: serde_json::Value,
        pub run_config: RunConfig,
        pub build: BuildProvenance,
    }

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

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Tag(pub String);

    pub mod standard_tags {
        pub const CONFIG: &str = "config";
        pub const FETCH_DATA: &str = "fetch_data";
        pub const COMPUTE: &str = "compute";
        pub const EXECUTE: &str = "execute";
        pub const REPORT: &str = "report";
        pub const APPLY_SIDE_EFFECT: &str = "apply_side_effect";
        pub const IMPURE: &str = "impure";
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum SideEffectKind {
        Pure,
        ReadOnlyIo,
        ApplySideEffect,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Idempotency {
        None,
        Key(String),
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DependencyStrategy {
        Latest,
        Earliest,
        LatestSuccessful,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StateMeta {
        pub tags: Vec<Tag>,
        pub depends_on: Vec<Tag>,
        pub depends_on_strategy: DependencyStrategy,
        pub side_effects: SideEffectKind,
        pub idempotency: Idempotency,
    }
}

pub mod errors {
    use super::*;
    use crate::ids::{ErrorCode, StateId};

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

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ErrorInfo {
        pub code: ErrorCode,
        pub category: ErrorCategory,
        pub retryable: bool,
        pub message: String,
        pub details: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StateError {
        pub state_id: Option<StateId>,
        pub info: ErrorInfo,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum IoError {
        /// Replay was asked to perform deterministic IO without a fact key.
        MissingFactKey(ErrorInfo),

        MissingFact { key: crate::ids::FactKey, info: ErrorInfo },

        Transport(ErrorInfo),
        RateLimited(ErrorInfo),
        Other(ErrorInfo),
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ContextError {
        MissingKey { key: crate::ids::ContextKey, info: ErrorInfo },
        Serialization(ErrorInfo),
        Other(ErrorInfo),
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum StorageError {
        Concurrency(ErrorInfo),
        NotFound(ErrorInfo),
        Corruption(ErrorInfo),
        Other(ErrorInfo),
    }

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

    pub trait DynContext: Send {
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError>;
        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError>;
        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError>;
        fn dump(&self) -> Result<serde_json::Value, ContextError>;
    }

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

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum RunStatus {
        Completed,
        Failed,
        Cancelled,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum KernelEvent {
        RunStarted {
            op_id: OpId,
            manifest_id: ArtifactId,
            initial_snapshot_id: ArtifactId,
        },
        StateEntered {
            state_id: StateId,
            attempt: u32,
            base_snapshot_id: ArtifactId,
        },
        StateCompleted {
            state_id: StateId,
            context_snapshot_id: ArtifactId,
        },
        StateFailed {
            state_id: StateId,
            error: StateError,
            failure_snapshot_id: Option<ArtifactId>,
        },
        RunCompleted {
            status: RunStatus,
            final_snapshot_id: Option<ArtifactId>,
        },
    }

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

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct EventEnvelope {
        pub run_id: RunId,
        pub seq: u64,
        pub ts_millis: Option<u64>,
        pub event: Event,
    }

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
        pub phase: String,
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

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct IoCall {
        pub namespace: String,
        pub request: serde_json::Value,
        pub fact_key: Option<FactKey>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct IoResult {
        pub response: serde_json::Value,
        pub recorded_payload_id: Option<ArtifactId>,
    }

    #[async_trait]
    pub trait IoProvider: Send {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError>;
        async fn get_recorded_fact(&mut self, key: &FactKey) -> Result<Option<ArtifactId>, IoError>;
        async fn now_millis(&mut self) -> Result<u64, IoError>;
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
    /// - Domain events are associated with the current state attempt (bounded by `StateEntered` and a terminal event).
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

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum PlanValidationError {
        EmptyPlan,
        DuplicateStateId { state_id: StateId },
        MissingStateForEdge { missing: StateId },
        CircularDependency { cycle: Vec<StateId> },
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
}

pub mod engine {
    use super::*;
    use crate::config::{RunConfig, RunManifest};
    use crate::context::DynContext;
    use crate::errors::RunError;
    use crate::ids::{ArtifactId, RunId};
    use crate::plan::ExecutionPlan;
    use crate::stores::{ArtifactStore, EventStore};

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

    pub struct StartRun {
        pub manifest: RunManifest,
        pub manifest_id: ArtifactId,
        pub plan: ExecutionPlan,
        pub run_config: RunConfig,
        pub initial_context: Box<dyn DynContext>,
    }

    pub struct Stores {
        pub events: Arc<dyn EventStore>,
        pub artifacts: Arc<dyn ArtifactStore>,
    }

    #[async_trait]
    pub trait ExecutionEngine: Send + Sync {
        async fn start(&self, stores: Stores, run: StartRun) -> Result<RunResult, RunError>;
        async fn resume(&self, stores: Stores, run_id: RunId) -> Result<RunResult, RunError>;
    }
}
