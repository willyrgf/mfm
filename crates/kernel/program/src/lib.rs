#![warn(missing_docs)]
//! Typed state-program authoring contracts for MFM.
//!
//! This crate owns the branded authoring surface for typed programs. Handles
//! are minted only by framework builders, carry invariant program/scope/value
//! brands, and are lowered into unbranded draft specs only after root public
//! outputs have been bound inside the generative build closure.

extern crate self as mfm_program;

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::ptr::NonNull;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    ApplySideEffect, CapabilitySet, CapabilitySetDescriptor, CapabilitySetFor, EffectDescriptor,
    EffectSpec, NoCaps, Pure, ReadExternal,
};
pub use mfm_facts as facts;
use mfm_ids::{
    AdapterKind, AdapterVersion, CellId, ContentDigest, ContextDescriptorId, ContextRef,
    DescriptorId, DigestAlgorithm, DigestBytes, EffectKind, FieldPath as CheckedFieldPath,
    FieldSegment as CheckedFieldSegment, NodeId, OperationInstanceId, OperationKind,
    OperationVersion, SchemaId, ScopeId, SeedId, SemanticTypeId, SideEffectPairId,
    StableAuthorKey as CheckedStableAuthorKey, StateKind, StateVersion,
};
use mfm_spec::v1::MediaType;
pub use mfm_spec::v1::{
    CanonicalizerIdentity, CertifiedContextSpec, ContextProducerSpec, FactDescriptorRef,
    ManualAuthorizationVerifierId, ManualSigningSchemeSpec, OperatorAuthorityId,
    OperatorAuthorityMemberSpec, OperatorId, OperatorPublicIdentity, ResourceNamespace,
    SideEffectVerificationSpec, StateContextDescriptorSpec, StateInputContextContractSpec,
    StateOutputContextContractSpec,
};
use mfm_spec::v1::{
    CellContextSpec, InputContextSpec, ManualAuthorizationQuorumSpec,
    ManualResolutionAuthorizationSpec, ManualResolutionEvidenceSpec, NodeContextSpec,
    OperatorAuthoritySnapshotSpec, ResourceClaimSpec, StateContextDescriptorRequirementSpec,
};
use mfm_values::{
    MfmConfig, MfmValue, NumberPolicy, PersistedSurfacePolicy, SchemaDescriptor, SchemaKind,
    SchemaShape, SecretPolicy, StateInput, ValueTerminalPolicy,
};
pub use mfm_values::{NonEmpty, ValidatedConfig};

#[cfg(test)]
mod tests;

#[path = "ids.rs"]
mod ids;
use self::ids::*;
#[path = "input_binding.rs"]
mod input_binding;
use self::input_binding::InputBindingNodeKind;
pub use self::input_binding::*;
#[path = "handles.rs"]
mod handles;
pub use self::handles::*;
#[path = "draft.rs"]
mod draft;
pub use self::draft::*;
#[path = "builder.rs"]
mod builder;
pub use self::builder::*;
#[path = "operation_expansion.rs"]
mod operation_expansion;
pub use self::operation_expansion::*;
#[path = "child_scope.rs"]
mod child_scope;
pub use self::child_scope::*;
#[path = "registry.rs"]
mod registry;
pub use self::registry::*;

#[path = "saga_policy.rs"]
mod saga_policy;
pub use self::saga_policy::*;

#[path = "context_authority.rs"]
mod context_authority;
pub use self::context_authority::*;

const LOWERING_VERSION: &str = "mfm.typed.lowering.v2";

/// Result type for typed program authoring operations.
pub type Result<T> = std::result::Result<T, PlanError>;

/// Builds the certified descriptor reference for a typed fact authoring contract.
pub fn fact_descriptor_ref<F: facts::MfmFactType>() -> Result<FactDescriptorRef> {
    let descriptor = F::descriptor().map_err(|error| PlanError::Registry(error.to_string()))?;
    fact_descriptor_ref_for_descriptor(&descriptor)
}

/// Builds the certified descriptor reference for canonical fact descriptor authority.
pub fn fact_descriptor_ref_for_descriptor(
    descriptor: &facts::FactDescriptor,
) -> Result<FactDescriptorRef> {
    Ok(FactDescriptorRef {
        descriptor_hash: facts::fact_descriptor_hash(descriptor)
            .map_err(|error| PlanError::Registry(error.to_string()))?,
    })
}

/// Derives the certified context reference for an authored context value.
///
/// This is the same context authority used by [`RootBuilder::declare_context`].
/// Planning code may use it to validate static context joins before operation
/// expansion; it does not mint a persisted context outside a typed program.
pub fn context_ref_for<C: MfmContext>(value: &C) -> Result<ContextRef> {
    Ok(certified_context_spec(value.clone())?.context_ref)
}

/// Error returned by typed program authoring operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// Author-supplied stable key failed validation.
    #[error("invalid typed program key: {0}")]
    Key(String),
    /// Value descriptor construction failed.
    #[error("typed value descriptor error: {0}")]
    Value(String),
    /// Canonical seed construction failed.
    #[error("canonical seed error: {0}")]
    Canonical(String),
    /// JSON serialization failed before canonicalization.
    #[error("seed serialization error: {0}")]
    Serialize(String),
    /// A root seed key was declared more than once.
    #[error("duplicate root seed key {0}")]
    DuplicateSeedKey(String),
    /// A child scope key was declared more than once under the same parent.
    #[error("duplicate child scope key {0}")]
    DuplicateChildScopeKey(String),
    /// A bridge key was declared more than once in the same child scope session.
    #[error("duplicate bridge key {0}")]
    DuplicateBridgeKey(String),
    /// A state key was declared more than once in the same scope.
    #[error("duplicate state key {0}")]
    DuplicateStateKey(String),
    /// An operation key was declared more than once in the same scope.
    #[error("duplicate operation key {0}")]
    DuplicateOperationKey(String),
    /// A stable domain key appeared more than once in a canonical collection.
    #[error("duplicate stable domain key {0}")]
    DuplicateDomainKey(String),
    /// A public output field path was declared more than once.
    #[error("duplicate public output field path {0}")]
    DuplicatePublicOutputPath(String),
    /// A certified context ref was declared more than once.
    #[error("duplicate certified context ref {0}")]
    DuplicateContextRef(String),
    /// State context binding disagreed with the registered descriptor contract.
    #[error("state context contract mismatch: {0}")]
    ContextContract(String),
    /// Root public outputs were bound more than once.
    #[error("root public outputs already bound")]
    PublicOutputsAlreadyBound,
    /// Public output binding must contain at least one cell.
    #[error("root public output binding is empty")]
    EmptyPublicOutputs,
    /// A non-empty input collection was empty.
    #[error("non-empty input collection is empty")]
    EmptyNonEmptyInput,
    /// A derived input struct had the same field path more than once.
    #[error("duplicate input field path {0}")]
    DuplicateInputFieldPath(String),
    /// Input binding tree did not match the declared state input descriptor.
    #[error("input binding shape mismatch: {0}")]
    InputBindingShape(String),
    /// Same-type same-scope lineage did not match the required lineage.
    #[error("value lineage mismatch: {0}")]
    LineageMismatch(String),
    /// State registry authority rejected planning.
    #[error("state registry error: {0}")]
    Registry(String),
    /// Live bridge evidence did not belong to the active child scope session.
    #[error("invalid bridge evidence: {0}")]
    InvalidBridgeEvidence(String),
    /// A persisted bridge reference was not backed by an emitted bridge node.
    #[error("bridge ref is not backed by an emitted node")]
    UnknownBridgeRef,
    /// Saga policy was declared more than once.
    #[error("saga policy was already declared")]
    SagaPolicyAlreadySet,
    /// A side-effecting draft did not declare its run-level saga policy.
    #[error("side-effecting programs must declare a saga policy")]
    MissingSagaPolicy,
    /// A side-effecting state was planned through a builder that cannot declare a resource claim.
    #[error("side-effect resource claim required: {0}")]
    SideEffectClaimRequired(String),
    /// The declared saga policy disagreed with the authored graph shape.
    #[error("saga policy graph mismatch: {0}")]
    SagaPolicyGraphMismatch(String),
    /// A compensating saga policy left a forward side-effect node unlinked.
    #[error("saga compensation coverage gap: {0}")]
    SagaCoverageGap(String),
    /// A remediation node binding referenced cells outside the linked forward node scope.
    #[error("remediation binding scope violation: {0}")]
    RemediationBindingScope(String),
    /// Manual-resolution policy construction failed.
    #[error("manual policy error: {0}")]
    ManualPolicy(String),
    /// Resource claim construction failed.
    #[error("resource claim error: {0}")]
    ResourceClaim(String),
}

/// Stable root or child scope author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeKey(CheckedStableAuthorKey);

impl ScopeKey {
    /// Creates a checked scope key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("scope key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Stable root seed author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeedKey(CheckedStableAuthorKey);

impl SeedKey {
    /// Creates a checked seed key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("seed key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Stable public-output binding key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicOutputKey(CheckedStableAuthorKey);

impl PublicOutputKey {
    /// Creates a checked public-output key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("public output key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Stable bridge node author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BridgeKey(CheckedStableAuthorKey);

impl BridgeKey {
    /// Creates a checked bridge key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("bridge key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Stable state node author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateKey(CheckedStableAuthorKey);

impl StateKey {
    /// Creates a checked state key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("state key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Stable operation author key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationKey(CheckedStableAuthorKey);

impl OperationKey {
    /// Creates a checked operation key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_key("operation key", value.as_ref()).map(Self)
    }

    /// Returns the stable key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Stable public-output field path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicFieldPath(CheckedFieldPath);

impl PublicFieldPath {
    /// Creates a checked public field path.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_field_path("public field path", value.as_ref()).map(Self)
    }

    /// Returns the stable field path string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
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

/// Descriptive contract for a versioned executable state.
pub trait StateSpec: Send + Sync + 'static {
    /// Deterministic planning config type.
    type Config: MfmConfig;
    /// Semantic transition context required by this state.
    type Context: StateContext;
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

    /// Returns the context-bound input resource contract for this state.
    fn input_context_contract() -> Result<StateInputContextContractSpec> {
        Ok(StateInputContextContractSpec::no_context())
    }

    /// Returns the context-bound output resource contract for this state.
    fn output_context_contract() -> Result<StateOutputContextContractSpec> {
        Ok(StateOutputContextContractSpec::no_context())
    }

    /// Constructs the executable state from validated config authority.
    fn new(config: ValidatedConfig<Self::Config>) -> Result<Self>
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
        config: ValidatedConfig<Self::Config>,
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
/// mints this token only inside [`ScopeBuilder::call`].
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

/// Typed dynamic fanout key for stable domain-key references.
pub trait StableDomainKey: MfmValue {
    /// Returns the domain-key schema descriptor.
    fn domain_key_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        Self::schema_descriptor()
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

/// Resource claim declared by a side-effect node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceClaim {
    spec: ResourceClaimSpec,
}

impl ResourceClaim {
    /// Creates an exclusive resource key claim.
    pub fn exclusive(namespace: ResourceNamespace, key_schema: SchemaId) -> Self {
        Self {
            spec: ResourceClaimSpec::Exclusive {
                namespace,
                key_schema,
            },
        }
    }

    /// Creates an exact touched-set evidence claim.
    pub fn exact_touched_set(namespace: ResourceNamespace, evidence_schema: SchemaId) -> Self {
        Self {
            spec: ResourceClaimSpec::ExactTouchedSet {
                namespace,
                evidence_schema,
            },
        }
    }

    /// Creates a claim that makes no framework-derived cross-run concurrency assertion.
    pub fn manual_only() -> Self {
        Self {
            spec: ResourceClaimSpec::ManualOnly,
        }
    }

    /// Returns the lowered resource claim spec.
    pub fn as_spec(&self) -> &ResourceClaimSpec {
        &self.spec
    }

    fn into_spec(self) -> ResourceClaimSpec {
        self.spec
    }
}

/// Effect-specific runner kind recorded by a registered state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunnerKind {
    /// Pure synchronous state runner.
    Pure,
    /// External read runner.
    ReadExternal,
    /// External side-effect runner.
    ApplySideEffect,
}

impl RunnerKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::ReadExternal => "read_external",
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
    context: StateContextDescriptorSpec,
    input_context: StateInputContextContractSpec,
    output_context: StateOutputContextContractSpec,
    config_schema_id: SchemaId,
    input_schema_id: SchemaId,
    output_schema_id: SchemaId,
    output_semantic_type_id: SemanticTypeId,
    effect: EffectDescriptor,
    capabilities: CapabilitySetDescriptor,
    emitted_fact_descriptors: Vec<FactDescriptorRef>,
    effect_contract_digest: Option<ContentDigest>,
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
        let effect_contract_digest = <S::Effect as EffectRunner<S>>::effect_contract_digest()?;
        let emitted_fact_descriptors = canonical_fact_descriptor_refs(
            <S::Effect as EffectRunner<S>>::emitted_fact_descriptors()?,
        )?;
        let context = S::Context::descriptor()?;
        let input_context = S::input_context_contract()?;
        let output_context = S::output_context_contract()?;
        let descriptor_id = state_descriptor_id(StateDescriptorIdParts {
            kind: &kind,
            version: &version,
            name: S::name(),
            context: &context,
            input_context: &input_context,
            output_context: &output_context,
            config_schema_id: &config_schema_id,
            input_schema_id: &input_schema_id,
            output_schema_id: &output_schema_id,
            output_semantic_type_id: &output_semantic_type_id,
            effect: &effect,
            capabilities: &capabilities,
            emitted_fact_descriptors: &emitted_fact_descriptors,
            effect_contract_digest: effect_contract_digest.as_ref(),
            runner,
        })?;
        Ok(Self {
            descriptor_id,
            kind,
            version,
            name: S::name(),
            context,
            input_context,
            output_context,
            config_schema_id,
            input_schema_id,
            output_schema_id,
            output_semantic_type_id,
            effect,
            capabilities,
            emitted_fact_descriptors,
            effect_contract_digest,
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

    /// Returns this state's context descriptor contract.
    pub fn context(&self) -> &StateContextDescriptorSpec {
        &self.context
    }

    /// Returns this state's input context contract.
    pub fn input_context(&self) -> &StateInputContextContractSpec {
        &self.input_context
    }

    /// Returns this state's output context contract.
    pub fn output_context(&self) -> &StateOutputContextContractSpec {
        &self.output_context
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

    /// Returns fact descriptor hashes this state type may emit.
    pub fn emitted_fact_descriptors(&self) -> &[FactDescriptorRef] {
        &self.emitted_fact_descriptors
    }

    /// Returns the hash-defining effect contract, when the effect declares one.
    pub fn effect_contract_digest(&self) -> Option<&ContentDigest> {
        self.effect_contract_digest.as_ref()
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

/// Sealed framework evidence that an effect has the matching state runner shape.
pub trait EffectRunner<S: StateSpec>: private::EffectRunnerSealed<S> {
    /// Returns the runner kind for this effect/state pair.
    fn runner_kind() -> RunnerKind;

    /// Returns an optional hash-defining effect contract for this state.
    fn effect_contract_digest() -> Result<Option<ContentDigest>> {
        Ok(None)
    }

    /// Returns fact descriptors derived from this effect/state pair.
    fn emitted_fact_descriptors() -> Result<Vec<FactDescriptorRef>> {
        Ok(Vec::new())
    }

    /// Returns canonical fact descriptors for framework-owned registration.
    #[doc(hidden)]
    fn emitted_fact_descriptor_artifacts() -> facts::Result<Vec<facts::FactDescriptor>> {
        Ok(Vec::new())
    }
}

mod read_fact_batch_private {
    use mfm_facts::MfmFactType;
    use mfm_values::NonEmpty;

    pub trait Sealed {}

    impl Sealed for () {}

    impl<F: MfmFactType> Sealed for NonEmpty<F> {}
}

/// Closed fact-batch shape returned by an external-read reducer.
///
/// Framework reads either emit no facts (`()`) or one homogeneous non-empty batch
/// (`NonEmpty<F>`). The sealed contract intentionally has no visitor or extension surface.
pub trait ReadFactBatch: read_fact_batch_private::Sealed {
    /// Returns the one emitted fact descriptor, or `None` for a no-fact read.
    fn fact_descriptor() -> facts::Result<Option<facts::FactDescriptor>>;
}

impl ReadFactBatch for () {
    fn fact_descriptor() -> facts::Result<Option<facts::FactDescriptor>> {
        Ok(None)
    }
}

impl<F: facts::MfmFactType> ReadFactBatch for NonEmpty<F> {
    fn fact_descriptor() -> facts::Result<Option<facts::FactDescriptor>> {
        F::descriptor().map(Some)
    }
}

/// Pure deterministic state runner.
pub trait PureState: StateSpec<Effect = Pure, Caps = NoCaps> {
    /// Executes this pure state.
    fn run(
        &self,
        input: Self::Input,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output>;
}

/// External read state runner.
pub trait ReadState: StateSpec<Effect = ReadExternal> {
    /// Deterministic, capability-independent plan executed by an adapter.
    type Plan: MfmValue;
    /// Canonical primary evidence returned by the adapter and consumed by the reducer.
    type Evidence: MfmValue;
    /// Homogeneous fact batch emitted by the reducer, or `()` when it emits no facts.
    type Facts: ReadFactBatch;

    /// Builds the complete deterministic external-read plan.
    fn plan(
        &self,
        input: &Self::Input,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan>;

    /// Reduces retained evidence into the state output without ambient IO.
    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<(Self::Output, Self::Facts)>;
}

/// Complete retained evidence supplied to an external-read state reducer.
///
/// Every read attempt has exactly one state-owned primary evidence value. Fact-query states may
/// additionally receive kernel-owned query receipts whose referenced fact artifacts remain under
/// their existing retention authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalReadEvidenceSet<E> {
    primary: E,
    fact_queries: Vec<facts::FactQueryEvidence>,
}

impl<E> ExternalReadEvidenceSet<E> {
    /// Creates an evidence set from one primary value and optional fact-query evidence.
    pub fn new(primary: E, fact_queries: Vec<facts::FactQueryEvidence>) -> Self {
        Self {
            primary,
            fact_queries,
        }
    }

    /// Creates an evidence set with no auxiliary fact queries.
    pub fn primary(primary: E) -> Self {
        Self::new(primary, Vec::new())
    }

    /// Returns the one state-owned primary evidence value.
    pub const fn primary_evidence(&self) -> &E {
        &self.primary
    }

    /// Returns kernel-owned auxiliary fact-query evidence in execution order.
    pub fn fact_query_evidence(&self) -> &[facts::FactQueryEvidence] {
        &self.fact_queries
    }

    /// Splits this set into its primary and auxiliary evidence.
    pub fn into_parts(self) -> (E, Vec<facts::FactQueryEvidence>) {
        (self.primary, self.fact_queries)
    }
}

/// Derives a hash-defining contract for one external-read plan/evidence/fact-mode tuple.
fn external_read_contract_digest<Plan, Evidence, Facts>() -> Result<ContentDigest>
where
    Plan: MfmValue,
    Evidence: MfmValue,
    Facts: ReadFactBatch,
{
    let fact_mode =
        match Facts::fact_descriptor().map_err(|error| PlanError::Registry(error.to_string()))? {
            None => serde_json::json!({"kind": "none"}),
            Some(descriptor) => serde_json::json!({
                "kind": "non_empty",
                "fact_descriptor_hash": facts::fact_descriptor_hash(&descriptor)
                    .map_err(|error| PlanError::Registry(error.to_string()))?
                    .to_string(),
            }),
        };
    canonical_digest(serde_json::json!({
        "contract_domain": "mfm.external_read",
        "contract_version": 2,
        "effect_class": "read_external",
        "evidence_schema_id": Evidence::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "evidence_semantic_type_id": Evidence::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "plan_schema_id": Plan::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "plan_semantic_type_id": Plan::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "fact_mode": fact_mode,
    }))
}

/// Derives the hash-defining contract for one side-effect evidence protocol.
pub fn side_effect_contract_digest<
    Intent,
    IdempotencyInput,
    PreparedInvocation,
    Submission,
    RecoveryEvidence,
    Receipt,
    Confirmation,
>() -> Result<ContentDigest>
where
    Intent: MfmValue,
    IdempotencyInput: MfmValue,
    PreparedInvocation: MfmValue,
    Submission: MfmValue,
    RecoveryEvidence: MfmValue,
    Receipt: MfmValue,
    Confirmation: MfmValue,
{
    canonical_digest(serde_json::json!({
        "contract_domain": "mfm.side_effect",
        "contract_version": 1,
        "effect_class": "apply_side_effect",
        "confirmation_schema_id": Confirmation::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "confirmation_semantic_type_id": Confirmation::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "idempotency_input_schema_id": IdempotencyInput::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "idempotency_input_semantic_type_id": IdempotencyInput::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "intent_schema_id": Intent::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "intent_semantic_type_id": Intent::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "prepared_invocation_schema_id": PreparedInvocation::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "prepared_invocation_semantic_type_id": PreparedInvocation::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "receipt_schema_id": Receipt::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "receipt_semantic_type_id": Receipt::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "recovery_evidence_schema_id": RecoveryEvidence::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "recovery_evidence_semantic_type_id": RecoveryEvidence::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "submission_schema_id": Submission::schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
        "submission_semantic_type_id": Submission::semantic_id()
            .map_err(|error| PlanError::Value(error.to_string()))?
            .as_str(),
    }))
}

/// Deterministic state-authored mutation intent and idempotency material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectIntent<Intent, Idempotency> {
    intent: Intent,
    idempotency: Idempotency,
}

impl<Intent, Idempotency> SideEffectIntent<Intent, Idempotency> {
    /// Creates one immutable authored side-effect intent.
    pub const fn new(intent: Intent, idempotency: Idempotency) -> Self {
        Self {
            intent,
            idempotency,
        }
    }

    /// Returns the mutation intent.
    pub const fn intent(&self) -> &Intent {
        &self.intent
    }

    /// Returns the material from which the adapter derives the idempotency key.
    pub const fn idempotency(&self) -> &Idempotency {
        &self.idempotency
    }

    /// Splits the authored intent into its typed parts.
    pub fn into_parts(self) -> (Intent, Idempotency) {
        (self.intent, self.idempotency)
    }
}

/// External mutation state runner governed by a side-effect protocol.
pub trait SideEffectState: StateSpec<Effect = ApplySideEffect> {
    /// Deterministic mutation intent.
    type Intent: MfmValue;
    /// Deterministic idempotency input.
    type IdempotencyInput: MfmValue;
    /// Immutable public invocation authority prepared before submission starts.
    type PreparedInvocation: MfmValue;
    /// Submission result value.
    type Submission: MfmValue;
    /// Public evidence retained when submission recovery is not yet terminal.
    type RecoveryEvidence: MfmValue;
    /// Receipt value observed after submission.
    type Receipt: MfmValue;
    /// Confirmation value used to produce terminal output.
    type Confirmation: MfmValue;
    /// Builds the one deterministic authored intent from materialized state input.
    fn intent(
        &self,
        input: &Self::Input,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<SideEffectIntent<Self::Intent, Self::IdempotencyInput>>;

    /// Constructs terminal output from receipt-level side-effect evidence.
    fn output_from_receipt(
        &self,
        input: &Self::Input,
        prepared: &Self::PreparedInvocation,
        submission: &Self::Submission,
        receipt: &Self::Receipt,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output>;

    /// Constructs terminal output from confirmed side-effect evidence.
    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        prepared: &Self::PreparedInvocation,
        submission: &Self::Submission,
        receipt: &Self::Receipt,
        confirmation: &Self::Confirmation,
        context: &CertifiedContext<Self::Context>,
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

    fn effect_contract_digest() -> Result<Option<ContentDigest>> {
        external_read_contract_digest::<S::Plan, S::Evidence, S::Facts>().map(Some)
    }

    fn emitted_fact_descriptors() -> Result<Vec<FactDescriptorRef>> {
        S::Facts::fact_descriptor()
            .map_err(|error| PlanError::Registry(error.to_string()))?
            .map(|descriptor| {
                fact_descriptor_ref_for_descriptor(&descriptor).map(|item| vec![item])
            })
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    fn emitted_fact_descriptor_artifacts() -> facts::Result<Vec<facts::FactDescriptor>> {
        S::Facts::fact_descriptor()
            .map(Option::into_iter)
            .map(Iterator::collect)
    }
}

impl<S> EffectRunner<S> for ApplySideEffect
where
    S: SideEffectState,
{
    fn runner_kind() -> RunnerKind {
        RunnerKind::ApplySideEffect
    }

    fn effect_contract_digest() -> Result<Option<ContentDigest>> {
        side_effect_contract_digest::<
            S::Intent,
            S::IdempotencyInput,
            S::PreparedInvocation,
            S::Submission,
            S::RecoveryEvidence,
            S::Receipt,
            S::Confirmation,
        >()
        .map(Some)
    }
}

mod private {
    use super::{
        ApplySideEffect, DeclaredContext, ForwardSideEffectHandle, Handle, MfmContext, MfmValue,
        NoContext, NonEmptyHandles, OperationInputHandles, Pure, ReadExternal, SideEffectState,
        StateContext, StateInput, StateSpec,
    };

    pub trait EffectRunnerSealed<S: StateSpec> {}
    pub trait OperationInputSealed {}
    pub trait BridgeableSealed {}
    pub trait StateTransitionContextSealed<'program, 'scope, C: StateContext> {}

    impl<S> EffectRunnerSealed<S> for Pure where S: super::PureState {}

    impl<S> EffectRunnerSealed<S> for ReadExternal where S: super::ReadState {}

    impl<S> EffectRunnerSealed<S> for ApplySideEffect where S: SideEffectState {}

    impl<'program, 'scope> StateTransitionContextSealed<'program, 'scope, NoContext> for NoContext {}

    impl<'program, 'scope, C> StateTransitionContextSealed<'program, 'scope, C>
        for &DeclaredContext<'program, 'scope, C>
    where
        C: MfmContext,
    {
    }

    impl OperationInputSealed for () {}

    impl<I, H> OperationInputSealed for OperationInputHandles<I, H> where I: StateInput {}

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
