use super::*;

/// Registers context-bound contract state runners with the supplied runtime factory.
pub fn register_contract_state_runners_with_factory(
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
    for descriptor in [
        &deploy_descriptor,
        &configure_descriptor,
        &validate_descriptor,
    ] {
        register_contract_capabilities(registry, descriptor.capabilities(), &implementation_id)?;
    }
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm-contracts",
        "evm-contract-states-context",
        env!("CARGO_PKG_VERSION"),
    )?;
    let side_effect_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(SIDE_EFFECT_FACTORY)?);
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    let adapter_kind = mfm_state_evm_contracts::contract_states_adapter_kind()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let adapter_version = mfm_state_evm_contracts::contract_states_adapter_version()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_adapter_executable_with_factory(
        adapter_kind,
        adapter_version,
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
    registrations.register_side_effect_verify_runner_with_factory(
        deploy.descriptor_id().clone(),
        &read_factory,
        Arc::new(ContextContractVerifyRunner {
            factory: factory.clone(),
            extractor: ContractStateContextOutputExtractor::new(),
        }),
    )?;
    registrations.register_side_effect_verify_runner_with_factory(
        configure.descriptor_id().clone(),
        &read_factory,
        Arc::new(ContextContractVerifyRunner {
            factory,
            extractor: ContractStateContextOutputExtractor::new(),
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
