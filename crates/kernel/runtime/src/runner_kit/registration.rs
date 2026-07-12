use super::*;

/// Builder for registering runner bindings while keeping executable identity explicit.
pub struct RunnerRegistrationBuilder<'a> {
    registry: &'a mut ErasedRunnerRegistry,
}

impl<'a> RunnerRegistrationBuilder<'a> {
    /// Creates a runner registration builder.
    pub fn new(registry: &'a mut ErasedRunnerRegistry) -> Self {
        Self { registry }
    }

    /// Registers one descriptor runner using caller-supplied factory and executable identity.
    pub fn register_runner(
        &mut self,
        descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        let binding = ErasedRunnerBinding::new(descriptor_id, factory_id, executable, runner)?;
        self.registry.register(binding)?;
        Ok(self)
    }

    /// Registers a typed state runner when its capabilities are bound by process assembly.
    pub fn register_state_runner_with_factory<S>(
        &mut self,
        factory: &RunnerFactoryBinding,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<mfm_program::StateDescriptorIdentity>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = mfm_program::state_descriptor::<S>()
            .map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?;
        self.register_runner(
            descriptor.descriptor_id().clone(),
            factory.factory_id(),
            factory.executable(),
            runner,
        )?;
        Ok(descriptor)
    }

    /// Registers the adapter-owned runner used by side-effect verify framework nodes
    /// for one certified side-effect submit descriptor.
    pub fn register_side_effect_verify_runner(
        &mut self,
        submit_descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        self.registry.register_side_effect_verify_runner(
            submit_descriptor_id,
            factory_id,
            executable,
            runner,
        )?;
        Ok(self)
    }

    /// Registers an adapter-owned side-effect verify runner using a bound factory.
    pub fn register_side_effect_verify_runner_with_factory(
        &mut self,
        submit_descriptor_id: DescriptorId,
        factory: &RunnerFactoryBinding,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<&mut Self> {
        self.register_side_effect_verify_runner(
            submit_descriptor_id,
            factory.factory_id(),
            factory.executable(),
            runner,
        )
    }

    /// Registers executable evidence for one certified adapter binding.
    pub fn register_adapter_executable(
        &mut self,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        executable: events::ExecutableIdentity,
    ) -> Result<&mut Self> {
        self.registry
            .register_adapter_executable(AdapterExecutableBinding::new(
                adapter_kind,
                adapter_version,
                executable,
            ))?;
        Ok(self)
    }

    /// Registers executable evidence for one certified adapter binding using a bound factory.
    pub fn register_adapter_executable_with_factory(
        &mut self,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        factory: &RunnerFactoryBinding,
    ) -> Result<&mut Self> {
        self.register_adapter_executable(adapter_kind, adapter_version, factory.executable())
    }
}
