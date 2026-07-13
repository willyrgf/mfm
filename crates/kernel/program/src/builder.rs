use super::*;

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

    pub(super) fn current_operation_lineage(&self) -> Result<OperationLineage> {
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
