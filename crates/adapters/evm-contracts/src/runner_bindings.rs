use super::*;

#[path = "runner_bindings/registration.rs"]
mod registration;
pub use self::registration::register_contract_lifecycle_runners_with_factory;

#[path = "runner_bindings/verify.rs"]
mod verify;

#[path = "runner_bindings/mutation.rs"]
mod mutation;
use self::mutation::{
    ContextConfigureMutationPlan, ContextDeployMutationPlan, ContractMutationPlanOps,
    ContractMutationRunner,
};

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
        Box::pin(async move { verify::run_context_verify(ctx, self.factory.as_ref()).await })
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
