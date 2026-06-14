#![warn(missing_docs)]
//! Typed state-program authoring contracts for MFM.
//!
//! This crate owns the branded authoring surface for typed programs. Handles
//! are minted only by framework builders, carry invariant program/scope/value
//! brands, and are lowered into unbranded draft specs only after root public
//! outputs have been bound inside the generative build closure.

extern crate self as mfm_program;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::ptr::NonNull;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySet, CapabilitySetDescriptor, CapabilitySetFor, NoCaps};
use mfm_effects::{
    ApplySideEffect, EffectDescriptor, EffectSpec, ManagedPlatformWrite, Pure, ReadExternal,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes,
    EffectKind, NodeId, OperationInstanceId, OperationKind, OperationVersion, SchemaId, ScopeId,
    SeedId, SemanticTypeId, StateKind, StateVersion,
};
pub use mfm_spec::v1::{ResourceClaimSpec, ResourceNamespace};
pub use mfm_values::NonEmpty;
use mfm_values::{
    MfmConfig, MfmValue, PublicOutputDescriptor, SchemaDescriptor, SchemaShape, StateInput,
    ValueTerminalPolicy,
};

#[cfg(test)]
mod tests;

const LOWERING_VERSION: &str = "mfm.typed.lowering.v1";

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
    /// An operation key was declared more than once in the same scope.
    DuplicateOperationKey(String),
    /// A stable domain key appeared more than once in a canonical collection.
    DuplicateDomainKey(String),
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
    /// Same-type same-scope lineage did not match the required lineage.
    LineageMismatch(String),
    /// State registry authority rejected planning.
    Registry(String),
    /// Live bridge evidence did not belong to the active child scope session.
    InvalidBridgeEvidence(String),
    /// A persisted bridge reference was not backed by an emitted bridge node.
    UnknownBridgeRef,
    /// Saga policy was declared more than once.
    SagaPolicyAlreadySet,
    /// A side-effecting draft did not declare its run-level saga policy.
    MissingSagaPolicy,
    /// A side-effecting state was planned through a builder that cannot declare a resource claim.
    SideEffectClaimRequired(String),
    /// The declared saga policy disagreed with the authored graph shape.
    SagaPolicyGraphMismatch(String),
    /// A compensating saga policy left a forward side-effect node unlinked.
    SagaCoverageGap(String),
    /// A remediation node binding referenced cells outside the linked forward node scope.
    RemediationBindingScope(String),
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
            Self::DuplicateOperationKey(key) => write!(f, "duplicate operation key {key}"),
            Self::DuplicateDomainKey(key) => write!(f, "duplicate stable domain key {key}"),
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
            Self::LineageMismatch(message) => write!(f, "value lineage mismatch: {message}"),
            Self::Registry(message) => write!(f, "state registry error: {message}"),
            Self::InvalidBridgeEvidence(message) => {
                write!(f, "invalid bridge evidence: {message}")
            }
            Self::UnknownBridgeRef => f.write_str("bridge ref is not backed by an emitted node"),
            Self::SagaPolicyAlreadySet => f.write_str("saga policy was already declared"),
            Self::MissingSagaPolicy => {
                f.write_str("side-effecting programs must declare a saga policy")
            }
            Self::SideEffectClaimRequired(message) => {
                write!(f, "side-effect resource claim required: {message}")
            }
            Self::SagaPolicyGraphMismatch(message) => {
                write!(f, "saga policy graph mismatch: {message}")
            }
            Self::SagaCoverageGap(message) => {
                write!(f, "saga compensation coverage gap: {message}")
            }
            Self::RemediationBindingScope(message) => {
                write!(f, "remediation binding scope violation: {message}")
            }
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

/// Stable operation author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationKey(String);

impl OperationKey {
    /// Creates a checked operation key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("operation key", value.as_ref()).map(Self)
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
        Self("root".to_owned())
    }

    /// Creates a checked input field path.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_field_path("input field path", value.as_ref()).map(Self)
    }

    /// Appends a checked child segment.
    pub fn child(&self, value: impl AsRef<str>) -> Result<Self> {
        let segment = checked_field_segment("input field segment", value.as_ref())?;
        if self.0 == "root" {
            Ok(Self(segment))
        } else {
            Ok(Self(format!("{}.{}", self.0, segment)))
        }
    }

    /// Returns the stable field path string.
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

    /// Returns behaviorally relevant adapter bindings for this state.
    fn adapter_bindings() -> Result<Vec<AdapterBindingSpec>> {
        Ok(Vec::new())
    }

    /// Constructs the executable state from validated config.
    fn new(config: Self::Config) -> Result<Self>
    where
        Self: Sized;
}

/// Descriptive contract for a deterministic typed operation expansion.
pub trait Operation: Send + Sync + 'static {
    /// Deterministic planning config type.
    type Config: MfmConfig;
    /// Typed handle input accepted by this operation expansion.
    type Input<'program, 'scope>: OperationInput<'program, 'scope>;
    /// Typed handle output produced by this operation expansion.
    type Output<'program, 'scope>: OperationOutput<'program, 'scope>;

    /// Returns the stable operation kind id.
    fn kind() -> Result<OperationKind>;

    /// Returns the operation descriptor version.
    fn version() -> Result<OperationVersion>;

    /// Returns the stable operation descriptor name.
    fn name() -> &'static str;

    /// Expands this operation into typed state and child-operation calls.
    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        dispatch: OperationExpansionDispatch<Self>,
    ) -> Result<Self::Output<'program, 'scope>>
    where
        Self: Sized;
}

/// Operation-specific framework dispatch authority for [`Operation::expand`].
///
/// The token is public so downstream crates can implement [`Operation`], but its fields and
/// constructor are private. Each token is bound to one operation type, so an operation body cannot
/// forward its own dispatch authority into another operation's `expand` method. Framework code
/// mints this token only inside [`ScopeBuilder::call_registered`].
#[doc(hidden)]
pub struct OperationExpansionDispatch<O: Operation> {
    _operation: PhantomData<fn() -> O>,
    _private: (),
}

impl<O: Operation> OperationExpansionDispatch<O> {
    fn new() -> Self {
        Self {
            _operation: PhantomData,
            _private: (),
        }
    }
}

/// Typed dynamic fanout key with canonical bytes for stable ordering.
pub trait StableDomainKey: MfmValue {
    /// Returns the domain-key schema descriptor.
    fn domain_key_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        Self::schema_descriptor()
    }

    /// Returns canonical domain-key bytes used for ordering and duplicate detection.
    fn canonical_domain_bytes(&self) -> Result<PlainCanonicalJsonBytes> {
        let json =
            serde_json::to_string(self).map_err(|error| PlanError::Serialize(error.to_string()))?;
        PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|error| PlanError::Canonical(error.to_string()))
    }
}

/// Behaviorally relevant adapter binding for a typed state node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterBindingSpec {
    /// Stable adapter kind id.
    pub adapter_kind: AdapterKind,
    /// Adapter contract version.
    pub adapter_version: AdapterVersion,
}

/// Finalized run-level saga policy recorded on a typed program draft.
///
/// Program builders derive [`SagaPolicy::NoSideEffects`] when the graph has no side-effecting
/// forward nodes. Authors select only [`SideEffectSagaPolicy`] for side-effecting workflows.
/// Certification lowers the finalized policy into the hash-defining `mfm-spec` contract and
/// re-verifies the hostile-bytes shape. Runtime decisions remain derived from this policy plus
/// recorded facts, not from author-emitted control events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SagaPolicy {
    /// Derived by finalization when the forward graph has no side-effect nodes.
    NoSideEffects,
    /// Failure after mutation carries no compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
    /// Failure after mutation blocks for typed operator evidence.
    ManualResolution {
        /// Required typed manual evidence.
        manual: ManualResolutionEvidence,
    },
    /// Failure after confirmed forward side effects compensates linked remediations.
    CompensateCompleted {
        /// Directive used when a forward or remediation ledger remains unresolved.
        on_remediation_unresolved: RemediationUnresolved,
    },
}

/// Author-selectable saga policy for side-effecting workflows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideEffectSagaPolicy {
    /// Failure after mutation carries no compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
    /// Failure after mutation blocks for typed operator evidence.
    ManualResolution {
        /// Required typed manual evidence.
        manual: ManualResolutionEvidence,
    },
    /// Failure after confirmed forward side effects compensates linked remediations.
    CompensateCompleted {
        /// Directive used when a forward or remediation ledger remains unresolved.
        on_remediation_unresolved: RemediationUnresolved,
    },
}

impl SideEffectSagaPolicy {
    fn is_compensating(&self) -> bool {
        matches!(self, Self::CompensateCompleted { .. })
    }
}

impl From<SideEffectSagaPolicy> for SagaPolicy {
    fn from(policy: SideEffectSagaPolicy) -> Self {
        match policy {
            SideEffectSagaPolicy::FailWithoutAcdcClaim => Self::FailWithoutAcdcClaim,
            SideEffectSagaPolicy::ManualResolution { manual } => Self::ManualResolution { manual },
            SideEffectSagaPolicy::CompensateCompleted {
                on_remediation_unresolved,
            } => Self::CompensateCompleted {
                on_remediation_unresolved,
            },
        }
    }
}

/// Directive for unresolved remediation under compensating policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemediationUnresolved {
    /// Block for typed operator evidence.
    ManualResolution {
        /// Required typed manual evidence.
        manual: Box<ManualResolutionEvidence>,
    },
    /// Terminally fail without a compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
}

/// Typed schema requirements for run-scoped manual saga resolution evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionEvidence {
    /// Schema id for the operator evidence artifact.
    pub evidence_schema: SchemaId,
    /// Schema id for the operator identity reference.
    pub operator_identity_ref_schema: SchemaId,
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
    side_effect_contract_digest: Option<ContentDigest>,
    runner: RunnerKind,
}

impl StateDescriptorIdentity {
    fn for_state<S: StateSpec>() -> Result<Self>
    where
        S::Effect: EffectRunner<S>,
    {
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
        let runner = <S::Effect as EffectRunner<S>>::runner_kind();
        let side_effect_contract_digest =
            <S::Effect as EffectRunner<S>>::side_effect_contract_digest()?;
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
            side_effect_contract_digest: side_effect_contract_digest.as_ref(),
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
            side_effect_contract_digest,
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

    /// Returns the side-effect contract digest when this descriptor mutates an external system.
    pub fn side_effect_contract_digest(&self) -> Option<&ContentDigest> {
        self.side_effect_contract_digest.as_ref()
    }

    /// Returns the registered runner kind.
    pub fn runner(&self) -> RunnerKind {
        self.runner
    }
}

/// Hash-defining registered operation descriptor identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationDescriptorIdentity {
    descriptor_id: DescriptorId,
    kind: OperationKind,
    version: OperationVersion,
    name: &'static str,
    config_schema_id: SchemaId,
    input_schema_id: SchemaId,
    output_schema_id: SchemaId,
    expansion_abi: &'static str,
}

impl OperationDescriptorIdentity {
    fn for_operation<O: Operation>() -> Result<Self> {
        let kind = O::kind()?;
        let version = O::version()?;
        let config_schema_id =
            O::Config::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let input_schema_id =
            <O::Input<'static, 'static> as OperationInput<'static, 'static>>::input_schema_id()
                .map_err(|error| PlanError::Value(error.to_string()))?;
        let output_schema_id =
            <O::Output<'static, 'static> as OperationOutput<'static, 'static>>::output_schema_id()?;
        let expansion_abi = "mfm.typed-operation-expand.v1";
        let descriptor_id = operation_descriptor_id(OperationDescriptorIdParts {
            kind: &kind,
            version: &version,
            name: O::name(),
            config_schema_id: &config_schema_id,
            input_schema_id: &input_schema_id,
            output_schema_id: &output_schema_id,
            expansion_abi,
        })?;
        Ok(Self {
            descriptor_id,
            kind,
            version,
            name: O::name(),
            config_schema_id,
            input_schema_id,
            output_schema_id,
            expansion_abi,
        })
    }

    /// Returns this descriptor's content-addressed identity.
    pub fn descriptor_id(&self) -> &DescriptorId {
        &self.descriptor_id
    }

    /// Returns the stable operation kind.
    pub fn kind(&self) -> &OperationKind {
        &self.kind
    }

    /// Returns the operation version.
    pub fn version(&self) -> &OperationVersion {
        &self.version
    }

    /// Returns the stable operation descriptor name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the config schema id.
    pub fn config_schema_id(&self) -> &SchemaId {
        &self.config_schema_id
    }

    /// Returns the operation input schema id.
    pub fn input_schema_id(&self) -> &SchemaId {
        &self.input_schema_id
    }

    /// Returns the operation output schema id.
    pub fn output_schema_id(&self) -> &SchemaId {
        &self.output_schema_id
    }

    /// Returns the deterministic expansion ABI name.
    pub fn expansion_abi(&self) -> &'static str {
        self.expansion_abi
    }
}

/// Error returned by typed registry operations.
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
    /// No registered operation matched the requested kind/version.
    UnregisteredOperation {
        /// Requested operation kind.
        kind: String,
        /// Requested operation version.
        version: String,
    },
    /// A different descriptor already owns this operation kind/version pair.
    DuplicateOperationRegistration {
        /// Registered operation kind.
        kind: String,
        /// Registered operation version.
        version: String,
    },
    /// Registry record and operation descriptor evidence diverged.
    OperationDescriptorMismatch {
        /// Registered operation kind.
        kind: String,
        /// Registered operation version.
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
            Self::UnregisteredOperation { kind, version } => {
                write!(f, "operation {kind}@{version} is not registered")
            }
            Self::DuplicateOperationRegistration { kind, version } => {
                write!(f, "duplicate operation registration for {kind}@{version}")
            }
            Self::OperationDescriptorMismatch { kind, version } => {
                write!(
                    f,
                    "operation registry descriptor mismatch for {kind}@{version}"
                )
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

/// Private registration evidence carried by a registered operation token.
#[derive(Debug, PartialEq, Eq)]
pub struct OperationRegistrationEvidence<O: Operation> {
    _operation: PhantomData<fn(O) -> O>,
    _private: (),
}

impl<O: Operation> Clone for OperationRegistrationEvidence<O> {
    fn clone(&self) -> Self {
        Self {
            _operation: PhantomData,
            _private: (),
        }
    }
}

/// Framework-owned authority token for a registered operation.
#[derive(Debug, PartialEq, Eq)]
pub struct RegisteredOperation<O: Operation> {
    descriptor: OperationDescriptorIdentity,
    evidence: OperationRegistrationEvidence<O>,
    _operation: PhantomData<fn(O) -> O>,
}

impl<O: Operation> Clone for RegisteredOperation<O> {
    fn clone(&self) -> Self {
        Self {
            descriptor: self.descriptor.clone(),
            evidence: self.evidence.clone(),
            _operation: PhantomData,
        }
    }
}

impl<O: Operation> RegisteredOperation<O> {
    fn new(descriptor: OperationDescriptorIdentity) -> Self {
        Self {
            descriptor,
            evidence: OperationRegistrationEvidence {
                _operation: PhantomData,
                _private: (),
            },
            _operation: PhantomData,
        }
    }

    /// Returns the validated operation descriptor identity.
    pub fn descriptor(&self) -> &OperationDescriptorIdentity {
        &self.descriptor
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OperationRegistrationKey {
    kind: String,
    version: String,
}

impl OperationRegistrationKey {
    fn from_descriptor(descriptor: &OperationDescriptorIdentity) -> Self {
        Self {
            kind: descriptor.kind().as_str().to_owned(),
            version: descriptor.version().as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OperationRegistrationRecord {
    descriptor_id: DescriptorId,
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

/// Immutable operation registry snapshot used by typed program builders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationRegistrySnapshot {
    records: std::collections::BTreeMap<OperationRegistrationKey, OperationRegistrationRecord>,
}

impl OperationRegistrySnapshot {
    /// Returns true when the registry has no registered operations.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Returns the number of registered operation kind/version pairs.
    pub fn len(&self) -> usize {
        self.records.len()
    }
}

/// Mutable framework operation registry builder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationRegistryBuilder {
    snapshot: OperationRegistrySnapshot,
}

impl OperationRegistryBuilder {
    /// Creates an empty registry builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an operation after validating descriptor evidence.
    pub fn register<O>(&mut self) -> std::result::Result<RegisteredOperation<O>, RegistryError>
    where
        O: Operation,
    {
        let descriptor = OperationDescriptorIdentity::for_operation::<O>()
            .map_err(|error| RegistryError::Descriptor(error.to_string()))?;
        let key = OperationRegistrationKey::from_descriptor(&descriptor);
        let record = OperationRegistrationRecord {
            descriptor_id: descriptor.descriptor_id().clone(),
        };
        if let Some(existing) = self.snapshot.records.get(&key) {
            if existing != &record {
                return Err(RegistryError::DuplicateOperationRegistration {
                    kind: key.kind,
                    version: key.version,
                });
            }
        } else {
            self.snapshot.records.insert(key, record);
        }
        Ok(RegisteredOperation::new(descriptor))
    }

    /// Returns an immutable registry snapshot.
    pub fn snapshot(&self) -> OperationRegistrySnapshot {
        self.snapshot.clone()
    }

    /// Converts this builder into an immutable registry snapshot.
    pub fn into_snapshot(self) -> OperationRegistrySnapshot {
        self.snapshot
    }
}

/// Framework-owned operation registry lookup contract.
pub trait OperationRegistry {
    /// Resolves a registered operation token for `O`.
    fn registered_operation<O>(&self) -> std::result::Result<RegisteredOperation<O>, RegistryError>
    where
        O: Operation;
}

impl OperationRegistry for OperationRegistrySnapshot {
    fn registered_operation<O>(&self) -> std::result::Result<RegisteredOperation<O>, RegistryError>
    where
        O: Operation,
    {
        let descriptor = OperationDescriptorIdentity::for_operation::<O>()
            .map_err(|error| RegistryError::Descriptor(error.to_string()))?;
        let key = OperationRegistrationKey::from_descriptor(&descriptor);
        let Some(record) = self.records.get(&key) else {
            return Err(RegistryError::UnregisteredOperation {
                kind: key.kind,
                version: key.version,
            });
        };
        if record.descriptor_id != *descriptor.descriptor_id() {
            return Err(RegistryError::OperationDescriptorMismatch {
                kind: key.kind,
                version: key.version,
            });
        }
        Ok(RegisteredOperation::new(descriptor))
    }
}

/// Sealed framework evidence that an effect has the matching state runner shape.
pub trait EffectRunner<S: StateSpec>: private::EffectRunnerSealed<S> {
    /// Returns the runner kind for this effect/state pair.
    fn runner_kind() -> RunnerKind;

    /// Returns the side-effect contract digest for external mutation states.
    fn side_effect_contract_digest() -> Result<Option<ContentDigest>> {
        Ok(None)
    }
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

    fn side_effect_contract_digest() -> Result<Option<ContentDigest>> {
        let digest = canonical_digest(serde_json::json!({
            "confirmation_schema_id": S::Confirmation::schema_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "confirmation_semantic_type_id": S::Confirmation::semantic_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "idempotency_input_schema_id": S::IdempotencyInput::schema_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "idempotency_input_semantic_type_id": S::IdempotencyInput::semantic_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "intent_schema_id": S::Intent::schema_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "intent_semantic_type_id": S::Intent::semantic_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "receipt_schema_id": S::Receipt::schema_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "receipt_semantic_type_id": S::Receipt::semantic_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "submission_schema_id": S::Submission::schema_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
            "submission_semantic_type_id": S::Submission::semantic_id()
                .map_err(|error| PlanError::Value(error.to_string()))?
                .as_str(),
        }))?;
        Ok(Some(digest))
    }
}

/// Producer of a typed cell in value-lineage evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CellProducer {
    /// Cell produced by a state or framework node.
    Node(NodeId),
    /// Cell produced by a launch seed.
    Seed(SeedId),
}

/// Policy describing how a value lineage relates to its inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LineageTransformPolicy {
    /// Source value with no same-type input dependency.
    Source,
    /// State output derived from declared input cells and config.
    StateOutput,
    /// Same-run same-value bridge.
    SameValueBridge,
}

impl LineageTransformPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::StateOutput => "state_output",
            Self::SameValueBridge => "same_value_bridge",
        }
    }
}

/// Canonical reference to a stable domain key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableDomainKeyRef {
    /// Domain-key schema id.
    pub schema_id: SchemaId,
    /// Canonical domain-key content digest.
    pub content_digest: ContentDigest,
}

impl StableDomainKeyRef {
    /// Builds a stable domain-key reference from a typed key.
    pub fn from_key<K: StableDomainKey>(key: &K) -> Result<Self> {
        let schema_id = K::domain_key_descriptor()
            .and_then(|descriptor| descriptor.schema_id())
            .map_err(|error| PlanError::Value(error.to_string()))?;
        let content_digest = key.canonical_domain_bytes()?.content_digest();
        Ok(Self {
            schema_id,
            content_digest,
        })
    }

    fn stable_sort_key(&self) -> String {
        format!(
            "{}:{}",
            self.schema_id.as_str(),
            self.content_digest.as_str()
        )
    }
}

fn stable_domain_key_refs<K: StableDomainKey>(keys: Vec<K>) -> Result<Vec<StableDomainKeyRef>> {
    let mut refs = keys
        .iter()
        .map(StableDomainKeyRef::from_key)
        .collect::<Result<Vec<_>>>()?;
    refs.sort();
    let mut seen = BTreeSet::new();
    for key_ref in &refs {
        if !seen.insert(key_ref.clone()) {
            return Err(PlanError::DuplicateDomainKey(key_ref.stable_sort_key()));
        }
    }
    Ok(refs)
}

/// Operation-lineage digest sequence visible to value-lineage records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationLineage {
    /// Operation instance ids from outermost to innermost active expansion.
    pub active_instances: Vec<OperationInstanceId>,
    /// Completed operation lineage frame digests already recorded in this scope.
    pub completed_frames: Vec<ContentDigest>,
    /// Digest of this lineage sequence.
    pub digest: ContentDigest,
}

impl OperationLineage {
    fn empty() -> Result<Self> {
        Self::from_parts(Vec::new(), Vec::new())
    }

    fn from_parts(
        active_instances: Vec<OperationInstanceId>,
        completed_frames: Vec<ContentDigest>,
    ) -> Result<Self> {
        let digest = canonical_digest(serde_json::json!({
            "active_instances": active_instances
                .iter()
                .map(OperationInstanceId::as_str)
                .collect::<Vec<_>>(),
            "completed_frames": completed_frames
                .iter()
                .map(ContentDigest::as_str)
                .collect::<Vec<_>>(),
        }))?;
        Ok(Self {
            active_instances,
            completed_frames,
            digest,
        })
    }
}

/// Hash-defining value-lineage evidence for a typed cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueLineage {
    /// Scope containing this value.
    pub scope_id: ScopeId,
    /// Producer for this value.
    pub producer: CellProducer,
    /// Input cells used to produce this value.
    pub input_cells: Vec<CellId>,
    /// Config reference digest used by the producer, when any.
    pub config_ref_digest: Option<ContentDigest>,
    /// Operation lineage active when this value was produced.
    pub operation_lineage: OperationLineage,
    /// Stable domain keys associated with this value.
    pub domain_keys: Vec<StableDomainKeyRef>,
    /// Lineage transform policy.
    pub transform_policy: LineageTransformPolicy,
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

/// Branded output handle for a forward side-effect node with linked remediation.
#[derive(Debug, PartialEq, Eq)]
pub struct ForwardSideEffectHandle<'program, 'scope, T: MfmValue> {
    node_id: NodeId,
    handle: Handle<'program, 'scope, T>,
}

impl<'program, 'scope, T: MfmValue> Clone for ForwardSideEffectHandle<'program, 'scope, T> {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id.clone(),
            handle: self.handle.clone(),
        }
    }
}

impl<'program, 'scope, T: MfmValue> ForwardSideEffectHandle<'program, 'scope, T> {
    fn new(node_id: NodeId, handle: Handle<'program, 'scope, T>) -> Self {
        Self { node_id, handle }
    }

    /// Returns the linked forward side-effect node id.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns this forward output as an ordinary typed handle reference.
    pub fn handle(&self) -> &Handle<'program, 'scope, T> {
        &self.handle
    }

    /// Converts this branded forward output into its ordinary typed handle.
    pub fn into_handle(self) -> Handle<'program, 'scope, T> {
        self.handle
    }

    /// Returns an unbranded typed handle reference for descriptors.
    pub fn typed_ref(&self) -> TypedHandleRef {
        self.handle.typed_ref()
    }
}

/// Branded handle for a remediation node.
///
/// This intentionally does not implement state-input conversion: remediation outputs are outside
/// the forward graph and cannot be scheduled by ordinary forward authoring APIs.
#[derive(Debug, PartialEq, Eq)]
pub struct RemediationHandle<'program, 'scope, T: MfmValue> {
    node_id: NodeId,
    handle: Handle<'program, 'scope, T>,
}

impl<'program, 'scope, T: MfmValue> Clone for RemediationHandle<'program, 'scope, T> {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id.clone(),
            handle: self.handle.clone(),
        }
    }
}

impl<'program, 'scope, T: MfmValue> RemediationHandle<'program, 'scope, T> {
    fn new(node_id: NodeId, handle: Handle<'program, 'scope, T>) -> Self {
        Self { node_id, handle }
    }

    /// Returns the remediation node id.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns an unbranded typed handle reference for audit and tests.
    pub fn typed_ref(&self) -> TypedHandleRef {
        self.handle.typed_ref()
    }
}

/// Parameters for a forward side-effect node in a linked compensation pair.
pub struct SideEffectNodeParams<S: SideEffectState, I> {
    /// Scope-local author key for the forward node.
    pub key: StateKey,
    /// Deterministic forward state config.
    pub config: S::Config,
    /// Forward state input binding source.
    pub input: I,
    /// Resource claim for the forward side-effect ledger.
    pub resource_claim: ResourceClaimSpec,
}

/// Parameters for a remediation node in a linked compensation pair.
pub struct RemediationNodeParams<R: SideEffectState> {
    /// Scope-local author key for the remediation node.
    pub key: StateKey,
    /// Deterministic remediation state config.
    pub config: R::Config,
    /// Resource claim for the remediation side-effect ledger.
    pub resource_claim: ResourceClaimSpec,
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
    /// Seed value lineage ref.
    pub value_lineage: ValueLineageRef,
    /// Canonical seed content digest.
    pub content_digest: ContentDigest,
    /// Canonical seed byte length.
    pub byte_len: usize,
}

impl RootSeedSpec {
    /// Returns this seed's typed cell reference.
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

/// Persisted typed scope specification emitted by the program builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSpec {
    /// Stable scope author key.
    pub key: ScopeKey,
    /// Derived scope id.
    pub scope_id: ScopeId,
    /// Parent scope id for child scopes.
    pub parent_scope_id: Option<ScopeId>,
    /// Operation lineage active when this scope id was derived.
    pub planning_lineage: OperationLineage,
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
    /// Canonical config reference digest.
    pub config_ref_digest: ContentDigest,
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
    /// Stable registered state descriptor name.
    pub state_descriptor_name: String,
    /// Registered runner kind.
    pub runner: RunnerKind,
    /// Framework effect kind required by the registered state.
    pub effect_kind: EffectKind,
    /// Framework capability descriptor set required by the registered state.
    pub capability_bindings: CapabilitySetDescriptor,
    /// Behaviorally relevant adapter bindings required by the registered state.
    pub adapter_bindings: Vec<AdapterBindingSpec>,
    /// Side-effect contract digest when this node mutates an external system.
    pub side_effect_contract_digest: Option<ContentDigest>,
    /// Cross-run resource claim when this node mutates an external system.
    pub side_effect_resource_claim: Option<ResourceClaimSpec>,
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
    /// Output value lineage ref.
    pub output_value_lineage: ValueLineageRef,
    /// Stable domain keys associated with the output value lineage.
    pub output_domain_keys: Vec<StableDomainKeyRef>,
    /// Planning lineage active while this node was emitted.
    pub planning_lineage: OperationLineage,
}

/// Operation lineage frame emitted by a registry-mediated call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationLineageFrameSpec {
    /// Derived operation instance id.
    pub operation_instance_id: OperationInstanceId,
    /// Stable operation author key.
    pub key: OperationKey,
    /// Owning scope id.
    pub scope_id: ScopeId,
    /// Registered operation kind.
    pub operation_kind: OperationKind,
    /// Registered operation version.
    pub operation_version: OperationVersion,
    /// Registered operation descriptor id.
    pub operation_descriptor_id: DescriptorId,
    /// Stable registered operation descriptor name.
    pub operation_name: String,
    /// Deterministic expansion ABI recorded by the descriptor.
    pub expansion_abi: &'static str,
    /// Operation lineage active before this operation expanded.
    pub parent_operation_lineage: OperationLineage,
    /// Canonical operation config binding.
    pub config: ConfigBindingSpec,
    /// Typed operation input binding.
    pub input: OperationInputBindingSpec,
    /// Operation output schema id.
    pub output_schema_id: SchemaId,
    /// Output handles actually returned by expansion.
    pub output_handles: Vec<TypedHandleRef>,
    /// Digest of this lineage frame.
    pub lineage_digest: ContentDigest,
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
    /// Target value lineage ref.
    pub target_value_lineage: ValueLineageRef,
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
    /// Planning lineage active while this bridge was emitted.
    pub planning_lineage: OperationLineage,
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OrderingEvidence {
    /// Author-provided vector order.
    ExplicitAuthorOrder,
    /// Canonical order by stable domain key.
    StableDomainKey,
}

impl OrderingEvidence {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ExplicitAuthorOrder => "explicit_author_order",
            Self::StableDomainKey => "stable_domain_key",
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
        domain_keys: Vec<StableDomainKeyRef>,
    },
    NonEmptyVec {
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
        domain_keys: Vec<StableDomainKeyRef>,
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

/// Read-only view over a typed input cell binding.
#[derive(Debug, Clone, Copy)]
pub struct InputCellBindingRef<'a> {
    binding: &'a InputCellBinding,
}

impl<'a> InputCellBindingRef<'a> {
    /// Returns the input field path.
    pub fn field_path(&self) -> &'a InputFieldPath {
        &self.binding.field_path
    }

    /// Returns the referenced cell id.
    pub fn cell_id(&self) -> &'a CellId {
        &self.binding.cell_id
    }

    /// Returns the referenced semantic type id.
    pub fn semantic_type_id(&self) -> &'a SemanticTypeId {
        &self.binding.semantic_type_id
    }

    /// Returns the referenced schema id.
    pub fn schema_id(&self) -> &'a SchemaId {
        &self.binding.schema_id
    }

    /// Returns the referenced value-lineage ref.
    pub fn value_lineage(&self) -> &'a ValueLineageRef {
        &self.binding.value_lineage
    }

    /// Returns the required terminal policy.
    pub fn required_terminal(&self) -> RequiredTerminal {
        self.binding.required_terminal
    }
}

/// Read-only view over a typed input binding node.
#[derive(Debug, Clone, Copy)]
pub enum InputBindingNodeRef<'a> {
    /// Unit input.
    Unit,
    /// Typed cell input.
    Cell(InputCellBindingRef<'a>),
    /// Tuple input.
    Tuple(&'a [InputBindingNode]),
    /// Struct input.
    Struct(&'a [NamedInputBinding]),
    /// Vector input.
    Vec {
        /// Element bindings.
        elements: &'a [InputBindingNode],
        /// Ordering evidence.
        ordering: &'a OrderingEvidence,
        /// Stable domain-key refs.
        domain_keys: &'a [StableDomainKeyRef],
    },
    /// Non-empty vector input.
    NonEmptyVec {
        /// Element bindings.
        elements: &'a [InputBindingNode],
        /// Ordering evidence.
        ordering: &'a OrderingEvidence,
        /// Stable domain-key refs.
        domain_keys: &'a [StableDomainKeyRef],
    },
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

    fn vector(
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
        domain_keys: Vec<StableDomainKeyRef>,
    ) -> Self {
        Self {
            kind: InputBindingNodeKind::Vec {
                elements,
                ordering,
                domain_keys,
            },
        }
    }

    fn non_empty_vector(
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
        domain_keys: Vec<StableDomainKeyRef>,
    ) -> Self {
        Self {
            kind: InputBindingNodeKind::NonEmptyVec {
                elements,
                ordering,
                domain_keys,
            },
        }
    }

    /// Returns a read-only structural view of this binding node.
    pub fn as_ref(&self) -> InputBindingNodeRef<'_> {
        match &self.kind {
            InputBindingNodeKind::Unit => InputBindingNodeRef::Unit,
            InputBindingNodeKind::Cell(binding) => {
                InputBindingNodeRef::Cell(InputCellBindingRef { binding })
            }
            InputBindingNodeKind::Tuple { elements } => InputBindingNodeRef::Tuple(elements),
            InputBindingNodeKind::Struct { fields } => InputBindingNodeRef::Struct(fields),
            InputBindingNodeKind::Vec {
                elements,
                ordering,
                domain_keys,
            } => InputBindingNodeRef::Vec {
                elements,
                ordering,
                domain_keys,
            },
            InputBindingNodeKind::NonEmptyVec {
                elements,
                ordering,
                domain_keys,
            } => InputBindingNodeRef::NonEmptyVec {
                elements,
                ordering,
                domain_keys,
            },
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

/// Persisted operation-input binding spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationInputBindingSpec {
    /// Input schema id.
    pub input_schema_id: SchemaId,
    /// Input descriptor id.
    pub input_descriptor_id: DescriptorId,
    /// Root input binding node.
    pub root: InputBindingNode,
    /// Canonical digest of the root binding tree.
    pub digest: ContentDigest,
}

impl OperationInputBindingSpec {
    /// Converts a validated state-input binding into an operation-input binding.
    pub fn from_binding<I: StateInput>(binding: InputBinding<I>) -> Self {
        Self::from_state_input_spec(binding.spec())
    }

    fn from_state_input_spec(spec: InputBindingSpec) -> Self {
        Self {
            input_schema_id: spec.input_schema_id,
            input_descriptor_id: spec.input_descriptor_id,
            root: spec.root,
            digest: spec.digest,
        }
    }
}

/// Typed operation input contract over framework-owned branded handle wrappers.
pub trait OperationInput<'program, 'scope>: private::OperationInputSealed {
    /// Runtime input descriptor represented by this handle-side input.
    type Runtime: StateInput;

    /// Returns the input schema id.
    fn input_schema_id() -> mfm_values::Result<SchemaId> {
        Self::Runtime::input_schema_id()
    }

    /// Returns the canonical operation input binding.
    fn input_binding(&self) -> Result<OperationInputBindingSpec>;
}

/// Author-side conversion into an operation's typed input value.
pub trait IntoOperationInput<'program, 'scope, I: OperationInput<'program, 'scope>> {
    /// Converts into the operation input expected by `Operation::expand`.
    fn into_operation_input(self) -> Result<I>;
}

impl<'program, 'scope, I> IntoOperationInput<'program, 'scope, I> for I
where
    I: OperationInput<'program, 'scope>,
{
    fn into_operation_input(self) -> Result<I> {
        Ok(self)
    }
}

/// Typed operation output contract over branded handles.
pub trait OperationOutput<'program, 'scope> {
    /// Returns the operation output schema id.
    fn output_schema_id() -> Result<SchemaId>
    where
        Self: Sized;

    /// Returns typed handles actually produced by this operation expansion.
    fn output_handles(&self) -> Result<Vec<TypedHandleRef>>;
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

impl<'program, 'scope, T> IntoInputBindingNode<T> for ForwardSideEffectHandle<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        self.into_handle().into_binding_node(field_path)
    }
}

impl<'program, 'scope, T> IntoStateInput<'program, 'scope, T>
    for ForwardSideEffectHandle<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<T>> {
        self.into_handle().into_binding()
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
            Vec::new(),
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
#[derive(Debug, PartialEq, Eq)]
pub struct NonEmptyHandles<'program, 'scope, T: MfmValue> {
    handles: Vec<Handle<'program, 'scope, T>>,
}

impl<'program, 'scope, T: MfmValue> Clone for NonEmptyHandles<'program, 'scope, T> {
    fn clone(&self) -> Self {
        Self {
            handles: self.handles.clone(),
        }
    }
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
            Vec::new(),
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

/// Author-side handles paired with stable domain keys.
#[derive(Debug, PartialEq, Eq)]
pub struct DomainKeyedHandles<'program, 'scope, K: StableDomainKey, T: MfmValue> {
    entries: Vec<DomainKeyedHandle<'program, 'scope, K, T>>,
}

/// Author-side non-empty handles paired with stable domain keys.
#[derive(Debug, PartialEq, Eq)]
pub struct DomainKeyedNonEmptyHandles<'program, 'scope, K: StableDomainKey, T: MfmValue> {
    entries: Vec<DomainKeyedHandle<'program, 'scope, K, T>>,
}

#[derive(Debug, PartialEq, Eq)]
struct DomainKeyedHandle<'program, 'scope, K: StableDomainKey, T: MfmValue> {
    key_ref: StableDomainKeyRef,
    canonical_domain_bytes: PlainCanonicalJsonBytes,
    handle: Handle<'program, 'scope, T>,
    _key: PhantomData<fn(K) -> K>,
}

impl<'program, 'scope, K, T> DomainKeyedHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    /// Creates domain-keyed handles, rejecting duplicate canonical keys and sorting by canonical bytes.
    pub fn new(entries: Vec<(K, Handle<'program, 'scope, T>)>) -> Result<Self> {
        Ok(Self {
            entries: Self::sorted_entries(entries)?,
        })
    }

    fn sorted_entries(
        entries: Vec<(K, Handle<'program, 'scope, T>)>,
    ) -> Result<Vec<DomainKeyedHandle<'program, 'scope, K, T>>> {
        let mut keyed = entries
            .into_iter()
            .map(|(key, handle)| {
                let schema_id = K::domain_key_descriptor()
                    .and_then(|descriptor| descriptor.schema_id())
                    .map_err(|error| PlanError::Value(error.to_string()))?;
                let canonical_domain_bytes = key.canonical_domain_bytes()?;
                let key_ref = StableDomainKeyRef {
                    schema_id,
                    content_digest: canonical_domain_bytes.content_digest(),
                };
                Ok(DomainKeyedHandle {
                    key_ref,
                    canonical_domain_bytes,
                    handle,
                    _key: PhantomData,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        keyed.sort_by(|left, right| {
            left.key_ref
                .schema_id
                .cmp(&right.key_ref.schema_id)
                .then_with(|| {
                    left.canonical_domain_bytes
                        .cmp(&right.canonical_domain_bytes)
                })
        });
        let mut seen = BTreeSet::new();
        for entry in &keyed {
            let key = (
                entry.key_ref.schema_id.clone(),
                entry.canonical_domain_bytes.clone(),
            );
            if !seen.insert(key) {
                return Err(PlanError::DuplicateDomainKey(
                    entry.key_ref.stable_sort_key(),
                ));
            }
        }
        Ok(keyed)
    }

    /// Returns the number of keyed handles.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no keyed handles are present.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<'program, 'scope, K, T> DomainKeyedNonEmptyHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    /// Creates non-empty domain-keyed handles, rejecting empty input and duplicate canonical keys.
    pub fn new(entries: Vec<(K, Handle<'program, 'scope, T>)>) -> Result<Self> {
        if entries.is_empty() {
            return Err(PlanError::EmptyNonEmptyInput);
        }
        Ok(Self {
            entries: DomainKeyedHandles::<K, T>::sorted_entries(entries)?,
        })
    }

    /// Returns the number of keyed handles.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no keyed handles are present.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<'program, 'scope, K, T> IntoInputBindingNode<Vec<T>>
    for DomainKeyedHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        let mut domain_keys = Vec::with_capacity(self.entries.len());
        let mut elements = Vec::with_capacity(self.entries.len());
        for (index, entry) in self.entries.into_iter().enumerate() {
            domain_keys.push(entry.key_ref);
            elements.push(
                entry
                    .handle
                    .into_binding_node(field_path.child(index.to_string())?)?,
            );
        }
        Ok(InputBindingNode::vector(
            elements,
            OrderingEvidence::StableDomainKey,
            domain_keys,
        ))
    }
}

impl<'program, 'scope, K, T> IntoStateInput<'program, 'scope, Vec<T>>
    for DomainKeyedHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<Vec<T>>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

impl<'program, 'scope, K, T> IntoInputBindingNode<NonEmpty<T>>
    for DomainKeyedNonEmptyHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        if self.entries.is_empty() {
            return Err(PlanError::EmptyNonEmptyInput);
        }
        let mut domain_keys = Vec::with_capacity(self.entries.len());
        let mut elements = Vec::with_capacity(self.entries.len());
        for (index, entry) in self.entries.into_iter().enumerate() {
            domain_keys.push(entry.key_ref);
            elements.push(
                entry
                    .handle
                    .into_binding_node(field_path.child(index.to_string())?)?,
            );
        }
        Ok(InputBindingNode::non_empty_vector(
            elements,
            OrderingEvidence::StableDomainKey,
            domain_keys,
        ))
    }
}

impl<'program, 'scope, K, T> IntoStateInput<'program, 'scope, NonEmpty<T>>
    for DomainKeyedNonEmptyHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<NonEmpty<T>>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

impl<'program, 'scope> OperationInput<'program, 'scope> for () {
    type Runtime = ();

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            InputBinding::<()>::from_root(InputBindingNode::Unit)?.spec(),
        ))
    }
}

impl<'program, 'scope, T> OperationInput<'program, 'scope> for Handle<'program, 'scope, T>
where
    T: MfmValue,
{
    type Runtime = T;

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            self.clone().into_binding()?.spec(),
        ))
    }
}

impl<'program, 'scope, T> OperationInput<'program, 'scope>
    for ForwardSideEffectHandle<'program, 'scope, T>
where
    T: MfmValue,
{
    type Runtime = T;

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            self.clone().into_binding()?.spec(),
        ))
    }
}

impl<'program, 'scope, T> OperationInput<'program, 'scope> for Vec<Handle<'program, 'scope, T>>
where
    T: MfmValue,
{
    type Runtime = Vec<T>;

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            self.clone().into_binding()?.spec(),
        ))
    }
}

impl<'program, 'scope, T> OperationInput<'program, 'scope> for NonEmptyHandles<'program, 'scope, T>
where
    T: MfmValue,
{
    type Runtime = NonEmpty<T>;

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            self.clone().into_binding()?.spec(),
        ))
    }
}

macro_rules! impl_tuple_operation_input {
    ($($name:ident),+ $(,)?) => {
        impl<'program, 'scope, $($name),+> OperationInput<'program, 'scope>
            for ($(Handle<'program, 'scope, $name>,)+)
        where
            $($name: MfmValue,)+
        {
            type Runtime = ($($name,)+);

            fn input_binding(&self) -> Result<OperationInputBindingSpec> {
                Ok(OperationInputBindingSpec::from_state_input_spec(
                    self.clone().into_binding()?.spec(),
                ))
            }
        }
    };
}

impl_tuple_operation_input!(A);
impl_tuple_operation_input!(A, B);
impl_tuple_operation_input!(A, B, C);
impl_tuple_operation_input!(A, B, C, D);
impl_tuple_operation_input!(A, B, C, D, E);
impl_tuple_operation_input!(A, B, C, D, E, F);
impl_tuple_operation_input!(A, B, C, D, E, F, G);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H, I);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H, I, J);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H, I, J, K);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H, I, J, K, L);

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
    saga_policy: SagaPolicy,
    seeds: Vec<RootSeedSpec>,
    scopes: Vec<ScopeSpec>,
    state_nodes: Vec<StateNodeSpec>,
    remediation_nodes: BTreeMap<NodeId, StateNodeSpec>,
    operation_lineage: Vec<OperationLineageFrameSpec>,
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

    /// Returns the run-level saga policy for this draft.
    pub fn saga_policy(&self) -> &SagaPolicy {
        &self.saga_policy
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

    /// Returns remediation nodes keyed by the forward side-effect node they compensate.
    pub fn remediation_nodes(&self) -> &BTreeMap<NodeId, StateNodeSpec> {
        &self.remediation_nodes
    }

    /// Returns registry-mediated operation lineage frames.
    pub fn operation_lineage(&self) -> &[OperationLineageFrameSpec] {
        &self.operation_lineage
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

    /// Validates same-scope same-type lineage equality for certification fixtures.
    pub fn validate_same_scope_same_type_lineage_for_certification(
        &self,
        expected: &TypedHandleRef,
        actual: &TypedHandleRef,
    ) -> Result<()> {
        if expected.scope_id == actual.scope_id
            && expected.schema_id == actual.schema_id
            && expected.semantic_type_id == actual.semantic_type_id
            && expected.value_lineage != actual.value_lineage
        {
            return Err(PlanError::LineageMismatch(format!(
                "scope={} schema={} semantic={} expected={} actual={}",
                expected.scope_id.as_str(),
                expected.schema_id.as_str(),
                expected.semantic_type_id.as_str(),
                expected.value_lineage.digest().as_str(),
                actual.value_lineage.digest().as_str()
            )));
        }
        Ok(())
    }
}

/// Root-scope builder for a typed program.
pub struct RootBuilder<'program, 'scope> {
    root_key: ScopeKey,
    scope: ScopeBuilder<'program, 'scope>,
    seeds: Vec<RootSeedSpec>,
    seed_keys: BTreeSet<String>,
    public_outputs_bound: bool,
    saga_policy: Option<SideEffectSagaPolicy>,
}

impl<'program, 'scope> RootBuilder<'program, 'scope> {
    /// Returns the root scope builder.
    pub fn scope(&mut self) -> &mut ScopeBuilder<'program, 'scope> {
        &mut self.scope
    }

    /// Declares the run-level saga policy for side-effecting workflows.
    pub fn set_saga_policy(&mut self, policy: SideEffectSagaPolicy) -> Result<()> {
        if self.saga_policy.is_some() {
            return Err(PlanError::SagaPolicyAlreadySet);
        }
        self.saga_policy = Some(policy);
        Ok(())
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

        let operation_lineage = self.scope.current_operation_lineage()?;
        let seed_id = seed_id(self.scope.scope_id(), &key, &value)?;
        let cell_id = seed_cell_id(self.scope.scope_id(), &seed_id, &value)?;
        let lineage = seed_value_lineage(self.scope.scope_id(), &seed_id, &operation_lineage)?;
        let value_lineage = value_lineage_ref(&lineage)?;
        let spec = RootSeedSpec {
            key,
            seed_id,
            cell_id: cell_id.clone(),
            scope_id: self.scope.scope_id().clone(),
            schema_id: value.schema_id.clone(),
            semantic_type_id: value.semantic_type_id.clone(),
            value_lineage: value_lineage.clone(),
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
    operation_registry: OperationRegistrySnapshot,
    state_keys: BTreeSet<String>,
    operation_keys: BTreeSet<String>,
    state_nodes: Vec<StateNodeSpec>,
    remediation_nodes: BTreeMap<NodeId, StateNodeSpec>,
    operation_lineage: Vec<OperationLineageFrameSpec>,
    active_operation_stack: Vec<OperationInstanceId>,
    child_scope_keys: BTreeSet<String>,
    child_scopes: Vec<ScopeSpec>,
    bridge_nodes: Vec<BridgeNodeSpec>,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

#[derive(Debug, Clone)]
struct ScopeBuilderCheckpoint {
    state_keys: BTreeSet<String>,
    operation_keys: BTreeSet<String>,
    state_nodes: Vec<StateNodeSpec>,
    remediation_nodes: BTreeMap<NodeId, StateNodeSpec>,
    operation_lineage: Vec<OperationLineageFrameSpec>,
    active_operation_stack: Vec<OperationInstanceId>,
    child_scope_keys: BTreeSet<String>,
    child_scopes: Vec<ScopeSpec>,
    bridge_nodes: Vec<BridgeNodeSpec>,
}

impl<'program, 'scope> ScopeBuilder<'program, 'scope> {
    /// Returns the typed scope id.
    pub fn scope_id(&self) -> &ScopeId {
        &self.scope_id
    }

    fn current_operation_lineage(&self) -> Result<OperationLineage> {
        OperationLineage::from_parts(
            self.active_operation_stack.clone(),
            self.operation_lineage
                .iter()
                .map(|frame| frame.lineage_digest.clone())
                .collect(),
        )
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

        let operation_lineage = self.current_operation_lineage()?;
        let child_scope_id = child_scope_id(&self.scope_id, &key, &operation_lineage)?;
        let session_token = bridge_session_token(&self.scope_id, &child_scope_id, &key);
        let mut child = ChildScopeBuilder {
            parent_scope_id: self.scope_id.clone(),
            scope: ScopeBuilder {
                scope_id: child_scope_id.clone(),
                state_registry: self.state_registry.clone(),
                operation_registry: self.operation_registry.clone(),
                state_keys: BTreeSet::new(),
                operation_keys: BTreeSet::new(),
                state_nodes: Vec::new(),
                remediation_nodes: BTreeMap::new(),
                operation_lineage: Vec::new(),
                active_operation_stack: self.active_operation_stack.clone(),
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
            planning_lineage: operation_lineage,
        });
        self.state_nodes.extend(child.scope.state_nodes);
        self.remediation_nodes.extend(child.scope.remediation_nodes);
        self.operation_lineage.extend(child.scope.operation_lineage);
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
        self.state_registered_with_domain_key_refs(key, registered, config, input, Vec::new())
    }

    /// Plans a registered typed state and attaches stable domain-key evidence to its output
    /// value lineage.
    pub fn state_with_domain_keys<S, I, K>(
        &mut self,
        key: StateKey,
        config: S::Config,
        input: I,
        domain_keys: Vec<K>,
    ) -> Result<Handle<'program, 'scope, S::Output>>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
        K: StableDomainKey,
    {
        let registered = self.state_registry.registered_state::<S>()?;
        self.state_registered_with_domain_key_refs(
            key,
            registered,
            config,
            input,
            stable_domain_key_refs(domain_keys)?,
        )
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
        self.state_registered_with_domain_key_refs(key, registered, config, input, Vec::new())
    }

    /// Plans a typed state from an explicit registration token and attaches stable domain-key
    /// evidence to its output value lineage.
    pub fn state_registered_with_domain_keys<S, I, K>(
        &mut self,
        key: StateKey,
        registered: RegisteredState<S>,
        config: S::Config,
        input: I,
        domain_keys: Vec<K>,
    ) -> Result<Handle<'program, 'scope, S::Output>>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
        K: StableDomainKey,
    {
        self.state_registered_with_domain_key_refs(
            key,
            registered,
            config,
            input,
            stable_domain_key_refs(domain_keys)?,
        )
    }

    /// Plans one forward side-effect state and one structurally separate remediation state.
    ///
    /// The forward node is appended to the ordinary forward graph. The remediation node is keyed
    /// by the forward node id in the draft remediation collection, so the forward scheduler cannot
    /// select it. The remediation input may reference only the linked forward output and cells
    /// that are transitive ancestors of that forward node.
    pub fn side_effect_with_compensation<F, R, I, J, B>(
        &mut self,
        forward_params: SideEffectNodeParams<F, I>,
        remediation_params: RemediationNodeParams<R>,
        build_remediation_input: B,
    ) -> Result<(
        ForwardSideEffectHandle<'program, 'scope, F::Output>,
        RemediationHandle<'program, 'scope, R::Output>,
    )>
    where
        F: SideEffectState,
        F::Caps: CapabilitySetFor<ApplySideEffect>,
        I: IntoStateInput<'program, 'scope, F::Input>,
        R: SideEffectState,
        R::Caps: CapabilitySetFor<ApplySideEffect>,
        J: IntoStateInput<'program, 'scope, R::Input>,
        B: FnOnce(ForwardSideEffectHandle<'program, 'scope, F::Output>) -> Result<J>,
    {
        let SideEffectNodeParams {
            key: forward_key,
            config: forward_config,
            input: forward_input,
            resource_claim: forward_resource_claim,
        } = forward_params;
        let RemediationNodeParams {
            key: remediation_key,
            config: remediation_config,
            resource_claim: remediation_resource_claim,
        } = remediation_params;
        let checkpoint = self.checkpoint();
        let forward_key_string = forward_key.as_str().to_owned();
        if self.state_keys.contains(&forward_key_string) {
            return Err(PlanError::DuplicateStateKey(
                forward_key.as_str().to_owned(),
            ));
        }
        let remediation_key_string = remediation_key.as_str().to_owned();
        if self.state_keys.contains(&remediation_key_string)
            || remediation_key_string == forward_key_string
        {
            return Err(PlanError::DuplicateStateKey(
                remediation_key.as_str().to_owned(),
            ));
        }

        let forward_registered = self.state_registry.registered_state::<F>()?;
        let (forward_node, forward_handle) = self.plan_state_node(
            forward_key,
            forward_registered,
            forward_config,
            forward_input,
            Vec::new(),
            Some(forward_resource_claim),
        )?;
        let forward = ForwardSideEffectHandle::new(forward_node.node_id.clone(), forward_handle);
        let remediation_input = match build_remediation_input(forward.clone()) {
            Ok(input) => input,
            Err(error) => {
                self.restore(checkpoint);
                return Err(error);
            }
        };

        let remediation_registered = match self.state_registry.registered_state::<R>() {
            Ok(registered) => registered,
            Err(error) => {
                self.restore(checkpoint);
                return Err(error.into());
            }
        };
        let (remediation_node, remediation_handle) = match self.plan_state_node(
            remediation_key,
            remediation_registered,
            remediation_config,
            remediation_input,
            Vec::new(),
            Some(remediation_resource_claim),
        ) {
            Ok(planned) => planned,
            Err(error) => {
                self.restore(checkpoint);
                return Err(error);
            }
        };
        if let Err(error) =
            self.validate_remediation_binding_scope(&forward_node, &remediation_node)
        {
            self.restore(checkpoint);
            return Err(error);
        }
        if self.remediation_nodes.contains_key(&forward_node.node_id) {
            self.restore(checkpoint);
            return Err(PlanError::SagaCoverageGap(format!(
                "forward side-effect node {} already has remediation",
                forward_node.node_id.as_str()
            )));
        }

        self.state_keys.insert(forward_key_string);
        self.state_keys.insert(remediation_key_string);
        self.state_nodes.push(forward_node.clone());
        self.remediation_nodes
            .insert(forward_node.node_id.clone(), remediation_node.clone());
        Ok((
            forward,
            RemediationHandle::new(remediation_node.node_id, remediation_handle),
        ))
    }

    fn state_registered_with_domain_key_refs<S, I>(
        &mut self,
        key: StateKey,
        registered: RegisteredState<S>,
        config: S::Config,
        input: I,
        output_domain_keys: Vec<StableDomainKeyRef>,
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
        let (node, handle) =
            self.plan_state_node(key, registered, config, input, output_domain_keys, None)?;
        self.state_keys.insert(key_string);
        self.state_nodes.push(node);
        Ok(handle)
    }

    /// Plans a registered side-effect state with an explicit resource claim.
    pub fn side_effect<S, I>(
        &mut self,
        key: StateKey,
        config: S::Config,
        input: I,
        resource_claim: ResourceClaimSpec,
    ) -> Result<ForwardSideEffectHandle<'program, 'scope, S::Output>>
    where
        S: SideEffectState,
        S::Caps: CapabilitySetFor<ApplySideEffect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        let key_string = key.as_str().to_owned();
        if self.state_keys.contains(&key_string) {
            return Err(PlanError::DuplicateStateKey(key.as_str().to_owned()));
        }
        let registered = self.state_registry.registered_state::<S>()?;
        let (node, handle) = self.plan_state_node(
            key,
            registered,
            config,
            input,
            Vec::new(),
            Some(resource_claim),
        )?;
        self.state_keys.insert(key_string);
        self.state_nodes.push(node.clone());
        Ok(ForwardSideEffectHandle::new(node.node_id, handle))
    }

    fn plan_state_node<S, I>(
        &self,
        key: StateKey,
        registered: RegisteredState<S>,
        config: S::Config,
        input: I,
        output_domain_keys: Vec<StableDomainKeyRef>,
        resource_claim: Option<ResourceClaimSpec>,
    ) -> Result<(StateNodeSpec, Handle<'program, 'scope, S::Output>)>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        let config_binding = canonical_config_binding::<S::Config>(&config)?;
        let input = input.into_binding()?;
        let adapter_bindings = S::adapter_bindings()?;
        let state = S::new(config)?;
        drop(state);

        let descriptor = registered.descriptor();
        let side_effect_contract_digest = descriptor.side_effect_contract_digest().cloned();
        match (&side_effect_contract_digest, &resource_claim) {
            (Some(_), Some(_)) | (None, None) => {}
            (Some(_), None) => {
                return Err(PlanError::SideEffectClaimRequired(format!(
                    "state {} must be planned with side_effect or side_effect_with_compensation",
                    descriptor.name()
                )));
            }
            (None, Some(_)) => {
                return Err(PlanError::SagaPolicyGraphMismatch(format!(
                    "non-side-effect state {} cannot declare a resource claim",
                    descriptor.name()
                )));
            }
        }
        let node_id = state_node_id(
            &self.scope_id,
            &key,
            descriptor.kind(),
            descriptor.version(),
            &config_binding.config_ref_digest,
            input.digest(),
        )?;
        let output_schema_id =
            S::Output::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let output_semantic_type_id =
            S::Output::semantic_id().map_err(|error| PlanError::Value(error.to_string()))?;
        let output_cell_id = state_output_cell_id(
            &self.scope_id,
            &node_id,
            &output_semantic_type_id,
            &output_schema_id,
        )?;
        let lineage = state_value_lineage(
            &self.scope_id,
            &node_id,
            input.root(),
            &config_binding.config_ref_digest,
            &self.current_operation_lineage()?,
            output_domain_keys.clone(),
        )?;
        let value_lineage = value_lineage_ref(&lineage)?;
        let handle = Handle::new(
            output_cell_id.clone(),
            self.scope_id.clone(),
            output_schema_id.clone(),
            output_semantic_type_id.clone(),
            value_lineage.clone(),
        );
        let planning_lineage = self.current_operation_lineage()?;
        Ok((
            StateNodeSpec {
                node_id,
                key,
                scope_id: self.scope_id.clone(),
                state_kind: descriptor.kind().clone(),
                state_version: descriptor.version().clone(),
                state_descriptor_id: descriptor.descriptor_id().clone(),
                state_descriptor_name: descriptor.name().to_owned(),
                runner: registered.runner(),
                effect_kind: descriptor.effect().kind.clone(),
                capability_bindings: descriptor.capabilities().clone(),
                adapter_bindings,
                side_effect_contract_digest,
                side_effect_resource_claim: resource_claim,
                config: config_binding,
                input: input.spec(),
                output_cell_id,
                output_schema_id,
                output_semantic_type_id,
                output_value_lineage: value_lineage,
                output_domain_keys,
                planning_lineage,
            },
            handle,
        ))
    }

    fn validate_remediation_binding_scope(
        &self,
        forward_node: &StateNodeSpec,
        remediation_node: &StateNodeSpec,
    ) -> Result<()> {
        let mut allowed = BTreeSet::new();
        allowed.insert(forward_node.output_cell_id.clone());

        let mut forward_inputs = Vec::new();
        collect_input_cell_ids(&forward_node.input.root, &mut forward_inputs);
        for cell in forward_inputs {
            self.collect_ancestor_cell(&cell, &mut allowed);
        }

        let mut remediation_inputs = Vec::new();
        collect_input_cell_ids(&remediation_node.input.root, &mut remediation_inputs);
        for cell in remediation_inputs {
            if !allowed.contains(&cell) {
                return Err(PlanError::RemediationBindingScope(format!(
                    "remediation node {} references cell {} outside linked forward node {} scope",
                    remediation_node.node_id.as_str(),
                    cell.as_str(),
                    forward_node.node_id.as_str()
                )));
            }
        }
        Ok(())
    }

    fn collect_ancestor_cell(&self, cell: &CellId, output: &mut BTreeSet<CellId>) {
        if !output.insert(cell.clone()) {
            return;
        }
        if let Some(node) = self
            .state_nodes
            .iter()
            .find(|node| node.output_cell_id == *cell)
        {
            let mut inputs = Vec::new();
            collect_input_cell_ids(&node.input.root, &mut inputs);
            for input in inputs {
                self.collect_ancestor_cell(&input, output);
            }
        }
        if let Some(bridge) = self
            .bridge_nodes
            .iter()
            .find(|bridge| bridge.target_cell_id == *cell)
        {
            self.collect_ancestor_cell(&bridge.source_cell_id, output);
        }
    }

    /// Expands a registered typed operation by resolving `O` through this builder's registry.
    pub fn call<O, I>(
        &mut self,
        key: OperationKey,
        operation: O,
        config: O::Config,
        input: I,
    ) -> Result<O::Output<'program, 'scope>>
    where
        O: Operation,
        I: IntoOperationInput<'program, 'scope, O::Input<'program, 'scope>>,
    {
        let registered = self.operation_registry.registered_operation::<O>()?;
        self.call_registered(key, registered, operation, config, input)
    }

    /// Expands a typed operation from an explicit framework-owned registration token.
    pub fn call_registered<O, I>(
        &mut self,
        key: OperationKey,
        registered: RegisteredOperation<O>,
        operation: O,
        config: O::Config,
        input: I,
    ) -> Result<O::Output<'program, 'scope>>
    where
        O: Operation,
        I: IntoOperationInput<'program, 'scope, O::Input<'program, 'scope>>,
    {
        let key_string = key.as_str().to_owned();
        if self.operation_keys.contains(&key_string) {
            return Err(PlanError::DuplicateOperationKey(key.as_str().to_owned()));
        }
        let config_binding = canonical_config_binding::<O::Config>(&config)?;
        let operation_input = input.into_operation_input()?;
        let input_binding = operation_input.input_binding()?;
        let output_schema_id =
            <O::Output<'program, 'scope> as OperationOutput<'program, 'scope>>::output_schema_id()?;
        let descriptor = registered.descriptor().clone();
        let parent_operation_lineage = self.current_operation_lineage()?;
        let operation_instance_id = operation_instance_id(OperationInstanceIdParts {
            scope_id: &self.scope_id,
            key: &key,
            descriptor: &descriptor,
            parent_operation_lineage: &parent_operation_lineage,
            config_digest: &config_binding.config_ref_digest,
            input_digest: &input_binding.digest,
        })?;
        let checkpoint = self.checkpoint();
        self.operation_keys.insert(key_string);
        self.active_operation_stack
            .push(operation_instance_id.clone());

        let mut expansion = OperationExpansion::new(self);
        let output = match operation.expand(
            config,
            operation_input,
            &mut expansion,
            OperationExpansionDispatch::<O>::new(),
        ) {
            Ok(output) => output,
            Err(error) => {
                self.restore(checkpoint);
                return Err(error);
            }
        };
        let output_handles = match output.output_handles() {
            Ok(handles) => handles,
            Err(error) => {
                self.restore(checkpoint);
                return Err(error);
            }
        };
        self.active_operation_stack.pop();
        let lineage_digest =
            match operation_lineage_frame_digest(OperationLineageFrameDigestParts {
                scope_id: &self.scope_id,
                key: &key,
                operation_instance_id: &operation_instance_id,
                descriptor: &descriptor,
                parent_operation_lineage: &parent_operation_lineage,
                config_digest: &config_binding.config_ref_digest,
                input_digest: &input_binding.digest,
                output_handles: &output_handles,
            }) {
                Ok(digest) => digest,
                Err(error) => {
                    self.restore(checkpoint);
                    return Err(error);
                }
            };
        self.operation_lineage.push(OperationLineageFrameSpec {
            operation_instance_id,
            key,
            scope_id: self.scope_id.clone(),
            operation_kind: descriptor.kind().clone(),
            operation_version: descriptor.version().clone(),
            operation_descriptor_id: descriptor.descriptor_id().clone(),
            operation_name: descriptor.name().to_owned(),
            expansion_abi: descriptor.expansion_abi(),
            parent_operation_lineage,
            config: config_binding,
            input: input_binding,
            output_schema_id,
            output_handles,
            lineage_digest,
        });
        Ok(output)
    }

    fn checkpoint(&self) -> ScopeBuilderCheckpoint {
        ScopeBuilderCheckpoint {
            state_keys: self.state_keys.clone(),
            operation_keys: self.operation_keys.clone(),
            state_nodes: self.state_nodes.clone(),
            remediation_nodes: self.remediation_nodes.clone(),
            operation_lineage: self.operation_lineage.clone(),
            active_operation_stack: self.active_operation_stack.clone(),
            child_scope_keys: self.child_scope_keys.clone(),
            child_scopes: self.child_scopes.clone(),
            bridge_nodes: self.bridge_nodes.clone(),
        }
    }

    fn restore(&mut self, checkpoint: ScopeBuilderCheckpoint) {
        self.state_keys = checkpoint.state_keys;
        self.operation_keys = checkpoint.operation_keys;
        self.state_nodes = checkpoint.state_nodes;
        self.remediation_nodes = checkpoint.remediation_nodes;
        self.operation_lineage = checkpoint.operation_lineage;
        self.active_operation_stack = checkpoint.active_operation_stack;
        self.child_scope_keys = checkpoint.child_scope_keys;
        self.child_scopes = checkpoint.child_scopes;
        self.bridge_nodes = checkpoint.bridge_nodes;
    }
}

/// Operation-local planning context minted only by [`ScopeBuilder::call`] and
/// [`ScopeBuilder::call_registered`].
///
/// The fields are private so downstream crates cannot construct this context or call
/// [`Operation::expand`] directly. Authoring code inside an operation can compose registered
/// states, nested operations, and child scopes, but it cannot insert raw graph nodes, raw
/// dependencies, raw cell ids, or lineage evidence.
pub struct OperationExpansion<'program, 'scope> {
    scope: NonNull<ScopeBuilder<'program, 'scope>>,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl<'program, 'scope> OperationExpansion<'program, 'scope> {
    fn new(scope: &mut ScopeBuilder<'program, 'scope>) -> Self {
        Self {
            scope: NonNull::from(scope),
            _program: PhantomData,
            _scope: PhantomData,
            _not_send_sync: PhantomData,
        }
    }

    fn scope_mut(&mut self) -> &mut ScopeBuilder<'program, 'scope> {
        // SAFETY: OperationExpansion is created only by ScopeBuilder::call_registered from its
        // exclusive `&mut self`. The raw pointer is never exposed, all mutation goes through
        // `&mut OperationExpansion`, and call_registered does not touch the ScopeBuilder again
        // until operation expansion returns.
        unsafe { self.scope.as_mut() }
    }

    /// Returns the typed scope id currently being expanded into.
    pub fn scope_id(&self) -> &ScopeId {
        // SAFETY: See `scope_mut`; shared access here does not permit mutation or aliasing escape.
        unsafe { self.scope.as_ref().scope_id() }
    }

    /// Opens a child scope and returns only values explicitly bridged back to this scope.
    pub fn child_scope<R>(
        &mut self,
        key: ScopeKey,
        f: impl for<'child> FnOnce(
            &mut ChildScopeBuilder<'program, 'scope, 'child>,
        ) -> Result<Bridged<'program, 'scope, R>>,
    ) -> Result<R> {
        self.scope_mut().child_scope(key, f)
    }

    /// Plans a registered typed state by resolving `S` through this expansion's registry.
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
        self.scope_mut().state::<S, I>(key, config, input)
    }

    /// Plans a registered typed state and attaches stable domain-key evidence to its output.
    pub fn state_with_domain_keys<S, I, K>(
        &mut self,
        key: StateKey,
        config: S::Config,
        input: I,
        domain_keys: Vec<K>,
    ) -> Result<Handle<'program, 'scope, S::Output>>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
        K: StableDomainKey,
    {
        self.scope_mut()
            .state_with_domain_keys::<S, I, K>(key, config, input, domain_keys)
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
        self.scope_mut()
            .state_registered::<S, I>(key, registered, config, input)
    }

    /// Plans a typed state from an explicit token and attaches stable domain-key evidence.
    pub fn state_registered_with_domain_keys<S, I, K>(
        &mut self,
        key: StateKey,
        registered: RegisteredState<S>,
        config: S::Config,
        input: I,
        domain_keys: Vec<K>,
    ) -> Result<Handle<'program, 'scope, S::Output>>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
        K: StableDomainKey,
    {
        self.scope_mut()
            .state_registered_with_domain_keys::<S, I, K>(
                key,
                registered,
                config,
                input,
                domain_keys,
            )
    }

    /// Plans a registered side-effect state with an explicit resource claim.
    pub fn side_effect<S, I>(
        &mut self,
        key: StateKey,
        config: S::Config,
        input: I,
        resource_claim: ResourceClaimSpec,
    ) -> Result<ForwardSideEffectHandle<'program, 'scope, S::Output>>
    where
        S: SideEffectState,
        S::Caps: CapabilitySetFor<ApplySideEffect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        self.scope_mut()
            .side_effect::<S, I>(key, config, input, resource_claim)
    }

    /// Plans a linked forward/remediation side-effect pair.
    pub fn side_effect_with_compensation<F, R, I, J, B>(
        &mut self,
        forward_params: SideEffectNodeParams<F, I>,
        remediation_params: RemediationNodeParams<R>,
        build_remediation_input: B,
    ) -> Result<(
        ForwardSideEffectHandle<'program, 'scope, F::Output>,
        RemediationHandle<'program, 'scope, R::Output>,
    )>
    where
        F: SideEffectState,
        F::Caps: CapabilitySetFor<ApplySideEffect>,
        I: IntoStateInput<'program, 'scope, F::Input>,
        R: SideEffectState,
        R::Caps: CapabilitySetFor<ApplySideEffect>,
        J: IntoStateInput<'program, 'scope, R::Input>,
        B: FnOnce(ForwardSideEffectHandle<'program, 'scope, F::Output>) -> Result<J>,
    {
        self.scope_mut()
            .side_effect_with_compensation::<F, R, I, J, B>(
                forward_params,
                remediation_params,
                build_remediation_input,
            )
    }

    /// Expands a registered typed operation by resolving `O` through this expansion's registry.
    pub fn call<O, I>(
        &mut self,
        key: OperationKey,
        operation: O,
        config: O::Config,
        input: I,
    ) -> Result<O::Output<'program, 'scope>>
    where
        O: Operation,
        I: IntoOperationInput<'program, 'scope, O::Input<'program, 'scope>>,
    {
        self.scope_mut().call::<O, I>(key, operation, config, input)
    }

    /// Expands a typed operation from an explicit framework-owned registration token.
    pub fn call_registered<O, I>(
        &mut self,
        key: OperationKey,
        registered: RegisteredOperation<O>,
        operation: O,
        config: O::Config,
        input: I,
    ) -> Result<O::Output<'program, 'scope>>
    where
        O: Operation,
        I: IntoOperationInput<'program, 'scope, O::Input<'program, 'scope>>,
    {
        self.scope_mut()
            .call_registered::<O, I>(key, registered, operation, config, input)
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
        let target_cell_id = bridge_cell_id(
            &target_scope_id,
            &node_id,
            &source.semantic_type_id,
            &source.schema_id,
        )?;
        let planning_lineage = self.scope.current_operation_lineage()?;
        let target_value_lineage = value_lineage_ref(&bridge_value_lineage(
            &target_scope_id,
            &node_id,
            &source.cell_id,
            &planning_lineage,
        )?)?;
        let spec = BridgeNodeSpec {
            node_id: node_id.clone(),
            key,
            source_scope_id,
            target_scope_id: target_scope_id.clone(),
            source_cell_id: source.cell_id.clone(),
            target_cell_id: target_cell_id.clone(),
            target_value_lineage: target_value_lineage.clone(),
            semantic_type_id: source.semantic_type_id.clone(),
            schema_id: source.schema_id.clone(),
            bridge_kind,
            policy,
            provenance: BridgeProvenance::FrameworkChildScopeV1,
            planning_lineage,
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
    build_root_with_registries(
        root_key,
        StateRegistrySnapshot::default(),
        OperationRegistrySnapshot::default(),
        f,
    )
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
    build_root_with_registries(
        root_key,
        state_registry,
        OperationRegistrySnapshot::default(),
        f,
    )
}

/// Builds a branded root typed program with state and operation registry snapshots.
pub fn build_root_with_registries<F>(
    root_key: ScopeKey,
    state_registry: StateRegistrySnapshot,
    operation_registry: OperationRegistrySnapshot,
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
            operation_registry,
            state_keys: BTreeSet::new(),
            operation_keys: BTreeSet::new(),
            state_nodes: Vec::new(),
            remediation_nodes: BTreeMap::new(),
            operation_lineage: Vec::new(),
            active_operation_stack: Vec::new(),
            child_scope_keys: BTreeSet::new(),
            child_scopes: Vec::new(),
            bridge_nodes: Vec::new(),
            _program: PhantomData,
            _scope: PhantomData,
        },
        seeds: Vec::new(),
        seed_keys: BTreeSet::new(),
        public_outputs_bound: false,
        saga_policy: None,
    };
    let bound = f(&mut builder)?;
    let saga_policy = finalize_saga_policy(builder.saga_policy, &builder.scope)?;
    Ok(TypedProgramDraft {
        root_key: builder.root_key,
        root_scope_id: root_scope_id.clone(),
        saga_policy,
        seeds: builder.seeds,
        scopes: {
            let mut scopes = vec![ScopeSpec {
                key: root_key,
                scope_id: root_scope_id,
                parent_scope_id: None,
                planning_lineage: OperationLineage::empty()?,
            }];
            scopes.extend(builder.scope.child_scopes);
            scopes
        },
        state_nodes: builder.scope.state_nodes,
        remediation_nodes: builder.scope.remediation_nodes,
        operation_lineage: builder.scope.operation_lineage,
        bridge_nodes: builder.scope.bridge_nodes,
        public_output_spec: bound.public_output_spec,
    })
}

/// Returns the public schema id for a derive-backed public output descriptor.
pub fn public_schema_id<P: PublicOutputDescriptor>() -> Result<SchemaId> {
    P::public_schema_id().map_err(|error| PlanError::Value(error.to_string()))
}

fn finalize_saga_policy(
    policy: Option<SideEffectSagaPolicy>,
    scope: &ScopeBuilder<'_, '_>,
) -> Result<SagaPolicy> {
    let forward_side_effects = scope
        .state_nodes
        .iter()
        .filter(|node| node.runner == RunnerKind::ApplySideEffect)
        .collect::<Vec<_>>();

    if forward_side_effects.is_empty() {
        if !scope.remediation_nodes.is_empty() {
            return Err(PlanError::SagaPolicyGraphMismatch(
                "remediation nodes require a compensating side-effecting forward graph".to_owned(),
            ));
        }
        return match policy {
            None => Ok(SagaPolicy::NoSideEffects),
            Some(_) => Err(PlanError::SagaPolicyGraphMismatch(
                "non-NoSideEffects policy requires at least one forward side-effect node"
                    .to_owned(),
            )),
        };
    }

    let Some(policy) = policy else {
        return Err(PlanError::MissingSagaPolicy);
    };
    if policy.is_compensating() {
        for node in &forward_side_effects {
            if !scope.remediation_nodes.contains_key(&node.node_id) {
                return Err(PlanError::SagaCoverageGap(format!(
                    "forward side-effect node {} has no linked remediation",
                    node.node_id.as_str()
                )));
            }
        }
        for (forward_node_id, remediation) in &scope.remediation_nodes {
            if !forward_side_effects
                .iter()
                .any(|node| node.node_id == *forward_node_id)
            {
                return Err(PlanError::SagaCoverageGap(format!(
                    "remediation node {} links missing forward node {}",
                    remediation.node_id.as_str(),
                    forward_node_id.as_str()
                )));
            }
        }
    } else if !scope.remediation_nodes.is_empty() {
        return Err(PlanError::SagaPolicyGraphMismatch(
            "remediation nodes are valid only under CompensateCompleted policy".to_owned(),
        ));
    }

    Ok(policy.into())
}

fn checked_key(label: &str, value: &str) -> Result<String> {
    if is_valid_author_key(value) {
        Ok(value.to_owned())
    } else {
        Err(PlanError::Key(format!(
            "{label} {value:?} must use stable key grammar"
        )))
    }
}

fn is_valid_author_key(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 256
        || value.starts_with("mfm.")
        || value.starts_with("sys.")
        || value.starts_with('_')
    {
        return false;
    }
    value.split('/').all(is_valid_author_key_segment)
}

fn is_valid_author_key_segment(segment: &str) -> bool {
    if segment.is_empty() || segment.len() > 64 {
        return false;
    }
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')
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

fn canonical_digest(value: serde_json::Value) -> Result<ContentDigest> {
    let json =
        serde_json::to_string(&value).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(canonical.content_digest())
}

fn canonical_digest_bytes(value: serde_json::Value) -> Result<DigestBytes> {
    Ok(*canonical_digest(value)?.digest())
}

fn stable_domain_key_refs_json(domain_keys: &[StableDomainKeyRef]) -> Vec<serde_json::Value> {
    domain_keys
        .iter()
        .map(|key| {
            serde_json::json!({
                "content_digest": key.content_digest.as_str(),
                "schema_id": key.schema_id.as_str(),
            })
        })
        .collect()
}

fn operation_lineage_json(lineage: &OperationLineage) -> serde_json::Value {
    serde_json::json!({
        "active_instances": lineage
            .active_instances
            .iter()
            .map(OperationInstanceId::as_str)
            .collect::<Vec<_>>(),
        "completed_frames": lineage
            .completed_frames
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>(),
        "digest": lineage.digest.as_str(),
    })
}

fn scope_id(key: &ScopeKey) -> Result<ScopeId> {
    scope_id_from_parts(None, key, &OperationLineage::empty()?)
}

fn scope_id_from_parts(
    parent_scope_id: Option<&ScopeId>,
    key: &ScopeKey,
    operation_lineage: &OperationLineage,
) -> Result<ScopeId> {
    Ok(ScopeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "local_scope_key": key.as_str(),
            "operation_lineage": operation_lineage.digest.as_str(),
            "parent_scope_id": parent_scope_id.map(ScopeId::as_str),
        }))?,
    ))
}

fn child_scope_id(
    parent_scope_id: &ScopeId,
    key: &ScopeKey,
    operation_lineage: &OperationLineage,
) -> Result<ScopeId> {
    scope_id_from_parts(Some(parent_scope_id), key, operation_lineage)
}

fn seed_id<T: MfmValue>(
    scope_id: &ScopeId,
    key: &SeedKey,
    seed: &CanonicalSeed<T>,
) -> Result<SeedId> {
    Ok(SeedId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": LOWERING_VERSION,
            "scope_id": scope_id.as_str(),
            "seed_key": key.as_str(),
            "semantic_type_id": seed.semantic_type_id.as_str(),
            "schema_id": seed.schema_id.as_str(),
        }))?,
    ))
}

fn seed_cell_id<T: MfmValue>(
    scope_id: &ScopeId,
    seed_id: &SeedId,
    seed: &CanonicalSeed<T>,
) -> Result<CellId> {
    cell_id(
        scope_id,
        &CellProducer::Seed(seed_id.clone()),
        &seed.semantic_type_id,
        &seed.schema_id,
    )
}

fn seed_value_lineage(
    scope_id: &ScopeId,
    seed_id: &SeedId,
    operation_lineage: &OperationLineage,
) -> Result<ValueLineage> {
    Ok(ValueLineage {
        scope_id: scope_id.clone(),
        producer: CellProducer::Seed(seed_id.clone()),
        input_cells: Vec::new(),
        config_ref_digest: None,
        operation_lineage: operation_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: LineageTransformPolicy::Source,
    })
}

fn bridge_value_lineage(
    target_scope_id: &ScopeId,
    node_id: &NodeId,
    source_cell_id: &CellId,
    operation_lineage: &OperationLineage,
) -> Result<ValueLineage> {
    Ok(ValueLineage {
        scope_id: target_scope_id.clone(),
        producer: CellProducer::Node(node_id.clone()),
        input_cells: vec![source_cell_id.clone()],
        config_ref_digest: None,
        operation_lineage: operation_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: LineageTransformPolicy::SameValueBridge,
    })
}

fn value_lineage_ref(lineage: &ValueLineage) -> Result<ValueLineageRef> {
    let mut input_cells = lineage.input_cells.clone();
    input_cells.sort();
    let mut domain_keys = lineage.domain_keys.clone();
    domain_keys.sort();
    let digest = canonical_digest(serde_json::json!({
        "config_ref_digest": lineage.config_ref_digest.as_ref().map(ContentDigest::as_str),
        "domain_keys": stable_domain_key_refs_json(&domain_keys),
        "input_cells": input_cells.iter().map(CellId::as_str).collect::<Vec<_>>(),
        "operation_lineage": operation_lineage_json(&lineage.operation_lineage),
        "producer": cell_producer_json(&lineage.producer),
        "scope_id": lineage.scope_id.as_str(),
        "transform_policy": lineage.transform_policy.as_str(),
    }))?;
    Ok(ValueLineageRef::new(digest))
}

fn bridge_node_id(
    source_scope_id: &ScopeId,
    target_scope_id: &ScopeId,
    source_cell_id: &CellId,
    key: &BridgeKey,
    bridge_kind: BridgeKind,
    policy: BridgePolicy,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "bridge_kind": bridge_kind.as_str(),
            "local_node_key": key.as_str(),
            "lowering_version": LOWERING_VERSION,
            "policy": policy.as_str(),
            "source_cell_id": source_cell_id.as_str(),
            "source_scope_id": source_scope_id.as_str(),
            "target_scope_id": target_scope_id.as_str(),
        }))?,
    ))
}

fn bridge_cell_id(
    target_scope_id: &ScopeId,
    node_id: &NodeId,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    cell_id(
        target_scope_id,
        &CellProducer::Node(node_id.clone()),
        semantic_type_id,
        schema_id,
    )
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
    let config_ref_digest = canonical_digest(serde_json::json!({
        "byte_len": byte_len,
        "content_digest": content_digest.as_str(),
        "schema_id": schema_id.as_str(),
    }))?;
    Ok(ConfigBindingSpec {
        schema_id,
        canonical_json: canonical,
        content_digest,
        config_ref_digest,
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
    side_effect_contract_digest: Option<&'a ContentDigest>,
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
                    "name": capability.name.as_str(),
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
        "side_effect_contract_digest": parts
            .side_effect_contract_digest
            .map(ContentDigest::as_str),
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

struct OperationDescriptorIdParts<'a> {
    kind: &'a OperationKind,
    version: &'a OperationVersion,
    name: &'static str,
    config_schema_id: &'a SchemaId,
    input_schema_id: &'a SchemaId,
    output_schema_id: &'a SchemaId,
    expansion_abi: &'static str,
}

fn operation_descriptor_id(parts: OperationDescriptorIdParts<'_>) -> Result<DescriptorId> {
    let json = serde_json::json!({
        "config_schema_id": parts.config_schema_id.as_str(),
        "expansion_abi": parts.expansion_abi,
        "input_schema_id": parts.input_schema_id.as_str(),
        "kind": parts.kind.as_str(),
        "name": parts.name,
        "output_schema_id": parts.output_schema_id.as_str(),
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

fn state_node_id(
    scope_id: &ScopeId,
    key: &StateKey,
    state_kind: &StateKind,
    state_version: &StateVersion,
    config_digest: &ContentDigest,
    input_digest: &ContentDigest,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": config_digest.as_str(),
            "input_binding_digest": input_digest.as_str(),
            "local_node_key": key.as_str(),
            "lowering_version": LOWERING_VERSION,
            "scope_id": scope_id.as_str(),
            "state_kind": state_kind.as_str(),
            "state_version": state_version.as_str(),
        }))?,
    ))
}

fn state_output_cell_id(
    scope_id: &ScopeId,
    node_id: &NodeId,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    cell_id(
        scope_id,
        &CellProducer::Node(node_id.clone()),
        semantic_type_id,
        schema_id,
    )
}

fn state_value_lineage(
    scope_id: &ScopeId,
    node_id: &NodeId,
    input_root: &InputBindingNode,
    config_ref_digest: &ContentDigest,
    operation_lineage: &OperationLineage,
    domain_keys: Vec<StableDomainKeyRef>,
) -> Result<ValueLineage> {
    let mut input_cells = Vec::new();
    collect_input_cell_ids(input_root, &mut input_cells);
    input_cells.sort();
    Ok(ValueLineage {
        scope_id: scope_id.clone(),
        producer: CellProducer::Node(node_id.clone()),
        input_cells,
        config_ref_digest: Some(config_ref_digest.clone()),
        operation_lineage: operation_lineage.clone(),
        domain_keys,
        transform_policy: LineageTransformPolicy::StateOutput,
    })
}

fn cell_id(
    scope_id: &ScopeId,
    producer: &CellProducer,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    Ok(CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": LOWERING_VERSION,
            "output_index": 0,
            "producer": cell_producer_json(producer),
            "schema_id": schema_id.as_str(),
            "scope_id": scope_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?,
    ))
}

fn cell_producer_json(producer: &CellProducer) -> serde_json::Value {
    match producer {
        CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
        CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
    }
}

struct OperationInstanceIdParts<'a> {
    scope_id: &'a ScopeId,
    key: &'a OperationKey,
    descriptor: &'a OperationDescriptorIdentity,
    parent_operation_lineage: &'a OperationLineage,
    config_digest: &'a ContentDigest,
    input_digest: &'a ContentDigest,
}

fn operation_instance_id(parts: OperationInstanceIdParts<'_>) -> Result<OperationInstanceId> {
    Ok(OperationInstanceId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": parts.config_digest.as_str(),
            "input_binding_digest": parts.input_digest.as_str(),
            "operation_descriptor_id": parts.descriptor.descriptor_id().as_str(),
            "operation_key": parts.key.as_str(),
            "operation_kind": parts.descriptor.kind().as_str(),
            "operation_version": parts.descriptor.version().as_str(),
            "parent_operation_lineage": parts.parent_operation_lineage.digest.as_str(),
            "parent_scope_id": parts.scope_id.as_str(),
        }))?,
    ))
}

struct OperationLineageFrameDigestParts<'a> {
    scope_id: &'a ScopeId,
    key: &'a OperationKey,
    operation_instance_id: &'a OperationInstanceId,
    descriptor: &'a OperationDescriptorIdentity,
    parent_operation_lineage: &'a OperationLineage,
    config_digest: &'a ContentDigest,
    input_digest: &'a ContentDigest,
    output_handles: &'a [TypedHandleRef],
}

fn operation_lineage_frame_digest(
    parts: OperationLineageFrameDigestParts<'_>,
) -> Result<ContentDigest> {
    let json = serde_json::json!({
        "config_digest": parts.config_digest.as_str(),
        "expansion_abi": parts.descriptor.expansion_abi(),
        "input_digest": parts.input_digest.as_str(),
        "operation_descriptor_id": parts.descriptor.descriptor_id().as_str(),
        "operation_instance_id": parts.operation_instance_id.as_str(),
        "operation_key": parts.key.as_str(),
        "operation_kind": parts.descriptor.kind().as_str(),
        "operation_version": parts.descriptor.version().as_str(),
        "parent_operation_lineage": parts.parent_operation_lineage.digest.as_str(),
        "output_handles": parts.output_handles
            .iter()
            .map(|handle| {
                serde_json::json!({
                    "cell_id": handle.cell_id().as_str(),
                    "schema_id": handle.schema_id().as_str(),
                    "scope_id": handle.scope_id().as_str(),
                    "semantic_type_id": handle.semantic_type_id().as_str(),
                    "value_lineage": handle.value_lineage().digest().as_str(),
                })
            })
            .collect::<Vec<_>>(),
        "scope_id": parts.scope_id.as_str(),
    });
    let json =
        serde_json::to_string(&json).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(canonical.content_digest())
}

fn input_descriptor_id(input_schema_id: &SchemaId) -> Result<DescriptorId> {
    digest_only_id(
        "input-descriptor",
        input_schema_id.as_str(),
        DescriptorId::from_digest,
    )
}

fn collect_input_cell_ids(node: &InputBindingNode, output: &mut Vec<CellId>) {
    match &node.kind {
        InputBindingNodeKind::Unit => {}
        InputBindingNodeKind::Cell(cell) => output.push(cell.cell_id.clone()),
        InputBindingNodeKind::Tuple { elements }
        | InputBindingNodeKind::Vec { elements, .. }
        | InputBindingNodeKind::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cell_ids(element, output);
            }
        }
        InputBindingNodeKind::Struct { fields } => {
            for field in fields {
                collect_input_cell_ids(&field.node, output);
            }
        }
    }
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
        InputBindingNodeKind::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": stable_domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_binding_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering.as_str(),
        }),
        InputBindingNodeKind::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": stable_domain_key_refs_json(domain_keys),
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
        ApplySideEffect, ForwardSideEffectHandle, Handle, ManagedPlatformWrite, MfmValue,
        NonEmptyHandles, Pure, ReadExternal, SideEffectState, StateSpec,
    };

    pub trait EffectRunnerSealed<S: StateSpec> {}
    pub trait OperationInputSealed {}
    pub trait BridgeableSealed {}

    impl<S> EffectRunnerSealed<S> for Pure where S: super::PureState {}

    impl<S> EffectRunnerSealed<S> for ReadExternal where S: super::ReadState {}

    impl<S> EffectRunnerSealed<S> for ManagedPlatformWrite where S: super::ManagedWriteState {}

    impl<S> EffectRunnerSealed<S> for ApplySideEffect where S: SideEffectState {}

    impl OperationInputSealed for () {}

    impl<'program, 'scope, T> OperationInputSealed for Handle<'program, 'scope, T> where T: MfmValue {}

    impl<'program, 'scope, T> OperationInputSealed for ForwardSideEffectHandle<'program, 'scope, T> where
        T: MfmValue
    {
    }

    impl<'program, 'scope, T> OperationInputSealed for Vec<Handle<'program, 'scope, T>> where T: MfmValue
    {}

    impl<'program, 'scope, T> OperationInputSealed for NonEmptyHandles<'program, 'scope, T> where
        T: MfmValue
    {
    }

    impl<'program, 'parent, T> BridgeableSealed for Handle<'program, 'parent, T> where T: MfmValue {}

    impl BridgeableSealed for () {}

    macro_rules! impl_tuple {
        ($($name:ident),+ $(,)?) => {
            impl<$($name),+> BridgeableSealed for ($($name,)+) {}
        };
    }

    macro_rules! impl_operation_input_tuple {
        ($($name:ident),+ $(,)?) => {
            impl<'program, 'scope, $($name),+> OperationInputSealed
                for ($(Handle<'program, 'scope, $name>,)+)
            where
                $($name: MfmValue,)+
            {
            }
        };
    }

    impl_tuple!(A);
    impl_tuple!(A, B);
    impl_tuple!(A, B, C);
    impl_tuple!(A, B, C, D);

    impl_operation_input_tuple!(A);
    impl_operation_input_tuple!(A, B);
    impl_operation_input_tuple!(A, B, C);
    impl_operation_input_tuple!(A, B, C, D);
    impl_operation_input_tuple!(A, B, C, D, E);
    impl_operation_input_tuple!(A, B, C, D, E, F);
    impl_operation_input_tuple!(A, B, C, D, E, F, G);
    impl_operation_input_tuple!(A, B, C, D, E, F, G, H);
    impl_operation_input_tuple!(A, B, C, D, E, F, G, H, I);
    impl_operation_input_tuple!(A, B, C, D, E, F, G, H, I, J);
    impl_operation_input_tuple!(A, B, C, D, E, F, G, H, I, J, K);
    impl_operation_input_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
}
