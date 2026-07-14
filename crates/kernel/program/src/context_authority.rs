use super::*;
use std::fmt;

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
    pub(crate) spec: CertifiedContextSpec,
    pub(crate) _program: PhantomData<fn(&'program ()) -> &'program ()>,
    pub(crate) _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    pub(crate) _context: PhantomData<fn(C) -> C>,
}

/// Type-erased context descriptor validator retained by in-memory program drafts.
#[derive(Clone)]
pub struct ContextValidatorSpec {
    pub(crate) requirement: StateContextDescriptorRequirementSpec,
    pub(crate) validate: fn(&CertifiedContextSpec) -> Result<()>,
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
    pub(crate) fn spec(&self) -> &CertifiedContextSpec {
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
