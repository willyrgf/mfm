//! `mfm-machine` public API contract (types + traits only).
//!
//! Source of truth: `docs/redesign.md` Appendix C.1.

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

    #[allow(dead_code)]
    pub(crate) fn is_valid_id_segment(segment: &str) -> bool {
        let b = segment.as_bytes();
        if b.is_empty() || b.len() > 63 {
            return false;
        }

        // ^[a-z][a-z0-9_]{0,62}$
        match b[0] {
            b'a'..=b'z' => {}
            _ => return false,
        }

        for &c in &b[1..] {
            match c {
                b'a'..=b'z' | b'0'..=b'9' | b'_' => {}
                _ => return false,
            }
        }

        true
    }

    #[allow(dead_code)]
    pub(crate) fn validate_op_path(value: &str) -> bool {
        let mut it = value.split('.');
        let Some(machine_id) = it.next() else {
            return false;
        };
        let Some(step_id) = it.next() else {
            return false;
        };
        if it.next().is_some() {
            return false;
        }
        is_valid_id_segment(machine_id) && is_valid_id_segment(step_id)
    }

    #[allow(dead_code)]
    pub(crate) fn validate_state_id(value: &str) -> bool {
        let mut it = value.split('.');
        let Some(machine_id) = it.next() else {
            return false;
        };
        let Some(step_id) = it.next() else {
            return false;
        };
        let Some(state_local_id) = it.next() else {
            return false;
        };
        if it.next().is_some() {
            return false;
        }
        is_valid_id_segment(machine_id)
            && is_valid_id_segment(step_id)
            && is_valid_id_segment(state_local_id)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn id_segment_validation() {
            assert!(is_valid_id_segment("a"));
            assert!(is_valid_id_segment("a0"));
            assert!(is_valid_id_segment("a_0b"));
            assert!(!is_valid_id_segment(""));
            assert!(!is_valid_id_segment("_a"));
            assert!(!is_valid_id_segment("A"));
            assert!(!is_valid_id_segment("a-1"));
            assert!(!is_valid_id_segment("0a"));
            assert!(!is_valid_id_segment(&"a".repeat(64)));
        }

        #[test]
        fn op_path_shape_and_segments() {
            assert!(validate_op_path("machine.main"));
            assert!(!validate_op_path("m0._"));
            assert!(!validate_op_path("machine"));
            assert!(!validate_op_path("machine.main.extra"));
            assert!(!validate_op_path("Machine.main"));
            assert!(!validate_op_path("machine.ma-in"));
        }

        #[test]
        fn state_id_shape_and_segments() {
            assert!(validate_state_id("machine.main.setup"));
            assert!(!validate_state_id("machine.main"));
            assert!(!validate_state_id("machine.main.setup.extra"));
            assert!(!validate_state_id("machine.Main.setup"));
            assert!(!validate_state_id("machine.main.set-up"));
        }
    }
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
        Fixed {
            delay: Duration,
        },
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
        FanOutJoin {
            max_concurrency: u32,
        },
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
        ///
        /// Example prefix: `github:willyrgf/mfm`.
        #[serde(default = "default_nix_flake_allowlist")]
        pub nix_flake_allowlist: Vec<String>,
    }

    pub fn default_nix_flake_allowlist() -> Vec<String> {
        vec!["github:willyrgf/mfm".to_string()]
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

    #[allow(dead_code)]
    pub(crate) const CODE_MISSING_FACT_KEY: &str = "missing_fact_key";

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

        MissingFact {
            key: crate::ids::FactKey,
            info: ErrorInfo,
        },
        Transport(ErrorInfo),
        RateLimited(ErrorInfo),
        Other(ErrorInfo),
    }

    /// Context errors.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ContextError {
        MissingKey {
            key: crate::ids::ContextKey,
            info: ErrorInfo,
        },
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn missing_fact_key_code_is_stable() {
            assert_eq!(CODE_MISSING_FACT_KEY, "missing_fact_key");

            let info = ErrorInfo {
                code: ErrorCode(CODE_MISSING_FACT_KEY.to_string()),
                category: ErrorCategory::Rpc,
                retryable: false,
                message: "missing fact key".to_string(),
                details: None,
            };

            let err = IoError::MissingFactKey(info);
            match err {
                IoError::MissingFactKey(info) => assert_eq!(info.code.0, "missing_fact_key"),
                _ => unreachable!("wrong error variant"),
            }
        }
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

    impl<C: DynContext + ?Sized> TypedContextExt for C {
        fn read_typed<T: serde::de::DeserializeOwned>(
            &self,
            key: &ContextKey,
        ) -> Result<Option<T>, ContextError> {
            let Some(value) = self.read(key)? else {
                return Ok(None);
            };

            serde_json::from_value(value).map(Some).map_err(|_| {
                ContextError::Serialization(crate::errors::ErrorInfo {
                    code: crate::ids::ErrorCode("context_deserialize_failed".to_string()),
                    category: crate::errors::ErrorCategory::Context,
                    retryable: false,
                    message: "context value deserialization failed".to_string(),
                    details: None,
                })
            })
        }

        fn write_typed<T: Serialize>(
            &mut self,
            key: ContextKey,
            value: &T,
        ) -> Result<(), ContextError> {
            let v = serde_json::to_value(value).map_err(|_| {
                ContextError::Serialization(crate::errors::ErrorInfo {
                    code: crate::ids::ErrorCode("context_serialize_failed".to_string()),
                    category: crate::errors::ErrorCategory::Context,
                    retryable: false,
                    message: "context value serialization failed".to_string(),
                    details: None,
                })
            })?;

            self.write(key, v)
        }
    }
}

pub mod events {
    use super::*;
    use crate::errors::StateError;
    use crate::ids::{ArtifactId, OpId, OpPath, RunId, StateId};

    /// Recommended stable `DomainEvent.name` values.
    pub const DOMAIN_EVENT_FACT_RECORDED: &str = "fact_recorded";
    pub const DOMAIN_EVENT_ARTIFACT_WRITTEN: &str = "artifact_written";
    pub const DOMAIN_EVENT_OP_BOUNDARY: &str = "op_boundary";
    pub const DOMAIN_EVENT_CHILD_RUN_SPAWNED: &str = "child_run_spawned";
    pub const DOMAIN_EVENT_CHILD_RUN_COMPLETED: &str = "child_run_completed";

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

    /// Reserved for later expansion (nested machines).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ChildRunSpawned {
        pub parent_run_id: RunId,
        pub child_run_id: RunId,
        pub child_manifest_id: ArtifactId,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ChildRunCompleted {
        pub child_run_id: RunId,
        pub status: RunStatus,
        pub final_snapshot_id: Option<ArtifactId>,
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        use crate::hashing::artifact_id_for_json;

        #[test]
        fn child_run_payloads_are_canonical_and_non_secret() {
            let v = serde_json::to_value(ChildRunSpawned {
                parent_run_id: RunId(uuid::Uuid::new_v4()),
                child_run_id: RunId(uuid::Uuid::new_v4()),
                child_manifest_id: ArtifactId("0".repeat(64)),
            })
            .expect("serialize");
            artifact_id_for_json(&v).expect("canonical-json-hashable");
            assert!(!crate::secrets::json_contains_secrets(&v));

            let v = serde_json::to_value(ChildRunCompleted {
                child_run_id: RunId(uuid::Uuid::new_v4()),
                status: RunStatus::Completed,
                final_snapshot_id: Some(ArtifactId("1".repeat(64))),
            })
            .expect("serialize");
            artifact_id_for_json(&v).expect("canonical-json-hashable");
            assert!(!crate::secrets::json_contains_secrets(&v));
        }
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

        /// Persist a deterministic structured value directly as a fact payload.
        async fn record_value(
            &mut self,
            key: FactKey,
            value: serde_json::Value,
        ) -> Result<ArtifactId, IoError>;

        /// Lookup recorded fact payload by key.
        async fn get_recorded_fact(&mut self, key: &FactKey)
            -> Result<Option<ArtifactId>, IoError>;

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
        DuplicateStateId {
            state_id: StateId,
        },
        MissingStateForEdge {
            missing: StateId,
        },
        CircularDependency {
            cycle: Vec<StateId>,
        },

        /// Optional: if planners derive edges from tag dependencies, they may validate those too.
        DanglingDependencyTag {
            state_id: StateId,
            missing_tag: crate::meta::Tag,
        },
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
        /// Encrypted secret payload (ciphertext bytes only).
        ///
        /// Secret plaintext MUST NOT be stored directly in the artifact store.
        SecretPayload,
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
        async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>)
            -> Result<ArtifactId, StorageError>;
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
    #[derive(Clone)]
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

/// Internal helpers for canonical JSON hashing and `ArtifactId` computation.
///
/// This module is not part of the stable API contract (Appendix C.1) and may change.
pub mod hashing;

/// Unstable v4 runtime implementation (executor + resume logic).
///
/// Not part of the stable API contract (Appendix C.1).
pub mod runtime;

/// Unstable Live IO implementation (facts recording).
///
/// Not part of the stable API contract (Appendix C.1).
pub mod live_io;

/// Unstable live IO transport for external program execution (`exec` namespace).
///
/// Not part of the stable API contract (Appendix C.1).
pub mod exec_transport;

/// Unstable Live IO transport router (namespace dispatch).
///
/// Not part of the stable API contract (Appendix C.1).
pub mod live_io_router;

/// Unstable Live IO transport registry (runtime wiring).
///
/// Not part of the stable API contract (Appendix C.1).
pub mod live_io_registry;

/// Unstable Replay IO implementation (facts replay).
///
/// Not part of the stable API contract (Appendix C.1).
pub mod replay_io;

pub(crate) mod attempt_envelope;
pub(crate) mod context_runtime;
pub(crate) mod event_profile;
pub(crate) mod secrets;
