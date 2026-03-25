#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Public API contract for the MFM runtime.
//!
//! `mfm-machine` defines the stable identifiers, execution-plan types, context and IO
//! abstractions, and storage interfaces used throughout the workspace. Runtime implementations
//! live in unstable submodules such as [`runtime`] and [`live_io`], while the types in this file
//! model the architectural contract described in `docs/design.md`.
//!
//! # Example
//!
//! ```rust
//! use async_trait::async_trait;
//! use mfm_machine::context::DynContext;
//! use mfm_machine::errors::{ContextError, StateError};
//! use mfm_machine::ids::{ContextKey, OpId, StateId};
//! use mfm_machine::io::IoProvider;
//! use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};
//! use mfm_machine::plan::{ExecutionPlan, StateGraph, StateNode};
//! use mfm_machine::recorder::EventRecorder;
//! use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
//! use serde_json::Value;
//! use std::sync::Arc;
//!
//! struct ExampleState;
//!
//! #[async_trait]
//! impl State for ExampleState {
//!     fn meta(&self) -> StateMeta {
//!         StateMeta {
//!             tags: vec![Tag("report".to_string())],
//!             depends_on: Vec::new(),
//!             depends_on_strategy: DependencyStrategy::Latest,
//!             side_effects: SideEffectKind::Pure,
//!             idempotency: Idempotency::None,
//!         }
//!     }
//!
//!     async fn handle(
//!         &self,
//!         _ctx: &mut dyn DynContext,
//!         _io: &mut dyn IoProvider,
//!         _rec: &mut dyn EventRecorder,
//!     ) -> Result<StateOutcome, StateError> {
//!         Ok(StateOutcome {
//!             snapshot: SnapshotPolicy::OnSuccess,
//!         })
//!     }
//! }
//!
//! struct NullContext;
//!
//! impl DynContext for NullContext {
//!     fn read(&self, _key: &ContextKey) -> Result<Option<Value>, ContextError> {
//!         Ok(None)
//!     }
//!
//!     fn write(&mut self, _key: ContextKey, _value: Value) -> Result<(), ContextError> {
//!         Ok(())
//!     }
//!
//!     fn delete(&mut self, _key: &ContextKey) -> Result<(), ContextError> {
//!         Ok(())
//!     }
//!
//!     fn dump(&self) -> Result<Value, ContextError> {
//!         Ok(serde_json::json!({}))
//!     }
//! }
//!
//! let _context = NullContext;
//! let plan = ExecutionPlan {
//!     op_id: OpId::must_new("portfolio_snapshot"),
//!     graph: StateGraph {
//!         states: vec![StateNode {
//!             id: StateId::must_new("machine.main.report"),
//!             state: Arc::new(ExampleState),
//!         }],
//!         edges: Vec::new(),
//!     },
//! };
//!
//! assert_eq!(plan.op_id.as_str(), "portfolio_snapshot");
//! ```
//!
//! Source of truth: `docs/design.md` Appendix C.1.
#![warn(missing_docs)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Stable identifier types shared by manifests, plans, events, and persisted records.
pub mod ids {
    use super::*;
    use std::fmt;

    /// Error returned when an identifier fails the runtime naming contract.
    ///
    /// The stable identifier family deliberately keeps validation strict so identifiers remain
    /// safe to embed in manifests, event streams, context keys, and artifact-derived metadata.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct IdValidationError {
        kind: &'static str,
        value: String,
    }

    impl IdValidationError {
        pub(crate) fn new(kind: &'static str, value: impl Into<String>) -> Self {
            Self {
                kind,
                value: value.into(),
            }
        }
    }

    impl fmt::Display for IdValidationError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "invalid {}: {}", self.kind, self.value)
        }
    }

    impl std::error::Error for IdValidationError {}

    /// Stable identifier for an operation (human meaningful).
    /// Invariant: stable across environments; should not be random.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(try_from = "String", into = "String")]
    pub struct OpId(String);

    /// Enforced: "<machine_id>.<step_id>(.<child_op_local_id>)*"
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(try_from = "String", into = "String")]
    pub struct OpPath(pub String);

    /// Enforced: "<machine_id>.<step_id>.<state_local_id>"
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(try_from = "String", into = "String")]
    pub struct StateId(String);

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

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct OpPathParts<'a> {
        machine: &'a str,
        step: &'a str,
        child_path: Option<&'a str>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct StateIdParts<'a> {
        machine: &'a str,
        step: &'a str,
        state_local_id: &'a str,
    }

    impl<'a> OpPathParts<'a> {
        fn parse(value: &'a str) -> Option<Self> {
            let (machine, suffix) = value.split_once('.')?;
            let (step, child_path) = match suffix.split_once('.') {
                Some((step, child_path)) => (step, Some(child_path)),
                None => (suffix, None),
            };
            Some(Self {
                machine,
                step,
                child_path,
            })
        }
    }

    impl<'a> StateIdParts<'a> {
        fn parse(value: &'a str) -> Option<Self> {
            let (machine, suffix) = value.split_once('.')?;
            let (step, state_local_id) = suffix.split_once('.')?;
            Some(Self {
                machine,
                step,
                state_local_id,
            })
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct ContextKeyParts<'a> {
        slot: ContextSlot,
        suffix: &'a str,
    }

    /// Canonical context slot prefix used for key qualification and fallback lookup.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ContextSlot {
        /// Input keys passed from parent operations.
        In,
        /// Output keys exposed by state exports.
        Out,
        /// Internal work keys written during planning/runtime.
        Work,
    }

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

    impl OpId {
        /// Creates an operation identifier after validating the naming contract.
        ///
        /// Accepted values match `^[a-z][a-z0-9_]{0,62}$`.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use mfm_machine::ids::OpId;
        ///
        /// let op_id = OpId::new("keystore_list")?;
        /// assert_eq!(op_id.as_str(), "keystore_list");
        /// # Ok::<(), mfm_machine::ids::IdValidationError>(())
        /// ```
        pub fn new(value: impl Into<String>) -> Result<Self, IdValidationError> {
            let value = value.into();
            if !is_valid_id_segment(&value) {
                return Err(IdValidationError::new("op_id", value));
            }
            Ok(Self(value))
        }

        /// Creates an [`OpId`] and panics if the value is invalid.
        pub fn must_new(value: impl Into<String>) -> Self {
            Self::new(value).expect("op id must satisfy ^[a-z][a-z0-9_]{0,62}$")
        }

        /// Returns the validated identifier as a borrowed string slice.
        pub fn as_str(&self) -> &str {
            &self.0
        }
    }

    impl TryFrom<String> for OpId {
        type Error = IdValidationError;

        fn try_from(value: String) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl TryFrom<&str> for OpId {
        type Error = IdValidationError;

        fn try_from(value: &str) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl From<OpId> for String {
        fn from(value: OpId) -> Self {
            value.0
        }
    }

    impl fmt::Display for OpId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    pub(crate) fn validate_op_path(value: &str) -> bool {
        let mut it = value.split('.');
        let Some(machine_id) = it.next() else {
            return false;
        };
        let Some(step_id) = it.next() else {
            return false;
        };
        if !is_valid_id_segment(machine_id) || !is_valid_id_segment(step_id) {
            return false;
        }
        it.all(is_valid_id_segment)
    }

    impl OpPath {
        /// Creates an operation path after validating the hierarchical naming contract.
        ///
        /// Accepted values match `<machine_id>.<step_id>(.<child_op_local_id>)*` where each
        /// segment satisfies the same naming contract as [`OpId`].
        pub fn new(value: impl Into<String>) -> Result<Self, IdValidationError> {
            let value = value.into();
            if !validate_op_path(&value) {
                return Err(IdValidationError::new("op_path", value));
            }
            Ok(Self(value))
        }

        /// Creates an [`OpPath`] and panics if the value is invalid.
        pub fn must_new(value: impl Into<String>) -> Self {
            Self::new(value)
                .expect("op path must satisfy <machine_id>.<step_id>(.<child_op_local_id>)*")
        }

        /// Returns the validated path as a borrowed string slice.
        pub fn as_str(&self) -> &str {
            &self.0
        }

        fn parts(&self) -> OpPathParts<'_> {
            OpPathParts::parse(&self.0).expect("OpPath format validated in constructor")
        }

        /// Returns the machine and step components of a validated operation path.
        pub fn machine_and_step(&self) -> (&str, &str) {
            let parts = self.parts();
            (parts.machine, parts.step)
        }

        /// Returns the child path suffix after `<machine>.<step>`, if any.
        pub fn child_path(&self) -> Option<&str> {
            self.parts().child_path
        }

        /// Returns the flattened child suffix using `__` between path segments.
        pub fn flattened_child_path(&self) -> Option<String> {
            self.child_path()
                .map(|child_path| child_path.replace('.', "__"))
        }
    }

    impl TryFrom<String> for OpPath {
        type Error = IdValidationError;

        fn try_from(value: String) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl TryFrom<&str> for OpPath {
        type Error = IdValidationError;

        fn try_from(value: &str) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl From<OpPath> for String {
        fn from(value: OpPath) -> Self {
            value.0
        }
    }

    impl fmt::Display for OpPath {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
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

    impl StateId {
        /// Creates a state identifier after validating the `<machine>.<step>.<state>` shape.
        ///
        /// Each segment must satisfy the same naming contract as [`OpId`].
        ///
        /// # Examples
        ///
        /// ```rust
        /// use mfm_machine::ids::StateId;
        ///
        /// let state_id = StateId::new("portfolio_snapshot.fetch_balances.read_eth")?;
        /// assert_eq!(state_id.as_str(), "portfolio_snapshot.fetch_balances.read_eth");
        /// # Ok::<(), mfm_machine::ids::IdValidationError>(())
        /// ```
        pub fn new(value: impl Into<String>) -> Result<Self, IdValidationError> {
            let value = value.into();
            if !validate_state_id(&value) {
                return Err(IdValidationError::new("state_id", value));
            }
            Ok(Self(value))
        }

        /// Creates a [`StateId`] and panics if the value is invalid.
        pub fn must_new(value: impl Into<String>) -> Self {
            Self::new(value).expect("state id must satisfy <machine_id>.<step_id>.<state_local_id>")
        }

        /// Returns the validated identifier as a borrowed string slice.
        pub fn as_str(&self) -> &str {
            &self.0
        }

        fn parts(&self) -> StateIdParts<'_> {
            StateIdParts::parse(&self.0).expect("StateId format validated in constructor")
        }

        /// Returns the machine component of a validated state id.
        pub fn machine(&self) -> &str {
            self.parts().machine
        }

        /// Returns the step component of a validated state id.
        pub fn step(&self) -> &str {
            self.parts().step
        }

        /// Returns the local state segment of a validated state id.
        pub fn state_local_id(&self) -> &str {
            self.parts().state_local_id
        }
    }

    impl ContextKey {
        fn explicit_slot_parts(&self) -> Option<ContextKeyParts<'_>> {
            let (prefix, suffix) = self.0.split_once('.')?;
            if suffix.is_empty() {
                return None;
            }

            let slot = match prefix {
                "in" => ContextSlot::In,
                "out" => ContextSlot::Out,
                "work" => ContextSlot::Work,
                _ => return None,
            };

            Some(ContextKeyParts { slot, suffix })
        }

        /// Returns the explicit slot prefix if this key is already slot-qualified.
        pub fn explicit_slot(&self) -> Option<(ContextSlot, &str)> {
            self.explicit_slot_parts()
                .map(|parts| (parts.slot, parts.suffix))
        }
    }

    impl ContextSlot {
        fn as_str(&self) -> &'static str {
            match self {
                ContextSlot::In => "in",
                ContextSlot::Out => "out",
                ContextSlot::Work => "work",
            }
        }

        fn matches_any(prefix: &str) -> bool {
            matches!(prefix, "in" | "out" | "work")
        }
    }

    impl TryFrom<String> for StateId {
        type Error = IdValidationError;

        fn try_from(value: String) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl TryFrom<&str> for StateId {
        type Error = IdValidationError;

        fn try_from(value: &str) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl From<StateId> for String {
        fn from(value: StateId) -> Self {
            value.0
        }
    }

    impl fmt::Display for StateId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    #[cfg(test)]
    mod ids_tests {
        include!("tests/ids_tests.rs");
    }
}

/// Marker traits for canonical JSON hashing policy.
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

/// Run configuration types that shape execution, replay, and provenance behavior.
pub mod config {
    use super::*;
    use crate::ids::OpId;
    use crate::meta::Tag;

    /// Whether a run is allowed to perform live IO or must replay from recorded facts/artifacts.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum IoMode {
        /// Execute side effects through live transports and record replay inputs.
        Live,
        /// Disallow live side effects and resolve deterministic IO from recorded facts.
        Replay,
    }

    /// Controls domain event verbosity. Kernel events are always emitted.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum EventProfile {
        /// Emit only the minimal domain-event surface alongside kernel events.
        Minimal,
        /// Emit the default domain-event surface for normal operation.
        Normal,
        /// Emit the richest built-in domain-event surface.
        Verbose,
        /// Use an operation-specific profile string.
        Custom(String),
    }

    /// Backoff policy for retryable errors.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum BackoffPolicy {
        /// Retry after the same delay for every attempt.
        Fixed {
            /// Delay applied before each retry.
            delay: Duration,
        },
        /// Increase the retry delay between attempts until reaching a cap.
        Exponential {
            /// Base delay used for the first exponential retry step.
            base_delay: Duration,
            /// Maximum retry delay once the exponential curve saturates.
            max_delay: Duration,
        },
    }

    /// Retry policy for retryable errors (including replay missing-fact errors if configured retryable).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RetryPolicy {
        /// Maximum number of total attempts, including the first execution.
        pub max_attempts: u32,
        /// Backoff policy applied between retry attempts.
        pub backoff: BackoffPolicy,
    }

    /// Execution mode.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ExecutionMode {
        /// Run one ready state at a time.
        Sequential,
        /// Explicit fan-out/join model. Avoids concurrent writes to shared context.
        FanOutJoin {
            /// Maximum number of states the executor may run concurrently.
            max_concurrency: u32,
        },
    }

    /// Run-level context checkpointing policy.
    ///
    /// Default is `AfterEveryState` to keep resume semantics simple (no replay required on resume).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ContextCheckpointing {
        /// Persist a fresh context snapshot after every state transition.
        AfterEveryState,
        /// Reserved for future: periodic/tag-based checkpointing policies.
        Custom(String),
    }

    /// Run-level execution configuration (policy).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunConfig {
        /// Whether execution uses live IO or replay-only IO.
        pub io_mode: IoMode,
        /// Retry policy applied to retryable failures.
        pub retry_policy: RetryPolicy,
        /// Desired domain-event verbosity profile.
        pub event_profile: EventProfile,
        /// Scheduler policy used to execute ready states.
        pub execution_mode: ExecutionMode,
        /// Policy for persisting run-level context snapshots.
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

    /// Returns the default allowlist for `nix.exec` flake resolution.
    pub fn default_nix_flake_allowlist() -> Vec<String> {
        vec!["github:willyrgf/mfm".to_string()]
    }

    /// Minimal run manifest shape (stored as an artifact; hashed via canonical JSON).
    /// Note: `input_params` MUST be canonical-JSON hashable and MUST NOT contain secrets.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunManifest {
        /// Stable identifier for the operation being executed.
        pub op_id: OpId,
        /// Operation version string included in the manifest hash.
        pub op_version: String,
        /// Canonical JSON input parameters for the operation.
        pub input_params: serde_json::Value,
        /// Run policy captured as part of the manifest.
        pub run_config: RunConfig,
        /// Build metadata used for reproducibility and provenance.
        pub build: BuildProvenance,
    }

    /// Build provenance (reproducibility metadata).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct BuildProvenance {
        /// Git commit of the build, if available.
        pub git_commit: Option<String>,
        /// Hash of `Cargo.lock`, if captured by the caller.
        pub cargo_lock_hash: Option<String>,
        /// Hash of `flake.lock`, if captured by the caller.
        pub flake_lock_hash: Option<String>,
        /// Rust compiler version used to build the operation.
        pub rustc_version: Option<String>,
        /// Target triple of the built artifact.
        pub target_triple: Option<String>,
        /// Environment-variable names intentionally allowed into provenance.
        pub env_allowlist: Vec<String>,
    }
}

/// Metadata used to classify states and express recovery or idempotency intent.
pub mod meta {
    use super::*;

    /// Tags are used for classification, filtering, and policy decisions.
    /// Recommended format: lowercase; allow separators for namespacing if needed.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Tag(pub String);

    /// Standard tags (stable identifiers).
    ///
    /// These constants are the stable classification vocabulary shared across planners,
    /// executors, and policy code. Prefer reusing them instead of inventing near-duplicate
    /// spellings in downstream crates.
    ///
    /// A good default mapping is:
    /// - `CONFIG` for validation and config-shaping nodes
    /// - `FETCH_DATA` for read/fact acquisition nodes
    /// - `COMPUTE` for pure transforms
    /// - `EXECUTE` for imperative runtime work
    /// - `REPORT` for user-facing output assembly
    ///
    /// `APPLY_SIDE_EFFECT` and `IMPURE` are stronger policy signals and should be reserved for
    /// states whose replay or retry behavior genuinely depends on those classifications.
    pub mod standard_tags {
        /// Marks a configuration or validation state.
        pub const CONFIG: &str = "config";
        /// Marks a state whose primary job is to retrieve external data.
        pub const FETCH_DATA: &str = "fetch_data";
        /// Marks a pure computation or transformation state.
        pub const COMPUTE: &str = "compute";
        /// Marks a state that performs or prepares executable work.
        pub const EXECUTE: &str = "execute";
        /// Marks a reporting or output-producing state.
        pub const REPORT: &str = "report";

        /// Marks a state that applies an external side effect.
        pub const APPLY_SIDE_EFFECT: &str = "apply_side_effect";
        /// Marks a state whose behavior is not purely deterministic.
        pub const IMPURE: &str = "impure";
    }

    /// Side-effect classification (affects replay and retry semantics).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum SideEffectKind {
        /// The state is pure and does not interact with external systems.
        Pure,
        /// The state performs read-only IO.
        ReadOnlyIo,
        /// The state applies a side effect to an external system.
        ApplySideEffect,
    }

    /// Optional idempotency declaration for side-effecting states.
    /// Key semantics: stable value used for dedupe (e.g., tx intent hash).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Idempotency {
        /// The state does not declare an idempotency key.
        None,
        /// Stable key used to deduplicate externally visible effects.
        Key(String),
    }

    /// Strategy for choosing a recovery point among dependency candidates.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DependencyStrategy {
        /// Recover from the latest matching dependency candidate.
        Latest,
        /// Recover from the earliest matching dependency candidate.
        Earliest,
        /// Recover from the latest dependency candidate that completed successfully.
        LatestSuccessful,
    }

    /// State metadata used for policy decisions and validation.
    ///
    /// Notes:
    /// - `depends_on` is an authoring-time *hint* (often used by planners).
    /// - Execution correctness is governed by explicit plan edges.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StateMeta {
        /// Classification tags applied to the state.
        pub tags: Vec<Tag>,

        /// Optional authoring-time dependency hints expressed as tags.
        pub depends_on: Vec<Tag>,
        /// Strategy used when resolving tag-based dependency hints.
        pub depends_on_strategy: DependencyStrategy,

        /// Side-effect classification used by replay and retry policy.
        pub side_effects: SideEffectKind,
        /// Declared idempotency behavior for the state.
        pub idempotency: Idempotency,
    }
}

/// Structured error types used by state handlers, storage layers, and the engine.
pub mod errors {
    use super::*;
    use crate::ids::{ErrorCode, StateId};

    #[allow(dead_code)]
    pub(crate) const CODE_MISSING_FACT_KEY: &str = "missing_fact_key";

    /// Error category used for stable handling and policies.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ErrorCategory {
        /// Failed while parsing user input or structured payloads.
        ParsingInput,
        /// Failed while interacting with on-chain state or transactions.
        OnChain,
        /// Failed while interacting with off-chain systems or services.
        OffChain,
        /// Failed while talking to RPC-like endpoints.
        Rpc,
        /// Failed while reading or writing persisted state.
        Storage,
        /// Failed while reading or mutating execution context.
        Context,
        /// Failed for an uncategorized reason.
        Unknown,
    }

    /// Structured error info (canonical-JSON compatible; MUST NOT contain secrets).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ErrorInfo {
        /// Stable machine-readable error code.
        pub code: ErrorCode,
        /// High-level category for policy decisions and display.
        pub category: ErrorCategory,
        /// Whether the operation may be retried safely.
        pub retryable: bool,
        /// Human-readable message safe to persist and display.
        pub message: String,
        /// Optional structured details that must remain secret-free.
        pub details: Option<serde_json::Value>,
    }

    /// Errors returned by state handlers (no secrets).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StateError {
        /// State identifier, if the failure can be attributed to a specific planned state.
        pub state_id: Option<StateId>,
        /// Structured error payload safe for persistence.
        pub info: ErrorInfo,
    }

    /// IO errors (live or replay).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum IoError {
        /// Replay was asked to perform deterministic IO without a fact key.
        MissingFactKey(ErrorInfo),

        /// A requested recorded fact was not found.
        MissingFact {
            /// Missing fact key.
            key: crate::ids::FactKey,
            /// Structured details about the lookup failure.
            info: ErrorInfo,
        },
        /// The underlying transport returned an error.
        Transport(ErrorInfo),
        /// The underlying transport reported a rate limit.
        RateLimited(ErrorInfo),
        /// Any other IO failure that does not fit the more specific variants.
        Other(ErrorInfo),
    }

    /// Context errors.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ContextError {
        /// A requested context key was not present.
        MissingKey {
            /// Missing context key.
            key: crate::ids::ContextKey,
            /// Structured details about the lookup failure.
            info: ErrorInfo,
        },
        /// JSON serialization or deserialization failed.
        Serialization(ErrorInfo),
        /// Any other context failure.
        Other(ErrorInfo),
    }

    /// Storage errors (stream store / artifact store).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum StorageError {
        /// Append failed because the optimistic concurrency expectation was stale.
        Concurrency(ErrorInfo),
        /// A requested record or artifact was not found.
        NotFound(ErrorInfo),
        /// Persisted data existed but failed integrity or shape checks.
        Corruption(ErrorInfo),
        /// Any other storage failure.
        Other(ErrorInfo),
    }

    /// Run-level errors from the engine.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum RunError {
        /// The supplied execution plan violated structural invariants.
        InvalidPlan(ErrorInfo),
        /// The run failed because a storage backend returned an error.
        Storage(StorageError),
        /// The run failed because the execution context returned an error.
        Context(ContextError),
        /// The run failed because the IO provider returned an error.
        Io(IoError),
        /// The run failed because a state handler returned an error.
        State(StateError),
        /// Any other run-level failure.
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

/// Context traits for reading, mutating, and snapshotting state-machine data.
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
        /// Reads a raw JSON value from context.
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError>;
        /// Writes a raw JSON value into context.
        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError>;
        /// Deletes a context entry if it exists.
        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError>;

        /// Full snapshot of current context state.
        fn dump(&self) -> Result<serde_json::Value, ContextError>;
    }

    /// Typed convenience extension (no default bodies; implementations may blanket-impl internally).
    pub trait TypedContextExt {
        /// Reads a context value and deserializes it into `T`.
        fn read_typed<T: serde::de::DeserializeOwned>(
            &self,
            key: &ContextKey,
        ) -> Result<Option<T>, ContextError>;

        /// Serializes `value` and writes it into context.
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

/// Event types emitted by the engine and by state handlers during a run.
pub mod events {
    use super::*;
    use crate::errors::{ErrorCategory, ErrorInfo, StateError, StorageError};
    use crate::ids::{ArtifactId, OpId, OpPath, RunId, StateId};
    use crate::stores::{NewStreamRecord, StreamRecord};

    /// Recommended stable `DomainEvent.name` values.
    pub const DOMAIN_EVENT_FACT_RECORDED: &str = "fact_recorded";
    /// Recommended `DomainEvent.name` for artifact-write notifications.
    pub const DOMAIN_EVENT_ARTIFACT_WRITTEN: &str = "artifact_written";
    /// Recommended `DomainEvent.name` for operation boundary markers.
    pub const DOMAIN_EVENT_OP_BOUNDARY: &str = "op_boundary";
    /// Recommended `DomainEvent.name` for child-run spawn notifications.
    pub const DOMAIN_EVENT_CHILD_RUN_SPAWNED: &str = "child_run_spawned";
    /// Recommended `DomainEvent.name` for child-run completion notifications.
    pub const DOMAIN_EVENT_CHILD_RUN_COMPLETED: &str = "child_run_completed";
    /// Stream record kind used for machine run events persisted in `run:*` streams.
    pub const STREAM_RECORD_KIND_MACHINE_EVENT: &str = "machine_event";

    /// Run completion status.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum RunStatus {
        /// The run completed successfully.
        Completed,
        /// The run terminated because a state failed.
        Failed,
        /// The run was cancelled before completion.
        Cancelled,
    }

    /// Kernel event variants required for recovery/resume correctness.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum KernelEvent {
        /// Marks the start of a run and records its manifest and initial snapshot.
        RunStarted {
            /// Operation identifier for the run.
            op_id: OpId,
            /// Manifest artifact identifier.
            manifest_id: ArtifactId,
            /// Snapshot of initial context at run start.
            initial_snapshot_id: ArtifactId,
        },
        /// Marks entry into a state attempt.
        StateEntered {
            /// Planned state being entered.
            state_id: StateId,
            /// Zero-based attempt number for this state.
            attempt: u32,
            /// Snapshot the attempt starts from (resume/retry boundary).
            base_snapshot_id: ArtifactId,
        },
        /// Marks successful completion of a state.
        StateCompleted {
            /// State that completed successfully.
            state_id: StateId,
            /// Context snapshot captured after completion.
            context_snapshot_id: ArtifactId,
        },
        /// Marks failure of a state attempt.
        StateFailed {
            /// State that failed.
            state_id: StateId,
            /// Structured failure payload.
            error: StateError,
            /// Diagnostic-only snapshot (must not be used as a resume boundary).
            failure_snapshot_id: Option<ArtifactId>,
        },
        /// Marks the terminal status of the run.
        RunCompleted {
            /// Final run status.
            status: RunStatus,
            /// Final context snapshot, if one was produced.
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
        /// Stable event name defined by the operation or a shared convention.
        pub name: String,
        /// Canonical JSON payload safe to persist.
        pub payload: serde_json::Value,
        /// Optional artifact reference for large payloads.
        pub payload_ref: Option<ArtifactId>,
    }

    /// Envelope type used by stream stores.
    ///
    /// The stream store contract expects values of this enum to be wrapped in [`EventEnvelope`]
    /// with strictly increasing per-run sequence numbers.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Event {
        /// Engine-level event required for resume and recovery.
        Kernel(KernelEvent),
        /// Operation-defined event emitted during state handling.
        Domain(DomainEvent),
    }

    /// Envelope stored in the stream store.
    ///
    /// `ts_millis` is informational only; replay semantics come from event order and payload
    /// content, not wall-clock time.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct EventEnvelope {
        /// Run to which this event belongs.
        pub run_id: RunId,
        /// Monotonic per-run sequence number.
        pub seq: u64,

        /// Informational timestamp; must not be required for deterministic replay semantics.
        pub ts_millis: Option<u64>,

        /// Event payload stored at this sequence number.
        pub event: Event,
    }

    fn stream_decode_error(code: &'static str, message: &'static str) -> StorageError {
        StorageError::Corruption(ErrorInfo {
            code: crate::ids::ErrorCode(code.to_string()),
            category: ErrorCategory::Storage,
            retryable: false,
            message: message.to_string(),
            details: None,
        })
    }

    fn stream_encode_error(code: &'static str, message: &'static str) -> StorageError {
        StorageError::Other(ErrorInfo {
            code: crate::ids::ErrorCode(code.to_string()),
            category: ErrorCategory::Storage,
            retryable: false,
            message: message.to_string(),
            details: None,
        })
    }

    /// Encodes a machine event into a generic stream record for `run:*` streams.
    pub fn new_stream_record_for_event(
        event: Event,
        ts_millis: Option<u64>,
    ) -> Result<NewStreamRecord, StorageError> {
        let payload = serde_json::to_value(event).map_err(|_| {
            stream_encode_error(
                "machine_event_encode_failed",
                "failed to encode machine event payload",
            )
        })?;

        Ok(NewStreamRecord {
            ts_millis,
            kind: STREAM_RECORD_KIND_MACHINE_EVENT.to_string(),
            payload,
        })
    }

    /// Decodes one generic stream record from a `run:*` stream into an [`EventEnvelope`].
    pub fn event_envelope_from_stream_record(
        run_id: RunId,
        record: StreamRecord,
    ) -> Result<EventEnvelope, StorageError> {
        if record.kind != STREAM_RECORD_KIND_MACHINE_EVENT {
            return Err(stream_decode_error(
                "machine_event_record_kind_mismatch",
                "unexpected record kind in run stream",
            ));
        }

        let event = serde_json::from_value::<Event>(record.payload).map_err(|_| {
            stream_decode_error(
                "machine_event_decode_failed",
                "failed to decode machine event payload",
            )
        })?;

        Ok(EventEnvelope {
            run_id,
            seq: record.seq,
            ts_millis: record.ts_millis,
            event,
        })
    }

    /// Decodes a run stream slice into the event envelopes expected by machine runtime logic.
    pub fn event_envelopes_from_stream_records(
        run_id: RunId,
        records: Vec<StreamRecord>,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        records
            .into_iter()
            .map(|record| event_envelope_from_stream_record(run_id, record))
            .collect()
    }

    /// Recommended standard domain event payloads (not required by engine).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct FactRecorded {
        /// Fact key that was recorded.
        pub key: crate::ids::FactKey,
        /// Artifact containing the recorded payload.
        pub payload_id: ArtifactId,
        /// Operation-defined metadata about the fact.
        pub meta: serde_json::Value,
    }

    /// Recommended payload for domain events that announce artifact writes.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ArtifactWritten {
        /// Artifact identifier that was written.
        pub artifact_id: ArtifactId,
        /// Kind assigned to the written artifact.
        pub kind: crate::stores::ArtifactKind,
        /// Operation-defined metadata about the artifact.
        pub meta: serde_json::Value,
    }

    /// Recommended payload for domain events that mark op boundaries.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct OpBoundary {
        /// Nested operation path being reported.
        pub op_path: OpPath,
        /// Boundary phase, typically values such as `started` or `completed`.
        pub phase: String, // e.g. "started" | "completed"
    }

    /// Reserved for later expansion (nested machines).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ChildRunSpawned {
        /// Parent run that spawned the child.
        pub parent_run_id: RunId,
        /// Newly created child run identifier.
        pub child_run_id: RunId,
        /// Manifest artifact for the child run.
        pub child_manifest_id: ArtifactId,
    }

    /// Recommended payload for domain events that announce child-run completion.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ChildRunCompleted {
        /// Child run that finished.
        pub child_run_id: RunId,
        /// Final status of the child run.
        pub status: RunStatus,
        /// Final snapshot produced by the child run, if any.
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

/// IO request and response abstractions used by live and replay providers.
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
        /// Executes an IO request and returns the structured response.
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

/// Traits for recording operation-defined domain events during state execution.
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
        /// Emits a single domain event.
        async fn emit(&mut self, event: DomainEvent) -> Result<(), RunError>;
        /// Emits multiple domain events in order.
        async fn emit_many(&mut self, events: Vec<DomainEvent>) -> Result<(), RunError>;
    }
}

/// State trait and outcome types used by planned execution nodes.
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
        /// Do not request an additional snapshot for this state outcome.
        Never,
        /// Request a snapshot only when the state succeeds.
        OnSuccess,
        /// Request a snapshot even if the state fails.
        Always,
    }

    /// Result returned by a successful state handler.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StateOutcome {
        /// Snapshot hint returned by the state handler.
        pub snapshot: SnapshotPolicy,
    }

    /// Note:
    /// - The engine MAY still checkpoint context according to `RunConfig.context_checkpointing`
    ///   regardless of `StateOutcome.snapshot`. This hint controls additional snapshot behavior
    ///   and/or diagnostic snapshots, not permission to bypass required checkpoints.
    /// State behavior. States do NOT own their `StateId` — IDs are assigned by the plan.
    #[async_trait]
    pub trait State: Send + Sync {
        /// Returns metadata used for planning, replay, and policy decisions.
        fn meta(&self) -> StateMeta;

        /// Executes the state against the provided context, IO provider, and event recorder.
        async fn handle(
            &self,
            ctx: &mut dyn DynContext,
            io: &mut dyn IoProvider,
            rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError>;
    }

    /// Shared trait-object form used by planners and executors.
    pub type DynState = Arc<dyn State>;
}

/// Types that represent the executable state graph for a run.
pub mod plan {
    use super::*;
    use crate::ids::{OpId, StateId};
    use crate::state::DynState;

    /// An edge `from -> to` means `from` must complete before `to` can run.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct DependencyEdge {
        /// Upstream state that must complete first.
        pub from: StateId,
        /// Downstream state that depends on `from`.
        pub to: StateId,
    }

    /// Planned state plus the runtime implementation that will execute it.
    #[derive(Clone)]
    pub struct StateNode {
        /// Stable identifier assigned by the planner.
        pub id: StateId,
        /// Runtime implementation invoked for this node.
        pub state: DynState,
    }

    /// A state graph is the executable structure derived from ops/pipelines.
    #[derive(Clone)]
    pub struct StateGraph {
        /// Planned states included in the graph.
        pub states: Vec<StateNode>,
        /// Dependency edges connecting those states.
        pub edges: Vec<DependencyEdge>,
    }

    /// Operation-specific executable plan passed to the engine.
    ///
    /// The engine treats this as immutable input: planners finish all graph shaping first, then
    /// the engine executes the resulting nodes and dependency edges exactly as supplied.
    #[derive(Clone)]
    pub struct ExecutionPlan {
        /// Operation identifier associated with this plan.
        pub op_id: OpId,
        /// Executable state graph for the run.
        pub graph: StateGraph,
    }

    /// Plan validation errors (fail-fast).
    ///
    /// These errors are intended for plan construction time, before a run is started. They help
    /// callers distinguish malformed planner output from runtime failures that occur later inside
    /// the execution engine.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum PlanValidationError {
        /// The plan contained no states.
        EmptyPlan,
        /// A state identifier appeared more than once.
        DuplicateStateId {
            /// Duplicated state identifier.
            state_id: StateId,
        },
        /// A dependency edge referenced a state that does not exist in the graph.
        MissingStateForEdge {
            /// Referenced state that could not be found.
            missing: StateId,
        },
        /// The graph contained a cycle.
        CircularDependency {
            /// State identifiers participating in the detected cycle.
            cycle: Vec<StateId>,
        },

        /// Optional: if planners derive edges from tag dependencies, they may validate those too.
        DanglingDependencyTag {
            /// State whose dependency hint could not be resolved.
            state_id: StateId,
            /// Missing tag referenced by that dependency hint.
            missing_tag: crate::meta::Tag,
        },
    }

    /// Validator interface for rejecting malformed execution plans before a run starts.
    ///
    /// Typical validators enforce graph-shape rules such as:
    /// - all state identifiers are unique
    /// - every dependency edge points to an existing state
    /// - the plan is acyclic
    pub trait PlanValidator: Send + Sync {
        /// Validates the supplied execution plan.
        fn validate(&self, plan: &ExecutionPlan) -> Result<(), PlanValidationError>;
    }
}

/// Storage traits for append-only events and immutable artifacts.
pub mod stores {
    use super::*;
    use crate::errors::StorageError;
    use crate::ids::{is_valid_id_segment, ArtifactId, RunId};
    use std::fmt;

    /// Artifact classification used for retention, validation, and output handling.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ArtifactKind {
        /// Run manifest artifact.
        Manifest,
        /// Serialized context snapshot artifact.
        ContextSnapshot,
        /// Recorded fact payload artifact.
        FactPayload,
        /// Encrypted secret payload (ciphertext bytes only).
        ///
        /// Secret plaintext MUST NOT be stored directly in the artifact store.
        SecretPayload,
        /// User-facing output artifact.
        Output,
        /// Operation- or backend-specific artifact kind.
        Other(String),
    }

    /// Stable identifier for one append-only stream.
    ///
    /// Format:
    /// - `<family>:<key>`
    /// - `family` must satisfy the same identifier contract as [`crate::ids::OpId`]
    /// - `key` must be non-empty and must not contain ASCII control characters or whitespace
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(try_from = "String", into = "String")]
    pub struct StreamId(String);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct StreamIdParts<'a> {
        family: &'a str,
        key: &'a str,
    }

    impl StreamId {
        /// Creates a stream identifier after validating the storage naming contract.
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

        /// Creates a [`StreamId`] and panics if the value is invalid.
        pub fn must_new(value: impl Into<String>) -> Self {
            Self::new(value).expect("stream id must satisfy <family>:<key>")
        }

        fn parts(&self) -> StreamIdParts<'_> {
            let (family, key) = self
                .0
                .split_once(':')
                .expect("StreamId format validated in constructor");
            StreamIdParts { family, key }
        }

        fn family_and_key(&self) -> (&str, &str) {
            let parts = self.parts();
            (parts.family, parts.key)
        }

        /// Returns the stream family prefix.
        pub fn family(&self) -> &str {
            self.family_and_key().0
        }

        /// Returns the stream key suffix.
        pub fn key(&self) -> &str {
            self.family_and_key().1
        }

        /// Returns the validated stream identifier as a borrowed string slice.
        pub fn as_str(&self) -> &str {
            &self.0
        }

        /// Returns the canonical stream identifier for one machine run.
        pub fn run(run_id: RunId) -> Self {
            Self(format!("run:{}", run_id.0))
        }
    }

    impl TryFrom<String> for StreamId {
        type Error = crate::ids::IdValidationError;

        fn try_from(value: String) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl TryFrom<&str> for StreamId {
        type Error = crate::ids::IdValidationError;

        fn try_from(value: &str) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl From<StreamId> for String {
        fn from(value: StreamId) -> Self {
            value.0
        }
    }

    impl fmt::Display for StreamId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    /// Persisted record in an append-only stream.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StreamRecord {
        /// Stream to which the record belongs.
        pub stream_id: StreamId,
        /// Monotonic per-stream sequence number.
        pub seq: u64,
        /// Informational timestamp carried with the record, if any.
        pub ts_millis: Option<u64>,
        /// Stable record kind discriminator scoped by the caller.
        pub kind: String,
        /// JSON payload for the record.
        pub payload: serde_json::Value,
    }

    /// Record to append into a stream.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct NewStreamRecord {
        /// Informational timestamp carried with the record, if any.
        pub ts_millis: Option<u64>,
        /// Stable record kind discriminator scoped by the caller.
        pub kind: String,
        /// JSON payload for the record.
        pub payload: serde_json::Value,
    }

    /// Atomic compare-and-append request for one stream.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StreamAppend {
        /// Stream to append to.
        pub stream_id: StreamId,
        /// Expected current head sequence for optimistic concurrency.
        pub expected_seq: u64,
        /// Records to append in order.
        pub records: Vec<NewStreamRecord>,
    }

    impl StreamAppend {
        /// Creates a stream append request.
        pub fn new(stream_id: StreamId, expected_seq: u64, records: Vec<NewStreamRecord>) -> Self {
            Self {
                stream_id,
                expected_seq,
                records,
            }
        }
    }

    /// Head sequences returned by an atomic multi-stream append.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct AppendBatchResult {
        /// Final head sequence for each stream touched by the batch.
        pub stream_heads: Vec<(StreamId, u64)>,
    }

    impl AppendBatchResult {
        /// Returns the recorded head for `stream_id`, if present.
        pub fn head_for(&self, stream_id: &StreamId) -> Option<u64> {
            self.stream_heads.iter().find_map(
                |(id, head)| {
                    if id == stream_id {
                        Some(*head)
                    } else {
                        None
                    }
                },
            )
        }
    }

    /// Append-only stream store with optimistic concurrency.
    #[async_trait]
    pub trait StreamStore: Send + Sync {
        /// Returns the current head sequence for `stream_id`.
        async fn head_seq(&self, stream_id: &StreamId) -> Result<u64, StorageError>;

        /// Appends records atomically if `expected_seq` matches the current head.
        async fn append(&self, append: StreamAppend) -> Result<u64, StorageError>;

        /// Appends records to multiple streams atomically.
        async fn append_batch(
            &self,
            appends: Vec<StreamAppend>,
        ) -> Result<AppendBatchResult, StorageError>;

        /// Reads a contiguous record range starting at `from_seq`.
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
        /// Stores bytes under a content-derived identifier and returns that identifier.
        async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>)
            -> Result<ArtifactId, StorageError>;
        /// Loads the bytes for an existing artifact identifier.
        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError>;
        /// Returns whether an artifact exists without loading its bytes.
        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError>;
    }
}

/// Engine traits and input/output types for starting or resuming runs.
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
        /// The run is still executing.
        Running,
        /// The run completed successfully.
        Completed,
        /// The run ended because of a failure.
        Failed,
        /// The run was cancelled.
        Cancelled,
    }

    /// Summary returned by the execution engine after start or resume.
    ///
    /// `phase` reports the final or current lifecycle state, while `final_snapshot_id` is populated
    /// only when a snapshot exists at the point the engine returns.
    ///
    /// This is intentionally compact so transport layers can expose a stable run result without
    /// forcing callers to inspect raw event streams for the common success/failure path.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunResult {
        /// Run identifier that was started or resumed.
        pub run_id: RunId,
        /// Final or current phase of the run.
        pub phase: RunPhase,
        /// Final context snapshot, if one exists.
        pub final_snapshot_id: Option<ArtifactId>,
    }

    /// Inputs required to start a run.
    ///
    /// Callers usually construct this once planning is complete and the manifest has already been
    /// content-addressed and persisted or prepared for persistence.
    ///
    /// Keeping the manifest, derived plan, run config, and initial context together makes the
    /// engine boundary explicit: everything above this type is planning/orchestration, everything
    /// below it is execution and persistence.
    pub struct StartRun {
        /// Manifest content for the run.
        pub manifest: RunManifest,
        /// Content-addressed manifest identifier.
        pub manifest_id: ArtifactId,
        /// Execution plan derived for the manifest.
        pub plan: ExecutionPlan,
        /// Effective run configuration used by the engine.
        pub run_config: RunConfig,
        /// Initial context snapshot used as the run's starting point.
        pub initial_context: Box<dyn DynContext>,
    }

    /// Store bundle passed to the engine.
    ///
    /// Keeping the stores grouped makes it easier for higher layers to swap persistence backends
    /// without threading each store separately through every engine constructor.
    #[derive(Clone)]
    pub struct Stores {
        /// Stream store used for append-only run events and other stream families.
        pub streams: Arc<dyn StreamStore>,
        /// Artifact store used for manifests, snapshots, facts, and outputs.
        pub artifacts: Arc<dyn ArtifactStore>,
    }

    /// Execution engine interface.
    #[async_trait]
    pub trait ExecutionEngine: Send + Sync {
        /// Starts a new run from the supplied manifest, plan, and initial context.
        async fn start(&self, stores: Stores, run: StartRun) -> Result<RunResult, RunError>;
        /// Resumes a previously started run from persisted state.
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

/// Unstable shared process execution helpers for live IO transports.
///
/// Not part of the stable API contract (Appendix C.1).
pub mod process_exec;

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
