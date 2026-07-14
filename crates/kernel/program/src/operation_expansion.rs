use super::*;

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
    pub(super) fn new(scope: &mut ScopeBuilder<'program, 'scope>) -> Self {
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
