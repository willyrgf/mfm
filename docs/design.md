# MFM Design Contract

> Last updated: 2026-03-24
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
2. [`docs/ops-and-states.md`](ops-and-states.md) for the current inventory of registered ops and production `State` implementations.
3. This contract for normative behavior.
4. `AGENTS.md` for contribution and CI rules.

`docs/ops-and-states.md` is descriptive and current-state oriented.
This contract remains the normative source of truth for semantics, invariants, and allowed boundaries.
For concrete `rpc.control` routing, bootstrap, and executor-boundary behavior, use
[`docs/evm-rpc-routing.md`](evm-rpc-routing.md) alongside this document.

Normative `rpc.control` contract:
- canonical managed requests MUST carry explicit `network_id`
- managed request identity MUST include the effective `control_scope`
- the default `shared` scope is allowed only as a caller-side convenience that is
  stamped into the serialized request before fact-key derivation
- managed bootstrap source catalogs MUST declare `network_id` on every source
- durable control-plane identities for `rpc_source:*` and `source_pool:*` MUST be
  keyed by `control_scope` plus the existing network/source dimensions

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
- deterministic expansion into sub-operations and/or state-graph structure
- output schema

Normative constraints:
- `expand()` MUST be deterministic for equivalent `(op_config, run_config)`.
- `expand()` MUST NOT perform ambient IO (`fs/network/time/env/process`).
- operation planning MAY be recursive, but it MUST flatten to one final execution plan before
  runtime starts.
- operation code MUST NOT execute workflow side effects.
- operation code SHOULD stay focused on config validation + state graph wiring.

### 3.2 Run
Concrete execution of an op or flattened pipeline:
- run ID
- content-addressed manifest
- append-only `run:*` stream
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
- state handlers MUST NOT invoke operations, re-enter planners, or reshape execution topology.
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
- The shared stream store is append-only; `run:*` streams append machine events and historical records are never mutated.

### 4.2 Per-append atomicity
- `StreamStore::append(...)` is all-or-nothing.
- `StreamStore::append_batch(...)` is all-or-nothing across every touched stream.
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
  - correctness-critical control-plane coordination uses a sibling Postgres storage crate
    (`crates/storages/control-plane-postgres`) that writes control-plane stream families into the
    shared append-only stream tables and updates its projection tables in the same SQL transaction
  - this does not move control-plane semantics into `crates/machine` or extend the generic
    `StreamStore` trait for v1
- shared-state layer crates:
  - `crates/states/common/`: cross-domain reusable state primitives
  - `crates/states/keystore/`: keystore-domain reusable states/helpers
  - `crates/states/aave-v3/`: Aave-domain reusable states/helpers
  - `crates/evm-runtime/`: runtime-facing reusable EVM states/helpers
- `crates/ops/*-op`: domain operation planners that compose state graphs
- `crates/sdk/`: operation/pipeline orchestration glue
- `bin/cli`, `bin/rest-api`: thin transport adapters

Inventory note:
- The current catalog of registered ops and production `State` implementations lives in [`docs/ops-and-states.md`](ops-and-states.md).
- This document defines where those components belong and what rules they must obey; it does not enumerate them exhaustively.

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
- recursive operation composition and flattening into one final `ExecutionPlan`
- run launch/resume glue over machine + stores

### 6.3A Recursive planning contract
MFM has one planning abstraction: recursive operation planning.

Normative rules:
- a root operation MAY expand into sub-operations
- sub-operations MAY expand recursively or directly into states
- planning MUST fully flatten into one final `ExecutionPlan` before `RunStarted`
- runtime executes states only
- states MUST NOT invoke operations or request planner re-entry
- caller-visible pipelines, when used, MUST converge to the same flattening semantics
- each child op MUST declare a unique parent-local identity
- child-op flattening order MUST be deterministic
- context namespacing and import/export wiring MUST use full hierarchical child `OpPath`
- duplicate child-op paths, duplicate exported ports, and duplicate flattened `StateId`s are
  invalid plans
- recursive composition MUST NOT rely on implicit "last writer wins" export behavior
- planner-visible state lineage MUST be carried by `StateAddr { op_path, state_local_id }`, not by
  overloading runtime `StateId`
- the exact SDK or `Operation` helper surface MAY evolve, but flatten-before-runtime is the
  invariant that implementations MUST preserve

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
- optional compiled execution-spec artifact reference for audit/inspection only; in v1 this MUST
  NOT replace `manifest.input_params` as the authoritative resume input

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

For the current registry-backed op catalog, see [`docs/ops-and-states.md`](ops-and-states.md).
`expand()` is a planning-only phase and MUST NOT execute runtime side effects.

### 11.2 Flattened composition
If op A has N states and op B has M states, pipeline graph size is `N + M` states in one run.

The same flattening contract applies whether composition comes from caller-visible pipelines or
recursive child-op expansion.

### 11.3 ID stability rules
ID conventions:
- `OpPath = <machine_id>.<step_id>(.<child_op_local_id>)*`
- `StateId = <machine_id>.<step_id>.<flattened_state_local_id>`
- each dotted path component must match: `^[a-z][a-z0-9_]{0,62}$`
- `flattened_state_local_id` MUST be derived by taking the `OpPath` segments after the root
  `<machine_id>.<step_id>`, appending `state_local_id`, and joining those validated local ids with
  `__`
- if the leaf op is the root step, `flattened_state_local_id = state_local_id`
- lowering does not escape underscores; collisions after lowering are invalid plans
- dots are separators only

Single-op run convention:
- wrap as one-step machine:
  - `machine_id = <op_id>`
  - `step_id = main`

### 11.4 Namespaced context and explicit wiring
- context keys are namespaced by the full hierarchical op path by default
- cross-op data flow uses explicit imports/exports validated by planner

### 11.4A Compiled execution spec
For audit/debug in v1, planners MAY persist a compiled execution spec artifact.

Normative rules:
- the compiled execution spec artifact is optional and is not the authoritative resume input
- resume MUST continue to rebuild from `RunManifest.input_params`
- if persisted, the artifact kind SHOULD be `ArtifactKind::Other("compiled_execution_spec")`
- persisted compiled specs MUST obey canonical-JSON and no-secrets invariants
- the minimum v1 schema MUST include:
  - `schema_version = "compiled_execution_spec/v1"`
  - root op identity: `root_op_id`, `root_op_version`, `root_op_path`
  - ordered op records with interface, import bindings, ordering edges, and re-exports
  - ordered root export resolution entries
  - ordered `state_lineage` entries mapping lowered `StateId` to `StateAddr`
  - `planner_payloads` for planner-owned semantic payloads such as compiled observation batches and
    valuation tasks
- emitted arrays MUST use deterministic planner order; hash-iteration order is forbidden

### 11.5 Nested runs
Engine-managed child runs are deferred.
When introduced, linkage events are required (`ChildRunSpawned`).

### 11.6 Public portfolio schema cuts
If a breaking change to a persisted or public portfolio JSON surface becomes necessary
(`PortfolioSnapshotRequest`, `PortfolioSnapshotResponse`, `PortfolioSnapshot`, or
`PortfolioReport`), the same change MUST:

- keep `portfolio_tracker` as `v1` (no compatibility-preserving migration phase on this branch)
- add/update top-level `schema_version` on each changed public JSON object to the next schema value
- update CLI/REST/docs in the same commit

Purely additive changes that preserve existing field meaning do not require this cut.

## 12. Storage Contract

### 12.1 Two mandatory roles
1. Stream store (append-only, optimistic concurrency)
2. Artifact store (immutable, content-addressed)

Optional later role:
- projection/index store

The stream store is one shared physical substrate:
- `run:*` stores machine runtime records
- future families such as `wallet_lane:*`, `tx_intent:*`, and `rpc_source:*` use the same append-only contract

### 12.2 Backend strategy
- PostgreSQL as primary transactional stream store
- MinIO/S3 as primary artifact storage
- local fast implementations retained for unit and dev loops

### 12.3 Security requirements
- no persisted secrets in any store-backed surface
- stream records may store references, never secret plaintext
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

    /// Storage errors (stream store / artifact store).
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

    /// Machine event envelope encoded into `run:*` stream records.
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

    /// Validated append-only stream identifier.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(try_from = "String", into = "String")]
    pub struct StreamId(String);

    impl StreamId {
        pub fn new(value: impl Into<String>) -> Result<Self, crate::ids::IdValidationError> {
            let value = value.into();
            let Some((family, key)) = value.split_once(':') else {
                return Err(crate::ids::IdValidationError::new("stream_id", value));
            };
            if !is_valid_id_segment(family)
                || key.is_empty()
                || key
                    .chars()
                    .any(|ch| ch.is_ascii_control() || ch.is_whitespace())
            {
                return Err(crate::ids::IdValidationError::new("stream_id", value));
            }
            Ok(Self(value))
        }

        pub fn as_str(&self) -> &str {
            &self.0
        }

        pub fn run(run_id: RunId) -> Self {
            Self(format!("run:{}", run_id.0))
        }
    }

    /// Persisted record in an append-only stream.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StreamRecord {
        pub stream_id: StreamId,
        pub seq: u64,
        pub ts_millis: Option<u64>,
        pub kind: String,
        pub payload: serde_json::Value,
    }

    /// Record to append into a stream.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct NewStreamRecord {
        pub ts_millis: Option<u64>,
        pub kind: String,
        pub payload: serde_json::Value,
    }

    /// Atomic compare-and-append request for one stream.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StreamAppend {
        pub stream_id: StreamId,
        pub expected_seq: u64,
        pub records: Vec<NewStreamRecord>,
    }

    /// Head sequences returned by an atomic multi-stream append.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct AppendBatchResult {
        pub stream_heads: Vec<(StreamId, u64)>,
    }

    /// Append-only stream store with optimistic concurrency.
    #[async_trait]
    pub trait StreamStore: Send + Sync {
        async fn head_seq(&self, stream_id: &StreamId) -> Result<u64, StorageError>;

        async fn append(&self, append: StreamAppend) -> Result<u64, StorageError>;

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
    use crate::stores::{ArtifactStore, StreamStore};

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
        pub streams: Arc<dyn StreamStore>,
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

    /// Declared op interface surface for pipeline validation.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct OpInterface {
        pub imports: Vec<PortKey>,
        pub exports: Vec<PortKey>,
    }

    /// A reusable operation definition.
    ///
    /// Contract:
    /// - `op_id` + `op_version` MUST be stable across environments.
    /// - `expand()` MUST be deterministic and MUST NOT perform IO.
    /// - `expand()` returns a `PlannedOp` (leaf or composite) that the SDK planner
    ///   recursively flattens into a flat execution plan.
    pub trait Operation: Send + Sync {
        fn op_id(&self) -> OpId;
        fn op_version(&self) -> String;

        fn expand(
            &self,
            op_path: OpPath,
            op_config: &serde_json::Value,
            run_config: &RunConfig,
        ) -> Result<PlannedOp, SdkError>;
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
        /// - support recursive child-op expansion before runtime starts
        /// - assign deterministic hierarchical child `OpPath`s during flattening
        /// - ensure all flattened `StateId`s are unique and match the
        ///   "<machine_id>.<step_id>.<flattened_state_local_id>" convention
        /// - reject duplicate child-op paths and duplicate exported ports
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
