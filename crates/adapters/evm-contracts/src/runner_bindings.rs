use super::*;

/// Registers context-bound contract lifecycle runners with the supplied runtime factory.
pub fn register_contract_lifecycle_runners_with_factory(
    registry: &mut ErasedRunnerRegistry,
    factory: Arc<dyn EvmContractRuntimeFactory>,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let deploy_descriptor = mfm_program::state_descriptor::<ContextBoundDeployContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let configure_descriptor =
        mfm_program::state_descriptor::<ContextBoundConfigureContractState>()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let validate_descriptor = mfm_program::state_descriptor::<ContextBoundValidateContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let import_deployed_descriptor =
        mfm_program::state_descriptor::<mfm_state_evm_contracts::ImportDeployedContractState>()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let import_configured_descriptor =
        mfm_program::state_descriptor::<mfm_state_evm_contracts::ImportConfiguredContractState>()
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    for descriptor in [
        &deploy_descriptor,
        &configure_descriptor,
        &validate_descriptor,
        &import_deployed_descriptor,
        &import_configured_descriptor,
    ] {
        registry.register_capability_set(descriptor.capabilities(), implementation_id.clone())?;
    }
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm-contracts",
        "evm-contract-lifecycle-context",
        env!("CARGO_PKG_VERSION"),
    )?;
    let side_effect_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(SIDE_EFFECT_FACTORY)?);
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    let adapter_binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_adapter_executable_with_factory(
        adapter_binding.adapter_kind().clone(),
        adapter_binding.adapter_version().clone(),
        &adapter_factory,
    )?;
    let deploy = registrations
        .register_state_runner_with_factory::<ContextBoundDeployContractState>(
            &side_effect_factory,
            Arc::new(ContractMutationRunner::<ContextDeployMutationPlan> {
                factory: factory.clone(),
                extractor: TypedContextOutputExtractor::new(),
                _phase: PhantomData,
            }),
        )?;
    let configure = registrations
        .register_state_runner_with_factory::<ContextBoundConfigureContractState>(
            &side_effect_factory,
            Arc::new(ContractMutationRunner::<ContextConfigureMutationPlan> {
                factory: factory.clone(),
                extractor: TypedContextOutputExtractor::new(),
                _phase: PhantomData,
            }),
        )?;
    registrations.register_state_runner_with_factory::<ContextBoundValidateContractState>(
        &read_factory,
        Arc::new(ContextContractValidateRunner {
            factory: factory.clone(),
            extractor: TypedContextOutputExtractor::new(),
        }),
    )?;
    registrations
        .register_state_runner_with_factory::<mfm_state_evm_contracts::ImportDeployedContractState>(
            &read_factory,
            Arc::new(ImportDeployedRunner {
                factory: factory.clone(),
                extractor: TypedContextOutputExtractor::new(),
            }),
        )?;
    registrations.register_state_runner_with_factory::<mfm_state_evm_contracts::ImportConfiguredContractState>(
        &read_factory,
        Arc::new(ImportConfiguredRunner {
            factory: factory.clone(),
            extractor: TypedContextOutputExtractor::new(),
        }),
    )?;
    registrations.register_side_effect_verify_runner_with_factory(
        deploy.descriptor_id().clone(),
        &read_factory,
        Arc::new(ContextContractVerifyRunner {
            factory: factory.clone(),
            extractor: ContractLifecycleContextOutputExtractor::new(),
        }),
    )?;
    registrations.register_side_effect_verify_runner_with_factory(
        configure.descriptor_id().clone(),
        &read_factory,
        Arc::new(ContextContractVerifyRunner {
            factory,
            extractor: ContractLifecycleContextOutputExtractor::new(),
        }),
    )?;
    Ok(())
}

struct ContextContractValidateRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: TypedContextOutputExtractor<ContextBoundValidationReport>,
}

impl ErasedNodeRunner for ContextContractValidateRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let context = ctx.context()?.certified_context::<EvmContractContext>()?;
        let binding = runtime_evm_network_binding_for_context(&context)?;
        self.factory
            .validate_runtime_for(&binding, None)
            .map(|_| ())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_context_validate(ctx, self.factory.as_ref()).await })
    }
}

struct ImportDeployedRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: TypedContextOutputExtractor<DeployedContractInstance>,
}

impl ErasedNodeRunner for ImportDeployedRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let context = ctx.context()?.certified_context::<EvmContractContext>()?;
        let binding = runtime_evm_network_binding_for_context(&context)?;
        self.factory
            .validate_runtime_for(&binding, None)
            .map(|_| ())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_import_deployed(ctx, self.factory.as_ref()).await })
    }
}

struct ImportConfiguredRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: TypedContextOutputExtractor<ConfiguredContractInstance>,
}

impl ErasedNodeRunner for ImportConfiguredRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let context = ctx.context()?.certified_context::<EvmContractContext>()?;
        let binding = runtime_evm_network_binding_for_context(&context)?;
        self.factory
            .validate_runtime_for(&binding, None)
            .map(|_| ())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_import_configured(ctx, self.factory.as_ref()).await })
    }
}

struct ContextContractVerifyRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: ContractLifecycleContextOutputExtractor,
}

impl ErasedNodeRunner for ContextContractVerifyRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let submit_node = side_effect_verify_submit_node_for_ingress(&ctx)?;
        validate_context_mutation_runtime_for_node(&ctx, submit_node, self.factory.as_ref())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_context_verify(ctx, self.factory.as_ref()).await })
    }
}

struct ContractLifecycleContextOutputExtractor {
    deployed: TypedContextOutputExtractor<DeployedContractInstance>,
    configured: TypedContextOutputExtractor<ConfiguredContractInstance>,
    validation_report: TypedContextOutputExtractor<ContextBoundValidationReport>,
}

impl ContractLifecycleContextOutputExtractor {
    const fn new() -> Self {
        Self {
            deployed: TypedContextOutputExtractor::new(),
            configured: TypedContextOutputExtractor::new(),
            validation_report: TypedContextOutputExtractor::new(),
        }
    }
}

impl mfm_runtime::ContextOutputExtractor for ContractLifecycleContextOutputExtractor {
    fn validate_context_output(
        &self,
        cell_context: &spec::CellContextSpec,
        artifact: &store::ArtifactEvidenceRef,
        bytes: &[u8],
    ) -> mfm_runtime::Result<()> {
        let spec::CellContextSpec::Bound { stage, .. } = cell_context else {
            return Ok(());
        };
        if stage == deployed_contract_stage() {
            return self
                .deployed
                .validate_context_output(cell_context, artifact, bytes);
        }
        if stage == configured_contract_stage() {
            return self
                .configured
                .validate_context_output(cell_context, artifact, bytes);
        }
        if stage == validation_report_stage() {
            return self
                .validation_report
                .validate_context_output(cell_context, artifact, bytes);
        }
        Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "unsupported EVM contract context output stage {}",
            stage
        )))
    }
}

struct ContractMutationRunner<P: ContractMutationPlanOps> {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: TypedContextOutputExtractor<<P as ContractMutationPlanOps>::Output>,
    _phase: PhantomData<P>,
}

impl<P> ErasedNodeRunner for ContractMutationRunner<P>
where
    P: ContractMutationPlanOps + 'static,
{
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        P::validate_ingress(&ctx, self.factory.as_ref())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let plan = P::load_plan(
                ctx.node(),
                ctx.inputs(),
                ctx.context(),
                self.factory.artifacts(),
            )
            .await?;
            let scope = plan.nonce_resource_scope()?;
            let resource_key =
                mutation_resource_key(ctx.node(), &scope, plan.expected_signer_address())?;
            preclaim_side_effect_resource_lane(
                ctx,
                plan.intent(),
                plan.idempotency(),
                idempotency_key_ref(plan.idempotency())?,
                evm_transaction_submit_binding()?,
                resource_key,
            )
        })
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_contract_mutation::<P>(ctx, self.factory.as_ref()).await })
    }
}

async fn run_contract_mutation<P>(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    P: ContractMutationPlanOps,
{
    let plan = P::load_plan(ctx.node(), ctx.inputs(), ctx.context(), factory.artifacts()).await?;
    let callbacks = ContractMutationSideEffectCallbacks { factory, plan };
    SideEffectDriver::drive(ctx, &callbacks).await
}

async fn run_context_verify(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let phase = {
        let submit_node = side_effect_verify_submit_node(&ctx)?;
        context_mutation_phase_for_submit_node(submit_node)?
    };
    match phase {
        ContractMutationPhase::Deploy => {
            let callbacks = ContractVerifyCallbacks::<ContextDeployMutationPlan> {
                factory,
                _phase: PhantomData,
            };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
        ContractMutationPhase::Configure => {
            let callbacks = ContractVerifyCallbacks::<ContextConfigureMutationPlan> {
                factory,
                _phase: PhantomData,
            };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
    }
}

fn side_effect_verify_submit_node<'a>(
    ctx: &'a ErasedRunCtx<'a>,
) -> mfm_runtime::Result<&'a spec::NodeSpec> {
    let submit_node_id = side_effect_verify_submit_node_id(ctx.node()).map_err(|_| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a side-effect verify node",
            ctx.node().node_id
        ))
    })?;
    ctx.certified_node(submit_node_id).ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} references missing submit node {}",
            ctx.node().node_id,
            submit_node_id
        ))
    })
}

fn side_effect_verify_submit_node_for_ingress<'a>(
    ctx: &'a RunnerIngressContext<'a>,
) -> mfm_runtime::Result<&'a spec::NodeSpec> {
    let submit_node_id = side_effect_verify_submit_node_id(ctx.node()).map_err(|_| {
        mfm_runtime::RuntimeError::RunnerBinding(format!(
            "node {} is not a side-effect verify node",
            ctx.node().node_id
        ))
    })?;
    ctx.runtime_spec().node(submit_node_id).ok_or_else(|| {
        mfm_runtime::RuntimeError::RunnerBinding(format!(
            "side-effect verify node {} references missing submit node {}",
            ctx.node().node_id,
            submit_node_id
        ))
    })
}

fn side_effect_verify_submit_node_id(node: &spec::NodeSpec) -> std::result::Result<&NodeId, ()> {
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
        return Err(());
    };
    Ok(&verify.submit_node_id)
}

fn context_mutation_phase_for_submit_node(
    node: &spec::NodeSpec,
) -> mfm_runtime::Result<ContractMutationPhase> {
    let deploy = mfm_program::state_descriptor::<ContextBoundDeployContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    if &node.descriptor_id == deploy.descriptor_id() {
        return Ok(ContractMutationPhase::Deploy);
    }
    let configure = mfm_program::state_descriptor::<ContextBoundConfigureContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    if &node.descriptor_id == configure.descriptor_id() {
        return Ok(ContractMutationPhase::Configure);
    }
    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "side-effect verify submit node {} is not a context-bound EVM contract mutation",
        node.node_id
    )))
}

fn validate_context_mutation_runtime_for_node(
    ctx: &RunnerIngressContext<'_>,
    node: &spec::NodeSpec,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<()> {
    let context = ctx
        .runtime_spec()
        .invocation_context_for_node(node)?
        .certified_context::<EvmContractContext>()?;
    let signer_ref = match context_mutation_phase_for_submit_node(node)? {
        ContractMutationPhase::Deploy => {
            let action = load_launch_config_for_node::<DeployAction>(ctx, node)?;
            action
                .as_ref()
                .signer()
                .signer_ref()
                .map_err(runtime_ingress_model_error)?
        }
        ContractMutationPhase::Configure => {
            let action = load_launch_config_for_node::<ConfigureAction>(ctx, node)?;
            action
                .as_ref()
                .signer()
                .signer_ref()
                .map_err(runtime_ingress_model_error)?
        }
    };
    let binding = runtime_evm_network_binding_for_context(&context)?;
    factory
        .validate_runtime_for(&binding, Some(&signer_ref))
        .map(|_| ())
}

struct ContextDeployMutationPlan {
    action: ValidatedConfig<DeployAction>,
    state: ContextBoundDeployContractState,
    context: mfm_program::CertifiedContext<EvmContractContext>,
    artifact: ContractArtifactConfig,
    intent: ContextContractDeployIntent,
    idempotency: ContractTransactionIdempotency,
}

struct ContextConfigureMutationPlan {
    node_id: NodeId,
    action: ValidatedConfig<ConfigureAction>,
    state: ContextBoundConfigureContractState,
    input: ContextConfigureContractInput,
    context: mfm_program::CertifiedContext<EvmContractContext>,
    artifact: Option<ContractArtifactConfig>,
    intent: ContextContractConfigureIntent,
    idempotency: ContractTransactionIdempotency,
}

pub(crate) struct ContractNonceResourceScope {
    pub(crate) evm_network_context_ref: String,
}

fn account_nonce_resource_key_for_node(
    node: &spec::NodeSpec,
    scope: &ContractNonceResourceScope,
    expected_signer_address: &str,
) -> Result<Option<events::ResourceKeyEvidence>> {
    let Some(side_effect) = &node.side_effect else {
        return Ok(None);
    };
    let spec::ResourceClaimSpec::Exclusive {
        namespace,
        key_schema,
    } = &side_effect.resource_claim
    else {
        return Ok(None);
    };

    let expected_namespace = account_nonce_resource_namespace()
        .map_err(|error| EvmContractAdapterError::State(error.to_string()))?;
    let expected_key_schema = account_nonce_resource_key_schema_id()
        .map_err(|error| EvmContractAdapterError::State(error.to_string()))?;
    if namespace != &expected_namespace || key_schema != &expected_key_schema {
        return Err(EvmContractAdapterError::State(
            "EVM mutation side effect must declare the account nonce resource claim".to_owned(),
        ));
    }

    let account = normalize_address(expected_signer_address)
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    let key = account_nonce_resource_key(scope, &account)?;

    Ok(Some(events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema.clone(),
        key,
    }))
}

pub(crate) fn account_nonce_resource_key(
    scope: &ContractNonceResourceScope,
    account: &str,
) -> Result<events::ResourceKey> {
    events::ResourceKey::new(format!(
        r#"{{"account":"{}","evm_network_context_ref":"{}"}}"#,
        account, scope.evm_network_context_ref
    ))
    .map_err(|error| EvmContractAdapterError::Model(error.to_string()))
}

fn mutation_resource_key(
    node: &spec::NodeSpec,
    scope: &ContractNonceResourceScope,
    expected_signer_address: &str,
) -> mfm_runtime::Result<events::ResourceKeyEvidence> {
    account_nonce_resource_key_for_node(node, scope, expected_signer_address)
        .map_err(mfm_runtime::RuntimeError::from)?
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "EVM mutation node {} requires an exclusive account nonce resource claim",
                node.node_id
            ))
        })
}

struct ContractMutationSideEffectCallbacks<'a, P> {
    factory: &'a dyn EvmContractRuntimeFactory,
    plan: P,
}

trait ContractMutationPlanOps: Send + Sync {
    type Intent: MfmValue + Clone + Send + Sync + 'static;
    type Output: mfm_values::ContextBoundOutput + 'static;

    fn validate_ingress(
        ctx: &RunnerIngressContext<'_>,
        factory: &dyn EvmContractRuntimeFactory,
    ) -> mfm_runtime::Result<()>;

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        inputs: &'a MaterializedInputs,
        context: &'a mfm_runtime::CertifiedInvocationContext,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self>
    where
        Self: Sized;

    fn action(&self) -> &dyn ContractMutationActionView;

    fn intent(&self) -> &Self::Intent;

    fn idempotency(&self) -> &ContractTransactionIdempotency;

    fn network_id(&self) -> &str;

    fn expected_chain_id(&self) -> u64;

    fn expected_signer_address(&self) -> &str {
        self.action().signer().expected_signer_address_str()
    }

    fn nonce_resource_scope(&self) -> mfm_runtime::Result<ContractNonceResourceScope>;

    fn prepare_invocation<'a>(
        &'a self,
        runtime: &'a EvmContractRuntime,
    ) -> SideEffectDriverFuture<'a, PreparedContractMutation>;

    fn reconstruct_prepared_invocation(
        &self,
        runtime: &EvmContractRuntime,
        prepared: &PreparedContractInvocation,
    ) -> mfm_runtime::Result<PreparedContractMutation>;
}

impl ContractMutationPlanOps for ContextDeployMutationPlan {
    type Intent = ContextContractDeployIntent;
    type Output = DeployedContractInstance;

    fn validate_ingress(
        ctx: &RunnerIngressContext<'_>,
        factory: &dyn EvmContractRuntimeFactory,
    ) -> mfm_runtime::Result<()> {
        validate_context_mutation_runtime_for_node(ctx, ctx.node(), factory)
    }

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        _inputs: &'a MaterializedInputs,
        context: &'a mfm_runtime::CertifiedInvocationContext,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self> {
        Box::pin(
            async move { context_deploy_mutation_plan_for_node(node, context, artifacts).await },
        )
    }

    fn action(&self) -> &dyn ContractMutationActionView {
        self.action.as_ref()
    }

    fn intent(&self) -> &Self::Intent {
        &self.intent
    }

    fn idempotency(&self) -> &ContractTransactionIdempotency {
        &self.idempotency
    }

    fn network_id(&self) -> &str {
        self.context.value().network.network_id.as_str()
    }

    fn expected_chain_id(&self) -> u64 {
        self.context.value().network.expected_chain_id()
    }

    fn nonce_resource_scope(&self) -> mfm_runtime::Result<ContractNonceResourceScope> {
        Ok(ContractNonceResourceScope {
            evm_network_context_ref: evm_network_context_ref(&self.context.value().network)
                .map_err(mfm_runtime::RuntimeError::from)?,
        })
    }

    fn prepare_invocation<'a>(
        &'a self,
        runtime: &'a EvmContractRuntime,
    ) -> SideEffectDriverFuture<'a, PreparedContractMutation> {
        Box::pin(async move {
            runtime
                .adapter()
                .prepare_context_deploy_invocation(
                    &self.action,
                    &self.context,
                    &self.artifact,
                    &self.intent,
                )
                .await
                .map_err(mfm_runtime::RuntimeError::from)
        })
    }

    fn reconstruct_prepared_invocation(
        &self,
        runtime: &EvmContractRuntime,
        prepared: &PreparedContractInvocation,
    ) -> mfm_runtime::Result<PreparedContractMutation> {
        runtime
            .adapter()
            .reconstruct_context_deploy_invocation(
                &self.action,
                &self.context,
                &self.artifact,
                &self.intent,
                prepared,
            )
            .map_err(mfm_runtime::RuntimeError::from)
    }
}

impl ContractMutationPlanOps for ContextConfigureMutationPlan {
    type Intent = ContextContractConfigureIntent;
    type Output = ConfiguredContractInstance;

    fn validate_ingress(
        ctx: &RunnerIngressContext<'_>,
        factory: &dyn EvmContractRuntimeFactory,
    ) -> mfm_runtime::Result<()> {
        validate_context_mutation_runtime_for_node(ctx, ctx.node(), factory)
    }

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        inputs: &'a MaterializedInputs,
        context: &'a mfm_runtime::CertifiedInvocationContext,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self> {
        Box::pin(async move {
            context_configure_mutation_plan_for_inputs(node, inputs, context, artifacts).await
        })
    }

    fn action(&self) -> &dyn ContractMutationActionView {
        self.action.as_ref()
    }

    fn intent(&self) -> &Self::Intent {
        &self.intent
    }

    fn idempotency(&self) -> &ContractTransactionIdempotency {
        &self.idempotency
    }

    fn network_id(&self) -> &str {
        self.context.value().network.network_id.as_str()
    }

    fn expected_chain_id(&self) -> u64 {
        self.context.value().network.expected_chain_id()
    }

    fn nonce_resource_scope(&self) -> mfm_runtime::Result<ContractNonceResourceScope> {
        Ok(ContractNonceResourceScope {
            evm_network_context_ref: evm_network_context_ref(&self.context.value().network)
                .map_err(mfm_runtime::RuntimeError::from)?,
        })
    }

    fn prepare_invocation<'a>(
        &'a self,
        runtime: &'a EvmContractRuntime,
    ) -> SideEffectDriverFuture<'a, PreparedContractMutation> {
        Box::pin(async move {
            runtime
                .adapter()
                .prepare_context_configure_invocation(
                    &self.action,
                    &self.input,
                    &self.context,
                    self.artifact.as_ref(),
                    &self.intent,
                )
                .await
                .map_err(mfm_runtime::RuntimeError::from)
        })
    }

    fn reconstruct_prepared_invocation(
        &self,
        runtime: &EvmContractRuntime,
        prepared: &PreparedContractInvocation,
    ) -> mfm_runtime::Result<PreparedContractMutation> {
        runtime
            .adapter()
            .reconstruct_context_configure_invocation(
                &self.action,
                &self.input,
                &self.context,
                self.artifact.as_ref(),
                &self.intent,
                prepared,
            )
            .map_err(mfm_runtime::RuntimeError::from)
    }
}

impl<P> SideEffectDriverCallbacks for ContractMutationSideEffectCallbacks<'_, P>
where
    P: ContractMutationPlanOps,
{
    type Intent = P::Intent;
    type Idempotency = ContractTransactionIdempotency;
    type PreparedInvocation = PreparedContractInvocation;
    type Submission = ContractTransactionSubmissions;
    type SubmissionUnknownEvidence = ContractTransactionSubmissions;
    type NotSubmittedProof = ContractNotSubmittedProof;
    type AmbiguityEvidence = ContractTransactionSubmissions;

    fn intent_and_idempotency<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
    ) -> SideEffectDriverFuture<'a, SideEffectIntentPlan<Self::Intent, Self::Idempotency>> {
        Box::pin(async move {
            Ok(SideEffectIntentPlan::new(
                self.plan.intent().clone(),
                self.plan.idempotency().clone(),
                idempotency_key_ref(self.plan.idempotency())?,
                evm_transaction_submit_binding()?,
            ))
        })
    }

    fn prepare_invocation<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _plan: &'a SideEffectIntentPlan<Self::Intent, Self::Idempotency>,
    ) -> SideEffectDriverFuture<'a, Option<Self::PreparedInvocation>> {
        Box::pin(async move {
            let binding =
                runtime_evm_network_binding(self.plan.network_id(), self.plan.expected_chain_id())?;
            let runtime = self.factory.runtime_for(binding)?;
            let prepared = self.plan.prepare_invocation(&runtime).await?;
            Ok(Some(prepared.evidence().clone()))
        })
    }

    fn reconstruct_prepared_invocation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation> {
        Box::pin(async move {
            load_prepared_invocation(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                ctx,
                self.factory.artifacts(),
            )
            .await
        })
    }

    fn submit_or_recover_submission<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        action: SideEffectProtocolAction,
        prepared: Option<Self::PreparedInvocation>,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectSubmissionDecision<
            Self::Submission,
            Self::SubmissionUnknownEvidence,
            Self::NotSubmittedProof,
            Self::AmbiguityEvidence,
        >,
    > {
        Box::pin(async move {
            let stored_prepared =
                prepared.ok_or_else(|| missing_side_effect_artifact("prepared invocation"))?;
            let binding =
                runtime_evm_network_binding(self.plan.network_id(), self.plan.expected_chain_id())?;
            let runtime = self.factory.runtime_for(binding)?;
            let prepared = self
                .plan
                .reconstruct_prepared_invocation(&runtime, &stored_prepared)?;
            submit_or_recover_contract_submission(&runtime, &prepared, action)
                .await
                .map_err(mfm_runtime::RuntimeError::from)
        })
    }
}

struct ContractVerifyCallbacks<'a, P> {
    factory: &'a dyn EvmContractRuntimeFactory,
    _phase: PhantomData<P>,
}

impl<P> ContractVerifyCallbacks<'_, P>
where
    P: ContractVerifyPhase,
{
    async fn load_plan(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
    ) -> mfm_runtime::Result<P> {
        let submit_context = ctx.invocation_context_for_node(submit_node)?;
        P::load_plan(
            submit_node,
            submit_inputs,
            &submit_context,
            self.factory.artifacts(),
        )
        .await
    }

    async fn load_plan_and_runtime(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
    ) -> mfm_runtime::Result<(P, EvmContractRuntime)> {
        let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
        let binding = runtime_evm_network_binding(plan.network_id(), plan.expected_chain_id())?;
        let runtime = self.factory.runtime_for(binding)?;
        Ok((plan, runtime))
    }

    async fn load_prepared_invocation(
        &self,
        submit_node: &spec::NodeSpec,
        projection: &store::SideEffectArtifactProjection,
    ) -> mfm_runtime::Result<PreparedContractInvocation> {
        load_prepared_invocation_for_node(
            projection,
            events::ArtifactRole::PreparedInvocation,
            self.factory.artifacts(),
            &submit_node.node_id,
        )
        .await
    }

    async fn load_reconstructed_prepared_invocation(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
        projection: &store::SideEffectArtifactProjection,
    ) -> mfm_runtime::Result<(P, EvmContractRuntime, PreparedContractMutation)> {
        let stored_prepared = self
            .load_prepared_invocation(submit_node, projection)
            .await?;
        let (plan, runtime) = self
            .load_plan_and_runtime(ctx, submit_node, submit_inputs)
            .await?;
        let prepared = plan.reconstruct_prepared_invocation(&runtime, &stored_prepared)?;
        Ok((plan, runtime, prepared))
    }
}

impl<P> SideEffectVerifyCallbacks for ContractVerifyCallbacks<'_, P>
where
    P: ContractVerifyPhase,
{
    type Submission = ContractTransactionSubmissions;
    type Receipt = P::Receipt;
    type Confirmation = P::Confirmation;
    type Output = <P as ContractMutationPlanOps>::Output;
    type NotSubmittedProof = ContractNotSubmittedProof;
    type AmbiguityEvidence = ContractTransactionSubmissions;

    fn recover_unknown_submission<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        prepared_invocation: Option<&'a store::SideEffectArtifactProjection>,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectUnknownSubmissionDecision<
            Self::Submission,
            Self::NotSubmittedProof,
            Self::AmbiguityEvidence,
        >,
    > {
        Box::pin(async move {
            let prepared_projection = prepared_invocation
                .ok_or_else(|| missing_side_effect_artifact("prepared invocation"))?;
            let (_plan, runtime, prepared) = self
                .load_reconstructed_prepared_invocation(
                    ctx,
                    submit_node,
                    submit_inputs,
                    prepared_projection,
                )
                .await?;
            recover_unknown_prepared_contract_submission(&runtime, &prepared).await
        })
    }

    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        Box::pin(async {
            let prepared_projection = projected_prepared_artifact_for_submit(ctx, submit_node)?;
            let (plan, runtime, prepared) = self
                .load_reconstructed_prepared_invocation(
                    ctx,
                    submit_node,
                    submit_inputs,
                    &prepared_projection,
                )
                .await?;
            let (submissions, _) =
                load_side_effect_artifact_for_node::<ContractTransactionSubmissions>(
                    submission,
                    events::ArtifactRole::Submission,
                    &submit_node.node_id,
                    self.factory.artifacts(),
                )
                .await?;
            let receipts =
                read_receipts_with_poll(&runtime, prepared.evidence(), &submissions).await?;
            Ok(SideEffectObservedEvidence::new(
                plan.receipt_from_observed(prepared.evidence(), receipts)?,
                contract_side_effect_replay_evidence()?,
            ))
        })
    }

    fn build_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        Box::pin(async {
            let (_plan, runtime) = self
                .load_plan_and_runtime(ctx, submit_node, submit_inputs)
                .await?;
            let required_depth = finalized_depth_for_submit_node(submit_node)?;
            let (receipt, receipt_evidence) = load_side_effect_artifact_for_node::<P::Receipt>(
                receipt,
                events::ArtifactRole::Receipt,
                &ctx.node().node_id,
                self.factory.artifacts(),
            )
            .await?;
            let receipt_evidence = CapabilityArtifactEvidenceRef::from(receipt_evidence);
            let receipt = P::receipt_with_evidence(receipt, &receipt_evidence);
            let confirmations = verified_finality_confirmations(
                &runtime,
                P::receipt_transactions(&receipt),
                required_depth,
            )
            .await?;
            Ok(SideEffectObservedEvidence::new(
                P::confirmation_from_receipt(receipt, confirmations),
                contract_side_effect_replay_evidence()?,
            ))
        })
    }

    fn map_receipt_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async {
            let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
            let (receipt, _) = load_side_effect_artifact_for_node::<P::Receipt>(
                receipt,
                events::ArtifactRole::Receipt,
                &ctx.node().node_id,
                self.factory.artifacts(),
            )
            .await?;
            plan.output_from_receipt(&receipt)
        })
    }

    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async {
            let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
            let (confirmation, _) = load_side_effect_artifact_for_node::<P::Confirmation>(
                confirmation,
                events::ArtifactRole::Confirmation,
                &ctx.node().node_id,
                self.factory.artifacts(),
            )
            .await?;
            plan.output_from_confirmation(&confirmation)
        })
    }
}

trait ContractVerifyPhase: ContractMutationPlanOps + Sized + Send + Sync + 'static {
    type Receipt: MfmValue + Send + Sync + 'static;
    type Confirmation: MfmValue + Send + Sync + 'static;

    fn receipt_from_observed(
        &self,
        prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt>;

    fn receipt_with_evidence(
        receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt;

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt];

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation;

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output>;

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output>;
}

impl ContractVerifyPhase for ContextDeployMutationPlan {
    type Receipt = ContractDeployReceipt;
    type Confirmation = ContractDeployConfirmation;

    fn receipt_from_observed(
        &self,
        prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt> {
        Ok(ContractDeployReceipt {
            receipt_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: prepared.resource_stage,
            contract_address: deploy_contract_address_from_prepared(prepared)?,
            receipt: single_receipt(receipts)?,
        })
    }

    fn receipt_with_evidence(
        receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt {
        deploy_receipt_with_evidence(receipt, evidence)
    }

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt] {
        std::slice::from_ref(&receipt.receipt)
    }

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation {
        ContractDeployConfirmation {
            confirmation_version: 1,
            context_ref: receipt.context_ref,
            evm_network_context_ref: receipt.evm_network_context_ref,
            resource_stage: receipt.resource_stage,
            confirmations,
            contract_address: receipt.contract_address,
            receipt: receipt.receipt,
        }
    }

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_receipt(&(), &self.intent, receipt, &self.context)
            .map_err(runtime_state_error)
    }

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_confirmation(&(), &self.intent, confirmation, &self.context)
            .map_err(runtime_state_error)
    }
}

impl ContractVerifyPhase for ContextConfigureMutationPlan {
    type Receipt = ContextContractConfigureReceipt;
    type Confirmation = ContextContractConfigureConfirmation;

    fn receipt_from_observed(
        &self,
        _prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt> {
        Ok(ContextContractConfigureReceipt {
            receipt_version: 1,
            context_ref: _prepared.context_ref.clone(),
            evm_network_context_ref: _prepared.evm_network_context_ref.clone(),
            resource_stage: _prepared.resource_stage,
            configure_node: LifecycleNodeIdRef::from(self.node_id.clone()),
            configured_block_number: receipts.iter().map(|receipt| receipt.block_number).max(),
            receipts,
            call_evidence_refs: Vec::new(),
            confirmation_evidence_refs: Vec::new(),
        })
    }

    fn receipt_with_evidence(
        mut receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt {
        let evidence = lifecycle_evidence_ref(evidence);
        for transaction_receipt in &mut receipt.receipts {
            transaction_receipt.receipt_evidence = Some(evidence.clone());
        }
        receipt
    }

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt] {
        &receipt.receipts
    }

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation {
        ContextContractConfigureConfirmation {
            confirmation_version: 1,
            context_ref: receipt.context_ref,
            evm_network_context_ref: receipt.evm_network_context_ref,
            resource_stage: receipt.resource_stage,
            confirmations,
            configure_node: receipt.configure_node,
            receipts: receipt.receipts,
            call_evidence_refs: receipt.call_evidence_refs,
            confirmation_evidence_refs: receipt.confirmation_evidence_refs,
            configured_block_number: receipt.configured_block_number,
        }
    }

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_receipt(&self.input, &self.intent, receipt, &self.context)
            .map_err(runtime_state_error)
    }

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_confirmation(&self.input, &self.intent, confirmation, &self.context)
            .map_err(runtime_state_error)
    }
}

async fn context_deploy_mutation_plan_for_node(
    node: &spec::NodeSpec,
    invocation_context: &mfm_runtime::CertifiedInvocationContext,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ContextDeployMutationPlan> {
    let action = load_runner_config_for_node::<DeployAction>(node, artifacts).await?;
    let context = invocation_context.certified_context::<EvmContractContext>()?;
    let artifact = load_context_profile_artifact(&context, artifacts).await?;
    let state = ContextBoundDeployContractState::new(action.clone()).map_err(runtime_plan_error)?;
    let intent = state
        .prepare_intent(&(), &context)
        .map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&(), &intent, &context)
        .map_err(runtime_state_error)?;
    Ok(ContextDeployMutationPlan {
        action,
        state,
        context,
        artifact,
        intent,
        idempotency,
    })
}

async fn context_configure_mutation_plan_for_inputs(
    node: &spec::NodeSpec,
    inputs: &mfm_runtime::MaterializedInputs,
    invocation_context: &mfm_runtime::CertifiedInvocationContext,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ContextConfigureMutationPlan> {
    let action = load_runner_config_for_node::<ConfigureAction>(node, artifacts).await?;
    let deployed = load_materialized_struct_field_value::<DeployedContractInstance>(
        inputs, "deployed", artifacts,
    )
    .await?;
    let input = ContextConfigureContractInput { deployed };
    let context = invocation_context.certified_context::<EvmContractContext>()?;
    ensure_deployed_input_context(&input, &context).map_err(mfm_runtime::RuntimeError::from)?;
    let artifact = if configure_action_requires_artifact(action.as_ref()) {
        Some(load_context_profile_artifact(&context, artifacts).await?)
    } else {
        None
    };
    let state =
        ContextBoundConfigureContractState::new(action.clone()).map_err(runtime_plan_error)?;
    let intent = state
        .prepare_intent(&input, &context)
        .map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&input, &intent, &context)
        .map_err(runtime_state_error)?;
    Ok(ContextConfigureMutationPlan {
        node_id: node.node_id.clone(),
        action,
        state,
        input,
        context,
        artifact,
        intent,
        idempotency,
    })
}

async fn run_context_validate(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let action =
        load_runner_config_for_node::<ValidateAction>(ctx.node(), factory.artifacts()).await?;
    let input = load_context_validate_input(ctx.inputs(), factory.artifacts()).await?;
    let context = ctx.certified_context::<EvmContractContext>()?;
    ensure_configured_input_context(&input, &context).map_err(mfm_runtime::RuntimeError::from)?;
    let state = ContextBoundValidateContractState::new(action.clone())
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let request = state
        .read_request(&input, &context)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let artifact = if validation_assertions_required(
        action.as_ref().read_assertions(),
        action.as_ref().event_assertions(),
    ) {
        Some(load_context_profile_artifact(&context, factory.artifacts()).await?)
    } else {
        None
    };
    let runtime = factory.read_runtime_for(runtime_evm_network_binding_for_context(&context)?)?;
    let response = runtime
        .validate_context_contract(&action, &input, &context, artifact.as_ref(), &request)
        .await?;
    let report = state
        .report_from_response(&input, response, &context)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    ErasedRunnerOutput::state_output(&ctx, &report)
}

async fn run_import_deployed(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let import =
        load_runner_config_for_node::<ImportDeployedSpec>(ctx.node(), factory.artifacts()).await?;
    let context = ctx.certified_context::<EvmContractContext>()?;
    let runtime = factory.read_runtime_for(runtime_evm_network_binding_for_context(&context)?)?;
    let deployed = runtime
        .import_deployed(import.as_ref(), &context, factory.artifacts())
        .await?;
    ErasedRunnerOutput::state_output(&ctx, &deployed)
}

async fn run_import_configured(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let import =
        load_runner_config_for_node::<ImportConfiguredSpec>(ctx.node(), factory.artifacts())
            .await?;
    let context = ctx.certified_context::<EvmContractContext>()?;
    let artifact = if import_configured_requires_artifact(import.as_ref()) {
        Some(load_context_profile_artifact(&context, factory.artifacts()).await?)
    } else {
        None
    };
    let runtime = factory.read_runtime_for(runtime_evm_network_binding_for_context(&context)?)?;
    let configured = runtime
        .import_configured(
            import.as_ref(),
            &context,
            artifact.as_ref(),
            factory.artifacts(),
        )
        .await?;
    ErasedRunnerOutput::state_output(&ctx, &configured)
}

pub(crate) async fn read_receipts_with_poll(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractInvocation,
    submissions: &ContractTransactionSubmissions,
) -> mfm_runtime::Result<Vec<mfm_state_evm_contracts::ContractTransactionReceipt>> {
    verify_prepared_submissions(prepared, submissions).map_err(mfm_runtime::RuntimeError::from)?;
    let receipt_policy =
        ReceiptRetryPolicy::new(prepared.poll_interval_ms, prepared.max_receipt_polls)
            .map_err(mfm_runtime::RuntimeError::InvalidRunnerOutput)?;

    let max_polls = receipt_policy.max_receipt_polls();
    for attempt in 0..max_polls {
        match runtime
            .adapter()
            .read_receipts(prepared, &submissions.transactions)
            .await
        {
            Ok(receipts) => return Ok(receipts),
            Err(EvmContractAdapterError::EvmCapability(EvmCapabilityError::ReceiptPending)) => {
                if attempt + 1 == max_polls {
                    return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                        "contract lifecycle receipt polling exhausted".to_owned(),
                    ));
                }
                sleep(Duration::from_millis(receipt_policy.poll_interval_ms())).await;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
        "contract lifecycle receipt polling exhausted".to_owned(),
    ))
}

fn finalized_depth_for_submit_node(submit_node: &spec::NodeSpec) -> mfm_runtime::Result<u64> {
    match submit_node
        .side_effect
        .as_ref()
        .map(|contract| &contract.verification)
    {
        Some(spec::SideEffectVerificationSpec::Finalized { depth }) => Ok(*depth),
        Some(spec::SideEffectVerificationSpec::Receipt) => {
            Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "contract lifecycle submit node {} has receipt-only verification",
                submit_node.node_id
            )))
        }
        None => Err(mfm_runtime::RuntimeError::InvalidSpec(format!(
            "contract lifecycle submit node {} lacks side-effect contract",
            submit_node.node_id
        ))),
    }
}

pub(crate) async fn verified_finality_confirmations(
    runtime: &EvmContractRuntime,
    receipts: &[ContractTransactionReceipt],
    required_depth: u64,
) -> mfm_runtime::Result<u64> {
    let Some(highest_receipt_block) = receipts.iter().map(|receipt| receipt.block_number).max()
    else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract lifecycle finality requires at least one receipt".to_owned(),
        ));
    };
    let latest_block = runtime
        .adapter()
        .latest_block_number()
        .await
        .map_err(mfm_runtime::RuntimeError::from)?;
    let confirmations = latest_block
        .checked_sub(highest_receipt_block)
        .map(|distance| distance.saturating_add(1))
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::Blocked(format!(
                "contract lifecycle finality latest block {latest_block} is behind receipt block {highest_receipt_block}"
            ))
        })?;
    if confirmations < required_depth {
        return Err(mfm_runtime::RuntimeError::Blocked(format!(
            "contract lifecycle finality depth {required_depth} not reached; observed {confirmations}"
        )));
    }
    Ok(confirmations)
}

async fn load_context_validate_input(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ContextValidateContractInput> {
    let configured = load_materialized_struct_field_value::<ConfiguredContractInstance>(
        inputs,
        "configured",
        artifacts,
    )
    .await?;
    Ok(ContextValidateContractInput { configured })
}

async fn load_context_profile_artifact(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ContractArtifactConfig> {
    let reference = context
        .value()
        .contract_profile
        .artifact_ref
        .as_ref()
        .ok_or(EvmContractAdapterError::MissingContractArtifact)
        .map_err(mfm_runtime::RuntimeError::from)?;
    let requirement = contract_profile_artifact_requirement(reference)
        .map_err(mfm_runtime::RuntimeError::from)?;
    let verified = artifacts
        .read_retained_artifact(&requirement)
        .await
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    serde_json::from_slice::<ContractArtifactConfig>(verified.bytes())
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

pub(crate) fn contract_profile_artifact_requirement(
    reference: &LifecycleArtifactEvidenceRef,
) -> Result<store::EventArtifactRequirement> {
    let schema_id = reference
        .schema_id()
        .map_err(EvmContractAdapterError::Model)?
        .ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let expected_schema = <ContractArtifactConfig as MfmConfig>::schema_id()
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    if schema_id != expected_schema
        || reference
            .semantic_type_id()
            .map_err(EvmContractAdapterError::Model)?
            .is_some()
    {
        return Err(EvmContractAdapterError::MissingContractArtifact);
    }
    let artifact_id = reference
        .artifact_id()
        .map_err(EvmContractAdapterError::Model)?;
    let digest = reference
        .content_digest()
        .map_err(EvmContractAdapterError::Model)?;
    let evidence_hash = reference
        .evidence_hash()
        .map_err(EvmContractAdapterError::Model)?;
    let media_type = spec::MediaType::new("application/json")
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    let store_evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: digest.clone(),
        byte_len: reference.byte_len(),
        media_type: media_type.clone(),
        schema_id: Some(schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let expected_evidence_hash = store_evidence
        .evidence_hash()
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    if evidence_hash != expected_evidence_hash {
        return Err(EvmContractAdapterError::MissingContractArtifact);
    }
    Ok(store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id,
        evidence_hash,
        digest: Some(digest),
        byte_len: Some(reference.byte_len()),
        media_type: Some(media_type),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    })
}

async fn load_prepared_invocation(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<PreparedContractInvocation> {
    load_prepared_invocation_for_node(artifact, role, artifacts, &ctx.node().node_id).await
}

async fn load_prepared_invocation_for_node(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    producer_node_id: &NodeId,
) -> mfm_runtime::Result<PreparedContractInvocation> {
    let (prepared, _) = load_side_effect_artifact_for_node::<PreparedContractInvocation>(
        artifact,
        role,
        producer_node_id,
        artifacts,
    )
    .await?;
    ensure_prepared_invocation_public(&prepared)?;
    Ok(prepared)
}

fn evm_transaction_submit_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    let binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    RunnerCapabilityBinding::for_capability::<EvmTransactionSubmitCapability>(
        binding.adapter_kind().clone(),
        binding.adapter_version().clone(),
    )
}

fn projected_side_effect_for_submit<'a>(
    ctx: &'a ErasedRunCtx<'_>,
    submit_node: &spec::NodeSpec,
) -> mfm_runtime::Result<&'a store::SideEffectProjection> {
    let contract = submit_node.side_effect.as_ref().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidSpec(format!(
            "contract lifecycle submit node {} lacks side-effect contract",
            submit_node.node_id
        ))
    })?;
    let pair_id =
        spec::side_effect_pair_id(&submit_node.node_id, &submit_node.output_cell, contract)
            .map_err(|error| {
                mfm_runtime::RuntimeError::InvalidSpec(format!(
                    "contract lifecycle submit node {} pair id is invalid: {error}",
                    submit_node.node_id
                ))
            })?;
    ctx.projections()
        .side_effect_for_pair(ctx.run_id(), &pair_id)
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "contract lifecycle side-effect projection missing for submit node {}",
                submit_node.node_id
            ))
        })
}

fn projected_prepared_artifact_for_submit(
    ctx: &ErasedRunCtx<'_>,
    submit_node: &spec::NodeSpec,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect_for_submit(ctx, submit_node)?
        .prepared_invocation
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("prepared invocation"))
}

fn missing_side_effect_artifact(label: &str) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "contract lifecycle side-effect {label} artifact is missing"
    ))
}

fn single_receipt(
    mut receipts: Vec<ContractTransactionReceipt>,
) -> mfm_runtime::Result<ContractTransactionReceipt> {
    if receipts.len() != 1 {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract deploy confirmation requires exactly one receipt".to_owned(),
        ));
    }
    Ok(receipts.remove(0))
}

fn deploy_receipt_with_evidence(
    mut receipt: ContractDeployReceipt,
    evidence: &mfm_artifact_capabilities::ArtifactEvidenceRef,
) -> ContractDeployReceipt {
    let evidence = lifecycle_evidence_ref(evidence);
    receipt.receipt.receipt_evidence = Some(evidence);
    receipt
}

fn idempotency_key_ref(
    idempotency: &ContractTransactionIdempotency,
) -> mfm_runtime::Result<events::IdempotencyKeyRef> {
    Ok(events::IdempotencyKeyRef::new(format!(
        "mfm.evm.contract.idem.{}",
        short_stable_id_fragment(&idempotency.key, 32)
    ))?)
}

fn runtime_evm_network_binding(
    network_id: &str,
    expected_chain_id: u64,
) -> mfm_runtime::Result<EvmNetworkBinding> {
    let network_id = EvmNetworkId::new(network_id).map_err(EvmContractAdapterError::from)?;
    EvmNetworkBinding::new(network_id, expected_chain_id)
        .map_err(EvmContractAdapterError::from)
        .map_err(Into::into)
}

fn runtime_evm_network_binding_for_context(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> mfm_runtime::Result<EvmNetworkBinding> {
    runtime_evm_network_binding(
        context.value().network.network_id.as_str(),
        context.value().network.expected_chain_id(),
    )
}

fn runtime_plan_error(error: mfm_program::PlanError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_state_error(error: mfm_program::StateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_ingress_model_error(error: String) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error)
}
