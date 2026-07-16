use super::*;

pub(super) struct ContractMutationRunner<P: ContractMutationPlanOps> {
    pub(super) factory: Arc<dyn EvmContractRuntimeFactory>,
    pub(super) extractor: TypedContextOutputExtractor<<P as ContractMutationPlanOps>::Output>,
    pub(super) _phase: PhantomData<P>,
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

pub(super) struct ContextDeployMutationPlan {
    pub(super) action: ValidatedConfig<DeployAction>,
    pub(super) state: ContextBoundDeployContractState,
    pub(super) context: mfm_program::CertifiedContext<EvmContractContext>,
    pub(super) artifact: ContractArtifactConfig,
    pub(super) intent: ContextContractDeployIntent,
    pub(super) idempotency: ContractTransactionIdempotency,
}

pub(super) struct ContextConfigureMutationPlan {
    pub(super) action: ValidatedConfig<ConfigureAction>,
    pub(super) state: ContextBoundConfigureContractState,
    pub(super) input: ContextConfigureContractInput,
    pub(super) context: mfm_program::CertifiedContext<EvmContractContext>,
    pub(super) artifact: Option<ContractArtifactConfig>,
    pub(super) intent: ContextContractConfigureIntent,
    pub(super) idempotency: ContractTransactionIdempotency,
}

struct ContractMutationSideEffectCallbacks<'a, P> {
    factory: &'a dyn EvmContractRuntimeFactory,
    plan: P,
}

pub(super) trait ContractMutationPlanOps: Send + Sync {
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
        action,
        state,
        input,
        context,
        artifact,
        intent,
        idempotency,
    })
}
