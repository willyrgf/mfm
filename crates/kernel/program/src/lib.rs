#![warn(missing_docs)]
//! Typed state-program authoring contracts for MFM.
//!
//! This crate owns the branded authoring surface for typed programs. Handles
//! are minted only by framework builders, carry invariant program/scope/value
//! brands, and are lowered into unbranded draft specs only after root public
//! outputs have been bound inside the generative build closure.

extern crate self as mfm_program;

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySet, CapabilitySetDescriptor, CapabilitySetFor, NoCaps};
use mfm_effects::{
    ApplySideEffect, EffectDescriptor, EffectSpec, ManagedPlatformWrite, Pure, ReadExternal,
};
use mfm_ids::{
    CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, NodeId, SchemaId, ScopeId,
    SeedId, SemanticTypeId, StateKind, StateVersion,
};
pub use mfm_values::NonEmpty;
use mfm_values::{
    MfmConfig, MfmValue, PublicOutputDescriptor, SchemaShape, StateInput, ValueTerminalPolicy,
};

#[cfg(test)]
mod tests;

/// Result type for typed program authoring operations.
pub type Result<T> = std::result::Result<T, PlanError>;

/// Error returned by typed program authoring operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// Author-supplied stable key failed validation.
    Key(String),
    /// Value descriptor construction failed.
    Value(String),
    /// Canonical seed construction failed.
    Canonical(String),
    /// JSON serialization failed before canonicalization.
    Serialize(String),
    /// A root seed key was declared more than once.
    DuplicateSeedKey(String),
    /// A child scope key was declared more than once under the same parent.
    DuplicateChildScopeKey(String),
    /// A bridge key was declared more than once in the same child scope session.
    DuplicateBridgeKey(String),
    /// A state key was declared more than once in the same scope.
    DuplicateStateKey(String),
    /// A public output field path was declared more than once.
    DuplicatePublicOutputPath(String),
    /// Root public outputs were bound more than once.
    PublicOutputsAlreadyBound,
    /// Public output binding must contain at least one cell.
    EmptyPublicOutputs,
    /// A non-empty input collection was empty.
    EmptyNonEmptyInput,
    /// A derived input struct had the same field path more than once.
    DuplicateInputFieldPath(String),
    /// Input binding tree did not match the declared state input descriptor.
    InputBindingShape(String),
    /// State registry authority rejected planning.
    Registry(String),
    /// Live bridge evidence did not belong to the active child scope session.
    InvalidBridgeEvidence(String),
    /// A persisted bridge reference was not backed by an emitted bridge node.
    UnknownBridgeRef,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(message) => write!(f, "invalid typed program key: {message}"),
            Self::Value(message) => write!(f, "typed value descriptor error: {message}"),
            Self::Canonical(message) => write!(f, "canonical seed error: {message}"),
            Self::Serialize(message) => write!(f, "seed serialization error: {message}"),
            Self::DuplicateSeedKey(key) => write!(f, "duplicate root seed key {key}"),
            Self::DuplicateChildScopeKey(key) => write!(f, "duplicate child scope key {key}"),
            Self::DuplicateBridgeKey(key) => write!(f, "duplicate bridge key {key}"),
            Self::DuplicateStateKey(key) => write!(f, "duplicate state key {key}"),
            Self::DuplicatePublicOutputPath(path) => {
                write!(f, "duplicate public output field path {path}")
            }
            Self::PublicOutputsAlreadyBound => f.write_str("root public outputs already bound"),
            Self::EmptyPublicOutputs => f.write_str("root public output binding is empty"),
            Self::EmptyNonEmptyInput => f.write_str("non-empty input collection is empty"),
            Self::DuplicateInputFieldPath(path) => {
                write!(f, "duplicate input field path {path}")
            }
            Self::InputBindingShape(message) => {
                write!(f, "input binding shape mismatch: {message}")
            }
            Self::Registry(message) => write!(f, "state registry error: {message}"),
            Self::InvalidBridgeEvidence(message) => {
                write!(f, "invalid bridge evidence: {message}")
            }
            Self::UnknownBridgeRef => f.write_str("bridge ref is not backed by an emitted node"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Stable root or child scope author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeKey(String);

impl ScopeKey {
    /// Creates a checked scope key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("scope key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable root seed author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeedKey(String);

impl SeedKey {
    /// Creates a checked seed key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("seed key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable public-output binding key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicOutputKey(String);

impl PublicOutputKey {
    /// Creates a checked public-output key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("public output key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable bridge node author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BridgeKey(String);

impl BridgeKey {
    /// Creates a checked bridge key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("bridge key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable state node author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateKey(String);

impl StateKey {
    /// Creates a checked state key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("state key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable public-output field path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicFieldPath(String);

impl PublicFieldPath {
    /// Creates a checked public field path.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_field_path("public field path", value.as_ref()).map(Self)
    }

    /// Returns the stable field path string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable field path inside a state input binding tree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputFieldPath(String);

impl InputFieldPath {
    /// Creates the root input field path.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Creates a checked input field path.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_field_path("input field path", value.as_ref()).map(Self)
    }

    /// Appends a checked child segment.
    pub fn child(&self, value: impl AsRef<str>) -> Result<Self> {
        let segment = checked_field_segment("input field segment", value.as_ref())?;
        if self.0.is_empty() {
            Ok(Self(segment))
        } else {
            Ok(Self(format!("{}.{}", self.0, segment)))
        }
    }

    /// Returns the stable field path string. The root path is the empty string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical launch seed material for a typed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSeed<T: MfmValue> {
    bytes: PlainCanonicalJsonBytes,
    content_digest: ContentDigest,
    byte_len: usize,
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    _value: PhantomData<fn(T) -> T>,
}

impl<T: MfmValue> CanonicalSeed<T> {
    /// Serializes and canonicalizes a launch seed value.
    pub fn from_value(value: &T) -> Result<Self> {
        let json = serde_json::to_string(value)
            .map_err(|error| PlanError::Serialize(error.to_string()))?;
        let bytes = PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|error| PlanError::Canonical(error.to_string()))?;
        Self::from_canonical_json(bytes)
    }

    /// Creates seed material from already-canonical JSON bytes.
    pub fn from_canonical_json(bytes: PlainCanonicalJsonBytes) -> Result<Self> {
        let _: T = serde_json::from_slice(bytes.as_bytes()).map_err(|error| {
            PlanError::Canonical(format!("seed bytes do not decode as value type: {error}"))
        })?;
        let schema_id = T::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let semantic_type_id =
            T::semantic_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let content_digest = bytes.content_digest();
        let byte_len = bytes.as_bytes().len();
        Ok(Self {
            bytes,
            content_digest,
            byte_len,
            schema_id,
            semantic_type_id,
            _value: PhantomData,
        })
    }

    /// Returns canonical JSON bytes for this seed.
    pub fn canonical_json(&self) -> &PlainCanonicalJsonBytes {
        &self.bytes
    }

    /// Returns the seed content digest.
    pub fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    /// Returns the canonical byte length.
    pub fn byte_len(&self) -> usize {
        self.byte_len
    }
}

/// Error returned while executing a typed state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// Stable, redacted state error message.
    Message(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for StateError {}

/// Result type returned by executable state traits.
pub type StateResult<T> = std::result::Result<T, StateError>;

/// Descriptive contract for a versioned executable state.
pub trait StateSpec: Send + Sync + 'static {
    /// Deterministic planning config type.
    type Config: MfmConfig;
    /// Runtime input type materialized from certified input bindings.
    type Input: StateInput;
    /// Runtime output value type.
    type Output: MfmValue;
    /// Framework-owned effect class.
    type Effect: EffectSpec;
    /// Capability set required by this state.
    type Caps: CapabilitySet;

    /// Returns the stable state kind id.
    fn kind() -> Result<StateKind>;

    /// Returns the state descriptor version.
    fn version() -> Result<StateVersion>;

    /// Returns the stable state descriptor name.
    fn name() -> &'static str;

    /// Constructs the executable state from validated config.
    fn new(config: Self::Config) -> Result<Self>
    where
        Self: Sized;
}

/// Effect-specific runner kind recorded by a registered state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunnerKind {
    /// Pure synchronous state runner.
    Pure,
    /// External read runner.
    ReadExternal,
    /// MFM-managed platform write runner.
    ManagedPlatformWrite,
    /// External side-effect runner.
    ApplySideEffect,
}

impl RunnerKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::ReadExternal => "read_external",
            Self::ManagedPlatformWrite => "managed_platform_write",
            Self::ApplySideEffect => "apply_side_effect",
        }
    }
}

/// Hash-defining registered state descriptor identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDescriptorIdentity {
    descriptor_id: DescriptorId,
    kind: StateKind,
    version: StateVersion,
    name: &'static str,
    config_schema_id: SchemaId,
    input_schema_id: SchemaId,
    output_schema_id: SchemaId,
    output_semantic_type_id: SemanticTypeId,
    effect: EffectDescriptor,
    capabilities: CapabilitySetDescriptor,
    runner: RunnerKind,
}

impl StateDescriptorIdentity {
    fn for_state<S: StateSpec>() -> Result<Self> {
        let kind = S::kind()?;
        let version = S::version()?;
        let config_schema_id =
            S::Config::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let input_schema_id =
            S::Input::input_schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let output_schema_id =
            S::Output::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let output_semantic_type_id =
            S::Output::semantic_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let effect =
            S::Effect::descriptor().map_err(|error| PlanError::Registry(error.to_string()))?;
        let capabilities =
            S::Caps::descriptor().map_err(|error| PlanError::Registry(error.to_string()))?;
        capabilities
            .validate_for_effect::<S::Effect>()
            .map_err(|error| PlanError::Registry(error.to_string()))?;
        let runner = effect_runner_kind_for_effect::<S::Effect>();
        let descriptor_id = state_descriptor_id(StateDescriptorIdParts {
            kind: &kind,
            version: &version,
            name: S::name(),
            config_schema_id: &config_schema_id,
            input_schema_id: &input_schema_id,
            output_schema_id: &output_schema_id,
            output_semantic_type_id: &output_semantic_type_id,
            effect: &effect,
            capabilities: &capabilities,
            runner,
        })?;
        Ok(Self {
            descriptor_id,
            kind,
            version,
            name: S::name(),
            config_schema_id,
            input_schema_id,
            output_schema_id,
            output_semantic_type_id,
            effect,
            capabilities,
            runner,
        })
    }

    /// Returns this descriptor's content-addressed identity.
    pub fn descriptor_id(&self) -> &DescriptorId {
        &self.descriptor_id
    }

    /// Returns the stable state kind.
    pub fn kind(&self) -> &StateKind {
        &self.kind
    }

    /// Returns the state version.
    pub fn version(&self) -> &StateVersion {
        &self.version
    }

    /// Returns the stable state descriptor name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the config schema id.
    pub fn config_schema_id(&self) -> &SchemaId {
        &self.config_schema_id
    }

    /// Returns the input schema id.
    pub fn input_schema_id(&self) -> &SchemaId {
        &self.input_schema_id
    }

    /// Returns the output schema id.
    pub fn output_schema_id(&self) -> &SchemaId {
        &self.output_schema_id
    }

    /// Returns the output semantic type id.
    pub fn output_semantic_type_id(&self) -> &SemanticTypeId {
        &self.output_semantic_type_id
    }

    /// Returns the effect descriptor.
    pub fn effect(&self) -> &EffectDescriptor {
        &self.effect
    }

    /// Returns the capability-set descriptor.
    pub fn capabilities(&self) -> &CapabilitySetDescriptor {
        &self.capabilities
    }

    /// Returns the registered runner kind.
    pub fn runner(&self) -> RunnerKind {
        self.runner
    }
}

/// Error returned by state registry operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// State descriptor construction failed.
    Descriptor(String),
    /// No registered state matched the requested kind/version.
    UnregisteredState {
        /// Requested state kind.
        kind: String,
        /// Requested state version.
        version: String,
    },
    /// A different descriptor already owns this kind/version pair.
    DuplicateStateRegistration {
        /// Registered state kind.
        kind: String,
        /// Registered state version.
        version: String,
    },
    /// Registry record and descriptor evidence diverged.
    DescriptorMismatch {
        /// Registered state kind.
        kind: String,
        /// Registered state version.
        version: String,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Descriptor(message) => write!(f, "state descriptor error: {message}"),
            Self::UnregisteredState { kind, version } => {
                write!(f, "state {kind}@{version} is not registered")
            }
            Self::DuplicateStateRegistration { kind, version } => {
                write!(f, "duplicate state registration for {kind}@{version}")
            }
            Self::DescriptorMismatch { kind, version } => {
                write!(f, "state registry descriptor mismatch for {kind}@{version}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<RegistryError> for PlanError {
    fn from(error: RegistryError) -> Self {
        Self::Registry(error.to_string())
    }
}

/// Private registration evidence carried by a registered state token.
#[derive(Debug, PartialEq, Eq)]
pub struct StateRegistrationEvidence<S: StateSpec> {
    _state: PhantomData<fn(S) -> S>,
    _private: (),
}

impl<S: StateSpec> Clone for StateRegistrationEvidence<S> {
    fn clone(&self) -> Self {
        Self {
            _state: PhantomData,
            _private: (),
        }
    }
}

/// Framework-owned authority token for a registered state.
#[derive(Debug, PartialEq, Eq)]
pub struct RegisteredState<S: StateSpec> {
    descriptor: StateDescriptorIdentity,
    runner: RunnerKind,
    evidence: StateRegistrationEvidence<S>,
    _state: PhantomData<fn(S) -> S>,
}

impl<S: StateSpec> Clone for RegisteredState<S> {
    fn clone(&self) -> Self {
        Self {
            descriptor: self.descriptor.clone(),
            runner: self.runner,
            evidence: self.evidence.clone(),
            _state: PhantomData,
        }
    }
}

impl<S: StateSpec> RegisteredState<S> {
    fn new(descriptor: StateDescriptorIdentity, runner: RunnerKind) -> Self {
        Self {
            descriptor,
            runner,
            evidence: StateRegistrationEvidence {
                _state: PhantomData,
                _private: (),
            },
            _state: PhantomData,
        }
    }

    /// Returns the validated state descriptor identity.
    pub fn descriptor(&self) -> &StateDescriptorIdentity {
        &self.descriptor
    }

    /// Returns the registered runner kind.
    pub fn runner(&self) -> RunnerKind {
        self.runner
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StateRegistrationKey {
    kind: String,
    version: String,
}

impl StateRegistrationKey {
    fn from_descriptor(descriptor: &StateDescriptorIdentity) -> Self {
        Self {
            kind: descriptor.kind().as_str().to_owned(),
            version: descriptor.version().as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateRegistrationRecord {
    descriptor_id: DescriptorId,
    runner: RunnerKind,
}

/// Immutable state registry snapshot used by typed program builders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateRegistrySnapshot {
    records: std::collections::BTreeMap<StateRegistrationKey, StateRegistrationRecord>,
}

impl StateRegistrySnapshot {
    /// Returns true when the registry has no registered states.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Returns the number of registered state kind/version pairs.
    pub fn len(&self) -> usize {
        self.records.len()
    }
}

/// Mutable framework state registry builder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateRegistryBuilder {
    snapshot: StateRegistrySnapshot,
}

impl StateRegistryBuilder {
    /// Creates an empty registry builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a state after validating descriptor, effect, capability, and runner evidence.
    pub fn register<S>(&mut self) -> std::result::Result<RegisteredState<S>, RegistryError>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = StateDescriptorIdentity::for_state::<S>()
            .map_err(|error| RegistryError::Descriptor(error.to_string()))?;
        let runner = <S::Effect as EffectRunner<S>>::runner_kind();
        if descriptor.runner() != runner {
            return Err(RegistryError::Descriptor(format!(
                "runner {:?} did not match descriptor runner {:?}",
                runner,
                descriptor.runner()
            )));
        }
        let key = StateRegistrationKey::from_descriptor(&descriptor);
        let record = StateRegistrationRecord {
            descriptor_id: descriptor.descriptor_id().clone(),
            runner,
        };
        if let Some(existing) = self.snapshot.records.get(&key) {
            if existing != &record {
                return Err(RegistryError::DuplicateStateRegistration {
                    kind: key.kind,
                    version: key.version,
                });
            }
        } else {
            self.snapshot.records.insert(key, record);
        }
        Ok(RegisteredState::new(descriptor, runner))
    }

    /// Returns an immutable registry snapshot.
    pub fn snapshot(&self) -> StateRegistrySnapshot {
        self.snapshot.clone()
    }

    /// Converts this builder into an immutable registry snapshot.
    pub fn into_snapshot(self) -> StateRegistrySnapshot {
        self.snapshot
    }
}

/// Framework-owned registry lookup contract.
pub trait StateRegistry {
    /// Resolves a registered state token for `S`.
    fn registered_state<S>(&self) -> std::result::Result<RegisteredState<S>, RegistryError>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>;
}

impl StateRegistry for StateRegistrySnapshot {
    fn registered_state<S>(&self) -> std::result::Result<RegisteredState<S>, RegistryError>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = StateDescriptorIdentity::for_state::<S>()
            .map_err(|error| RegistryError::Descriptor(error.to_string()))?;
        let key = StateRegistrationKey::from_descriptor(&descriptor);
        let Some(record) = self.records.get(&key) else {
            return Err(RegistryError::UnregisteredState {
                kind: key.kind,
                version: key.version,
            });
        };
        if record.descriptor_id != *descriptor.descriptor_id()
            || record.runner != descriptor.runner()
        {
            return Err(RegistryError::DescriptorMismatch {
                kind: key.kind,
                version: key.version,
            });
        }
        Ok(RegisteredState::new(descriptor, record.runner))
    }
}

/// Sealed framework evidence that an effect has the matching state runner shape.
pub trait EffectRunner<S: StateSpec>: private::EffectRunnerSealed<S> {
    /// Returns the runner kind for this effect/state pair.
    fn runner_kind() -> RunnerKind;
}

/// Pure deterministic state runner.
pub trait PureState: StateSpec<Effect = Pure, Caps = NoCaps> {
    /// Executes this pure state.
    fn run(&self, input: Self::Input) -> StateResult<Self::Output>;
}

/// External read state runner.
pub trait ReadState: StateSpec<Effect = ReadExternal> {
    /// Future returned by [`ReadState::run`].
    type RunFuture<'a>: Future<Output = StateResult<Self::Output>> + Send + 'a
    where
        Self: 'a;

    /// Executes this read state through declared capabilities.
    fn run<'a>(&'a self, input: Self::Input, caps: &'a Self::Caps) -> Self::RunFuture<'a>;
}

/// MFM-managed platform write state runner.
pub trait ManagedWriteState: StateSpec<Effect = ManagedPlatformWrite> {
    /// Future returned by [`ManagedWriteState::run`].
    type RunFuture<'a>: Future<Output = StateResult<Self::Output>> + Send + 'a
    where
        Self: 'a;

    /// Executes this managed write through declared capabilities.
    fn run<'a>(&'a self, input: Self::Input, caps: &'a Self::Caps) -> Self::RunFuture<'a>;
}

/// Typed idempotency key for side-effect submission protocols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyKey<T: MfmValue> {
    digest: ContentDigest,
    _input: PhantomData<fn(T) -> T>,
}

impl<T: MfmValue> IdempotencyKey<T> {
    /// Creates an idempotency key from a typed digest.
    pub fn new(digest: ContentDigest) -> Self {
        Self {
            digest,
            _input: PhantomData,
        }
    }

    /// Returns the idempotency digest.
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// External mutation state runner governed by a side-effect protocol.
pub trait SideEffectState: StateSpec<Effect = ApplySideEffect> {
    /// Deterministic mutation intent.
    type Intent: MfmValue;
    /// Deterministic idempotency input.
    type IdempotencyInput: MfmValue;
    /// Submission result value.
    type Submission: MfmValue;
    /// Receipt value observed after submission.
    type Receipt: MfmValue;
    /// Confirmation value used to produce terminal output.
    type Confirmation: MfmValue;
    /// Future returned by [`SideEffectState::submit`].
    type SubmitFuture<'a>: Future<Output = StateResult<Self::Submission>> + Send + 'a
    where
        Self: 'a;

    /// Builds a deterministic mutation intent from materialized input.
    fn prepare_intent(&self, input: &Self::Input) -> StateResult<Self::Intent>;

    /// Builds deterministic idempotency input from materialized input and intent.
    fn idempotency_input(
        &self,
        input: &Self::Input,
        intent: &Self::Intent,
    ) -> StateResult<Self::IdempotencyInput>;

    /// Submits the intent through declared capabilities.
    fn submit<'a>(
        &'a self,
        intent: &'a Self::Intent,
        key: &'a IdempotencyKey<Self::IdempotencyInput>,
        caps: &'a Self::Caps,
    ) -> Self::SubmitFuture<'a>;

    /// Constructs terminal output from confirmed side-effect evidence.
    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        intent: &Self::Intent,
        confirmation: &Self::Confirmation,
    ) -> StateResult<Self::Output>;
}

impl<S> EffectRunner<S> for Pure
where
    S: PureState,
{
    fn runner_kind() -> RunnerKind {
        RunnerKind::Pure
    }
}

impl<S> EffectRunner<S> for ReadExternal
where
    S: ReadState,
{
    fn runner_kind() -> RunnerKind {
        RunnerKind::ReadExternal
    }
}

impl<S> EffectRunner<S> for ManagedPlatformWrite
where
    S: ManagedWriteState,
{
    fn runner_kind() -> RunnerKind {
        RunnerKind::ManagedPlatformWrite
    }
}

impl<S> EffectRunner<S> for ApplySideEffect
where
    S: SideEffectState,
{
    fn runner_kind() -> RunnerKind {
        RunnerKind::ApplySideEffect
    }
}

/// Reference to a value-lineage record used by input bindings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueLineageRef {
    /// Digest identifying the value-lineage record.
    digest: ContentDigest,
}

impl ValueLineageRef {
    /// Creates a lineage reference from an already typed digest.
    pub fn new(digest: ContentDigest) -> Self {
        Self { digest }
    }

    /// Returns the lineage digest.
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// Non-forgeable typed reference to a planned cell.
#[derive(Debug, PartialEq, Eq)]
pub struct Handle<'program, 'scope, T: MfmValue> {
    cell_id: CellId,
    scope_id: ScopeId,
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    value_lineage: ValueLineageRef,
    origin: HandleOrigin,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    _value: PhantomData<fn(T) -> T>,
}

impl<'program, 'scope, T: MfmValue> Clone for Handle<'program, 'scope, T> {
    fn clone(&self) -> Self {
        Self {
            cell_id: self.cell_id.clone(),
            scope_id: self.scope_id.clone(),
            schema_id: self.schema_id.clone(),
            semantic_type_id: self.semantic_type_id.clone(),
            value_lineage: self.value_lineage.clone(),
            origin: self.origin.clone(),
            _program: PhantomData,
            _scope: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<'program, 'scope, T: MfmValue> Handle<'program, 'scope, T> {
    fn new(
        cell_id: CellId,
        scope_id: ScopeId,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        value_lineage: ValueLineageRef,
    ) -> Self {
        Self {
            cell_id,
            scope_id,
            schema_id,
            semantic_type_id,
            value_lineage,
            origin: HandleOrigin::Local,
            _program: PhantomData,
            _scope: PhantomData,
            _value: PhantomData,
        }
    }

    fn new_bridge(
        cell_id: CellId,
        scope_id: ScopeId,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        value_lineage: ValueLineageRef,
        evidence: BridgeEvidenceCore,
    ) -> Self {
        Self {
            cell_id,
            scope_id,
            schema_id,
            semantic_type_id,
            value_lineage,
            origin: HandleOrigin::Bridge {
                evidence: Box::new(evidence),
            },
            _program: PhantomData,
            _scope: PhantomData,
            _value: PhantomData,
        }
    }

    /// Returns an unbranded typed handle reference for descriptors.
    pub fn typed_ref(&self) -> TypedHandleRef {
        TypedHandleRef {
            cell_id: self.cell_id.clone(),
            scope_id: self.scope_id.clone(),
            schema_id: self.schema_id.clone(),
            semantic_type_id: self.semantic_type_id.clone(),
            value_lineage: self.value_lineage.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HandleOrigin {
    Local,
    Bridge { evidence: Box<BridgeEvidenceCore> },
}

impl HandleOrigin {
    fn bridge_evidence<'program, 'parent>(&self) -> Vec<BridgeEvidence<'program, 'parent>> {
        match self {
            Self::Local => Vec::new(),
            Self::Bridge { evidence } => vec![BridgeEvidence {
                core: (**evidence).clone(),
                _program: PhantomData,
                _parent: PhantomData,
                _private: (),
            }],
        }
    }
}

/// Unbranded typed handle reference emitted into draft specs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedHandleRef {
    /// Planned cell id.
    cell_id: CellId,
    /// Planned scope id.
    scope_id: ScopeId,
    /// Value schema id.
    schema_id: SchemaId,
    /// Value semantic type id.
    semantic_type_id: SemanticTypeId,
    /// Value lineage reference.
    value_lineage: ValueLineageRef,
}

impl TypedHandleRef {
    /// Returns the planned cell id.
    pub fn cell_id(&self) -> &CellId {
        &self.cell_id
    }

    /// Returns the planned scope id.
    pub fn scope_id(&self) -> &ScopeId {
        &self.scope_id
    }

    /// Returns the value schema id.
    pub fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns the value semantic type id.
    pub fn semantic_type_id(&self) -> &SemanticTypeId {
        &self.semantic_type_id
    }

    /// Returns the value lineage reference.
    pub fn value_lineage(&self) -> &ValueLineageRef {
        &self.value_lineage
    }
}

/// Root seed cell specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSeedSpec {
    /// Stable seed author key.
    pub key: SeedKey,
    /// Derived seed id.
    pub seed_id: SeedId,
    /// Seed output cell id.
    pub cell_id: CellId,
    /// Root scope id.
    pub scope_id: ScopeId,
    /// Seed value schema id.
    pub schema_id: SchemaId,
    /// Seed value semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Canonical seed content digest.
    pub content_digest: ContentDigest,
    /// Canonical seed byte length.
    pub byte_len: usize,
}

/// Persisted typed scope specification emitted by the program builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSpec {
    /// Stable scope author key.
    pub key: ScopeKey,
    /// Derived scope id.
    pub scope_id: ScopeId,
    /// Parent scope id for child scopes.
    pub parent_scope_id: Option<ScopeId>,
}

/// Canonical config reference embedded in a typed state node draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigBindingSpec {
    /// Config schema id.
    pub schema_id: SchemaId,
    /// Canonical config bytes.
    pub canonical_json: PlainCanonicalJsonBytes,
    /// Canonical config content digest.
    pub content_digest: ContentDigest,
    /// Canonical byte length.
    pub byte_len: usize,
}

/// Persisted typed state node draft emitted by the program builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateNodeSpec {
    /// Derived state node id.
    pub node_id: NodeId,
    /// Stable state author key.
    pub key: StateKey,
    /// Owning scope id.
    pub scope_id: ScopeId,
    /// Registered state kind.
    pub state_kind: StateKind,
    /// Registered state version.
    pub state_version: StateVersion,
    /// Registered state descriptor id.
    pub state_descriptor_id: DescriptorId,
    /// Registered runner kind.
    pub runner: RunnerKind,
    /// Canonical config binding.
    pub config: ConfigBindingSpec,
    /// Typed input binding.
    pub input: InputBindingSpec,
    /// Output cell id.
    pub output_cell_id: CellId,
    /// Output schema id.
    pub output_schema_id: SchemaId,
    /// Output semantic type id.
    pub output_semantic_type_id: SemanticTypeId,
}

/// Framework bridge direction for same-value cross-scope movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeKind {
    /// Parent value imported into a child scope.
    ImportFromParent,
    /// Child value exported into the parent scope.
    ExportToParent,
}

impl BridgeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ImportFromParent => "import-from-parent",
            Self::ExportToParent => "export-to-parent",
        }
    }
}

/// Bridge policy supported by typed program v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgePolicy {
    /// Same-run, same-value movement with no semantic transform.
    SameRunSameValueV1,
}

impl BridgePolicy {
    /// Returns the v1 same-run same-value bridge policy.
    pub const fn same_run_same_value() -> Self {
        Self::SameRunSameValueV1
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::SameRunSameValueV1 => "same-run-same-value-v1",
        }
    }
}

/// Framework provenance for an emitted bridge node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeProvenance {
    /// Bridge emitted by the typed kernel child-scope builder.
    FrameworkChildScopeV1,
}

/// Persisted bridge reference. This is audit evidence, not live authority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BridgeRef {
    /// Source scope id.
    pub source_scope_id: ScopeId,
    /// Target scope id.
    pub target_scope_id: ScopeId,
    /// Source cell id.
    pub source_cell_id: CellId,
    /// Target cell id created by the bridge.
    pub target_cell_id: CellId,
    /// Value semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Value schema id.
    pub schema_id: SchemaId,
    /// Framework bridge node id.
    pub bridge_node_id: NodeId,
}

/// Bridge node specification emitted into the typed program draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeNodeSpec {
    /// Derived bridge node id.
    pub node_id: NodeId,
    /// Stable bridge author key.
    pub key: BridgeKey,
    /// Source scope id.
    pub source_scope_id: ScopeId,
    /// Target scope id.
    pub target_scope_id: ScopeId,
    /// Source cell id.
    pub source_cell_id: CellId,
    /// Target cell id.
    pub target_cell_id: CellId,
    /// Value semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Value schema id.
    pub schema_id: SchemaId,
    /// Bridge direction.
    pub bridge_kind: BridgeKind,
    /// Same-value bridge policy.
    pub policy: BridgePolicy,
    /// Framework provenance.
    pub provenance: BridgeProvenance,
}

impl BridgeNodeSpec {
    /// Returns the persisted bridge reference for this emitted node.
    pub fn bridge_ref(&self) -> BridgeRef {
        BridgeRef {
            source_scope_id: self.source_scope_id.clone(),
            target_scope_id: self.target_scope_id.clone(),
            source_cell_id: self.source_cell_id.clone(),
            target_cell_id: self.target_cell_id.clone(),
            semantic_type_id: self.semantic_type_id.clone(),
            schema_id: self.schema_id.clone(),
            bridge_node_id: self.node_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BridgeSessionToken(DigestBytes);

#[derive(Debug, Clone, PartialEq, Eq)]
struct BridgeEvidenceCore {
    bridge_ref: BridgeRef,
    session_token: BridgeSessionToken,
}

/// Live bridge authority owned by one active child-scope builder invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvidence<'program, 'parent> {
    core: BridgeEvidenceCore,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
    _private: (),
}

impl<'program, 'parent> BridgeEvidence<'program, 'parent> {
    /// Returns the persisted bridge reference carried by this live evidence.
    pub fn bridge_ref(&self) -> &BridgeRef {
        &self.core.bridge_ref
    }
}

/// Public-output cell binding specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputCellSpec {
    /// Stable public field path.
    public_field_path: PublicFieldPath,
    /// Bound typed cell reference.
    cell: TypedHandleRef,
}

impl PublicOutputCellSpec {
    /// Creates a public-output cell binding from a branded typed handle.
    pub fn from_handle<'program, 'scope, T: MfmValue>(
        public_field_path: PublicFieldPath,
        handle: &Handle<'program, 'scope, T>,
    ) -> Self {
        Self {
            public_field_path,
            cell: handle.typed_ref(),
        }
    }

    /// Returns the public field path.
    pub fn public_field_path(&self) -> &PublicFieldPath {
        &self.public_field_path
    }

    /// Returns the bound typed cell reference.
    pub fn cell(&self) -> &TypedHandleRef {
        &self.cell
    }
}

/// Public-output binding specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputSpec {
    /// Stable public-output binding key.
    key: PublicOutputKey,
    /// Public output schema id.
    public_schema_id: SchemaId,
    /// Bound output cells.
    outputs: Vec<PublicOutputCellSpec>,
}

impl PublicOutputSpec {
    /// Returns the stable public-output binding key.
    pub fn key(&self) -> &PublicOutputKey {
        &self.key
    }

    /// Returns the public output schema id.
    pub fn public_schema_id(&self) -> &SchemaId {
        &self.public_schema_id
    }

    /// Returns bound output cells.
    pub fn outputs(&self) -> &[PublicOutputCellSpec] {
        &self.outputs
    }
}

/// Required terminal behavior for an input cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequiredTerminal {
    /// The input requires a produced value.
    ProducedOnly,
    /// The input accepts produced or skipped optional values.
    MaybeSkipped,
}

impl RequiredTerminal {
    fn from_value_policy(policy: ValueTerminalPolicy) -> Self {
        match policy {
            ValueTerminalPolicy::ProducedOnly => Self::ProducedOnly,
            ValueTerminalPolicy::MaybeSkipped => Self::MaybeSkipped,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::ProducedOnly => "produced_only",
            Self::MaybeSkipped => "maybe_skipped",
        }
    }
}

/// Ordering evidence for vector input bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderingEvidence {
    /// Author-provided vector order.
    ExplicitAuthorOrder,
}

impl OrderingEvidence {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitAuthorOrder => "explicit_author_order",
        }
    }
}

/// Named field binding inside a struct input binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedInputBinding {
    /// Field path for this named binding.
    pub field_path: InputFieldPath,
    /// Binding node for this field.
    pub node: InputBindingNode,
}

impl NamedInputBinding {
    /// Creates a named input binding.
    pub fn new(field_path: InputFieldPath, node: InputBindingNode) -> Self {
        Self { field_path, node }
    }
}

/// Canonical typed state-input binding tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBindingNode {
    kind: InputBindingNodeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputBindingNodeKind {
    Unit,
    Cell(Box<InputCellBinding>),
    Tuple {
        elements: Vec<InputBindingNode>,
    },
    Struct {
        fields: Vec<NamedInputBinding>,
    },
    Vec {
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
    },
    NonEmptyVec {
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputCellBinding {
    field_path: InputFieldPath,
    cell_id: CellId,
    semantic_type_id: SemanticTypeId,
    schema_id: SchemaId,
    value_lineage: ValueLineageRef,
    required_terminal: RequiredTerminal,
}

impl InputBindingNode {
    /// Unit input binding node.
    #[allow(non_upper_case_globals)]
    pub const Unit: Self = Self {
        kind: InputBindingNodeKind::Unit,
    };

    /// Builds a struct binding from named fields, rejecting duplicate paths and sorting canonically.
    pub fn struct_fields(mut fields: Vec<NamedInputBinding>) -> Result<Self> {
        let mut seen = BTreeSet::new();
        for field in &fields {
            if !seen.insert(field.field_path.as_str().to_owned()) {
                return Err(PlanError::DuplicateInputFieldPath(
                    field.field_path.as_str().to_owned(),
                ));
            }
        }
        fields.sort_by(|left, right| left.field_path.cmp(&right.field_path));
        Ok(Self::struct_fields_unchecked(fields))
    }

    fn cell(
        field_path: InputFieldPath,
        cell_id: CellId,
        semantic_type_id: SemanticTypeId,
        schema_id: SchemaId,
        value_lineage: ValueLineageRef,
        required_terminal: RequiredTerminal,
    ) -> Self {
        Self {
            kind: InputBindingNodeKind::Cell(Box::new(InputCellBinding {
                field_path,
                cell_id,
                semantic_type_id,
                schema_id,
                value_lineage,
                required_terminal,
            })),
        }
    }

    fn tuple(elements: Vec<InputBindingNode>) -> Self {
        Self {
            kind: InputBindingNodeKind::Tuple { elements },
        }
    }

    fn struct_fields_unchecked(fields: Vec<NamedInputBinding>) -> Self {
        Self {
            kind: InputBindingNodeKind::Struct { fields },
        }
    }

    fn vector(elements: Vec<InputBindingNode>, ordering: OrderingEvidence) -> Self {
        Self {
            kind: InputBindingNodeKind::Vec { elements, ordering },
        }
    }

    fn non_empty_vector(elements: Vec<InputBindingNode>, ordering: OrderingEvidence) -> Self {
        Self {
            kind: InputBindingNodeKind::NonEmptyVec { elements, ordering },
        }
    }
}

/// Author-side conversion from handles into a binding node for a runtime input type.
pub trait IntoInputBindingNode<I> {
    /// Converts this author-side input into a binding node at `field_path`.
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode>;
}

/// Typed state-input binding with descriptor and canonical digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBinding<I: StateInput> {
    input_schema_id: SchemaId,
    input_descriptor_id: DescriptorId,
    root: InputBindingNode,
    digest: ContentDigest,
    _input: PhantomData<fn(I) -> I>,
}

impl<I: StateInput> InputBinding<I> {
    /// Creates an input binding from a canonical root binding node.
    pub fn from_root(root: InputBindingNode) -> Result<Self> {
        validate_input_binding_node(&root)?;
        let descriptor =
            I::input_schema_descriptor().map_err(|error| PlanError::Value(error.to_string()))?;
        validate_input_binding_node_shape(&root, &descriptor.identity.shape)?;
        let input_schema_id = descriptor
            .schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?;
        let input_descriptor_id = input_descriptor_id(&input_schema_id)?;
        let digest = input_binding_digest(&root)?;
        Ok(Self {
            input_schema_id,
            input_descriptor_id,
            root,
            digest,
            _input: PhantomData,
        })
    }

    /// Returns the input schema id.
    pub fn input_schema_id(&self) -> &SchemaId {
        &self.input_schema_id
    }

    /// Returns the input descriptor id.
    pub fn input_descriptor_id(&self) -> &DescriptorId {
        &self.input_descriptor_id
    }

    /// Returns the root binding node.
    pub fn root(&self) -> &InputBindingNode {
        &self.root
    }

    /// Returns the canonical binding digest.
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// Returns an unbranded persisted input binding spec.
    pub fn spec(&self) -> InputBindingSpec {
        InputBindingSpec {
            input_schema_id: self.input_schema_id.clone(),
            input_descriptor_id: self.input_descriptor_id.clone(),
            root: self.root.clone(),
            digest: self.digest.clone(),
        }
    }
}

/// Persisted state-input binding spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBindingSpec {
    /// Input schema id.
    pub input_schema_id: SchemaId,
    /// Input descriptor id.
    pub input_descriptor_id: DescriptorId,
    /// Root input binding node.
    pub root: InputBindingNode,
    /// Canonical digest of the root binding tree.
    pub digest: ContentDigest,
}

/// Converts author-side handle values into a typed state-input binding.
pub trait IntoStateInput<'program, 'scope, I: StateInput> {
    /// Converts into a typed input binding.
    fn into_binding(self) -> Result<InputBinding<I>>;
}

impl IntoInputBindingNode<()> for () {
    fn into_binding_node(self, _field_path: InputFieldPath) -> Result<InputBindingNode> {
        Ok(InputBindingNode::Unit)
    }
}

impl<'program, 'scope> IntoStateInput<'program, 'scope, ()> for () {
    fn into_binding(self) -> Result<InputBinding<()>> {
        InputBinding::from_root(InputBindingNode::Unit)
    }
}

impl<'program, 'scope, T> IntoInputBindingNode<T> for Handle<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        let typed_ref = self.typed_ref();
        Ok(InputBindingNode::cell(
            field_path,
            typed_ref.cell_id,
            typed_ref.semantic_type_id,
            typed_ref.schema_id,
            typed_ref.value_lineage,
            RequiredTerminal::from_value_policy(T::terminal_policy()),
        ))
    }
}

impl<'program, 'scope, T> IntoStateInput<'program, 'scope, T> for Handle<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<T>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

impl<'program, 'scope, T> IntoInputBindingNode<Vec<T>> for Vec<Handle<'program, 'scope, T>>
where
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        let elements = self
            .into_iter()
            .enumerate()
            .map(|(index, handle)| handle.into_binding_node(field_path.child(index.to_string())?))
            .collect::<Result<Vec<_>>>()?;
        Ok(InputBindingNode::vector(
            elements,
            OrderingEvidence::ExplicitAuthorOrder,
        ))
    }
}

impl<'program, 'scope, T> IntoStateInput<'program, 'scope, Vec<T>>
    for Vec<Handle<'program, 'scope, T>>
where
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<Vec<T>>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

macro_rules! impl_tuple_input_binding {
    ($($name:ident:$index:literal),+ $(,)?) => {
        impl<'program, 'scope, $($name),+> IntoInputBindingNode<($($name,)+)>
            for ($(Handle<'program, 'scope, $name>,)+)
        where
            $($name: MfmValue,)+
        {
            fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                let elements = vec![
                    $($name.into_binding_node(field_path.child($index.to_string())?)?,)+
                ];
                Ok(InputBindingNode::tuple(elements))
            }
        }

        impl<'program, 'scope, $($name),+> IntoStateInput<'program, 'scope, ($($name,)+)>
            for ($(Handle<'program, 'scope, $name>,)+)
        where
            $($name: MfmValue,)+
        {
            fn into_binding(self) -> Result<InputBinding<($($name,)+)>> {
                InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
            }
        }
    };
}

impl_tuple_input_binding!(A:0);
impl_tuple_input_binding!(A:0, B:1);
impl_tuple_input_binding!(A:0, B:1, C:2);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

/// Author-side non-empty handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyHandles<'program, 'scope, T: MfmValue> {
    handles: Vec<Handle<'program, 'scope, T>>,
}

impl<'program, 'scope, T: MfmValue> NonEmptyHandles<'program, 'scope, T> {
    /// Creates a non-empty handle collection from a first handle and optional rest.
    pub fn new(
        first: Handle<'program, 'scope, T>,
        mut rest: Vec<Handle<'program, 'scope, T>>,
    ) -> Self {
        let mut handles = Vec::with_capacity(rest.len() + 1);
        handles.push(first);
        handles.append(&mut rest);
        Self { handles }
    }

    /// Attempts to create a non-empty handle collection from a vector.
    pub fn try_from_vec(handles: Vec<Handle<'program, 'scope, T>>) -> Result<Self> {
        if handles.is_empty() {
            return Err(PlanError::EmptyNonEmptyInput);
        }
        Ok(Self { handles })
    }
}

impl<'program, 'scope, T> IntoInputBindingNode<NonEmpty<T>> for NonEmptyHandles<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        if self.handles.is_empty() {
            return Err(PlanError::EmptyNonEmptyInput);
        }
        let elements = self
            .handles
            .into_iter()
            .enumerate()
            .map(|(index, handle)| handle.into_binding_node(field_path.child(index.to_string())?))
            .collect::<Result<Vec<_>>>()?;
        Ok(InputBindingNode::non_empty_vector(
            elements,
            OrderingEvidence::ExplicitAuthorOrder,
        ))
    }
}

impl<'program, 'scope, T> IntoStateInput<'program, 'scope, NonEmpty<T>>
    for NonEmptyHandles<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<NonEmpty<T>>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

/// Root-bound public output evidence returned by `RootBuilder::bind_public_outputs`.
#[derive(Debug, PartialEq, Eq)]
pub struct RootBound<'program, 'scope> {
    public_output_spec: PublicOutputSpec,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    _private: (),
}

/// Parent-visible value returned from a child scope after bridge validation.
#[derive(Debug, PartialEq, Eq)]
pub struct Bridged<'program, 'parent, R> {
    value: R,
    bridge_evidence: Vec<BridgeEvidence<'program, 'parent>>,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
    _private: (),
}

/// Framework-owned trait for values that may leave a child scope.
pub trait BridgeableToParent<'program, 'parent>: private::BridgeableSealed {
    /// Returns live bridge evidence that must validate against the active child session.
    fn bridge_evidence(&self) -> Vec<BridgeEvidence<'program, 'parent>>;
}

impl<'program, 'parent, T> BridgeableToParent<'program, 'parent> for Handle<'program, 'parent, T>
where
    T: MfmValue,
{
    fn bridge_evidence(&self) -> Vec<BridgeEvidence<'program, 'parent>> {
        self.origin.bridge_evidence()
    }
}

impl<'program, 'parent> BridgeableToParent<'program, 'parent> for () {
    fn bridge_evidence(&self) -> Vec<BridgeEvidence<'program, 'parent>> {
        Vec::new()
    }
}

macro_rules! impl_bridgeable_tuple {
    ($($name:ident),+ $(,)?) => {
        impl<'program, 'parent, $($name),+> BridgeableToParent<'program, 'parent>
            for ($($name,)+)
        where
            $($name: BridgeableToParent<'program, 'parent>,)+
        {
            fn bridge_evidence(&self) -> Vec<BridgeEvidence<'program, 'parent>> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                let mut output = Vec::new();
                $(output.extend($name.bridge_evidence());)+
                output
            }
        }
    };
}

impl_bridgeable_tuple!(A);
impl_bridgeable_tuple!(A, B);
impl_bridgeable_tuple!(A, B, C);
impl_bridgeable_tuple!(A, B, C, D);

/// Unbranded typed program draft produced by [`build_root`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedProgramDraft {
    root_key: ScopeKey,
    root_scope_id: ScopeId,
    seeds: Vec<RootSeedSpec>,
    scopes: Vec<ScopeSpec>,
    state_nodes: Vec<StateNodeSpec>,
    bridge_nodes: Vec<BridgeNodeSpec>,
    public_output_spec: PublicOutputSpec,
}

impl TypedProgramDraft {
    /// Returns the root scope key.
    pub fn root_key(&self) -> &ScopeKey {
        &self.root_key
    }

    /// Returns the root scope id.
    pub fn root_scope_id(&self) -> &ScopeId {
        &self.root_scope_id
    }

    /// Returns root seed specs.
    pub fn seeds(&self) -> &[RootSeedSpec] {
        &self.seeds
    }

    /// Returns emitted scope specs, including the root scope.
    pub fn scopes(&self) -> &[ScopeSpec] {
        &self.scopes
    }

    /// Returns emitted typed state nodes.
    pub fn state_nodes(&self) -> &[StateNodeSpec] {
        &self.state_nodes
    }

    /// Returns emitted framework bridge nodes.
    pub fn bridge_nodes(&self) -> &[BridgeNodeSpec] {
        &self.bridge_nodes
    }

    /// Returns the public-output spec bound at the root.
    pub fn public_output_spec(&self) -> &PublicOutputSpec {
        &self.public_output_spec
    }

    /// Validates that a persisted bridge ref is backed by an emitted bridge node.
    pub fn validate_bridge_ref_for_certification(
        &self,
        bridge_ref: &BridgeRef,
    ) -> Result<&BridgeNodeSpec> {
        self.bridge_nodes
            .iter()
            .find(|node| node.bridge_ref() == *bridge_ref)
            .ok_or(PlanError::UnknownBridgeRef)
    }
}

/// Root-scope builder for a typed program.
pub struct RootBuilder<'program, 'scope> {
    root_key: ScopeKey,
    scope: ScopeBuilder<'program, 'scope>,
    seeds: Vec<RootSeedSpec>,
    seed_keys: BTreeSet<String>,
    public_outputs_bound: bool,
}

impl<'program, 'scope> RootBuilder<'program, 'scope> {
    /// Returns the root scope builder.
    pub fn scope(&mut self) -> &mut ScopeBuilder<'program, 'scope> {
        &mut self.scope
    }

    /// Declares a root launch seed and returns its typed cell handle.
    pub fn seed<T: MfmValue>(
        &mut self,
        key: SeedKey,
        value: CanonicalSeed<T>,
    ) -> Result<Handle<'program, 'scope, T>> {
        if !self.seed_keys.insert(key.as_str().to_owned()) {
            return Err(PlanError::DuplicateSeedKey(key.as_str().to_owned()));
        }

        let seed_id = seed_id(self.scope.scope_id(), &key)?;
        let cell_id = seed_cell_id(&seed_id)?;
        let value_lineage = seed_value_lineage_ref(&cell_id)?;
        let spec = RootSeedSpec {
            key,
            seed_id,
            cell_id: cell_id.clone(),
            scope_id: self.scope.scope_id().clone(),
            schema_id: value.schema_id.clone(),
            semantic_type_id: value.semantic_type_id.clone(),
            content_digest: value.content_digest,
            byte_len: value.byte_len,
        };
        let handle = Handle::new(
            cell_id,
            spec.scope_id.clone(),
            spec.schema_id.clone(),
            spec.semantic_type_id.clone(),
            value_lineage,
        );
        self.seeds.push(spec);
        Ok(handle)
    }

    /// Binds root public outputs and returns unforgeable root-bound evidence.
    pub fn bind_public_outputs<P>(
        &mut self,
        key: PublicOutputKey,
        outputs: &P,
    ) -> Result<RootBound<'program, 'scope>>
    where
        P: PublicOutputs<'program, 'scope>,
    {
        if self.public_outputs_bound {
            return Err(PlanError::PublicOutputsAlreadyBound);
        }
        self.public_outputs_bound = true;
        let output_cells = outputs.output_cells()?;
        if output_cells.is_empty() {
            return Err(PlanError::EmptyPublicOutputs);
        }
        let mut seen = BTreeSet::new();
        for output in &output_cells {
            if !seen.insert(output.public_field_path().as_str().to_owned()) {
                return Err(PlanError::DuplicatePublicOutputPath(
                    output.public_field_path().as_str().to_owned(),
                ));
            }
        }
        Ok(RootBound {
            public_output_spec: PublicOutputSpec {
                key,
                public_schema_id: outputs.public_schema_id()?,
                outputs: output_cells,
            },
            _program: PhantomData,
            _scope: PhantomData,
            _private: (),
        })
    }
}

/// Scope-local builder. State and operation planning APIs are added in later
/// typed-core sections.
pub struct ScopeBuilder<'program, 'scope> {
    scope_id: ScopeId,
    state_registry: StateRegistrySnapshot,
    state_keys: BTreeSet<String>,
    state_nodes: Vec<StateNodeSpec>,
    child_scope_keys: BTreeSet<String>,
    child_scopes: Vec<ScopeSpec>,
    bridge_nodes: Vec<BridgeNodeSpec>,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

impl<'program, 'scope> ScopeBuilder<'program, 'scope> {
    /// Returns the typed scope id.
    pub fn scope_id(&self) -> &ScopeId {
        &self.scope_id
    }

    /// Opens a child scope and returns only values explicitly bridged back to this scope.
    pub fn child_scope<R>(
        &mut self,
        key: ScopeKey,
        f: impl for<'child> FnOnce(
            &mut ChildScopeBuilder<'program, 'scope, 'child>,
        ) -> Result<Bridged<'program, 'scope, R>>,
    ) -> Result<R> {
        if !self.child_scope_keys.insert(key.as_str().to_owned()) {
            return Err(PlanError::DuplicateChildScopeKey(key.as_str().to_owned()));
        }

        let child_scope_id = child_scope_id(&self.scope_id, &key)?;
        let session_token = bridge_session_token(&self.scope_id, &child_scope_id, &key);
        let mut child = ChildScopeBuilder {
            parent_scope_id: self.scope_id.clone(),
            scope: ScopeBuilder {
                scope_id: child_scope_id.clone(),
                state_registry: self.state_registry.clone(),
                state_keys: BTreeSet::new(),
                state_nodes: Vec::new(),
                child_scope_keys: BTreeSet::new(),
                child_scopes: Vec::new(),
                bridge_nodes: Vec::new(),
                _program: PhantomData,
                _scope: PhantomData,
            },
            session_token,
            bridge_keys: BTreeSet::new(),
            active_bridge_refs: BTreeSet::new(),
            bridge_nodes: Vec::new(),
            _parent: PhantomData,
        };

        let bridged = f(&mut child)?;
        child.validate_bridge_evidence_set(&bridged.bridge_evidence)?;
        self.child_scopes.push(ScopeSpec {
            key,
            scope_id: child_scope_id,
            parent_scope_id: Some(self.scope_id.clone()),
        });
        self.state_nodes.extend(child.scope.state_nodes);
        self.child_scopes.extend(child.scope.child_scopes);
        self.bridge_nodes.extend(child.bridge_nodes);
        self.bridge_nodes.extend(child.scope.bridge_nodes);
        Ok(bridged.value)
    }

    /// Plans a registered typed state by resolving `S` through this builder's registry.
    pub fn state<S, I>(
        &mut self,
        key: StateKey,
        config: S::Config,
        input: I,
    ) -> Result<Handle<'program, 'scope, S::Output>>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        let registered = self.state_registry.registered_state::<S>()?;
        self.state_registered(key, registered, config, input)
    }

    /// Plans a typed state from an explicit framework-owned registration token.
    pub fn state_registered<S, I>(
        &mut self,
        key: StateKey,
        registered: RegisteredState<S>,
        config: S::Config,
        input: I,
    ) -> Result<Handle<'program, 'scope, S::Output>>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        let key_string = key.as_str().to_owned();
        if self.state_keys.contains(&key_string) {
            return Err(PlanError::DuplicateStateKey(key.as_str().to_owned()));
        }
        let config_binding = canonical_config_binding::<S::Config>(&config)?;
        let input = input.into_binding()?;
        let state = S::new(config)?;
        drop(state);

        let descriptor = registered.descriptor();
        let node_id = state_node_id(
            &self.scope_id,
            &key,
            descriptor.kind(),
            descriptor.version(),
            &config_binding.content_digest,
            input.digest(),
        )?;
        let output_cell_id = state_output_cell_id(&node_id)?;
        let output_schema_id =
            S::Output::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let output_semantic_type_id =
            S::Output::semantic_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let value_lineage = state_value_lineage_ref(&node_id)?;
        let handle = Handle::new(
            output_cell_id.clone(),
            self.scope_id.clone(),
            output_schema_id.clone(),
            output_semantic_type_id.clone(),
            value_lineage,
        );
        self.state_keys.insert(key_string);
        self.state_nodes.push(StateNodeSpec {
            node_id,
            key,
            scope_id: self.scope_id.clone(),
            state_kind: descriptor.kind().clone(),
            state_version: descriptor.version().clone(),
            state_descriptor_id: descriptor.descriptor_id().clone(),
            runner: registered.runner(),
            config: config_binding,
            input: input.spec(),
            output_cell_id,
            output_schema_id,
            output_semantic_type_id,
        });
        Ok(handle)
    }
}

/// Child-scope builder with live bridge-session authority.
pub struct ChildScopeBuilder<'program, 'parent, 'child> {
    parent_scope_id: ScopeId,
    scope: ScopeBuilder<'program, 'child>,
    session_token: BridgeSessionToken,
    bridge_keys: BTreeSet<String>,
    active_bridge_refs: BTreeSet<String>,
    bridge_nodes: Vec<BridgeNodeSpec>,
    _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
}

impl<'program, 'parent, 'child> ChildScopeBuilder<'program, 'parent, 'child> {
    /// Returns the child scope builder for nested child scopes.
    pub fn scope(&mut self) -> &mut ScopeBuilder<'program, 'child> {
        &mut self.scope
    }

    /// Returns the child scope id.
    pub fn scope_id(&self) -> &ScopeId {
        self.scope.scope_id()
    }

    /// Imports a parent handle into this child scope through an emitted bridge node.
    pub fn import_from_parent<T: MfmValue>(
        &mut self,
        key: BridgeKey,
        value: Handle<'program, 'parent, T>,
        policy: BridgePolicy,
    ) -> Result<Handle<'program, 'child, T>> {
        let source = value.typed_ref();
        let (target, evidence) = self.emit_bridge(
            key,
            BridgeKind::ImportFromParent,
            source,
            self.parent_scope_id.clone(),
            self.scope.scope_id().clone(),
            policy,
        )?;
        Ok(Handle::new_bridge(
            target.cell_id,
            target.scope_id,
            target.schema_id,
            target.semantic_type_id,
            target.value_lineage,
            evidence,
        ))
    }

    /// Exports a child handle into the parent scope through an emitted bridge node.
    pub fn export_to_parent<T: MfmValue>(
        &mut self,
        key: BridgeKey,
        value: Handle<'program, 'child, T>,
        policy: BridgePolicy,
    ) -> Result<Handle<'program, 'parent, T>> {
        let source = value.typed_ref();
        let (target, evidence) = self.emit_bridge(
            key,
            BridgeKind::ExportToParent,
            source,
            self.scope.scope_id().clone(),
            self.parent_scope_id.clone(),
            policy,
        )?;
        Ok(Handle::new_bridge(
            target.cell_id,
            target.scope_id,
            target.schema_id,
            target.semantic_type_id,
            target.value_lineage,
            evidence,
        ))
    }

    /// Wraps parent-visible values after validating their live bridge evidence.
    pub fn bridge_to_parent<R>(&mut self, value: R) -> Result<Bridged<'program, 'parent, R>>
    where
        R: BridgeableToParent<'program, 'parent>,
    {
        let evidence = value.bridge_evidence();
        self.validate_bridge_evidence_set(&evidence)?;
        Ok(Bridged {
            value,
            bridge_evidence: evidence,
            _program: PhantomData,
            _parent: PhantomData,
            _private: (),
        })
    }

    fn emit_bridge(
        &mut self,
        key: BridgeKey,
        bridge_kind: BridgeKind,
        source: TypedHandleRef,
        source_scope_id: ScopeId,
        target_scope_id: ScopeId,
        policy: BridgePolicy,
    ) -> Result<(TypedHandleRef, BridgeEvidenceCore)> {
        if source.scope_id != source_scope_id {
            return Err(PlanError::InvalidBridgeEvidence(format!(
                "source handle scope {} did not match bridge source {}",
                source.scope_id.as_str(),
                source_scope_id.as_str()
            )));
        }
        if !self.bridge_keys.insert(key.as_str().to_owned()) {
            return Err(PlanError::DuplicateBridgeKey(key.as_str().to_owned()));
        }

        let node_id = bridge_node_id(
            &source_scope_id,
            &target_scope_id,
            &source.cell_id,
            &key,
            bridge_kind,
            policy,
        )?;
        let target_cell_id = bridge_cell_id(&node_id)?;
        let target_value_lineage = bridge_value_lineage_ref(&node_id)?;
        let spec = BridgeNodeSpec {
            node_id: node_id.clone(),
            key,
            source_scope_id,
            target_scope_id: target_scope_id.clone(),
            source_cell_id: source.cell_id,
            target_cell_id: target_cell_id.clone(),
            semantic_type_id: source.semantic_type_id.clone(),
            schema_id: source.schema_id.clone(),
            bridge_kind,
            policy,
            provenance: BridgeProvenance::FrameworkChildScopeV1,
        };
        let bridge_ref = spec.bridge_ref();
        self.active_bridge_refs.insert(bridge_ref_key(&bridge_ref));
        self.bridge_nodes.push(spec);
        let target = TypedHandleRef {
            cell_id: target_cell_id,
            scope_id: target_scope_id,
            schema_id: source.schema_id,
            semantic_type_id: source.semantic_type_id,
            value_lineage: target_value_lineage,
        };
        let evidence = BridgeEvidenceCore {
            bridge_ref,
            session_token: self.session_token,
        };
        Ok((target, evidence))
    }

    fn validate_bridge_evidence_set(
        &self,
        evidence_set: &[BridgeEvidence<'program, 'parent>],
    ) -> Result<()> {
        for evidence in evidence_set {
            self.validate_bridge_evidence(evidence)?;
        }
        Ok(())
    }

    fn validate_bridge_evidence(&self, evidence: &BridgeEvidence<'program, 'parent>) -> Result<()> {
        let bridge_ref = &evidence.core.bridge_ref;
        if evidence.core.session_token != self.session_token {
            if bridge_ref.target_scope_id == self.parent_scope_id
                && bridge_ref.source_scope_id != *self.scope.scope_id()
            {
                return Ok(());
            }
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge evidence belongs to a different child-scope session".to_owned(),
            ));
        }
        if bridge_ref.source_scope_id != *self.scope.scope_id()
            || bridge_ref.target_scope_id != self.parent_scope_id
        {
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge evidence does not export from this child to its parent".to_owned(),
            ));
        }
        if !self
            .active_bridge_refs
            .contains(&bridge_ref_key(bridge_ref))
        {
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge ref was not created by this child-scope builder".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Program-level public output binding contract generated by derives.
pub trait PublicOutputs<'program, 'scope> {
    /// Returns the public output schema id.
    fn public_schema_id(&self) -> Result<SchemaId>;

    /// Returns bound public output cells.
    fn output_cells(&self) -> Result<Vec<PublicOutputCellSpec>>;
}

/// Builds a branded root typed program and returns an unbranded draft.
pub fn build_root<F>(root_key: ScopeKey, f: F) -> Result<TypedProgramDraft>
where
    F: for<'program, 'root> FnOnce(
        &mut RootBuilder<'program, 'root>,
    ) -> Result<RootBound<'program, 'root>>,
{
    build_root_with_registry(root_key, StateRegistrySnapshot::default(), f)
}

/// Builds a branded root typed program with a state registry snapshot.
pub fn build_root_with_registry<F>(
    root_key: ScopeKey,
    state_registry: StateRegistrySnapshot,
    f: F,
) -> Result<TypedProgramDraft>
where
    F: for<'program, 'root> FnOnce(
        &mut RootBuilder<'program, 'root>,
    ) -> Result<RootBound<'program, 'root>>,
{
    let root_scope_id = scope_id(&root_key)?;
    let mut builder = RootBuilder {
        root_key: root_key.clone(),
        scope: ScopeBuilder {
            scope_id: root_scope_id.clone(),
            state_registry,
            state_keys: BTreeSet::new(),
            state_nodes: Vec::new(),
            child_scope_keys: BTreeSet::new(),
            child_scopes: Vec::new(),
            bridge_nodes: Vec::new(),
            _program: PhantomData,
            _scope: PhantomData,
        },
        seeds: Vec::new(),
        seed_keys: BTreeSet::new(),
        public_outputs_bound: false,
    };
    let bound = f(&mut builder)?;
    Ok(TypedProgramDraft {
        root_key: builder.root_key,
        root_scope_id: root_scope_id.clone(),
        seeds: builder.seeds,
        scopes: {
            let mut scopes = vec![ScopeSpec {
                key: root_key,
                scope_id: root_scope_id,
                parent_scope_id: None,
            }];
            scopes.extend(builder.scope.child_scopes);
            scopes
        },
        state_nodes: builder.scope.state_nodes,
        bridge_nodes: builder.scope.bridge_nodes,
        public_output_spec: bound.public_output_spec,
    })
}

/// Returns the public schema id for a derive-backed public output descriptor.
pub fn public_schema_id<P: PublicOutputDescriptor>() -> Result<SchemaId> {
    P::public_schema_id().map_err(|error| PlanError::Value(error.to_string()))
}

fn checked_key(label: &str, value: &str) -> Result<String> {
    if is_valid_author_key(value) {
        Ok(value.to_owned())
    } else {
        Err(PlanError::Key(format!(
            "{label} {value:?} must match [a-z0-9][a-z0-9._/-]*"
        )))
    }
}

fn is_valid_author_key(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|ch| {
        ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-' | '/')
    })
}

fn checked_field_path(label: &str, value: &str) -> Result<String> {
    if value.split('.').all(is_valid_field_segment) {
        Ok(value.to_owned())
    } else {
        Err(PlanError::Key(format!(
            "{label} {value:?} must contain non-empty ASCII field segments"
        )))
    }
}

fn checked_field_segment(label: &str, value: &str) -> Result<String> {
    if is_valid_field_segment(value) {
        Ok(value.to_owned())
    } else {
        Err(PlanError::Key(format!(
            "{label} {value:?} must be a non-empty ASCII field segment"
        )))
    }
}

fn is_valid_field_segment(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/'))
}

fn scope_id(key: &ScopeKey) -> Result<ScopeId> {
    digest_only_id("scope", key.as_str(), ScopeId::from_digest)
}

fn child_scope_id(parent_scope_id: &ScopeId, key: &ScopeKey) -> Result<ScopeId> {
    digest_only_id(
        "child-scope",
        &format!("{}:{}", parent_scope_id.as_str(), key.as_str()),
        ScopeId::from_digest,
    )
}

fn seed_id(scope_id: &ScopeId, key: &SeedKey) -> Result<SeedId> {
    digest_only_id(
        "seed",
        &format!("{}:{}", scope_id.as_str(), key.as_str()),
        SeedId::from_digest,
    )
}

fn seed_cell_id(seed_id: &SeedId) -> Result<CellId> {
    digest_only_id("seed-cell", seed_id.as_str(), CellId::from_digest)
}

fn seed_value_lineage_ref(cell_id: &CellId) -> Result<ValueLineageRef> {
    value_lineage_ref("seed", cell_id.as_str())
}

fn bridge_value_lineage_ref(node_id: &NodeId) -> Result<ValueLineageRef> {
    value_lineage_ref("bridge", node_id.as_str())
}

fn value_lineage_ref(domain: &str, value: &str) -> Result<ValueLineageRef> {
    let digest =
        sha256_digest_bytes(format!("mfm.program:value-lineage:{domain}:{value}").as_bytes());
    Ok(ValueLineageRef::new(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest,
    )))
}

fn bridge_node_id(
    source_scope_id: &ScopeId,
    target_scope_id: &ScopeId,
    source_cell_id: &CellId,
    key: &BridgeKey,
    bridge_kind: BridgeKind,
    policy: BridgePolicy,
) -> Result<NodeId> {
    digest_only_id(
        "bridge-node",
        &format!(
            "{}:{}:{}:{}:{}:{}",
            source_scope_id.as_str(),
            target_scope_id.as_str(),
            source_cell_id.as_str(),
            key.as_str(),
            bridge_kind.as_str(),
            policy.as_str()
        ),
        NodeId::from_digest,
    )
}

fn bridge_cell_id(node_id: &NodeId) -> Result<CellId> {
    digest_only_id("bridge-cell", node_id.as_str(), CellId::from_digest)
}

fn bridge_session_token(
    parent_scope_id: &ScopeId,
    child_scope_id: &ScopeId,
    key: &ScopeKey,
) -> BridgeSessionToken {
    BridgeSessionToken(sha256_digest_bytes(
        format!(
            "mfm.program:bridge-session:{}:{}:{}",
            parent_scope_id.as_str(),
            child_scope_id.as_str(),
            key.as_str()
        )
        .as_bytes(),
    ))
}

fn bridge_ref_key(bridge_ref: &BridgeRef) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        bridge_ref.source_scope_id.as_str(),
        bridge_ref.target_scope_id.as_str(),
        bridge_ref.source_cell_id.as_str(),
        bridge_ref.target_cell_id.as_str(),
        bridge_ref.semantic_type_id.as_str(),
        bridge_ref.schema_id.as_str(),
        bridge_ref.bridge_node_id.as_str()
    )
}

fn canonical_config_binding<C: MfmConfig>(config: &C) -> Result<ConfigBindingSpec> {
    config
        .validate()
        .map_err(|error| PlanError::Value(error.to_string()))?;
    let json =
        serde_json::to_string(config).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    let schema_id = C::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
    let content_digest = canonical.content_digest();
    let byte_len = canonical.as_bytes().len();
    Ok(ConfigBindingSpec {
        schema_id,
        canonical_json: canonical,
        content_digest,
        byte_len,
    })
}

struct StateDescriptorIdParts<'a> {
    kind: &'a StateKind,
    version: &'a StateVersion,
    name: &'static str,
    config_schema_id: &'a SchemaId,
    input_schema_id: &'a SchemaId,
    output_schema_id: &'a SchemaId,
    output_semantic_type_id: &'a SemanticTypeId,
    effect: &'a EffectDescriptor,
    capabilities: &'a CapabilitySetDescriptor,
    runner: RunnerKind,
}

fn state_descriptor_id(parts: StateDescriptorIdParts<'_>) -> Result<DescriptorId> {
    let json = serde_json::json!({
        "capabilities": parts.capabilities
            .capabilities
            .iter()
            .map(|capability| {
                serde_json::json!({
                    "kind": capability.kind.as_str(),
                    "name": capability.name,
                    "role": capability.role.as_str(),
                    "version": capability.version.as_str(),
                })
            })
            .collect::<Vec<_>>(),
        "config_schema_id": parts.config_schema_id.as_str(),
        "effect": {
            "class": parts.effect.class.as_str(),
            "kind": parts.effect.kind.as_str(),
            "name": parts.effect.name,
            "version": parts.effect.version.as_str(),
        },
        "input_schema_id": parts.input_schema_id.as_str(),
        "kind": parts.kind.as_str(),
        "name": parts.name,
        "output_schema_id": parts.output_schema_id.as_str(),
        "output_semantic_type_id": parts.output_semantic_type_id.as_str(),
        "runner": parts.runner.as_str(),
        "version": parts.version.as_str(),
    });
    let json =
        serde_json::to_string(&json).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(canonical.as_bytes()),
    ))
}

fn effect_runner_kind_for_effect<E: EffectSpec>() -> RunnerKind {
    match E::class() {
        mfm_effects::EffectClass::Pure => RunnerKind::Pure,
        mfm_effects::EffectClass::ReadExternal => RunnerKind::ReadExternal,
        mfm_effects::EffectClass::ManagedPlatformWrite => RunnerKind::ManagedPlatformWrite,
        mfm_effects::EffectClass::ApplySideEffect => RunnerKind::ApplySideEffect,
    }
}

fn state_node_id(
    scope_id: &ScopeId,
    key: &StateKey,
    state_kind: &StateKind,
    state_version: &StateVersion,
    config_digest: &ContentDigest,
    input_digest: &ContentDigest,
) -> Result<NodeId> {
    digest_only_id(
        "state-node",
        &format!(
            "{}:{}:{}:{}:{}:{}",
            scope_id.as_str(),
            key.as_str(),
            state_kind.as_str(),
            state_version.as_str(),
            config_digest.as_str(),
            input_digest.as_str()
        ),
        NodeId::from_digest,
    )
}

fn state_output_cell_id(node_id: &NodeId) -> Result<CellId> {
    digest_only_id("state-output-cell", node_id.as_str(), CellId::from_digest)
}

fn state_value_lineage_ref(node_id: &NodeId) -> Result<ValueLineageRef> {
    value_lineage_ref("state", node_id.as_str())
}

fn input_descriptor_id(input_schema_id: &SchemaId) -> Result<DescriptorId> {
    digest_only_id(
        "input-descriptor",
        input_schema_id.as_str(),
        DescriptorId::from_digest,
    )
}

fn validate_input_binding_node(node: &InputBindingNode) -> Result<()> {
    match &node.kind {
        InputBindingNodeKind::Unit | InputBindingNodeKind::Cell(_) => Ok(()),
        InputBindingNodeKind::Tuple { elements } | InputBindingNodeKind::Vec { elements, .. } => {
            for element in elements {
                validate_input_binding_node(element)?;
            }
            Ok(())
        }
        InputBindingNodeKind::Struct { fields } => {
            let mut seen = BTreeSet::new();
            for field in fields {
                if !seen.insert(field.field_path.as_str().to_owned()) {
                    return Err(PlanError::DuplicateInputFieldPath(
                        field.field_path.as_str().to_owned(),
                    ));
                }
                validate_input_binding_node(&field.node)?;
            }
            Ok(())
        }
        InputBindingNodeKind::NonEmptyVec { elements, .. } => {
            if elements.is_empty() {
                return Err(PlanError::EmptyNonEmptyInput);
            }
            for element in elements {
                validate_input_binding_node(element)?;
            }
            Ok(())
        }
    }
}

fn validate_input_binding_node_shape(node: &InputBindingNode, shape: &SchemaShape) -> Result<()> {
    match (&node.kind, shape) {
        (InputBindingNodeKind::Unit, SchemaShape::Unit) => Ok(()),
        (
            InputBindingNodeKind::Cell(cell),
            SchemaShape::ValueRef {
                schema_id: expected_schema_id,
                semantic_type_id: expected_semantic_type_id,
            },
        ) => {
            if &cell.schema_id != expected_schema_id
                || &cell.semantic_type_id != expected_semantic_type_id
            {
                return Err(PlanError::InputBindingShape(format!(
                    "cell {} expected ({}, {}) but got ({}, {})",
                    cell.field_path.as_str(),
                    expected_schema_id,
                    expected_semantic_type_id,
                    cell.schema_id,
                    cell.semantic_type_id
                )));
            }
            Ok(())
        }
        (InputBindingNodeKind::Tuple { elements }, SchemaShape::Tuple(expected)) => {
            if elements.len() != expected.len() {
                return Err(PlanError::InputBindingShape(format!(
                    "tuple expected {} elements but got {}",
                    expected.len(),
                    elements.len()
                )));
            }
            for (element, expected_shape) in elements.iter().zip(expected) {
                validate_input_binding_node_shape(element, expected_shape)?;
            }
            Ok(())
        }
        (InputBindingNodeKind::Struct { fields }, SchemaShape::Struct { fields: expected }) => {
            if fields.len() != expected.len() {
                return Err(PlanError::InputBindingShape(format!(
                    "struct expected {} fields but got {}",
                    expected.len(),
                    fields.len()
                )));
            }
            for (field, expected_field) in fields.iter().zip(expected) {
                if field.field_path.as_str() != expected_field.name {
                    return Err(PlanError::InputBindingShape(format!(
                        "struct expected field {} but got {}",
                        expected_field.name,
                        field.field_path.as_str()
                    )));
                }
                validate_input_binding_node_shape(&field.node, &expected_field.shape)?;
            }
            Ok(())
        }
        (InputBindingNodeKind::Vec { elements, .. }, SchemaShape::Vec(expected)) => {
            for element in elements {
                validate_input_binding_node_shape(element, expected)?;
            }
            Ok(())
        }
        (
            InputBindingNodeKind::NonEmptyVec { elements, .. },
            SchemaShape::NonEmptyVec(expected),
        ) => {
            for element in elements {
                validate_input_binding_node_shape(element, expected)?;
            }
            Ok(())
        }
        (node, shape) => Err(PlanError::InputBindingShape(format!(
            "expected {} but got {}",
            schema_shape_kind(shape),
            input_binding_node_kind_from_kind(node)
        ))),
    }
}

fn schema_shape_kind(shape: &SchemaShape) -> &'static str {
    match shape {
        SchemaShape::Unit => "unit",
        SchemaShape::Bool => "bool",
        SchemaShape::String => "string",
        SchemaShape::Bytes => "bytes",
        SchemaShape::SignedInteger { .. } => "signed_integer",
        SchemaShape::UnsignedInteger { .. } => "unsigned_integer",
        SchemaShape::DecimalString { .. } => "decimal_string",
        SchemaShape::Option(_) => "option",
        SchemaShape::Vec(_) => "vec",
        SchemaShape::NonEmptyVec(_) => "non_empty_vec",
        SchemaShape::Tuple(_) => "tuple",
        SchemaShape::Struct { .. } => "struct",
        SchemaShape::BTreeMapString { .. } => "btree_map_string",
        SchemaShape::Enum { .. } => "enum",
        SchemaShape::ValueRef { .. } => "value_ref",
        SchemaShape::Generic { .. } => "generic",
    }
}

fn input_binding_node_kind_from_kind(kind: &InputBindingNodeKind) -> &'static str {
    match kind {
        InputBindingNodeKind::Unit => "unit",
        InputBindingNodeKind::Cell(_) => "cell",
        InputBindingNodeKind::Tuple { .. } => "tuple",
        InputBindingNodeKind::Struct { .. } => "struct",
        InputBindingNodeKind::Vec { .. } => "vec",
        InputBindingNodeKind::NonEmptyVec { .. } => "non_empty_vec",
    }
}

fn input_binding_digest(root: &InputBindingNode) -> Result<ContentDigest> {
    let json = serde_json::to_string(&input_binding_node_json(root))
        .map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(canonical.content_digest())
}

fn input_binding_node_json(node: &InputBindingNode) -> serde_json::Value {
    match &node.kind {
        InputBindingNodeKind::Unit => serde_json::json!({
            "kind": "unit",
        }),
        InputBindingNodeKind::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": cell.required_terminal.as_str(),
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.digest().as_str(),
        }),
        InputBindingNodeKind::Tuple { elements } => serde_json::json!({
            "elements": elements.iter().map(input_binding_node_json).collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        InputBindingNodeKind::Struct { fields } => serde_json::json!({
            "fields": fields
                .iter()
                .map(|field| {
                    serde_json::json!({
                        "field_path": field.field_path.as_str(),
                        "node": input_binding_node_json(&field.node),
                    })
                })
                .collect::<Vec<_>>(),
            "kind": "struct",
        }),
        InputBindingNodeKind::Vec { elements, ordering } => serde_json::json!({
            "elements": elements.iter().map(input_binding_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering.as_str(),
        }),
        InputBindingNodeKind::NonEmptyVec { elements, ordering } => serde_json::json!({
            "elements": elements.iter().map(input_binding_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering.as_str(),
        }),
    }
}

fn digest_only_id<I>(
    domain: &str,
    value: &str,
    construct: fn(DigestAlgorithm, DigestBytes) -> I,
) -> Result<I> {
    let digest = sha256_digest_bytes(format!("mfm.program:{domain}:{value}").as_bytes());
    Ok(construct(DigestAlgorithm::Sha256JcsV1, digest))
}

mod private {
    use super::{
        ApplySideEffect, Handle, ManagedPlatformWrite, MfmValue, Pure, ReadExternal,
        SideEffectState, StateSpec,
    };

    pub trait EffectRunnerSealed<S: StateSpec> {}
    pub trait BridgeableSealed {}

    impl<S> EffectRunnerSealed<S> for Pure where S: super::PureState {}

    impl<S> EffectRunnerSealed<S> for ReadExternal where S: super::ReadState {}

    impl<S> EffectRunnerSealed<S> for ManagedPlatformWrite where S: super::ManagedWriteState {}

    impl<S> EffectRunnerSealed<S> for ApplySideEffect where S: SideEffectState {}

    impl<'program, 'parent, T> BridgeableSealed for Handle<'program, 'parent, T> where T: MfmValue {}

    impl BridgeableSealed for () {}

    macro_rules! impl_tuple {
        ($($name:ident),+ $(,)?) => {
            impl<$($name),+> BridgeableSealed for ($($name,)+) {}
        };
    }

    impl_tuple!(A);
    impl_tuple!(A, B);
    impl_tuple!(A, B, C);
    impl_tuple!(A, B, C, D);
}
