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
        register_contract_capabilities(registry, descriptor.capabilities(), &implementation_id)?;
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

fn register_contract_capabilities(
    registry: &mut ErasedRunnerRegistry,
    capabilities: &mfm_capabilities::CapabilitySetDescriptor,
    contract_implementation_id: &CapabilityImplementationId,
) -> mfm_runtime::Result<()> {
    let generic_call = EvmCallReadCapability::descriptor()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let generic_call_implementation =
        CapabilityImplementationId::new(EVM_JSONRPC_CAPABILITY_IMPLEMENTATION_ID)?;
    for capability in &capabilities.capabilities {
        // Generic call reads are shared with hash-anchored collectors, so a capability descriptor
        // must retain one provider binding even when the consuming adapter differs.
        let implementation_id =
            if capability.kind == generic_call.kind && capability.version == generic_call.version {
                generic_call_implementation.clone()
            } else {
                contract_implementation_id.clone()
            };
        registry.register_capability(CapabilityImplementationBinding::new(
            capability.clone(),
            implementation_id,
        ))?;
    }
    Ok(())
}
