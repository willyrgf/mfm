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
pub use mfm_facts as facts;
use mfm_ids::{
    AdapterKind, AdapterVersion, CellId, ContentDigest, ContextDescriptorId, DescriptorId,
    DigestAlgorithm, DigestBytes, EffectKind, FieldPath as CheckedFieldPath,
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

const LOWERING_VERSION: &str = "mfm.typed.lowering.v2";

/// Result type for typed program authoring operations.
pub type Result<T> = std::result::Result<T, PlanError>;

/// Authoring helper for typed fact publication.
///
/// This trait is not fact publication authority. Production recording must still validate
/// descriptor bytes, certified node allow-lists, response artifacts, and store append rules.
pub trait MfmFactType: MfmValue {
    /// Typed subject value used to derive fact identity.
    type Subject: MfmValue;

    /// Typed response value retained as fact response evidence.
    type Response: MfmValue;

    /// Returns the fact descriptor emitted by the derive-owned authoring path.
    fn descriptor() -> facts::Result<facts::FactDescriptor>;

    /// Returns this fact's subject value.
    fn subject(&self) -> &Self::Subject;

    /// Returns this fact's response value.
    fn response(&self) -> &Self::Response;
}

/// Builds the certified descriptor reference for a typed fact authoring contract.
pub fn fact_descriptor_ref<F: MfmFactType>() -> Result<FactDescriptorRef> {
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

/// Error returned while executing a typed state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// Stable, redacted state error message.
    #[error("{0}")]
    Message(String),
}

/// Result type returned by executable state traits.
pub type StateResult<T> = std::result::Result<T, StateError>;

/// Framework-owned marker for states that do not execute under semantic transition context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoContext;

/// Marker contract for typed, canonical, non-secret context values.
///
/// Implementing this trait is an explicit opt-in: ordinary persisted values do not become
/// transition contexts automatically.
pub trait MfmContext: MfmValue + Clone {
    /// Returns the canonicalizer identity used for context values.
    fn canonicalizer_identity() -> Result<CanonicalizerIdentity> {
        CanonicalizerIdentity::new("sha256-jcs-v1")
            .map_err(|error| PlanError::Value(error.to_string()))
    }
}

/// Context contract accepted by a state descriptor.
pub trait StateContext: Send + Sync + Sized + 'static {
    /// Returns the descriptor-level state context contract.
    fn descriptor() -> Result<StateContextDescriptorSpec>;

    /// Materializes state-facing context authority from an optional certified context spec.
    fn materialize_certified(spec: Option<&CertifiedContextSpec>)
        -> Result<CertifiedContext<Self>>;
}

impl StateContext for NoContext {
    fn descriptor() -> Result<StateContextDescriptorSpec> {
        Ok(StateContextDescriptorSpec::no_context())
    }

    fn materialize_certified(
        spec: Option<&CertifiedContextSpec>,
    ) -> Result<CertifiedContext<Self>> {
        if let Some(context) = spec {
            return Err(PlanError::ContextContract(format!(
                "NoContext state received certified context {}",
                context.context_ref
            )));
        }
        Ok(CertifiedContext::no_context())
    }
}

impl<C> StateContext for C
where
    C: MfmContext,
{
    fn descriptor() -> Result<StateContextDescriptorSpec> {
        context_descriptor_for::<C>()
    }

    fn materialize_certified(
        spec: Option<&CertifiedContextSpec>,
    ) -> Result<CertifiedContext<Self>> {
        let spec = spec.ok_or_else(|| {
            PlanError::ContextContract(
                "typed context state received no-context authority".to_owned(),
            )
        })?;
        CertifiedContext::from_certified_spec(spec)
    }
}

/// State-facing typed context authority materialized from a certified spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedContext<C: StateContext> {
    context_ref: Option<mfm_ids::ContextRef>,
    context_descriptor_id: Option<ContextDescriptorId>,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<SemanticTypeId>,
    canonicalizer_identity: Option<CanonicalizerIdentity>,
    value: Option<C>,
}

impl CertifiedContext<NoContext> {
    /// Creates framework-owned no-context invocation authority.
    pub fn no_context() -> Self {
        Self {
            context_ref: None,
            context_descriptor_id: None,
            schema_id: None,
            semantic_type_id: None,
            canonicalizer_identity: None,
            value: None,
        }
    }
}

impl<C: MfmContext> CertifiedContext<C> {
    /// Materializes typed context authority from a certified context table entry.
    pub fn from_certified_spec(spec: &CertifiedContextSpec) -> Result<Self> {
        let StateContextDescriptorSpec::Required(requirement) = C::descriptor()? else {
            return Err(PlanError::ContextContract(
                "typed context resolved to no-context descriptor".to_owned(),
            ));
        };
        if spec.context_descriptor_id != requirement.context_descriptor_id
            || spec.schema_id != requirement.schema_id
            || spec.semantic_type_id != requirement.semantic_type_id
            || spec.canonicalizer_identity != requirement.canonicalizer_identity
        {
            return Err(PlanError::ContextContract(format!(
                "certified context {} does not match requested typed context descriptor",
                spec.context_ref
            )));
        }
        let value = serde_json::from_slice::<C>(spec.canonical_context.as_bytes())
            .map_err(|error| PlanError::Value(error.to_string()))?;
        Ok(Self {
            context_ref: Some(spec.context_ref.clone()),
            context_descriptor_id: Some(spec.context_descriptor_id.clone()),
            schema_id: Some(spec.schema_id.clone()),
            semantic_type_id: Some(spec.semantic_type_id.clone()),
            canonicalizer_identity: Some(spec.canonicalizer_identity.clone()),
            value: Some(value),
        })
    }

    /// Returns the content-addressed certified context ref.
    pub fn context_ref(&self) -> &mfm_ids::ContextRef {
        self.context_ref
            .as_ref()
            .expect("typed certified context must carry a context ref")
    }

    /// Returns the context descriptor identity.
    pub fn context_descriptor_id(&self) -> &ContextDescriptorId {
        self.context_descriptor_id
            .as_ref()
            .expect("typed certified context must carry a descriptor id")
    }

    /// Returns the context schema id.
    pub fn schema_id(&self) -> &SchemaId {
        self.schema_id
            .as_ref()
            .expect("typed certified context must carry a schema id")
    }

    /// Returns the context semantic type id.
    pub fn semantic_type_id(&self) -> &SemanticTypeId {
        self.semantic_type_id
            .as_ref()
            .expect("typed certified context must carry a semantic type id")
    }

    /// Returns the context canonicalizer identity.
    pub fn canonicalizer_identity(&self) -> &CanonicalizerIdentity {
        self.canonicalizer_identity
            .as_ref()
            .expect("typed certified context must carry a canonicalizer identity")
    }

    /// Returns the decoded typed context value.
    pub fn value(&self) -> &C {
        self.value
            .as_ref()
            .expect("typed certified context must carry a decoded value")
    }
}

/// Scope-bound declared certified context handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredContext<'program, 'scope, C: MfmContext> {
    spec: CertifiedContextSpec,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    _context: PhantomData<fn(C) -> C>,
}

/// Type-erased context descriptor validator retained by in-memory program drafts.
#[derive(Clone)]
pub struct ContextValidatorSpec {
    requirement: StateContextDescriptorRequirementSpec,
    validate: fn(&CertifiedContextSpec) -> Result<()>,
}

impl fmt::Debug for ContextValidatorSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextValidatorSpec")
            .field(
                "context_descriptor_id",
                &self.requirement.context_descriptor_id,
            )
            .field("schema_id", &self.requirement.schema_id)
            .field("semantic_type_id", &self.requirement.semantic_type_id)
            .field(
                "canonicalizer_identity",
                &self.requirement.canonicalizer_identity,
            )
            .finish_non_exhaustive()
    }
}

impl PartialEq for ContextValidatorSpec {
    fn eq(&self, other: &Self) -> bool {
        self.requirement == other.requirement
    }
}

impl Eq for ContextValidatorSpec {}

impl ContextValidatorSpec {
    /// Returns the context descriptor requirement this validator decodes.
    pub const fn requirement(&self) -> &StateContextDescriptorRequirementSpec {
        &self.requirement
    }

    /// Returns the type-erased validation callback.
    pub const fn validate_fn(&self) -> fn(&CertifiedContextSpec) -> Result<()> {
        self.validate
    }
}

impl<'program, 'scope, C: MfmContext> DeclaredContext<'program, 'scope, C> {
    fn spec(&self) -> &CertifiedContextSpec {
        &self.spec
    }
}

/// Explicit transition-context authority accepted by state planning APIs.
///
/// This trait is sealed by the framework. Authoring code can pass [`NoContext`] for states whose
/// [`StateSpec::Context`] is [`NoContext`], or `&DeclaredContext<C>` for states bound to a declared
/// typed context `C`.
pub trait StateTransitionContext<'program, 'scope, C: StateContext>:
    private::StateTransitionContextSealed<'program, 'scope, C>
{
    /// Returns the certified context table entry to attach to the planned node, if any.
    fn certified_context_spec(&self) -> Option<&CertifiedContextSpec>;
}

impl<'program, 'scope> StateTransitionContext<'program, 'scope, NoContext> for NoContext {
    fn certified_context_spec(&self) -> Option<&CertifiedContextSpec> {
        None
    }
}

impl<'program, 'scope, C> StateTransitionContext<'program, 'scope, C>
    for &DeclaredContext<'program, 'scope, C>
where
    C: MfmContext,
{
    fn certified_context_spec(&self) -> Option<&CertifiedContextSpec> {
        Some(self.spec())
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

    /// Returns fact descriptor hashes this state type may emit.
    fn emitted_fact_descriptors() -> Result<Vec<FactDescriptorRef>> {
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
        manual: ManualResolutionPolicyDraft,
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
        manual: ManualResolutionPolicyDraft,
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
        manual: Box<ManualResolutionPolicyDraft>,
    },
    /// Terminally fail without a compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
}

/// Typed schema requirements for run-scoped manual saga resolution evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionPolicyDraft {
    evidence_schema: SchemaId,
    authorization: ManualAuthorizationDraft,
}

impl ManualResolutionPolicyDraft {
    /// Creates a manual-resolution policy draft from typed evidence and authorization authority.
    pub fn new(evidence_schema: SchemaId, authorization: ManualAuthorizationDraft) -> Self {
        Self {
            evidence_schema,
            authorization,
        }
    }

    /// Returns the schema id required for the operator evidence artifact.
    pub fn evidence_schema(&self) -> &SchemaId {
        &self.evidence_schema
    }

    /// Returns the authorization policy draft required for the manual decision.
    pub fn authorization(&self) -> &ManualAuthorizationDraft {
        &self.authorization
    }

    /// Returns the lowered manual-resolution evidence spec.
    pub fn to_spec(&self) -> ManualResolutionEvidenceSpec {
        ManualResolutionEvidenceSpec {
            evidence_schema: self.evidence_schema.clone(),
            authorization: self.authorization.to_spec(),
        }
    }
}

/// Certified authorization policy draft for run-scoped manual saga resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAuthorizationDraft {
    verifier_id: ManualAuthorizationVerifierId,
    signing_scheme: ManualSigningSchemeSpec,
    authority: OperatorAuthoritySnapshotDraft,
    quorum: ThresholdQuorum,
}

impl ManualAuthorizationDraft {
    /// Creates a threshold manual authorization draft.
    pub fn threshold(
        verifier_id: ManualAuthorizationVerifierId,
        signing_scheme: ManualSigningSchemeSpec,
        authority: OperatorAuthoritySnapshotDraft,
        quorum: ThresholdQuorum,
    ) -> Result<Self> {
        if quorum.required_signatures() as usize > authority.operator_count() {
            return Err(PlanError::ManualPolicy(format!(
                "manual authorization quorum {} exceeds operator authority size {}",
                quorum.required_signatures(),
                authority.operator_count()
            )));
        }
        Ok(Self {
            verifier_id,
            signing_scheme,
            authority,
            quorum,
        })
    }

    /// Returns the verifier identity.
    pub fn verifier_id(&self) -> &ManualAuthorizationVerifierId {
        &self.verifier_id
    }

    /// Returns the signing scheme.
    pub fn signing_scheme(&self) -> &ManualSigningSchemeSpec {
        &self.signing_scheme
    }

    /// Returns the operator authority snapshot draft.
    pub fn authority(&self) -> &OperatorAuthoritySnapshotDraft {
        &self.authority
    }

    /// Returns the threshold quorum.
    pub fn quorum(&self) -> ThresholdQuorum {
        self.quorum
    }

    fn to_spec(&self) -> ManualResolutionAuthorizationSpec {
        ManualResolutionAuthorizationSpec {
            verifier_id: self.verifier_id.clone(),
            signing_scheme: self.signing_scheme.clone(),
            authority: self.authority.to_spec(),
            quorum: ManualAuthorizationQuorumSpec::new(self.quorum.required_signatures())
                .expect("ThresholdQuorum is non-zero"),
        }
    }
}

/// Non-empty, operator-id-unique collection for manual authorization snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyUniqueOperators {
    operators: Vec<OperatorAuthorityMemberSpec>,
}

impl NonEmptyUniqueOperators {
    /// Creates a non-empty unique collection from a first operator and optional rest.
    pub fn new(
        first: OperatorAuthorityMemberSpec,
        mut rest: Vec<OperatorAuthorityMemberSpec>,
    ) -> Result<Self> {
        let mut operators = Vec::with_capacity(rest.len() + 1);
        operators.push(first);
        operators.append(&mut rest);
        Self::try_from_vec(operators)
    }

    /// Attempts to create a non-empty unique operator collection from a vector.
    pub fn try_from_vec(operators: Vec<OperatorAuthorityMemberSpec>) -> Result<Self> {
        if operators.is_empty() {
            return Err(PlanError::ManualPolicy(
                "manual operator authority must contain at least one operator".to_owned(),
            ));
        }
        let mut seen = BTreeSet::new();
        for operator in &operators {
            if !seen.insert(operator.operator_id.as_str().to_owned()) {
                return Err(PlanError::ManualPolicy(format!(
                    "duplicate manual operator id {}",
                    operator.operator_id
                )));
            }
        }
        Ok(Self { operators })
    }

    /// Returns the operators in retained snapshot order.
    pub fn operators(&self) -> &[OperatorAuthorityMemberSpec] {
        &self.operators
    }

    fn into_vec(self) -> Vec<OperatorAuthorityMemberSpec> {
        self.operators
    }
}

/// Manual authorization threshold that requires at least one signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThresholdQuorum {
    required_signatures: u32,
}

impl ThresholdQuorum {
    /// Creates a non-zero threshold quorum.
    pub fn new(required_signatures: u32) -> Result<Self> {
        if required_signatures == 0 {
            return Err(PlanError::ManualPolicy(
                "manual authorization quorum must require at least one signature".to_owned(),
            ));
        }
        Ok(Self {
            required_signatures,
        })
    }

    /// Returns the required signature count.
    pub fn required_signatures(self) -> u32 {
        self.required_signatures
    }
}

/// Operator authority snapshot draft with a non-empty unique operator set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorAuthoritySnapshotDraft {
    authority_id: OperatorAuthorityId,
    operators: NonEmptyUniqueOperators,
}

impl OperatorAuthoritySnapshotDraft {
    /// Creates an operator authority snapshot draft.
    pub fn new(authority_id: OperatorAuthorityId, operators: NonEmptyUniqueOperators) -> Self {
        Self {
            authority_id,
            operators,
        }
    }

    /// Returns the authority id.
    pub fn authority_id(&self) -> &OperatorAuthorityId {
        &self.authority_id
    }

    /// Returns operators in retained snapshot order.
    pub fn operators(&self) -> &[OperatorAuthorityMemberSpec] {
        self.operators.operators()
    }

    fn operator_count(&self) -> usize {
        self.operators.operators().len()
    }

    fn to_spec(&self) -> OperatorAuthoritySnapshotSpec {
        OperatorAuthoritySnapshotSpec {
            authority_id: self.authority_id.clone(),
            operators: self.operators.clone().into_vec(),
        }
    }
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
        let emitted_fact_descriptors =
            canonical_fact_descriptor_refs(S::emitted_fact_descriptors()?)?;
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
            side_effect_contract_digest: side_effect_contract_digest.as_ref(),
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// State descriptor construction failed.
    #[error("state descriptor error: {0}")]
    Descriptor(String),
    /// No registered state matched the requested kind/version.
    #[error("state {kind}@{version} is not registered")]
    UnregisteredState {
        /// Requested state kind.
        kind: String,
        /// Requested state version.
        version: String,
    },
    /// A different descriptor already owns this kind/version pair.
    #[error("duplicate state registration for {kind}@{version}")]
    DuplicateStateRegistration {
        /// Registered state kind.
        kind: String,
        /// Registered state version.
        version: String,
    },
    /// Registry record and descriptor evidence diverged.
    #[error("state registry descriptor mismatch for {kind}@{version}")]
    DescriptorMismatch {
        /// Registered state kind.
        kind: String,
        /// Registered state version.
        version: String,
    },
    /// No registered operation matched the requested kind/version.
    #[error("operation {kind}@{version} is not registered")]
    UnregisteredOperation {
        /// Requested operation kind.
        kind: String,
        /// Requested operation version.
        version: String,
    },
    /// A different descriptor already owns this operation kind/version pair.
    #[error("duplicate operation registration for {kind}@{version}")]
    DuplicateOperationRegistration {
        /// Registered operation kind.
        kind: String,
        /// Registered operation version.
        version: String,
    },
    /// Registry record and operation descriptor evidence diverged.
    #[error("operation registry descriptor mismatch for {kind}@{version}")]
    OperationDescriptorMismatch {
        /// Registered operation kind.
        kind: String,
        /// Registered operation version.
        version: String,
    },
}

impl From<RegistryError> for PlanError {
    fn from(error: RegistryError) -> Self {
        Self::Registry(error.to_string())
    }
}

/// Returns the validated descriptor identity for a typed state.
pub fn state_descriptor<S>() -> std::result::Result<StateDescriptorIdentity, RegistryError>
where
    S: StateSpec,
    S::Effect: EffectRunner<S>,
    S::Caps: CapabilitySetFor<S::Effect>,
{
    StateDescriptorIdentity::for_state::<S>()
        .map_err(|error| RegistryError::Descriptor(error.to_string()))
}

/// Returns the validated descriptor identity for a typed operation.
pub fn operation_descriptor<O>() -> std::result::Result<OperationDescriptorIdentity, RegistryError>
where
    O: Operation,
{
    OperationDescriptorIdentity::for_operation::<O>()
        .map_err(|error| RegistryError::Descriptor(error.to_string()))
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

    /// Resolves the validated descriptor identity for a state registered in this snapshot.
    pub fn state_descriptor<S>(&self) -> std::result::Result<StateDescriptorIdentity, RegistryError>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = state_descriptor::<S>()?;
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
        Ok(descriptor)
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
    pub fn register<S>(&mut self) -> std::result::Result<(), RegistryError>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = state_descriptor::<S>()?;
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
        Ok(())
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

    /// Resolves the validated descriptor identity for an operation registered in this snapshot.
    pub fn operation_descriptor<O>(
        &self,
    ) -> std::result::Result<OperationDescriptorIdentity, RegistryError>
    where
        O: Operation,
    {
        let descriptor = operation_descriptor::<O>()?;
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
        Ok(descriptor)
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
    pub fn register<O>(&mut self) -> std::result::Result<(), RegistryError>
    where
        O: Operation,
    {
        let descriptor = operation_descriptor::<O>()?;
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
        Ok(())
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
    fn run(
        &self,
        input: Self::Input,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output>;
}

/// External read state runner.
pub trait ReadState: StateSpec<Effect = ReadExternal> {
    /// Future returned by [`ReadState::run`].
    type RunFuture<'a>: Future<Output = StateResult<Self::Output>> + Send + 'a
    where
        Self: 'a;

    /// Executes this read state through declared capabilities.
    fn run<'a>(
        &'a self,
        input: Self::Input,
        caps: &'a Self::Caps,
        context: &'a CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a>;
}

/// MFM-managed platform write state runner.
pub trait ManagedWriteState: StateSpec<Effect = ManagedPlatformWrite> {
    /// Future returned by [`ManagedWriteState::run`].
    type RunFuture<'a>: Future<Output = StateResult<Self::Output>> + Send + 'a
    where
        Self: 'a;

    /// Executes this managed write through declared capabilities.
    fn run<'a>(
        &'a self,
        input: Self::Input,
        caps: &'a Self::Caps,
        context: &'a CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a>;
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
    fn prepare_intent(
        &self,
        input: &Self::Input,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent>;

    /// Builds deterministic idempotency input from materialized input and intent.
    fn idempotency_input(
        &self,
        input: &Self::Input,
        intent: &Self::Intent,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput>;

    /// Submits the intent through declared capabilities.
    fn submit<'a>(
        &'a self,
        intent: &'a Self::Intent,
        key: &'a IdempotencyKey<Self::IdempotencyInput>,
        caps: &'a Self::Caps,
        context: &'a CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a>;

    /// Constructs terminal output from receipt-level side-effect evidence.
    fn output_from_receipt(
        &self,
        input: &Self::Input,
        intent: &Self::Intent,
        receipt: &Self::Receipt,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output>;

    /// Constructs terminal output from confirmed side-effect evidence.
    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        intent: &Self::Intent,
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
        let content_digest = canonical_domain_key_bytes(key)?.content_digest();
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

fn canonical_domain_key_bytes<K: StableDomainKey>(key: &K) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(key).map_err(|error| PlanError::Serialize(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))
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
    context: CellContextSpec,
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
            context: self.context.clone(),
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
        context: CellContextSpec,
    ) -> Self {
        Self {
            cell_id,
            scope_id,
            schema_id,
            semantic_type_id,
            value_lineage,
            context,
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
        context: CellContextSpec,
        evidence: BridgeEvidenceCore,
    ) -> Self {
        Self {
            cell_id,
            scope_id,
            schema_id,
            semantic_type_id,
            value_lineage,
            context,
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
            context: self.context.clone(),
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

/// Branded forward/remediation handles returned by linked side-effect construction.
pub type LinkedSideEffectHandles<'program, 'scope, ForwardOutput, RemediationOutput> = (
    ForwardSideEffectHandle<'program, 'scope, ForwardOutput>,
    RemediationHandle<'program, 'scope, RemediationOutput>,
);

/// Parameters for a forward side-effect node in a linked compensation pair.
pub struct SideEffectNodeParams<S: SideEffectState, I> {
    /// Scope-local author key for the forward node.
    pub key: StateKey,
    /// Deterministic forward state config.
    pub config: S::Config,
    /// Forward state input binding source.
    pub input: I,
    /// Resource claim for the forward side-effect ledger.
    pub resource_claim: ResourceClaim,
    /// Terminal verification policy for the forward side-effect ledger.
    pub verification: SideEffectVerificationSpec,
}

/// Parameters for a remediation node in a linked compensation pair.
pub struct RemediationNodeParams<R: SideEffectState> {
    /// Scope-local author key for the remediation node.
    pub key: StateKey,
    /// Deterministic remediation state config.
    pub config: R::Config,
    /// Resource claim for the remediation side-effect ledger.
    pub resource_claim: ResourceClaim,
    /// Terminal verification policy for the remediation side-effect ledger.
    pub verification: SideEffectVerificationSpec,
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
    /// Certified context constraint carried by the planned cell.
    context: CellContextSpec,
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

    /// Returns the certified context constraint carried by the planned cell.
    pub fn context(&self) -> &CellContextSpec {
        &self.context
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
            context: CellContextSpec::no_context(),
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
    /// Registered state context descriptor contract.
    pub context_descriptor: StateContextDescriptorSpec,
    /// Registered state input context contract.
    pub input_context_contract: StateInputContextContractSpec,
    /// Registered state output context contract.
    pub output_context_contract: StateOutputContextContractSpec,
    /// Certified transition-context requirement for this node.
    pub context: NodeContextSpec,
    /// Registered runner kind.
    pub runner: RunnerKind,
    /// Framework effect kind required by the registered state.
    pub effect_kind: EffectKind,
    /// Framework capability descriptor set required by the registered state.
    pub capability_bindings: CapabilitySetDescriptor,
    /// Behaviorally relevant adapter bindings required by the registered state.
    pub adapter_bindings: Vec<AdapterBindingSpec>,
    /// Fact descriptors this producing node may emit.
    pub fact_descriptor_allowlist: Vec<FactDescriptorRef>,
    /// Side-effect contract digest when this node mutates an external system.
    pub side_effect_contract_digest: Option<ContentDigest>,
    /// Cross-run resource claim when this node mutates an external system.
    pub side_effect_resource_claim: Option<ResourceClaimSpec>,
    /// Terminal verification policy when this node mutates an external system.
    pub side_effect_verification: Option<SideEffectVerificationSpec>,
    /// Forward side-effect verify framework node lowered for this submit node.
    pub side_effect_verify: Option<SideEffectVerifyDraftSpec>,
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
    /// Certified transition-context constraint for the output cell.
    pub output_context: CellContextSpec,
    /// Stable domain keys associated with the output value lineage.
    pub output_domain_keys: Vec<StableDomainKeyRef>,
    /// Planning lineage active while this node was emitted.
    pub planning_lineage: OperationLineage,
}

/// Draft metadata for a framework-owned side-effect verify node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectVerifyDraftSpec {
    /// Certified side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Derived verify framework node id.
    pub node_id: NodeId,
    /// Verify framework output cell id.
    pub output_cell_id: CellId,
    /// Verify framework output value lineage ref.
    pub output_value_lineage: ValueLineageRef,
    /// Certified transition-context constraint for the verify output cell.
    pub output_context: CellContextSpec,
    /// Stable domain keys associated with the verify output value lineage.
    pub output_domain_keys: Vec<StableDomainKeyRef>,
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
    /// Certified transition-context constraint preserved by the same-value bridge.
    pub context: CellContextSpec,
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
    contexts: Vec<CertifiedContextSpec>,
    context_validators: Vec<ContextValidatorSpec>,
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

    /// Returns certified transition contexts declared by the draft.
    pub fn contexts(&self) -> &[CertifiedContextSpec] {
        &self.contexts
    }

    /// Returns typed context validators retained by this in-memory draft.
    pub fn context_validators(&self) -> &[ContextValidatorSpec] {
        &self.context_validators
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

fn validate_state_context_binding(
    descriptor: &StateDescriptorIdentity,
    context: Option<&CertifiedContextSpec>,
) -> Result<()> {
    match (descriptor.context(), context) {
        (StateContextDescriptorSpec::NoContext, None) => Ok(()),
        (StateContextDescriptorSpec::NoContext, Some(context)) => {
            Err(PlanError::ContextContract(format!(
                "state {} is NoContext but was planned with {}",
                descriptor.name(),
                context.context_ref
            )))
        }
        (StateContextDescriptorSpec::Required(_), None) => Err(PlanError::ContextContract(
            format!("state {} requires a certified context", descriptor.name()),
        )),
        (StateContextDescriptorSpec::Required(requirement), Some(context)) => {
            if context.context_descriptor_id != requirement.context_descriptor_id
                || context.schema_id != requirement.schema_id
                || context.semantic_type_id != requirement.semantic_type_id
                || context.canonicalizer_identity != requirement.canonicalizer_identity
            {
                return Err(PlanError::ContextContract(format!(
                    "state {} context descriptor mismatch for {}",
                    descriptor.name(),
                    context.context_ref
                )));
            }
            Ok(())
        }
    }
}

fn output_context_from_contract(
    descriptor: &StateDescriptorIdentity,
    context: Option<&CertifiedContextSpec>,
) -> Result<CellContextSpec> {
    match descriptor.output_context() {
        StateOutputContextContractSpec::NoContext => Ok(CellContextSpec::NoContext),
        StateOutputContextContractSpec::Produces {
            resource_kind,
            stage,
        } => {
            let context = context.ok_or_else(|| {
                PlanError::ContextContract(format!(
                    "state {} produces a context-bound resource without context",
                    descriptor.name()
                ))
            })?;
            Ok(CellContextSpec::Bound {
                context_ref: context.context_ref.clone(),
                resource_kind: resource_kind.clone(),
                stage: stage.clone(),
                producer: Box::new(ContextProducerSpec {
                    producer_descriptor_ids: vec![descriptor.descriptor_id().clone()],
                    seed_producers_allowed: false,
                }),
            })
        }
    }
}

/// Pre-certification typed program plus launch material produced by deterministic planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedProgramLaunchPlan {
    /// Typed program draft to be lowered and certified by app assembly.
    pub draft: TypedProgramDraft,
    /// Canonical non-secret config material required by the draft.
    pub config_material: Vec<TypedProgramConfigMaterial>,
    /// Canonical non-secret seed material required by the draft.
    pub seed_material: Vec<TypedProgramSeedMaterial>,
}

impl TypedProgramLaunchPlan {
    /// Builds a launch plan from a draft that does not require launch seed material.
    pub fn from_draft(draft: TypedProgramDraft) -> Result<Self> {
        Self::from_draft_and_seed_material(draft, BTreeMap::new())
    }

    /// Builds a launch plan from a draft and canonical seed material keyed by seed id.
    ///
    /// Seed material is matched by [`SeedId`] and returned in draft seed order so callers do not
    /// depend on positional seed input.
    pub fn from_draft_and_seed_material(
        draft: TypedProgramDraft,
        seeds: BTreeMap<SeedId, PlainCanonicalJsonBytes>,
    ) -> Result<Self> {
        let config_material = config_material_for_draft(&draft)?;
        let seed_material = seed_material_for_draft(&draft, seeds)?;
        Ok(Self {
            draft,
            config_material,
            seed_material,
        })
    }
}

/// Canonical non-secret config material required to launch a typed program draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedProgramConfigMaterial {
    /// Config schema id consumed by the typed program.
    pub schema_id: SchemaId,
    /// Canonical JSON config bytes.
    pub bytes: PlainCanonicalJsonBytes,
    /// Media type for the canonical config bytes.
    pub media_type: MediaType,
}

/// Canonical non-secret seed material required to launch a typed program draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedProgramSeedMaterial {
    /// Seed id consumed by the typed program.
    pub seed_id: SeedId,
    /// Canonical JSON seed bytes.
    pub bytes: PlainCanonicalJsonBytes,
    /// Media type for the canonical seed bytes.
    pub media_type: MediaType,
}

fn config_material_for_draft(draft: &TypedProgramDraft) -> Result<Vec<TypedProgramConfigMaterial>> {
    let media_type = typed_program_launch_json_media_type()?;
    Ok(draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        .map(|config| TypedProgramConfigMaterial {
            schema_id: config.schema_id.clone(),
            bytes: config.canonical_json.clone(),
            media_type: media_type.clone(),
        })
        .collect())
}

fn seed_material_for_draft(
    draft: &TypedProgramDraft,
    mut seeds: BTreeMap<SeedId, PlainCanonicalJsonBytes>,
) -> Result<Vec<TypedProgramSeedMaterial>> {
    let media_type = typed_program_launch_json_media_type()?;
    let mut material = Vec::with_capacity(draft.seeds().len());
    for seed in draft.seeds() {
        let bytes = seeds.remove(&seed.seed_id).ok_or_else(|| {
            PlanError::Key(format!(
                "missing entry-point seed material for {}",
                seed.seed_id
            ))
        })?;
        let digest = bytes.content_digest();
        let byte_len = bytes.as_bytes().len();
        if digest != seed.content_digest || byte_len != seed.byte_len {
            return Err(PlanError::Canonical(format!(
                "entry-point seed material did not match draft seed {}",
                seed.seed_id
            )));
        }
        material.push(TypedProgramSeedMaterial {
            seed_id: seed.seed_id.clone(),
            bytes,
            media_type: media_type.clone(),
        });
    }
    if let Some(seed_id) = seeds.keys().next() {
        return Err(PlanError::Key(format!(
            "unknown entry-point seed material for {seed_id}"
        )));
    }
    Ok(material)
}

fn typed_program_launch_json_media_type() -> Result<MediaType> {
    MediaType::new("application/json").map_err(|error| PlanError::Key(error.to_string()))
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
            CellContextSpec::no_context(),
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
    context_refs: BTreeSet<String>,
    contexts: Vec<CertifiedContextSpec>,
    context_validators: Vec<ContextValidatorSpec>,
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
    context_refs: BTreeSet<String>,
    contexts: Vec<CertifiedContextSpec>,
    context_validators: Vec<ContextValidatorSpec>,
    state_nodes: Vec<StateNodeSpec>,
    remediation_nodes: BTreeMap<NodeId, StateNodeSpec>,
    operation_lineage: Vec<OperationLineageFrameSpec>,
    active_operation_stack: Vec<OperationInstanceId>,
    child_scope_keys: BTreeSet<String>,
    child_scopes: Vec<ScopeSpec>,
    bridge_nodes: Vec<BridgeNodeSpec>,
}

#[derive(Debug, Default)]
struct StateNodePlanningOptions {
    context: Option<CertifiedContextSpec>,
    output_domain_keys: Vec<StableDomainKeyRef>,
    side_effect_contract: Option<(ResourceClaim, SideEffectVerificationSpec)>,
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
                context_refs: BTreeSet::new(),
                contexts: Vec::new(),
                context_validators: Vec::new(),
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
        for context in &child.scope.contexts {
            if self.context_refs.contains(context.context_ref.as_str()) {
                return Err(PlanError::DuplicateContextRef(
                    context.context_ref.as_str().to_owned(),
                ));
            }
        }
        self.state_nodes.extend(child.scope.state_nodes);
        self.remediation_nodes.extend(child.scope.remediation_nodes);
        self.context_refs.extend(child.scope.context_refs);
        self.contexts.extend(child.scope.contexts);
        self.context_validators
            .extend(child.scope.context_validators);
        self.operation_lineage.extend(child.scope.operation_lineage);
        self.child_scopes.extend(child.scope.child_scopes);
        self.bridge_nodes.extend(child.bridge_nodes);
        self.bridge_nodes.extend(child.scope.bridge_nodes);
        Ok(bridged.value)
    }

    /// Declares one typed certified transition context in this scope.
    pub fn declare_context<C>(&mut self, value: C) -> Result<DeclaredContext<'program, 'scope, C>>
    where
        C: MfmContext,
    {
        let spec = certified_context_spec(value)?;
        let validator = context_validator_spec_for::<C>()?;
        if !self
            .context_refs
            .insert(spec.context_ref.as_str().to_owned())
        {
            return Err(PlanError::DuplicateContextRef(
                spec.context_ref.as_str().to_owned(),
            ));
        }
        self.contexts.push(spec.clone());
        self.context_validators.push(validator);
        Ok(DeclaredContext {
            spec,
            _program: PhantomData,
            _scope: PhantomData,
            _context: PhantomData,
        })
    }

    /// Plans a registry-resolved typed state by resolving `S` through this builder's registry.
    pub fn state<S, I>(
        &mut self,
        key: StateKey,
        context: impl StateTransitionContext<'program, 'scope, S::Context>,
        config: S::Config,
        input: I,
    ) -> Result<Handle<'program, 'scope, S::Output>>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        let descriptor = self.state_registry.state_descriptor::<S>()?;
        self.state_with_domain_key_refs::<S, I>(key, descriptor, context, config, input, Vec::new())
    }

    /// Plans a registry-resolved typed state and attaches stable domain-key evidence to its output
    /// value lineage.
    pub fn state_with_domain_keys<S, I, K>(
        &mut self,
        key: StateKey,
        context: impl StateTransitionContext<'program, 'scope, S::Context>,
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
        let descriptor = self.state_registry.state_descriptor::<S>()?;
        self.state_with_domain_key_refs::<S, I>(
            key,
            descriptor,
            context,
            config,
            input,
            stable_domain_key_refs(domain_keys)?,
        )
    }

    /// Plans a registered side-effect state with an explicit transition context and resource claim.
    pub fn side_effect<S, I>(
        &mut self,
        key: StateKey,
        context: impl StateTransitionContext<'program, 'scope, S::Context>,
        config: S::Config,
        input: I,
        resource_claim: ResourceClaim,
        verification: SideEffectVerificationSpec,
    ) -> Result<ForwardSideEffectHandle<'program, 'scope, S::Output>>
    where
        S: SideEffectState,
        S::Caps: CapabilitySetFor<ApplySideEffect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        let descriptor = self.state_registry.state_descriptor::<S>()?;
        let context = context.certified_context_spec().cloned();
        let key_string = key.as_str().to_owned();
        if self.state_keys.contains(&key_string) {
            return Err(PlanError::DuplicateStateKey(key.as_str().to_owned()));
        }
        let (node, handle) = self.plan_state_node::<S, I>(
            key,
            descriptor,
            config,
            input,
            StateNodePlanningOptions {
                context,
                side_effect_contract: Some((resource_claim, verification)),
                ..StateNodePlanningOptions::default()
            },
        )?;
        self.state_keys.insert(key_string);
        self.state_nodes.push(node.clone());
        Ok(ForwardSideEffectHandle::new(node.node_id, handle))
    }

    /// Plans one forward side-effect state and one structurally separate remediation state.
    ///
    /// The forward node is appended to the ordinary forward graph. The remediation node is keyed
    /// by the forward node id in the draft remediation collection, so the forward scheduler cannot
    /// select it. The remediation input may reference only the linked forward output and cells
    /// that are transitive ancestors of that forward node.
    pub fn side_effect_with_compensation<F, R, I, J, B>(
        &mut self,
        forward_context: impl StateTransitionContext<'program, 'scope, F::Context>,
        remediation_context: impl StateTransitionContext<'program, 'scope, R::Context>,
        forward_params: SideEffectNodeParams<F, I>,
        remediation_params: RemediationNodeParams<R>,
        build_remediation_input: B,
    ) -> Result<LinkedSideEffectHandles<'program, 'scope, F::Output, R::Output>>
    where
        F: SideEffectState,
        F::Caps: CapabilitySetFor<ApplySideEffect>,
        I: IntoStateInput<'program, 'scope, F::Input>,
        R: SideEffectState,
        R::Caps: CapabilitySetFor<ApplySideEffect>,
        J: IntoStateInput<'program, 'scope, R::Input>,
        B: FnOnce(ForwardSideEffectHandle<'program, 'scope, F::Output>) -> Result<J>,
    {
        let forward_context = forward_context.certified_context_spec().cloned();
        let remediation_context = remediation_context.certified_context_spec().cloned();
        let SideEffectNodeParams {
            key: forward_key,
            config: forward_config,
            input: forward_input,
            resource_claim: forward_resource_claim,
            verification: forward_verification,
        } = forward_params;
        let RemediationNodeParams {
            key: remediation_key,
            config: remediation_config,
            resource_claim: remediation_resource_claim,
            verification: remediation_verification,
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

        let forward_descriptor = self.state_registry.state_descriptor::<F>()?;
        let (forward_node, forward_handle) = self.plan_state_node::<F, I>(
            forward_key,
            forward_descriptor,
            forward_config,
            forward_input,
            StateNodePlanningOptions {
                context: forward_context,
                side_effect_contract: Some((forward_resource_claim, forward_verification)),
                ..StateNodePlanningOptions::default()
            },
        )?;
        let forward = ForwardSideEffectHandle::new(forward_node.node_id.clone(), forward_handle);
        let remediation_input = match build_remediation_input(forward.clone()) {
            Ok(input) => input,
            Err(error) => {
                self.restore(checkpoint);
                return Err(error);
            }
        };

        let remediation_descriptor = match self.state_registry.state_descriptor::<R>() {
            Ok(descriptor) => descriptor,
            Err(error) => {
                self.restore(checkpoint);
                return Err(error.into());
            }
        };
        let (remediation_node, remediation_handle) = match self.plan_state_node::<R, J>(
            remediation_key,
            remediation_descriptor,
            remediation_config,
            remediation_input,
            StateNodePlanningOptions {
                context: remediation_context,
                side_effect_contract: Some((remediation_resource_claim, remediation_verification)),
                ..StateNodePlanningOptions::default()
            },
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

    fn state_with_domain_key_refs<S, I>(
        &mut self,
        key: StateKey,
        descriptor: StateDescriptorIdentity,
        context: impl StateTransitionContext<'program, 'scope, S::Context>,
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
        let context = context.certified_context_spec().cloned();
        let (node, handle) = self.plan_state_node::<S, I>(
            key,
            descriptor,
            config,
            input,
            StateNodePlanningOptions {
                context,
                output_domain_keys,
                ..StateNodePlanningOptions::default()
            },
        )?;
        self.state_keys.insert(key_string);
        self.state_nodes.push(node);
        Ok(handle)
    }

    fn plan_state_node<S, I>(
        &self,
        key: StateKey,
        descriptor: StateDescriptorIdentity,
        config: S::Config,
        input: I,
        options: StateNodePlanningOptions,
    ) -> Result<(StateNodeSpec, Handle<'program, 'scope, S::Output>)>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        let config =
            ValidatedConfig::new(config).map_err(|error| PlanError::Value(error.to_string()))?;
        let config_binding = canonical_config_binding::<S::Config>(&config)?;
        let input = input.into_binding()?;
        let adapter_bindings = S::adapter_bindings()?;
        let state = S::new(config)?;
        drop(state);
        let StateNodePlanningOptions {
            context,
            output_domain_keys,
            side_effect_contract,
        } = options;

        let descriptor = &descriptor;
        validate_state_context_binding(descriptor, context.as_ref())?;
        let side_effect_contract_digest = descriptor.side_effect_contract_digest().cloned();
        let side_effect_contract =
            side_effect_contract.map(|(claim, verification)| (claim.into_spec(), verification));
        match (&side_effect_contract_digest, &side_effect_contract) {
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
        let node_context = context
            .as_ref()
            .map(|context| NodeContextSpec::Required {
                context_ref: context.context_ref.clone(),
            })
            .unwrap_or_else(NodeContextSpec::no_context);
        let node_id = state_node_id(
            &self.scope_id,
            &key,
            descriptor.kind(),
            descriptor.version(),
            &config_binding.config_ref_digest,
            input.digest(),
            &node_context,
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
        let side_effect_is_paired = side_effect_contract.is_some();
        let lineage = state_value_lineage(
            &self.scope_id,
            &node_id,
            input.root(),
            &config_binding.config_ref_digest,
            &self.current_operation_lineage()?,
            if side_effect_is_paired {
                Vec::new()
            } else {
                output_domain_keys.clone()
            },
        )?;
        let value_lineage = value_lineage_ref(&lineage)?;
        let output_context = output_context_from_contract(descriptor, context.as_ref())?;
        let side_effect_verify = match (
            side_effect_contract_digest.as_ref(),
            side_effect_contract.as_ref(),
        ) {
            (Some(contract_digest), Some((resource_claim, verification))) => {
                let contract = mfm_spec::v1::SideEffectContractSpec {
                    contract_digest: contract_digest.clone(),
                    resource_claim: resource_claim.clone(),
                    verification: verification.clone(),
                };
                let pair_id =
                    mfm_spec::v1::side_effect_pair_id(&node_id, &output_cell_id, &contract)
                        .map_err(|error| PlanError::Value(error.to_string()))?;
                let verify_node_id = mfm_spec::v1::side_effect_verify_node_id(&node_id, &pair_id)
                    .map_err(|error| PlanError::Value(error.to_string()))?;
                let verify_output_cell_id = state_output_cell_id(
                    &self.scope_id,
                    &verify_node_id,
                    &output_semantic_type_id,
                    &output_schema_id,
                )?;
                let verify_config_ref =
                    mfm_spec::v1::framework_config_ref("side_effect_verify", &verify_node_id)
                        .map_err(|error| PlanError::Value(error.to_string()))?;
                let verify_lineage = state_value_lineage_from_cells(
                    &self.scope_id,
                    &verify_node_id,
                    vec![output_cell_id.clone()],
                    Some(spec_config_ref_digest(&verify_config_ref)?),
                    &self.current_operation_lineage()?,
                    output_domain_keys.clone(),
                )?;
                Some(SideEffectVerifyDraftSpec {
                    pair_id,
                    node_id: verify_node_id,
                    output_cell_id: verify_output_cell_id,
                    output_value_lineage: value_lineage_ref(&verify_lineage)?,
                    output_context: output_context.clone(),
                    output_domain_keys: output_domain_keys.clone(),
                })
            }
            _ => None,
        };
        let (handle_cell_id, handle_value_lineage) =
            if let Some(verify) = side_effect_verify.as_ref() {
                (
                    verify.output_cell_id.clone(),
                    verify.output_value_lineage.clone(),
                )
            } else {
                (output_cell_id.clone(), value_lineage.clone())
            };
        let handle = Handle::new(
            handle_cell_id,
            self.scope_id.clone(),
            output_schema_id.clone(),
            output_semantic_type_id.clone(),
            handle_value_lineage,
            output_context.clone(),
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
                context_descriptor: descriptor.context().clone(),
                input_context_contract: descriptor.input_context().clone(),
                output_context_contract: descriptor.output_context().clone(),
                context: node_context,
                runner: descriptor.runner(),
                effect_kind: descriptor.effect().kind.clone(),
                capability_bindings: descriptor.capabilities().clone(),
                adapter_bindings,
                fact_descriptor_allowlist: descriptor.emitted_fact_descriptors().to_vec(),
                side_effect_contract_digest,
                side_effect_resource_claim: side_effect_contract
                    .as_ref()
                    .map(|(claim, _)| claim.clone()),
                side_effect_verification: side_effect_contract
                    .as_ref()
                    .map(|(_, verification)| verification.clone()),
                side_effect_verify,
                config: config_binding,
                input: input.spec(),
                output_cell_id,
                output_schema_id,
                output_semantic_type_id,
                output_value_lineage: value_lineage,
                output_context,
                output_domain_keys: if side_effect_is_paired {
                    Vec::new()
                } else {
                    output_domain_keys
                },
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
        if let Some(verify) = &forward_node.side_effect_verify {
            allowed.insert(verify.output_cell_id.clone());
        }

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

    /// Expands a registry-resolved typed operation by resolving `O` through this builder's registry.
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
        let descriptor = self.operation_registry.operation_descriptor::<O>()?;
        self.call_with_descriptor(key, descriptor, operation, config, input)
    }

    fn call_with_descriptor<O, I>(
        &mut self,
        key: OperationKey,
        descriptor: OperationDescriptorIdentity,
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
        let config =
            ValidatedConfig::new(config).map_err(|error| PlanError::Value(error.to_string()))?;
        let config_binding = canonical_config_binding::<O::Config>(&config)?;
        let operation_input = input.into_operation_input()?;
        let input_binding = operation_input.input_binding()?;
        let output_schema_id =
            <O::Output<'program, 'scope> as OperationOutput<'program, 'scope>>::output_schema_id()?;
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
            context_refs: self.context_refs.clone(),
            contexts: self.contexts.clone(),
            context_validators: self.context_validators.clone(),
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
        self.context_refs = checkpoint.context_refs;
        self.contexts = checkpoint.contexts;
        self.context_validators = checkpoint.context_validators;
        self.state_nodes = checkpoint.state_nodes;
        self.remediation_nodes = checkpoint.remediation_nodes;
        self.operation_lineage = checkpoint.operation_lineage;
        self.active_operation_stack = checkpoint.active_operation_stack;
        self.child_scope_keys = checkpoint.child_scope_keys;
        self.child_scopes = checkpoint.child_scopes;
        self.bridge_nodes = checkpoint.bridge_nodes;
    }
}

/// Operation-local planning context minted only by [`ScopeBuilder::call`].
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
        // SAFETY: OperationExpansion is created only by ScopeBuilder::call from its exclusive
        // `&mut self`. The raw pointer is never exposed, all mutation goes through
        // `&mut OperationExpansion`, and call does not touch the ScopeBuilder again until
        // operation expansion returns.
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

    /// Declares one typed certified transition context in the current expansion scope.
    pub fn declare_context<C>(&mut self, value: C) -> Result<DeclaredContext<'program, 'scope, C>>
    where
        C: MfmContext,
    {
        self.scope_mut().declare_context(value)
    }

    /// Plans a registry-resolved typed state by resolving `S` through this expansion's registry.
    pub fn state<S, I>(
        &mut self,
        key: StateKey,
        context: impl StateTransitionContext<'program, 'scope, S::Context>,
        config: S::Config,
        input: I,
    ) -> Result<Handle<'program, 'scope, S::Output>>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        self.scope_mut().state::<S, I>(key, context, config, input)
    }

    /// Plans a registry-resolved typed state and attaches stable domain-key evidence to its output.
    pub fn state_with_domain_keys<S, I, K>(
        &mut self,
        key: StateKey,
        context: impl StateTransitionContext<'program, 'scope, S::Context>,
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
            .state_with_domain_keys::<S, I, K>(key, context, config, input, domain_keys)
    }

    /// Plans a registered side-effect state with an explicit transition context and resource claim.
    pub fn side_effect<S, I>(
        &mut self,
        key: StateKey,
        context: impl StateTransitionContext<'program, 'scope, S::Context>,
        config: S::Config,
        input: I,
        resource_claim: ResourceClaim,
        verification: SideEffectVerificationSpec,
    ) -> Result<ForwardSideEffectHandle<'program, 'scope, S::Output>>
    where
        S: SideEffectState,
        S::Caps: CapabilitySetFor<ApplySideEffect>,
        I: IntoStateInput<'program, 'scope, S::Input>,
    {
        self.scope_mut().side_effect::<S, I>(
            key,
            context,
            config,
            input,
            resource_claim,
            verification,
        )
    }

    /// Plans a linked forward/remediation side-effect pair.
    pub fn side_effect_with_compensation<F, R, I, J, B>(
        &mut self,
        forward_context: impl StateTransitionContext<'program, 'scope, F::Context>,
        remediation_context: impl StateTransitionContext<'program, 'scope, R::Context>,
        forward_params: SideEffectNodeParams<F, I>,
        remediation_params: RemediationNodeParams<R>,
        build_remediation_input: B,
    ) -> Result<LinkedSideEffectHandles<'program, 'scope, F::Output, R::Output>>
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
                forward_context,
                remediation_context,
                forward_params,
                remediation_params,
                build_remediation_input,
            )
    }

    /// Expands a registry-resolved typed operation by resolving `O` through this expansion's registry.
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
            target.context,
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
            target.context,
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
            context: source.context.clone(),
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
            context: source.context,
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
            context_refs: BTreeSet::new(),
            contexts: Vec::new(),
            context_validators: Vec::new(),
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
        contexts: builder.scope.contexts,
        context_validators: builder.scope.context_validators,
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

mod private {
    use super::{
        ApplySideEffect, DeclaredContext, ForwardSideEffectHandle, Handle, ManagedPlatformWrite,
        MfmContext, MfmValue, NoContext, NonEmptyHandles, Pure, ReadExternal, SideEffectState,
        StateContext, StateSpec,
    };

    pub trait EffectRunnerSealed<S: StateSpec> {}
    pub trait OperationInputSealed {}
    pub trait BridgeableSealed {}
    pub trait StateTransitionContextSealed<'program, 'scope, C: StateContext> {}

    impl<S> EffectRunnerSealed<S> for Pure where S: super::PureState {}

    impl<S> EffectRunnerSealed<S> for ReadExternal where S: super::ReadState {}

    impl<S> EffectRunnerSealed<S> for ManagedPlatformWrite where S: super::ManagedWriteState {}

    impl<S> EffectRunnerSealed<S> for ApplySideEffect where S: SideEffectState {}

    impl<'program, 'scope> StateTransitionContextSealed<'program, 'scope, NoContext> for NoContext {}

    impl<'program, 'scope, C> StateTransitionContextSealed<'program, 'scope, C>
        for &DeclaredContext<'program, 'scope, C>
    where
        C: MfmContext,
    {
    }

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
