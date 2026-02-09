# MFM — REDESIGN_FINAL Implementation Plan (Changelog Edition)

> **Purpose**: Granular, actionable todo list to reach `REDESIGN_FINAL.md` (v4) from the current codebase.
> **Contract rule**: `REDESIGN_FINAL.md` is authoritative. Any deviation MUST be recorded in [Deviations](#deviations).
> Last updated: 2026-02-10 (v2 — post-review revision)
>
> Legend: `[ ]` = pending, `[x]` = done, `[~]` = in progress, `[-]` = deferred/skipped

---

## Current State Summary

The codebase has 4 crates:

| Crate | Package | Purpose | Status |
|-------|---------|---------|--------|
| `mfm_machine/` | `mfm_machine` v0.1.0 | Async state machine (Tag/Label, StateHandler, Context, Scheduler, Tracker) | Functional, tested |
| `mfm_machine_derive/` | `mfm_machine_derive` v0.1.0 | Proc macros (#[state_handler], StateMetadataReqs) | Functional |
| `mfm_core/` | `mfm_core` v0.1.29 | Keystore (AES-256-GCM + Argon2id) + config models (YAML) | Mature, well-tested |
| `mfm_cli/` | `mfm` v0.1.29 | CLI (keystore import/list/delete) | Functional, tested |

**What is missing** (per REDESIGN_FINAL.md):

- Typed IDs (`RunId`, `ArtifactId`, `StateId`, `OpId`, `OpPath`, `FactKey`, `ContextKey`)
- Canonical JSON + SHA-256 hashing (RFC 8785)
- Event model (kernel + domain events, `EventEnvelope`)
- Storage traits + backends (`EventStore`, `ArtifactStore`)
- IO abstraction (`IoProvider`, `LiveIo`, `ReplayIo`, fact recording)
- Execution engine (attempt envelopes, snapshots-as-artifacts, crash/resume)
- Run manifests (content-addressed)
- State handler redesign (`State` trait with `ctx`, `io`, `rec` params)
- Execution planning (`StateGraph`, `ExecutionPlan`, `PlanValidator`)
- SDK (`Operation` trait, `Pipeline` builder, run launcher)
- Collectors crate (EVM, coingecko stubs)
- Ops crate (proof operation)
- CLI expansion (start/resume runs, inspect events/artifacts)
- Workspace restructuring (`mfm_*/` -> `crates/*/`, `bin/*/`)

---

## Acceptance Tests (Named)

Every invariant MUST be verifiable. Tasks reference these IDs.

| ID | Invariant | REDESIGN_FINAL.md |
|----|-----------|-------------------|
| AT-01 | Append-only event stream per run | SS1.1, SS6.1 |
| AT-02 | Atomic append visibility (no partial reads) | SS1.1, SS6.2 |
| AT-03 | Optimistic concurrency (expected_seq) | SS12.2 |
| AT-04 | Artifact content-addressing (SHA-256, ArtifactId = hex digest) | SS7.2 |
| AT-05 | Canonical JSON hashing (RFC 8785, no floats/NaN/Inf) | SS2.1, SS7.2 |
| AT-06 | Live-then-replay determinism (same output hashes) | SS8.1, SS7.3 |
| AT-07 | Crash/resume determinism (orphan handling) | SS6.2, SS6.4 |
| AT-08 | Side-effect idempotency across crash/resume | SS9.3 |
| AT-09 | Secrets never persisted (manifests, events, artifacts, outputs, errors) | SS2.7, SS14.2 |
| AT-10 | FactKey single-assignment (first durable wins) | SS7.3 |
| AT-11 | StateId stability (3-segment, no random components) | SS10.4, SS10.7 |
| AT-12 | Context snapshot = full snapshot as artifact | SS2.2, SS6.3 |
| AT-13 | Replay MissingFactKey / MissingFact structured errors | SS8.1, SS8.2 |
| AT-14 | Time/random recorded as facts | SS8.3 |

---

## Milestone 1 Non-Goals (Explicitly Deferred)

- REST API binary (`bin/rest-api/`)
- ClickHouse projections/indexer (`crates/storages/indexer/`)
- Nested machines / child-run spawning
- Parallel execution (fan-out/join)
- Encrypted secret-bearing artifacts
- Artifact GC / compaction
- Manual operator interventions (DLQ, force-complete)
- Typed schema registry (SS17.4)

---

## Phase 0 — Workspace Restructuring + Dev Infrastructure

> Move from flat `mfm_*/` layout to the `crates/` + `bin/` layout defined in REDESIGN_FINAL.md SS4.
> Set up Nix devshell with PostgreSQL + MinIO for the primary storage backends.

### 0.1 Directory migration

- [ ] **0.1.1** Create `crates/` and `bin/` directories
- [ ] **0.1.2** Move `mfm_machine/` -> `crates/machine/`
  - Update `Cargo.toml` path in workspace members
  - Rename package to `mfm-machine` (hyphenated per SS2.6)
  - Update internal `extern crate self as mfm_machine;` if needed
- [ ] **0.1.3** Move `mfm_machine_derive/` -> `crates/machine-derive/`
  - Rename package to `mfm-machine-derive`
  - Update dependency references in `crates/machine/Cargo.toml`
- [ ] **0.1.4** Move `mfm_core/` -> `crates/core/`
  - Rename package to `mfm-core`
  - Update dependency references
  - **Note**: The existing config module (`Config`, `Network`, `Token`, `Dex`) is domain config
    for EVM operations. It is preserved unchanged and is unrelated to the new `RunConfig`/`RunManifest`.
- [ ] **0.1.5** Move `mfm_cli/` -> `bin/cli/`
  - Keep package name `mfm`
  - Rename binary from `mfm_cli` to `mfm` (user-facing ergonomics)
  - Update workspace members
  - **Preserve all existing keystore subcommands** (`import`, `list`, `delete`)
- [ ] **0.1.6** Update root `Cargo.toml` workspace members to reflect new paths
- [ ] **0.1.7** Verify `cargo build && cargo test` passes after migration
- [ ] **0.1.8** Update `.gitignore` if needed for new layout

### 0.2 Nix devshell: PostgreSQL + MinIO services

> Per user decision: go straight to PostgreSQL + MinIO as primary storage backends.
> Nix-first approach makes production-like dependencies trivially available.

- [ ] **0.2.1** Add PostgreSQL to `flake.nix` devshell:
  - Add `pkgs.postgresql` to `buildInputs`
  - Add a `shellHook` or `process-compose`/`devenv` config to start a local PostgreSQL instance
  - Configure data directory under `.mfm-dev/pg-data/` (gitignored)
  - Create init script for `mfm_events` database
- [ ] **0.2.2** Add MinIO to `flake.nix` devshell:
  - Add `pkgs.minio` (or `pkgs.minio-client`) to `buildInputs`
  - Configure to start on a local port with `.mfm-dev/minio-data/` storage
  - Create init script for `mfm-artifacts` bucket
- [ ] **0.2.3** Update `flake.nix` binary path reference from `mfm_cli` to `bin/cli/`
- [ ] **0.2.4** Add `.mfm-dev/` to `.gitignore`
- [ ] **0.2.5** Verify `nix develop` brings up PostgreSQL + MinIO and `cargo test` can connect
- [ ] **0.2.6** Document dev setup in `README.md` or `CONTRIBUTING.md` (brief section)

### 0.3 Create stub crates (empty, compilable)

- [ ] **0.3.1** Create `crates/storages/event-store/` — package `mfm-event-store`
  - `Cargo.toml`, `src/lib.rs` (empty module)
- [ ] **0.3.2** Create `crates/storages/artifact-store/` — package `mfm-artifact-store`
- [ ] **0.3.3** Create `crates/collectors/evm/` — package `mfm-collector-evm`
- [ ] **0.3.4** Create `crates/collectors/coingecko/` — package `mfm-collector-coingecko`
- [ ] **0.3.5** Create `crates/ops/` directory (ops crates created later)
- [ ] **0.3.6** Create `crates/sdk/` — package `mfm-sdk`
- [ ] **0.3.7** Add all new crates to workspace `Cargo.toml` members
- [ ] **0.3.8** Verify `cargo check --workspace` passes

### 0.4 Documentation housekeeping

- [ ] **0.4.1** Archive/remove `REDESIGN.md`, `REDESIGN_v4.md`, `REDESIGN_PLAN.md`, `REDESIGN_FINAL_PLAN.md` (replaced by `REDESIGN_FINAL.md` + this plan)
- [ ] **0.4.2** Update `ARCHITECTURE.md` to reference the new layout
- [ ] **0.4.3** Update `CURRENT_STATE.md` after restructuring

---

## Phase 1 — Foundational Types in `mfm-machine`

> Define all public types from REDESIGN_FINAL.md Appendix C.1: IDs, config, meta, errors.
> These are types + traits only, no implementations yet.

### 1.1 Typed IDs module (`crates/machine/src/ids.rs`)

- [ ] **1.1.1** Define `OpId(String)` with Serialize/Deserialize/Clone/Debug/Eq/Hash
- [ ] **1.1.2** Define `OpPath(String)` — Milestone 1 enforced: `<machine_id>.<step_id>`
- [ ] **1.1.3** Define `StateId(String)` — Milestone 1 enforced: `<machine_id>.<step_id>.<state_local_id>`
- [ ] **1.1.4** Define `RunId(uuid::Uuid)` with Copy
- [ ] **1.1.5** Define `ArtifactId(String)` — invariant: 64-char lowercase hex (SHA-256)
- [ ] **1.1.6** Define `FactKey(String)`
- [ ] **1.1.7** Define `ContextKey(String)`
- [ ] **1.1.8** Define `ErrorCode(String)`
- [ ] **1.1.9** Add segment validation (SS10.7):
  - Parse by splitting on `.` (dot is the reserved separator, forbidden inside segments)
  - Validate segment count: `OpPath::new()` requires exactly 2 segments, `StateId::new()` requires exactly 3
  - Validate each individual segment against `^[a-z][a-z0-9_]{0,62}$`
  - Return validation error on bad input (wrong count, invalid segment, empty string)
- [ ] **1.1.10** Add `ArtifactId` format validation (64-char lowercase hex)
- [ ] **1.1.11** Add `uuid` crate dependency to `mfm-machine`
- [ ] **1.1.12** Tests: segment regex, bad IDs rejected, round-trip serde, dot-splitting edge cases
  - AT-11 coverage

### 1.2 Canonical JSON module (`crates/machine/src/canonical.rs`)

- [ ] **1.2.1** Define `CanonicalJsonPolicy` marker trait (SS Appendix C.1)
  - **Note**: This is a design placeholder / marker only. The actual serialization functions
    (`canonical_json_bytes`, `canonical_hash`, etc.) are standalone functions, not trait methods.
    The marker reserves the concept for future typed enforcement.
- [ ] **1.2.2** Implement canonical JSON serialization (RFC 8785 / JCS semantics):
  - Sorted object keys (lexicographic)
  - No trailing commas, no whitespace
  - Numbers: no leading zeros, no trailing decimal zeros, no NaN/Infinity
- [ ] **1.2.3** Add float rejection: error if `serde_json::Value` contains floats in hashed paths
- [ ] **1.2.4** Implement `canonical_hash(value: &serde_json::Value) -> ArtifactId`:
  - Canonical JSON bytes -> SHA-256 -> 64-char lowercase hex
- [ ] **1.2.5** Implement `hash_bytes(bytes: &[u8]) -> ArtifactId` for binary blobs
- [ ] **1.2.6** Add `sha2` crate dependency to `mfm-machine`
- [ ] **1.2.7** Tests: JCS test vectors, float rejection, hash stability, round-trip
  - AT-05 coverage

### 1.3 Config types module (`crates/machine/src/config.rs`)

- [ ] **1.3.1** Define `IoMode { Live, Replay }`
- [ ] **1.3.2** Define `EventProfile { Minimal, Normal, Verbose, Custom(String) }`
- [ ] **1.3.3** Define `BackoffPolicy { Fixed, Exponential }` with duration fields
  - **Note**: `std::time::Duration` does not implement `Serialize`/`Deserialize` by default.
    Use `u64` millisecond fields (e.g., `delay_ms: u64`, `base_delay_ms: u64`, `max_delay_ms: u64`)
    and provide `fn as_duration(&self) -> Duration` convenience methods.
    This avoids `serde_with` dependency and keeps canonical JSON clean.
- [ ] **1.3.4** Define `RetryPolicy { max_attempts, backoff }`
- [ ] **1.3.5** Define `ExecutionMode { Sequential, FanOutJoin { max_concurrency } }`
- [ ] **1.3.6** Define `ContextCheckpointing { AfterEveryState, Custom(String) }`
- [ ] **1.3.7** Define `RunConfig` struct with all fields from Appendix C.1
  - Include `replay_missing_fact_retryable: bool` (default false)
  - Include `skip_tags: Vec<Tag>`
- [ ] **1.3.8** Define `RunManifest` struct (op_id, op_version, input_params, run_config, build)
  - **Note**: SS7.1 mentions additional fields (`config_refs`, `env_allowlist` with captured values,
    `io_mode`). Appendix C.1 keeps the struct minimal. For Milestone 1, we follow Appendix C.1.
    `io_mode` is embedded in `run_config`. `config_refs` and captured env are deferred to post-M1.
    Recorded as Deviation D-01.
- [ ] **1.3.9** Define `BuildProvenance` struct (git_commit, cargo_lock_hash, flake_lock_hash, rustc_version, target_triple, env_allowlist)
  - **Note**: `env_allowlist: Vec<String>` lists *which* env vars are allowed, but Appendix C.1
    does not include a `captured_env` map for their actual values. This is an intentional
    simplification for Milestone 1. Recorded as Deviation D-02.
- [ ] **1.3.10** Implement `Default` for `RunConfig` (sequential, live, normal profile, no skip tags)
- [ ] **1.3.11** Tests: default config, serde round-trip, no secrets in manifest

### 1.4 Meta types module (`crates/machine/src/meta.rs`)

- [ ] **1.4.1** Define `Tag(String)` — reuse/migrate from existing `state::Tag`
- [ ] **1.4.2** Define `standard_tags` constants (SS Appendix C.1):
  - Kind: `CONFIG`, `FETCH_DATA`, `COMPUTE`, `EXECUTE`, `REPORT`
  - Behavior: `APPLY_SIDE_EFFECT`, `IMPURE`
  - **Note**: The existing code uses `fn config() -> Tag` helpers that return owned `Tag` values.
    The new design uses `&str` constants. Both can coexist; new code should use the constants.
- [ ] **1.4.3** Define `SideEffectKind { Pure, ReadOnlyIo, ApplySideEffect }`
- [ ] **1.4.4** Define `Idempotency { None, Key(String) }`
- [ ] **1.4.5** Define `DependencyStrategy { Latest, Earliest, LatestSuccessful }`
- [ ] **1.4.6** Define `StateMeta` struct (tags, depends_on, depends_on_strategy, side_effects, idempotency)
- [ ] **1.4.7** Tests: tag validation, StateMeta construction

### 1.5 Error types module (`crates/machine/src/errors.rs`)

- [ ] **1.5.1** Define `ErrorCategory` enum (ParsingInput, OnChain, OffChain, Rpc, Storage, Context, Unknown)
- [ ] **1.5.2** Define `ErrorInfo` struct (code, category, retryable, message, details)
  - Invariant: `details` MUST NOT contain secrets
- [ ] **1.5.3** Define `StateError { state_id: Option<StateId>, info }`
  - **Note**: `state_id` is `Option` because handlers do not own their `StateId` (IDs are assigned
    by the plan, per SS9.2). The engine fills in `StateFailed.state_id` at the kernel event level.
    This means `StateFailed { state_id, error: StateError { state_id: None, .. } }` is the
    expected pattern — the outer `state_id` is authoritative.
- [ ] **1.5.4** Define `IoError` enum (MissingFactKey, MissingFact, Transport, RateLimited, Other)
- [ ] **1.5.5** Define `ContextError` enum (MissingKey, Serialization, Other)
- [ ] **1.5.6** Define `StorageError` enum (Concurrency, NotFound, Corruption, Other)
- [ ] **1.5.7** Define `RunError` enum (InvalidPlan, Storage, Context, Io, State, Other)
- [ ] **1.5.8** Implement `std::error::Error` and `Display` for all error types:
  - `ErrorInfo`, `StateError`, `IoError`, `ContextError`, `StorageError`, `RunError`
  - All six types need both `Error` and `Display` impls
- [ ] **1.5.9** Tests: error construction, serde round-trip, no secrets in serialized form

### 1.6 Reconcile with existing types

- [ ] **1.6.1** Map existing `state::Tag` / `state::Label` to new `meta::Tag`
  - Existing `Label` is used for state identity; new design uses `StateId` instead
  - `Label` has no equivalent in the new design — it is superseded by `StateId`
- [ ] **1.6.2** Map existing `StateError` (6 variants + recoverability) to new `errors::StateError` + `ErrorInfo`
  - Existing: `StateError::Unknown(Recoverability, Error)`, `ParsingInput(...)`, etc.
  - New: `StateError { state_id: Option<StateId>, info: ErrorInfo { code, category, retryable, ... } }`
  - The 6 variants map to `ErrorCategory` values; `Recoverability` maps to `ErrorInfo.retryable`
  - Provide migration path or adapter
- [ ] **1.6.3** Map existing `DependencyStrategy` to new `meta::DependencyStrategy`
  - Direct 1:1 mapping (same variants)
- [ ] **1.6.4** Decide on backward compatibility strategy:
  - Option A: New module in `mfm-machine`, old module deprecated
  - Option B: Replace in-place, update all consumers
  - **Recommended**: Option A for Phase 1, migrate consumers in later phases, Option B cleanup at Phase 12

---

## Phase 2 — Runtime Abstractions in `mfm-machine`

> Define context, event model, IO provider, and event recorder traits from Appendix C.1.

### 2.1 Context trait (`crates/machine/src/context.rs`)

- [ ] **2.1.1** Define `DynContext` trait (SS Appendix C.1):
  - `read(&self, key: &ContextKey) -> Result<Option<Value>, ContextError>`
  - `write(&mut self, key: ContextKey, value: Value) -> Result<(), ContextError>`
  - `delete(&mut self, key: &ContextKey) -> Result<(), ContextError>`
  - `dump(&self) -> Result<Value, ContextError>` (full snapshot)
  - **Breaking change vs existing `Context` trait**:
    - New: `DynContext: Send` (not `Send + Sync`). The old trait was `Context: Send + Sync`.
      This is intentional: the engine passes `&mut dyn DynContext` (exclusive access),
      eliminating the need for interior mutability / `Arc<RwLock<...>>`.
    - New: `write(&mut self, ...)` returns `Result<(), ...>` (in-place mutation).
      Old: `write(&self, ...)` returns `Result<Box<dyn Context>, ...>` (functional/immutable, clones HashMap on every write).
    - New: `delete()` method added (did not exist in old `Context` trait).
    - These changes eliminate the need for `SafeContext` (the `Arc<RwLock<...>>` wrapper).
- [ ] **2.1.2** Define `TypedContextExt` trait (read_typed, write_typed)
- [ ] **2.1.3** Implement `InMemoryContext` (new name for clarity) implementing `DynContext`:
  - Backed by `BTreeMap<ContextKey, Value>` (sorted for deterministic `dump()`)
  - `dump()` returns JSON object with keys in sorted order
  - Replaces existing `Local` context for new API consumers
- [ ] **2.1.4** Implement `TypedContextExt` blanket impl or helper
- [ ] **2.1.5** Tests: read/write/delete/dump, typed access, missing key handling, dump determinism

### 2.2 Event model (`crates/machine/src/events.rs`)

- [ ] **2.2.1** Define `RunStatus { Completed, Failed, Cancelled }`
- [ ] **2.2.2** Define `KernelEvent` enum (SS6.1):
  - `RunStarted { op_id, manifest_id, initial_snapshot_id }`
  - `StateEntered { state_id, attempt, base_snapshot_id }`
  - `StateCompleted { state_id, context_snapshot_id }`
  - `StateFailed { state_id, error, failure_snapshot_id? }`
  - `RunCompleted { status, final_snapshot_id? }`
- [ ] **2.2.3** Define `DomainEvent { name, payload, payload_ref? }`
- [ ] **2.2.4** Define `Event { Kernel(KernelEvent), Domain(DomainEvent) }`
- [ ] **2.2.5** Define `EventEnvelope { run_id, seq, ts_millis?, event }`
- [ ] **2.2.6** Define standard domain event payloads:
  - `FactRecorded { key, payload_id, meta }`
  - `ArtifactWritten { artifact_id, kind, meta }`
  - `OpBoundary { op_path, phase }`
  - `ChildRunSpawned { parent_run_id, child_run_id, child_manifest_id }` (reserved, not used in M1)
- [ ] **2.2.7** Tests: event serde round-trip, kernel event completeness

### 2.3 IO provider trait (`crates/machine/src/io.rs`)

- [ ] **2.3.1** Define `IoCall { namespace, request, fact_key? }`
- [ ] **2.3.2** Define `IoResult { response, recorded_payload_id? }`
- [ ] **2.3.3** Define `IoProvider` async trait (SS Appendix C.1):
  - `call(&mut self, call: IoCall) -> Result<IoResult, IoError>`
  - `get_recorded_fact(&mut self, key: &FactKey) -> Result<Option<ArtifactId>, IoError>`
  - `now_millis(&mut self) -> Result<u64, IoError>`
  - `random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError>`
  - **Note**: `IoProvider: Send` (not `Send + Sync`). The engine creates a fresh `IoProvider`
    per state attempt and passes it as `&mut dyn IoProvider`. This means the engine owns the
    provider and can safely give exclusive access.
- [ ] **2.3.4** Tests: trait object creation, mock implementations

### 2.4 Event recorder trait (`crates/machine/src/recorder.rs`)

- [ ] **2.4.1** Define `EventRecorder` async trait:
  - `emit(&mut self, event: DomainEvent) -> Result<(), RunError>`
  - `emit_many(&mut self, events: Vec<DomainEvent>) -> Result<(), RunError>`
  - **Note**: `EventRecorder: Send` (not `Send + Sync`). Same ownership model as `IoProvider` —
    engine creates per-attempt, passes `&mut` to handler.
- [ ] **2.4.2** Tests: mock recorder, emit ordering

### 2.5 State trait (new design) (`crates/machine/src/state.rs`)

- [ ] **2.5.1** Define `SnapshotPolicy { Never, OnSuccess, Always }`
- [ ] **2.5.2** Define `StateOutcome { snapshot: SnapshotPolicy }`
- [ ] **2.5.3** Define `State` async trait (SS Appendix C.1):
  - `fn meta(&self) -> StateMeta`
  - `async fn handle(&self, ctx: &mut dyn DynContext, io: &mut dyn IoProvider, rec: &mut dyn EventRecorder) -> Result<StateOutcome, StateError>`
- [ ] **2.5.4** Define `DynState = Arc<dyn State>`
- [ ] **2.5.5** Tests: mock state implementation, trait object usage

### 2.6 Planning types (`crates/machine/src/plan.rs`)

- [ ] **2.6.1** Define `DependencyEdge { from: StateId, to: StateId }`
- [ ] **2.6.2** Define `StateNode { id: StateId, state: DynState }`
- [ ] **2.6.3** Define `StateGraph { states: Vec<StateNode>, edges: Vec<DependencyEdge> }`
- [ ] **2.6.4** Define `ExecutionPlan { op_id: OpId, graph: StateGraph }`
- [ ] **2.6.5** Define `PlanValidationError` enum:
  - `EmptyPlan`, `DuplicateStateId`, `MissingStateForEdge`, `CircularDependency`, `DanglingDependencyTag`
- [ ] **2.6.6** Define `PlanValidator` trait
- [ ] **2.6.7** Implement `PlanValidator` — topological sort, cycle detection, edge validation
  - Adapt existing `StateMachineBuilder::build()` validation logic from `state_machine/mod.rs`
    (it already performs: empty check, duplicate label detection, dangling dependency check,
    cycle detection via topological sort). Rewrite to operate on `StateId` instead of `Label`.
- [ ] **2.6.8** Tests: valid plan, empty plan, duplicate IDs, cycles, dangling edges
  - AT-11 coverage (StateId stability)

### 2.7 Storage traits (`crates/machine/src/stores.rs`)

- [ ] **2.7.1** Define `ArtifactKind { Manifest, ContextSnapshot, FactPayload, Output, Other(String) }`
- [ ] **2.7.2** Define `EventStore` async trait (SS12.2):
  - `head_seq(run_id) -> Result<u64, StorageError>` — returns 0 for empty run (SS6.1)
  - `append(run_id, expected_seq, events) -> Result<u64, StorageError>` — returns new `head_seq`
    (the seq of the last event appended). Store validates `first_event.seq == expected_seq + 1`
    and that all event seqs are contiguous. Rejects with `StorageError::Concurrency` on mismatch.
  - `read_range(run_id, from_seq, to_seq?) -> Result<Vec<EventEnvelope>, StorageError>`
  - **Note**: `put()` is content-addressing: the implementation hashes the bytes and returns
    `ArtifactId` computed from the hash. This is not just storage — the store *computes* the ID.
- [ ] **2.7.3** Define `ArtifactStore` async trait (SS12.2):
  - `put(kind, bytes) -> Result<ArtifactId, StorageError>` — hashes bytes, stores, returns content-addressed ID
  - `get(id) -> Result<Vec<u8>, StorageError>` — implementations SHOULD verify hash on read
  - `exists(id) -> Result<bool, StorageError>`
- [ ] **2.7.4** Tests: trait object creation

### 2.8 Engine trait (`crates/machine/src/engine.rs`)

- [ ] **2.8.1** Define `RunPhase { Running, Completed, Failed, Cancelled }`
- [ ] **2.8.2** Define `RunResult { run_id, phase, final_snapshot_id? }`
- [ ] **2.8.3** Define `StartRun { manifest, manifest_id, plan, run_config, initial_context }`
  - **Note**: `manifest_id` is pre-computed by the SDK launcher (Phase 7.3). The engine
    receives it already stored. The engine does NOT re-hash/re-store the manifest.
- [ ] **2.8.4** Define `Stores { events: Arc<dyn EventStore>, artifacts: Arc<dyn ArtifactStore> }`
- [ ] **2.8.5** Define `ExecutionEngine` async trait:
  - `start(stores, run) -> Result<RunResult, RunError>`
  - `resume(stores, run_id) -> Result<RunResult, RunError>`
- [ ] **2.8.6** Tests: trait object creation

### 2.9 Re-export public API from `crates/machine/src/lib.rs`

- [ ] **2.9.1** Organize module re-exports matching Appendix C.1 structure:
  - `pub mod ids`, `pub mod canonical`, `pub mod config`, `pub mod meta`, `pub mod errors`
  - `pub mod context`, `pub mod events`, `pub mod io`, `pub mod recorder`
  - `pub mod state`, `pub mod plan`, `pub mod stores`, `pub mod engine`
- [ ] **2.9.2** Mark old modules (`state/mod.rs`, `state_machine/mod.rs`) as `#[deprecated]` or feature-gated
  - Keep them compilable for existing tests until Phase 12
- [ ] **2.9.3** Verify `cargo doc --no-deps -p mfm-machine` builds cleanly

---

## Phase 3 — Canonical JSON + SHA-256 Hashing

> Implementation of hashing infrastructure referenced by SS2.1, SS7.2.

### 3.1 Canonical JSON implementation

- [ ] **3.1.1** Implement `canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>, CanonicalError>`:
  - Sorted keys, minimal whitespace, proper number formatting
  - Reject `f64` values that are NaN, Infinity, or have fractional parts in hashed paths
- [ ] **3.1.2** Implement `canonical_json_string(value: &serde_json::Value) -> Result<String, CanonicalError>`
- [ ] **3.1.3** Add `CanonicalError` type (FloatDetected, NaN, Infinity, SerializationFailed)

### 3.2 SHA-256 hashing

- [ ] **3.2.1** Implement `sha256_hex(bytes: &[u8]) -> String` (64-char lowercase hex)
- [ ] **3.2.2** Implement `artifact_id_from_json(value: &serde_json::Value) -> Result<ArtifactId, CanonicalError>`
  - canonical JSON bytes -> SHA-256 -> ArtifactId
- [ ] **3.2.3** Implement `artifact_id_from_bytes(bytes: &[u8]) -> ArtifactId`
  - raw bytes -> SHA-256 -> ArtifactId
- [ ] **3.2.4** Tests:
  - Known test vectors (SHA-256 of known inputs)
  - Canonical JSON ordering stability
  - Float rejection
  - AT-04 + AT-05 coverage

---

## Phase 4 — Storage Backends (PostgreSQL + MinIO)

> Per user decision: go straight to PostgreSQL event store + MinIO/S3 artifact store.
> In-memory test doubles are provided for unit tests that don't need real backends.
> Both backends and test doubles MUST satisfy the same correctness contract.

### 4.1 PostgreSQL event store (`crates/storages/event-store/`)

- [ ] **4.1.1** Add dependencies: `mfm-machine`, `sqlx` (postgres, runtime-tokio), `tokio`, `async-trait`
- [ ] **4.1.2** Define schema:
  ```sql
  CREATE TABLE events (
      run_id UUID NOT NULL,
      seq BIGINT NOT NULL,
      ts_millis BIGINT,
      event_json JSONB NOT NULL,
      PRIMARY KEY (run_id, seq)
  );
  ```
- [ ] **4.1.3** Implement `PgEventStore` struct:
  - Holds `sqlx::PgPool`
  - `head_seq()`: `SELECT COALESCE(MAX(seq), 0) FROM events WHERE run_id = $1`
  - `append()`: INSERT within transaction, validate `expected_seq` matches current head
    - On mismatch: return `StorageError::Concurrency`
    - Validate contiguous seq numbers in the batch
  - `read_range()`: `SELECT ... WHERE run_id = $1 AND seq >= $2 [AND seq <= $3] ORDER BY seq`
- [ ] **4.1.4** Add `sqlx` migration support (embedded migrations)
- [ ] **4.1.5** Tests:
  - Append + read round-trip
  - Optimistic concurrency rejection
  - Empty run (head_seq = 0)
  - Range queries (from_seq, to_seq)
  - Atomic append (no partial reads — use concurrent reader during append)
  - AT-01, AT-02, AT-03 coverage

### 4.2 MinIO/S3 artifact store (`crates/storages/artifact-store/`)

- [ ] **4.2.1** Add dependencies: `mfm-machine`, `aws-sdk-s3`, `sha2`, `tokio`, `async-trait`
- [ ] **4.2.2** Implement `S3ArtifactStore` struct:
  - Holds S3 client + bucket name
  - Object key layout: `<kind>/<artifact_id_hex>` (e.g., `manifest/abcd1234...`, `snapshot/ef567890...`)
  - `put()`: hash bytes -> `ArtifactId`, upload to S3 with computed key, return `ArtifactId`
  - `get()`: download object, verify SHA-256 matches `ArtifactId` (return `Corruption` on mismatch)
  - `exists()`: HEAD object check
- [ ] **4.2.3** Handle content-addressing idempotency (PUT to same key is safe — same content)
- [ ] **4.2.4** Tests:
  - Put + get round-trip
  - Content-addressing (same bytes -> same ID)
  - Corruption detection (tampered object — mock or integration test)
  - AT-04 coverage

### 4.3 In-memory test doubles (for unit tests without backends)

- [ ] **4.3.1** Implement `InMemoryEventStore`:
  - `HashMap<RunId, Vec<EventEnvelope>>` + `RwLock`
  - Same `append()` contract as `PgEventStore` (validates `expected_seq`, contiguous seqs)
  - Same `head_seq()` / `read_range()` semantics
- [ ] **4.3.2** Implement `InMemoryArtifactStore`:
  - `HashMap<ArtifactId, Vec<u8>>` + `RwLock`
  - `put()`: hash bytes -> ArtifactId, insert
  - `get()`: lookup, verify hash
  - `exists()`: contains_key
- [ ] **4.3.3** Tests: same contract assertions as production backends (AT-01..AT-04)

### 4.4 Shared storage contract tests

- [ ] **4.4.1** Create a test harness module that takes `dyn EventStore` + `dyn ArtifactStore`:
  - Runs the full contract test suite against any backend
  - Tests: append-only, atomic append, optimistic concurrency, content-addressing, corruption detection
- [ ] **4.4.2** Wire in-memory, PostgreSQL, and S3/MinIO backends into the harness
  - PostgreSQL + MinIO tests gated behind `#[cfg(feature = "integration")]` or env var check
  - In-memory tests run unconditionally

---

## Phase 5 — Execution Engine (Core)

> The heart of the redesign: attempt envelopes, kernel events, snapshots-as-artifacts,
> crash/resume semantics. Per SS6, SS9.

### 5.1 Staged context

- [ ] **5.1.1** Implement snapshot deserialization helper:
  - `load_context_from_snapshot(artifacts: &dyn ArtifactStore, snapshot_id: &ArtifactId) -> Result<impl DynContext, RunError>`
  - Load bytes via `artifacts.get(snapshot_id)`
  - Deserialize canonical JSON into `BTreeMap<ContextKey, Value>`
  - Construct `InMemoryContext` from that map
- [ ] **5.1.2** Implement `StagedContext` wrapper:
  - Wraps a `DynContext` base (loaded from `base_snapshot_id` via 5.1.1)
  - Tracks writes in a staging layer (overlay on base)
  - Implements `DynContext` (reads check staging first, then base)
  - `commit(artifacts: &dyn ArtifactStore) -> Result<ArtifactId, RunError>`:
    merge staged writes into base -> `dump()` -> canonical JSON bytes -> store as artifact -> return `ArtifactId`
  - `discard()` -> drops staged writes, base unchanged
- [ ] **5.1.3** Secret exclusion: `commit()` MUST validate no secret patterns in snapshot
  - AT-09 coverage
- [ ] **5.1.4** Tests:
  - Stage writes, commit, verify snapshot artifact
  - Stage writes, discard, verify base unchanged
  - Deterministic snapshot hashing (same context -> same ArtifactId)
  - Load-from-snapshot round-trip (store -> load -> dump -> compare)
  - AT-12 coverage

### 5.2 Engine skeleton (sequential executor)

- [ ] **5.2.1** Implement `SequentialEngine` struct implementing `ExecutionEngine`:
  - Accepts `Stores` + `StartRun`
  - Topologically sorts `ExecutionPlan.graph`
  - Executes states sequentially
- [ ] **5.2.2** Implement `start()`:
  1. Verify manifest artifact exists in store (`artifacts.exists(&run.manifest_id)`)
     — the SDK launcher (Phase 7.3) is responsible for storing the manifest before calling the engine
  2. Store initial context snapshot -> `initial_snapshot_id`
  3. Append `RunStarted { op_id, manifest_id, initial_snapshot_id }` (seq=1)
  4. For each state in topological order:
     a. Append `StateEntered { state_id, attempt=1, base_snapshot_id }`
     b. Create `StagedContext` from base snapshot (via `load_context_from_snapshot`)
     c. Create IO provider (live or replay based on RunConfig)
     d. Create EventRecorder
     e. Call `state.handle(ctx, io, rec)`
     f. On success: commit staged context -> `context_snapshot_id`, append `StateCompleted`
     g. On failure: discard staged context, append `StateFailed`
  5. Append `RunCompleted { status, final_snapshot_id }`
- [ ] **5.2.3** Implement seq management:
  - Engine tracks current seq number
  - Each append increments seq
  - Seq is 1-indexed (SS6.1 note)
  - `append()` returns `new_head_seq` — engine uses this to confirm advancement
- [ ] **5.2.4** Tests:
  - Simple 2-state workflow (start -> complete)
  - State failure (start -> fail)
  - Event stream correctness (all kernel events present)
  - AT-01 coverage

### 5.3 Retry logic

- [ ] **5.3.1** Implement retry policy from `RunConfig`:
  - On retryable `StateError`, increment attempt, re-enter state with same `base_snapshot_id`
  - Respect `max_attempts`
  - Apply backoff (Fixed or Exponential) — sleep in live mode, skip sleep in replay mode
- [ ] **5.3.2** On non-retryable error: emit `StateFailed`, proceed to `RunCompleted { Failed }`
- [ ] **5.3.3** Tests: retry 3 times then succeed, retry exhausted then fail

### 5.4 Crash/resume (orphan handling)

- [ ] **5.4.1** Implement `resume()`:
  1. Read event stream from store
  2. Scan for last kernel event
  3. If last event is `StateEntered` (orphan): retry from `base_snapshot_id`
  4. If last event is `StateCompleted`: continue to next state
  5. If last event is `RunCompleted`: return result (already done)
  6. If last event is `StateFailed`: check retry policy, retry or complete as failed
- [ ] **5.4.2** Orphan domain events: remain in stream, do not advance progression
- [ ] **5.4.3** Fact deduplication on resume: build fact index from existing events
  - Scan ALL events (including those from orphaned/in-flight attempts) for `FactRecorded` domain events
  - Facts from orphaned attempts are still valid for dedupe and replay (per SS7.3 and SS6.2)
  - Populate `HashMap<FactKey, ArtifactId>` for use by IO providers
- [ ] **5.4.4** Tests:
  - Simulate crash after `StateEntered` (no terminal) -> resume -> completes
  - Simulate crash after `StateCompleted` -> resume -> continues next state
  - Facts from orphaned attempt available on retry
  - AT-07 coverage

### 5.5 Skip-tags semantics

- [ ] **5.5.1** Implement `RunConfig.skip_tags`:
  - If a state's meta tags intersect with `skip_tags`, skip it (emit `StateCompleted` with unchanged snapshot)
- [ ] **5.5.2** Tests: skip side-effect states for dry runs

### 5.6 Event profile gating

- [ ] **5.6.1** `EventRecorder` implementation checks `RunConfig.event_profile`:
  - `Minimal`: suppress all domain events
  - `Normal`: allow facts + artifacts + boundaries
  - `Verbose`: allow all
- [ ] **5.6.2** Kernel events always emitted regardless of profile
- [ ] **5.6.3** Tests: profile filtering

---

## Phase 6 — IO Providers (Live + Replay)

> Per SS8: IoProvider implementations.

### 6.1 LiveIo implementation

- [ ] **6.1.1** Implement `LiveIo` struct:
  - Holds: `Arc<dyn ArtifactStore>` (for storing fact payloads)
  - Holds: a fact-event buffer (`Vec<DomainEvent>`) that the engine drains after `handle()` returns
  - Maintains fact index (`HashMap<FactKey, ArtifactId>`) for single-assignment enforcement
  - **Ownership note**: `LiveIo` does NOT hold an `EventRecorder` directly. Instead, the engine
    creates `LiveIo` per state attempt. After `handle()` returns, the engine collects any buffered
    fact events from `LiveIo` and appends them via its own `EventRecorder`. This avoids the
    double-mutable-borrow problem (handler receives both `&mut io` and `&mut rec`).
  - Provide `fn drain_fact_events(&mut self) -> Vec<DomainEvent>` for the engine to call post-handle.
- [ ] **6.1.2** `call()` implementation:
  - If `fact_key` present and already recorded: return stored payload (no transport)
  - Else: delegate to transport, store payload as artifact, buffer `FactRecorded` event, cache in index
  - If `fact_key` absent: perform IO, do not record (non-replayable)
- [ ] **6.1.3** `now_millis()` implementation:
  - Generate attempt-scoped fact key: `(run_id, state_id, call_ordinal, "time")`
  - **Note**: `call_ordinal` is per-state-attempt (resets to 0 on each new attempt of the same state).
    This is critical for crash/resume: if attempt 1 crashes, attempt 2 starts with `call_ordinal=0`
    and the same keys. Since facts are single-assignment, attempt 2 reuses attempt 1's recorded values.
  - Record as fact payload
  - AT-14 coverage
- [ ] **6.1.4** `random_bytes()` implementation:
  - Same fact-key pattern as `now_millis()` (with kind `"random"`)
  - Record as fact payload
  - AT-14 coverage
- [ ] **6.1.5** `get_recorded_fact()`: lookup in fact index
- [ ] **6.1.6** FactKey single-assignment enforcement:
  - If `fact_key` already in index, return existing (do not overwrite)
  - AT-10 coverage
- [ ] **6.1.7** Tests:
  - Fact recording + retrieval
  - Single-assignment (second call returns same payload)
  - Time/random recording
  - Non-replayable call (no fact_key)
  - `drain_fact_events()` returns buffered events

### 6.2 ReplayIo implementation

- [ ] **6.2.1** Implement `ReplayIo` struct:
  - Holds references to `ArtifactStore`
  - Preloaded fact index from event stream (built during resume or from live run's events)
- [ ] **6.2.2** `call()` implementation:
  - If `fact_key` absent: return `IoError::MissingFactKey` (non-retryable)
  - If `fact_key` present but not in index: return `IoError::MissingFact`
    - Retryability from `RunConfig.replay_missing_fact_retryable`
  - If `fact_key` present and recorded: load artifact, return as `IoResult`
- [ ] **6.2.3** `now_millis()`: lookup recorded time fact or return `MissingFact`
- [ ] **6.2.4** `random_bytes()`: lookup recorded random fact or return `MissingFact`
- [ ] **6.2.5** `get_recorded_fact()`: lookup in preloaded index
- [ ] **6.2.6** Tests:
  - Replay with all facts present -> success
  - Replay missing fact_key -> MissingFactKey error
  - Replay missing fact -> MissingFact error
  - Retryability flag respected
  - AT-13 coverage

### 6.3 End-to-end live-then-replay test

- [ ] **6.3.1** Run a multi-state workflow in live mode (recording facts)
- [ ] **6.3.2** Re-run same workflow in replay mode (no external IO)
- [ ] **6.3.3** Assert: same output artifact IDs / context snapshots
- [ ] **6.3.4** AT-06 coverage

---

## Phase 7 — SDK (`mfm-sdk`)

> Per SS5.3 and Appendix A: Operation trait, Pipeline, run launcher.

### 7.1 Operation trait

- [ ] **7.1.1** Define `Operation` trait in `crates/sdk/`:
  - `fn op_id(&self) -> OpId`
  - `fn op_version(&self) -> &str`
  - `fn expand(&self, op_config: &serde_json::Value, run_config: &RunConfig) -> Result<StateGraph, PlanError>`
- [ ] **7.1.2** Define `PlanError` type

### 7.2 Pipeline builder

- [ ] **7.2.1** Implement `Pipeline` builder:
  - `Pipeline::new(machine_id: &str)`
  - `.then(step_id: &str, op: Box<dyn Operation>) -> Pipeline`
  - `.build(run_config: &RunConfig) -> Result<ExecutionPlan, PlanError>`
- [ ] **7.2.2** Flattening: merge state graphs from all steps with proper namespacing
  - State IDs: `<machine_id>.<step_id>.<state_local_id>`
  - Inter-step edges based on step ordering
- [ ] **7.2.3** Single-op convenience: wrap as 1-step machine with `step_id = "main"` (SS10.7)
- [ ] **7.2.4** Tests: pipeline build, flattening, ID shape validation

### 7.3 Run launcher / resume helpers

- [ ] **7.3.1** Implement `RunLauncher`:
  - `fn start(engine, stores, manifest, plan, initial_context) -> Result<RunResult, RunError>`
    - Hashes manifest -> `manifest_id`, stores as artifact via `artifacts.put()`
    - Constructs `StartRun { manifest, manifest_id, plan, run_config, initial_context }`
    - Delegates to `engine.start()`
  - `fn resume(engine, stores, run_id) -> Result<RunResult, RunError>`
    - Delegates to `engine.resume()`
- [ ] **7.3.2** Build provenance helper:
  - Capture `git_commit`, `cargo_lock_hash`, `rustc_version`, `target_triple` at build/run time
- [ ] **7.3.3** Tests: launcher start + resume smoke test

### 7.4 Op registry (optional, simple)

- [ ] **7.4.1** Implement simple `OpRegistry`:
  - `register(op: Box<dyn Operation>)`
  - `get(op_id: &OpId) -> Option<&dyn Operation>`
- [ ] **7.4.2** Tests: register + lookup

---

## Phase 8 — Proof Operation (End-to-End Validation)

> A concrete operation that exercises the full stack. Per REDESIGN_PLAN.md PR15 requirements.

### 8.1 Create proof op crate

- [ ] **8.1.1** Create `crates/ops/proof-op/` — package `mfm-proof-op`
- [ ] **8.1.2** Dependencies: `mfm-machine`, `mfm-sdk`

### 8.2 Proof op states (minimum set to exercise all invariants)

- [ ] **8.2.1** `FetchState` (READ_ONLY_IO):
  - Makes an IO call with `fact_key`
  - Records current time via `io.now_millis()`
  - Writes derived data to context
- [ ] **8.2.2** `ValidateState` (PURE):
  - Reads context, validates invariants
  - No IO
- [ ] **8.2.3** `ComputeState` (PURE):
  - Reads context, computes derived values
  - Uses `io.random_bytes()` for salt/nonce generation (recorded as fact)
- [ ] **8.2.4** `ExecuteState` (APPLY_SIDE_EFFECT):
  - Declares idempotency key
  - "Submits" a side effect (mock: writes an artifact)
  - AT-08 coverage
- [ ] **8.2.5** `ReportState` (PURE):
  - Produces output artifact from context

### 8.3 Proof op definition

- [ ] **8.3.1** Implement `Operation` for `ProofOp`:
  - `expand()` returns a `StateGraph` with the 5 states + edges
- [ ] **8.3.2** Test: expansion produces valid plan (PlanValidator passes)

### 8.4 End-to-end acceptance tests

- [ ] **8.4.1** **AT-06: Live-then-replay determinism**
  - Run proof op in live mode
  - Re-run in replay mode
  - Assert same final snapshot + output artifact IDs
- [ ] **8.4.2** **AT-07: Crash/resume determinism**
  - Run with failpoint after N events
  - Resume
  - Assert run completes with same result
- [ ] **8.4.3** **AT-08: Side-effect idempotency**
  - Crash during ExecuteState
  - Resume
  - Assert side effect not re-applied (idempotency key check)
- [ ] **8.4.4** **AT-09: Secrets never persisted**
  - Scan all events + artifacts for secret patterns
  - Assert none found
- [ ] **8.4.5** **AT-10: FactKey single-assignment**
  - First fact recorded for key X
  - Second attempt for same key returns same payload
- [ ] **8.4.6** **AT-14: Time/random as facts**
  - Replay consumes recorded time/random values
  - Assert deterministic output

---

## Phase 9 — CLI Expansion

> Per SS13.1: CLI starts/resumes runs, queries history, stable outputs.
> Existing keystore subcommands are preserved.

### 9.1 Preserve existing keystore commands

- [ ] **9.1.1** Verify existing `keystore import`, `keystore list`, `keystore delete` commands
  work after workspace restructure (should already work from Phase 0, but verify here)
- [ ] **9.1.2** Maintain existing `--output-format text|json` support for keystore commands

### 9.2 Run subcommand

- [ ] **9.2.1** Add `run start` subcommand:
  - `--op <op_id>` — operation to run
  - `--config <path>` — op config (JSON/YAML)
  - `--io-mode <live|replay>` — IO mode
  - `--event-profile <minimal|normal|verbose>` — event verbosity
  - `--db-url <postgres_url>` — PostgreSQL connection string (default from env `MFM_DATABASE_URL`)
  - `--s3-endpoint <url>` — MinIO/S3 endpoint (default from env `MFM_S3_ENDPOINT`)
  - `--s3-bucket <name>` — artifact bucket (default `mfm-artifacts`)
  - Outputs: `run_id`, status, final_snapshot_id
- [ ] **9.2.2** Add `run resume` subcommand:
  - `--run-id <uuid>` — run to resume
  - `--db-url`, `--s3-endpoint`, `--s3-bucket` (same as start)
- [ ] **9.2.3** Add `run inspect` subcommand:
  - `--run-id <uuid>`
  - `--events` — list events
  - `--artifact <id>` — fetch artifact
  - `--manifest` — show manifest
- [ ] **9.2.4** Wire `--output-format text|json` for all new commands

### 9.3 Integration with storage + engine

- [ ] **9.3.1** CLI creates storage backends (PostgreSQL event store + S3 artifact store) from connection params
- [ ] **9.3.2** CLI creates engine, builds plan via SDK, launches run
- [ ] **9.3.3** CLI handles resume by loading existing run state from event store

### 9.4 Tests

- [ ] **9.4.1** CLI start -> complete -> inspect flow
- [ ] **9.4.2** CLI resume after crash
- [ ] **9.4.3** JSON output format stability

---

## Phase 10 — Proc Macro Updates (`mfm-machine-derive`)

> Update proc macros to work with the new `State` trait.

### 10.1 Upgrade `syn` dependency

- [ ] **10.1.1** Upgrade from `syn = "1.0"` to `syn = "2.x"`
  - Since we are rewriting the proc macros for the new `State` trait anyway,
    use the current `syn` version for better error messages and API.

### 10.2 New `#[state]` macro

- [ ] **10.2.1** Create attribute macro for new `State` trait:
  - Generates `StateMeta` from attributes
  - Generates `meta()` method
  - User implements `handle()` manually
- [ ] **10.2.2** Support attributes: `tags`, `depends_on`, `side_effects`, `idempotency`
- [ ] **10.2.3** Deprecate old `#[state_handler]` macro (keep compilable until Phase 12)

### 10.3 Tests

- [ ] **10.3.1** Verify new macro generates correct StateMeta
- [ ] **10.3.2** Verify backward compat (old macro still works until Phase 12)

---

## Phase 11 — Collectors (Stubs / First Implementations)

> Per SS5.2: collectors fetch/normalize data, usable under live or replay IO.

### 11.1 HTTP collector pattern

- [ ] **11.1.1** Define collector adapter pattern:
  - Collector wraps `IoProvider`
  - Provides typed methods (e.g., `fetch_json(url, fact_key)`)
  - Internally constructs `IoCall` with proper namespace
- [ ] **11.1.2** Implement generic `HttpCollector` in a shared location

### 11.2 CoinGecko collector (`crates/collectors/coingecko/`)

- [ ] **11.2.1** Implement price fetching with fact recording
- [ ] **11.2.2** Tests: live + replay round-trip

### 11.3 EVM collector (`crates/collectors/evm/`)

- [ ] **11.3.1** Implement basic RPC call wrapper with fact recording
  - `eth_getBalance`, `eth_call`, `eth_blockNumber` etc.
- [ ] **11.3.2** Add `alloy-provider` dependency
- [ ] **11.3.3** Tests: mock RPC + replay round-trip

---

## Phase 12 — Legacy Cleanup

> Remove old abstractions after all consumers are migrated.

### 12.1 Remove deprecated modules

- [ ] **12.1.1** Remove old `state::StateHandler` trait
- [ ] **12.1.2** Remove old `state::StateMetadata` trait
- [ ] **12.1.3** Remove old `state::StateError` (6-variant)
- [ ] **12.1.4** Remove old `state::context::Context` trait and `Local` impl
- [ ] **12.1.5** Remove old `state::safe_context::SafeContext`
- [ ] **12.1.6** Remove old `state_machine::StateMachine` + `StateMachineBuilder`
- [ ] **12.1.7** Remove old `state_machine::scheduler` module
- [ ] **12.1.8** Remove old `state_machine::tracker` module
- [ ] **12.1.9** Remove `mfm_core` re-export of old `SafeContext`
  - Any remaining `mfm-core` consumers of `SafeContext` must be migrated to `DynContext` first

### 12.2 Remove deprecated proc macros

- [ ] **12.2.1** Remove `#[state_handler]` attribute macro
- [ ] **12.2.2** Remove `#[derive(StateMetadataReqs)]` derive macro
- [ ] **12.2.3** Replace with new equivalents or remove if superseded

### 12.3 Update all tests

- [ ] **12.3.1** Migrate `mfm_machine` integration tests to new API
- [ ] **12.3.2** Verify `cargo test --workspace` passes

### 12.4 Final verification

- [ ] **12.4.1** All AT-01..AT-14 passing
- [ ] **12.4.2** `cargo clippy --workspace` clean
- [ ] **12.4.3** `cargo doc --workspace --no-deps` clean
- [ ] **12.4.4** Update `CURRENT_STATE.md` to reflect final Milestone 1 state

---

## Dependency Graph (Phase-Level Critical Path)

```
Phase 0 (workspace restructure + Nix devshell with Pg/MinIO)
    |
    v
Phase 1 (foundational types)
    |
    v
Phase 2 (runtime abstractions) -----> Phase 10 (proc macro update)
    |                                       |
    v                                       v
Phase 3 (canonical JSON + hashing)    Phase 11 (collectors)
    |                                       |
    v                                       |
Phase 4 (PostgreSQL + MinIO backends)       |
    |                                       |
    v                                       |
Phase 5 (execution engine) <----------------+
    |
    v
Phase 6 (IO providers)
    |
    v
Phase 7 (SDK)
    |
    v
Phase 8 (proof op) -------> Phase 9 (CLI expansion)
    |                              |
    v                              v
Phase 12 (legacy cleanup)
```

Tasks within a phase may run in parallel where no intra-phase dependencies exist.

---

## Deviations (Plan vs Contract)

| # | SS Ref | Nature | Resolution |
|---|--------|--------|------------|
| D-01 | SS7.1 vs Appendix C.1 | SS7.1 lists `config_refs`, `env_allowlist` with captured values, and standalone `io_mode` in the manifest. Appendix C.1 `RunManifest` struct omits these (keeps `io_mode` inside `run_config`). | Follow Appendix C.1 for M1. `config_refs` and captured env deferred to post-M1. If needed, update Appendix C.1 in a future contract revision. |
| D-02 | SS7.1 | SS7.1 mentions "env_allowlist + captured env values". Appendix C.1 `BuildProvenance` has `env_allowlist: Vec<String>` but no `captured_env: HashMap<String, String>`. | Appendix C.1 is authoritative for M1. Captured env values deferred. |
| D-03 | SS12.3 | SS12.3 recommends fast-lane stores (in-memory/sqlite + filesystem) for unit tests, with Pg + MinIO as integration lane. Per user decision, Pg + MinIO are the primary backends from the start. | In-memory test doubles still exist for isolated unit tests. Pg + MinIO are available in devshell via Nix. The "two-lane" split is simplified: unit tests use in-memory doubles; all other tests use Pg + MinIO. |

If any further deviation is discovered during implementation, it MUST be recorded here with:
- Section reference (SS#)
- Nature of deviation
- Resolution (contract update or plan adjustment)

---

## Risk Register

| Risk | Mitigation |
|------|-----------|
| Phase 0 rename breaks imports/CI | Do as one atomic commit; verify `cargo test --workspace` before merge |
| New `State` trait incompatible with existing tests | Keep old API behind `#[deprecated]` until Phase 12; dual-path during migration |
| Canonical JSON edge cases (number formatting) | Use RFC 8785 test vectors; add fuzz tests |
| Nix devshell PostgreSQL/MinIO setup complexity | Use `process-compose` or `devenv` for service management; provide fallback env vars for manual setup |
| PostgreSQL performance for large event streams | Milestone 1 is correctness-focused; add indexes and optimize later |
| Fact index rebuild on resume is O(n) in event count | Acceptable for Milestone 1; index/cache if needed |
| Proc macro changes break derive usage | Version proc macros independently; deprecate, don't remove, until Phase 12 |
| `aws-sdk-s3` crate adds substantial compile time | Acceptable trade-off; consider `rust-s3` as lighter alternative if needed |
| `DynContext: Send` (not `Sync`) breaks existing `SafeContext` consumers | Old API is preserved during migration; new API uses exclusive `&mut` access pattern |

---

## Decision Log

| # | Decision | Rationale | REDESIGN_FINAL.md |
|---|----------|-----------|-------------------|
| D1 | Phase 0 restructure before any new code | Clean foundation; avoid path confusion | SS4 |
| D2 | Keep old API during migration (deprecated) | Incremental migration; tests keep passing | SS0 (guiding principles) |
| D3 | PostgreSQL + MinIO as primary backends from the start | User decision: Nix-first approach makes production-like deps trivially available; avoids building throwaway fast-lane stores | SS12.3 (deviation D-03) |
| D4 | Proof op before CLI expansion | Validates engine correctness before UX work | SS15 |
| D5 | Sequential engine only for Milestone 1 | Simplicity; parallelism deferred | SS2.5, SS9.4 |
| D6 | `sha2` crate for hashing (not `ring`) | Pure Rust, widely used, no C deps | SS7.2 |
| D7 | Keep existing keystore CLI commands in new CLI | User decision: preserve working functionality during redesign | SS13.1 |
| D8 | Binary renamed from `mfm_cli` to `mfm` | User-facing ergonomics; package was already named `mfm` | SS4.1 |
| D9 | `BackoffPolicy` uses `u64` millis not `Duration` | `Duration` lacks `Serialize`/`Deserialize`; millis are canonical-JSON friendly | Appendix C.1 |
| D10 | `DynContext: Send` (drops `Sync` vs old `Context: Send + Sync`) | Engine uses `&mut dyn DynContext`; exclusive access eliminates need for interior mutability | Appendix C.1 |
| D11 | `syn` 2.x for proc macros | Modern API, better error messages; old macros being rewritten anyway | — |
| D12 | In-memory test doubles kept alongside production backends | Needed for fast unit tests that don't require database; shared contract tests ensure parity | SS12.3 |
