# MFM Design Contract

> Last updated: 2026-02-14
> Status: Authoritative design contract.
>
> This document defines non-negotiable architecture and execution semantics.
> If code disagrees with this document, the code is wrong until this contract is intentionally changed.

## 1. Why This Document Exists

MFM is designed for reproducible, auditable workflow execution with strict security boundaries.

This contract exists to lock in:
- execution correctness (resume/replay semantics)
- boundary correctness (where logic belongs)
- persistence correctness (append-only and content-addressed rules)
- secret-handling correctness (no secret leakage in persisted surfaces)

Contributor read order:
1. [`docs/architecture.md`](architecture.md) for boundaries and placement.
2. This contract for normative behavior.
3. `AGENTS.md` for contribution and CI rules.

## 2. Guiding Priorities

1. Correctness and security over convenience.
2. Reproducibility and auditability as architectural properties.
3. Small, reviewable changes over broad rewrites.
4. Stable crate boundaries so libraries remain reusable outside binaries.
5. Thin binaries: transport adapters only.
6. Three-tier thin-layer design:
   - executable logic lives in reusable states
   - ops assemble state graphs
   - binaries stay transport-only

## 3. Core Concepts

### 3.1 Operation (Op)
Reusable planning definition (`impl Operation`) with:
- input schema
- run configuration schema
- deterministic expansion into a state graph
- output schema

Normative constraints:
- `expand()` MUST be deterministic for equivalent `(op_config, run_config)`.
- `expand()` MUST NOT perform ambient IO (`fs/network/time/env/process`).
- operation code MUST NOT execute workflow side effects.
- operation code SHOULD stay focused on config validation + state graph wiring.

### 3.2 Run
Concrete execution of an op or flattened pipeline:
- run ID
- content-addressed manifest
- append-only event stream
- content-addressed artifacts

### 3.3 State
Execution step (`impl State`) that:
- reads/writes context
- performs side effects through IO provider abstractions
- emits domain events via recorder

Normative constraints:
- state handlers MUST execute runtime behavior through explicit dependencies (`DynContext`, `IoProvider`, `EventRecorder`).
- side effects MUST route through `IoProvider` only (no ambient IO).
- reusable executable behavior SHOULD live in shared-state crates.
- op-local states are allowed only for domain-specific output/aggregation behavior that is not a shared primitive.

### 3.4 Context
Deterministically serialized working state:
- typed read/write support
- full snapshot persistence
- no secret payloads

### 3.5 Event
Immutable run record in append-only stream:
- kernel events are mandatory for engine correctness
- domain events are op/runtime-level audit signals

### 3.6 Artifact
Immutable content-addressed blob/document:
- manifests
- context snapshots
- fact payloads
- outputs

### 3.7 Fact
External or non-deterministic input captured for replay:
- RPC/HTTP payloads
- time/random values
- external process results

## 4. Non-Negotiable Invariants

### 4.1 Append-only stream
- Runs append events; historical events are never mutated.

### 4.2 Per-append atomicity
- `EventStore::append([...])` is all-or-nothing.
- Partially visible append results are forbidden.

### 4.3 Attempt envelope semantics
A state attempt is bounded by kernel events:
- `StateEntered { ... }`
- zero or more domain events
- exactly one terminal kernel event:
  - `StateCompleted { ... }` or
  - `StateFailed { ... }`

A state attempt may span multiple appends.

### 4.4 Checkpoint advancement rule
- Only `StateCompleted { context_snapshot_id }` advances effective resume checkpoint.
- `StateFailed` never advances checkpoint.
- `failure_snapshot_id` is diagnostic only.

### 4.5 Content addressing
Manifests, snapshots, facts, and outputs are immutable artifacts addressed by digest.

### 4.6 Canonical hashing format
- Structured data participating in hashing uses canonical JSON semantics (RFC 8785/JCS-style target).
- NaN/Infinity are invalid.
- Floats (fractional JSON numbers) in hashed structures are forbidden.

### 4.7 No ambient IO in state logic
- Handlers use the IO provider abstraction (`LiveIo` / `ReplayIo`).
- Hidden network/filesystem side effects in state logic are contract violations.

### 4.8 No secrets in persisted or contract surfaces
Secrets must never appear in:
- manifests
- events
- artifacts (including fact payloads and context snapshots)
- CLI/API outputs
- error details

### 4.9 Thin transport boundaries
`bin/cli` and `bin/rest-api`:
- parse requests
- start/resume/query runs
- render outputs

They do not own domain execution logic.

## 5. Resolved Design Decisions

1. Canonical JSON is the hashing standard for structured data.
2. Context snapshots are full snapshots (not deltas).
3. Runtime emits mandatory kernel events; domain event sets are configurable.
4. Replay returns structured IO errors for missing replay data; no hard process crash.
5. Execution mode is sequential; fan-out/join is deferred.
6. Cargo package names remain namespaced (`mfm-*`).
7. Persisted secrets are forbidden absolutely.

## 6. Architecture and Boundary Rules

### 6.1 Workspace structure
Canonical responsibilities:
- `crates/machine/`: runtime model + executor + kernel contracts
- `crates/machine-derive/`: proc-macro ergonomics
- `crates/core/`: primitives + security-sensitive keystore/crypto
- `crates/collectors/*`: external data adapters
- `crates/transports/*`: local/internal live transport factories
- `crates/storages/*`: persistence implementations
- shared-state layer crates:
  - `crates/states/common/`: cross-domain reusable state primitives
  - `crates/states/keystore/`: keystore-domain reusable states/helpers
  - `crates/states/aave-v3/`: Aave-domain reusable states/helpers
  - `crates/evm-runtime/`: runtime-facing reusable EVM states/helpers
- `crates/ops/*-op`: domain operation planners that compose state graphs
- `crates/sdk/`: operation/pipeline orchestration glue
- `bin/cli`, `bin/rest-api`: thin transport adapters

Three-tier rule (normative):
- Tier 1 (`bin/*`) MUST remain transport-only.
- Tier 2 (`crates/ops/*-op`) SHOULD remain thin and primarily perform config validation + graph wiring.
- Tier 3 (shared-state crates) SHOULD contain reusable executable workflow logic.
- Approved Tier 3 roots include:
  - `crates/states/common/src/states/*`
  - `crates/states/keystore/src/states/*`
  - `crates/states/aave-v3/src/*`
  - `crates/evm-runtime/src/states/*`
- Non-reusable domain-specific output/aggregation states MAY remain op-local.

Migration policy for internal crate/module paths:
- Breaking cutovers are allowed.
- Compatibility re-exports and deprecated internal aliases are not required.
- All internal consumers MUST be updated in the same change.

### 6.2 Dependency contract
- Binaries depend on sdk/ops and render outputs.
- Ops compose machine + collectors + storages + core primitives.
- Machine remains generic (no chain-specific business logic).
- Storages do persistence only, no workflow behavior.
- Core must not depend on collectors/storages/ops/sdk.

### 6.3 Machine vs SDK ownership
`crates/machine/` owns correctness-critical runtime semantics:
- state IDs, state graph, plan representation
- kernel event model
- executor start/resume behavior

`crates/sdk/` owns orchestration ergonomics:
- operation registry
- pipeline planner helpers
- run launch/resume glue over machine + stores

### 6.4 Live IO adapter model
Canonical layering:
- Layer 1: domain adapter clients wrap `IoProvider` and provide typed domain requests/responses.
- Layer 2: live transport factories execute concrete IO and are selected by namespace routing.

Normative rules:
- States MUST depend on `IoProvider` (or Layer 1 typed adapters over `IoProvider`), not on live transport factories.
- Live transport factories MUST expose a non-empty `namespace_group()` and registrations MUST reject duplicates.
- App/runtime transport assembly MUST use `TransportRegistry` + `RouterLiveIoTransportFactory`.
- Router dispatch MUST resolve by hierarchical longest-prefix group match (`g` or `g.*`).
- Computed deterministic values that need replay/fact durability SHOULD use `IoProvider::record_value(...)` rather than passthrough no-op transports.
- Transport factories SHOULD own their config parsing (`from_env()` or equivalent), while app wiring stays assembly-only.

## 7. Execution Model

### 7.1 Required kernel events
Runtime must emit:
- `RunStarted { op_id, manifest_id, initial_snapshot_id }`
- `StateEntered { state_id, attempt, base_snapshot_id }`
- `StateCompleted { state_id, context_snapshot_id }`
- `StateFailed { state_id, error, failure_snapshot_id? }`
- `RunCompleted { status, final_snapshot_id? }`

### 7.2 Domain event rules
Domain events are optional for planner correctness, but must obey:
- no secrets
- canonical JSON compatible payloads
- large payloads referenced by artifact ID

Runtime-critical rule:
- When runtime records replayable facts, the `FactKey -> payload_id` mapping must be durably emitted as `fact_recorded` regardless of configured domain event profile.

### 7.3 Seq and attempts
- `seq` is strictly increasing per run.
- Convention: 1-indexed sequence, empty run head is 0.
- `attempt` increments on retry for a state.

### 7.4 Crash/resume semantics
- Trailing `StateEntered` without terminal kernel event is treated as in-flight/orphaned attempt.
- Resume retries from `base_snapshot_id`.
- Facts recorded during orphaned attempts remain valid for replay/dedupe.

### 7.5 Checkpointing
Checkpointing policy:
- checkpoint after every successful state
- snapshot IDs may dedupe to same digest when context is unchanged

## 8. Reproducibility Contract

### 8.1 Run manifest
Each run has a content-addressed manifest referenced from `RunStarted`.

Recommended fields:
- `op_id`, `op_version`
- build provenance (`git_commit`, lock hashes, toolchain)
- canonical `input_params` (non-secret)
- config references
- env allowlist + captured values
- run configuration and IO mode

### 8.2 Hashing contract
- Structured payloads: canonical JSON bytes -> hash -> artifact ID.
- Binary payloads: raw bytes -> hash -> artifact ID.
- Hash algorithm: SHA-256.

### 8.3 Facts as single-assignment
Within a run:
- first durable `FactRecorded { key, payload_id }` binds the key
- key binding is immutable for replay consistency

## 9. IO and Replay Determinism

### 9.1 IO provider model
Handlers do not perform ambient IO directly.

- `LiveIo`: real IO + optional fact recording
- `ReplayIo`: fact-backed replay, returns structured errors when data is unavailable

### 9.2 Fact-key policy
- Live mode: replayable IO should provide `fact_key`.
- Replay mode: deterministic IO must provide `fact_key`.
- Missing `fact_key` in replay returns stable `missing_fact_key` error.

### 9.3 Missing facts
If replay cannot resolve a requested key:
- return `MissingFact { key, ... }`
- retryability controlled by `RunConfig.replay_missing_fact_retryable` (default false)

### 9.4 Time and randomness
`now_millis()` and `random_bytes()` are facts when used in deterministic logic.

Recording rule:
- `LiveIo` records values under deterministic attempt-scoped keys derived from:
  - `run_id`, `state_id`, `attempt`, `call_ordinal`, `kind`
- `ReplayIo` returns recorded values or `MissingFact`

### 9.5 External program execution (`nix.exec` / `exec`)
Deterministic external execution is supported through IO provider calls only.

Preflight for app refs must:
1. validate flake allowlist
2. resolve app program path
3. enforce `/nix/store/` path constraint
4. build/realize executable
5. return safe execution descriptor

Security constraints:
- do not persist raw stdout/stderr in error details
- enforce no-secrets policy for persisted payloads

## 10. Runtime Requirements

Runtime must support:
- async handlers
- crash-resume and deterministic replay
- explicit side-effect typing/idempotency
- stable metadata and plan validation

Execution mode contract:
- `Sequential` supported
- `FanOutJoin` requested mode must return structured `unsupported_execution_mode` error

## 11. Operations, Pipelines, and IDs

### 11.1 Ops expand to state graphs
`expand()` takes op config + run config and returns deterministic graph output.
`expand()` is a planning-only phase and MUST NOT execute runtime side effects.

### 11.2 Flattened composition
If op A has N states and op B has M states, pipeline graph size is `N + M` states in one run.

### 11.3 ID stability rules
ID conventions:
- `OpPath = <machine_id>.<step_id>`
- `StateId = <machine_id>.<step_id>.<state_local_id>`
- each segment must match: `^[a-z][a-z0-9_]{0,62}$`
- dots are separators only

Single-op run convention:
- wrap as one-step machine:
  - `machine_id = <op_id>`
  - `step_id = main`

### 11.4 Namespaced context and explicit wiring
- context keys are namespaced by op path by default
- cross-op data flow uses explicit imports/exports validated by planner

### 11.5 Nested runs
Engine-managed child runs are deferred.
When introduced, linkage events are required (`ChildRunSpawned`).

## 12. Storage Contract

### 12.1 Two mandatory roles
1. Event store (append-only, optimistic concurrency)
2. Artifact store (immutable, content-addressed)

Optional later role:
- projection/index store

### 12.2 Backend strategy
- PostgreSQL as primary transactional event store
- MinIO/S3 as primary artifact storage
- local fast implementations retained for unit and dev loops

### 12.3 Security requirements
- no persisted secrets in any store-backed surface
- events may store references, never secret plaintext
- encrypted secret-bearing artifacts are deferred to a future encrypted-artifact layer

## 13. CLI and REST Responsibilities

### 13.1 CLI
- run start/resume/query
- stable text/json output
- no business logic execution ownership

### 13.2 REST API
- same functional boundary over HTTP
- start/resume/status/events/artifact retrieval surfaces
- no workflow logic in handlers

## 14. Security and Secret Handling

Keystore paths remain high-risk and require strict handling:
- constant-time comparisons where required
- tamper detection
- DoS guards
- zeroization
- strict input parsing

Hard rule remains absolute:
- secrets are never persisted in manifests/events/artifacts/outputs/errors

## 15. Testing Contract

### 15.1 For ops
- replay determinism tests (live capture -> replay)
- crash/resume recovery tests
- output contract stability tests

### 15.2 For runtime/storage
- optimistic concurrency checks
- snapshot integrity and hashing tests
- idempotency behavior tests
- resume boundary and orphan-attempt semantics tests

## 16. Migration Roadmap

1. stabilize event/artifact store traits and local implementations
2. stabilize run manifest hashing and storage
3. guarantee kernel event emission for all runs
4. enforce async state handler + IO provider model
5. harden replay missing-fact semantics
6. adopt pipeline flattening as default composition
7. keep binaries strictly start/resume/report adapters
8. maintain parity lanes (local-fast and integration-parity)

## 17. Open Questions (Deferred)

1. Standard reusable state pattern library breadth.
2. Op schema versioning and compatibility policies.
3. Formal event profile schema standardization.
4. Typed schema registry rollout timing.

## Appendix A - Planning API Sketch

Execution engine runs states, not ops.

Core flow:
- `Operation::expand(...) -> StateGraph`
- `Pipeline::then(...)`
- `Pipeline::build(...) -> ExecutionPlan`
- `ExecutionPlan` is the runtime input.

## Appendix B - Event Profile Baseline

Recommended profiles:
- `minimal`: kernel + determinism-critical domain events
- `normal`: `minimal` + common audit domain events
- `verbose`: `normal` + diagnostic domain events

Kernel events are always emitted.

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

    /// Enforced: "<machine_id>.<step_id>"
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct OpPath(pub String);

    /// Enforced: "<machine_id>.<step_id>.<state_local_id>"
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

        /// Allowlisted flake prefixes for `nix.exec` preflight resolution.
        /// Example: `github:willyrgf/mfm`.
        pub nix_flake_allowlist: Vec<String>,
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

    /// Reserved for future nested-run support.
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

    /// Enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct MachineId(pub String);

    /// Enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct StepId(pub String);

    /// Local state id within an operation.
    ///
    /// Enforced: `^[a-z][a-z0-9_]{0,62}$`
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

    /// A flattened machine definition (ordered steps).
    ///
    /// Contract:
    /// - `machine_id` + `pipeline_version` map to `RunManifest.{op_id, op_version}`.
    /// - Step `OpPath` is "<machine_id>.<step_id>".
    /// - Single-op convention: wrap a single op as a 1-step pipeline with:
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
