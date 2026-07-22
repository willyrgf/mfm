use super::*;

/// Immutable semantic universe declared by one published typed program.
///
/// Catalogs contain only authoring-time domain and framework contracts. Concrete runtime
/// implementations, configured routes, and executable identities are deliberately excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgramAuthoringCatalog {
    operations: BTreeMap<DescriptorId, spec::OperationDescriptorIdentity>,
    states: BTreeMap<DescriptorId, spec::StateDescriptorIdentity>,
    capabilities: BTreeMap<
        (mfm_ids::CapabilityKind, mfm_ids::CapabilityVersion),
        mfm_capabilities::CapabilityDescriptor,
    >,
    facts: BTreeMap<ContentDigest, program::FactDescriptorRef>,
    adapters:
        BTreeMap<(mfm_ids::AdapterKind, mfm_ids::AdapterVersion), program::AdapterBindingSpec>,
    side_effect_states: BTreeSet<DescriptorId>,
    framework_states: BTreeSet<DescriptorId>,
}

impl ProgramAuthoringCatalog {
    /// Returns the declared operation descriptors in descriptor-id order.
    pub fn operation_descriptors(
        &self,
    ) -> impl ExactSizeIterator<Item = &spec::OperationDescriptorIdentity> {
        self.operations.values()
    }

    /// Returns the declared state descriptors in descriptor-id order.
    pub fn state_descriptors(
        &self,
    ) -> impl ExactSizeIterator<Item = &spec::StateDescriptorIdentity> {
        self.states.values()
    }

    /// Returns the capability descriptors derived from declared states.
    pub fn capability_descriptors(
        &self,
    ) -> impl ExactSizeIterator<Item = &mfm_capabilities::CapabilityDescriptor> {
        self.capabilities.values()
    }

    /// Returns emitted fact descriptor references derived from declared states.
    pub fn emitted_fact_descriptors(
        &self,
    ) -> impl ExactSizeIterator<Item = &program::FactDescriptorRef> {
        self.facts.values()
    }

    /// Returns adapter binding contracts derived from declared states.
    pub fn adapter_bindings(&self) -> impl ExactSizeIterator<Item = &program::AdapterBindingSpec> {
        self.adapters.values()
    }

    /// Returns descriptor ids for declared side-effect states.
    pub fn side_effect_state_descriptor_ids(&self) -> impl ExactSizeIterator<Item = &DescriptorId> {
        self.side_effect_states.iter()
    }

    /// Returns whether a state descriptor belongs to the sealed framework bootstrap.
    pub fn is_framework_state(&self, descriptor_id: &DescriptorId) -> bool {
        self.framework_states.contains(descriptor_id)
    }

    /// Unions two independently declared catalogs, rejecting conflicting semantic keys.
    pub fn union(&self, other: &Self) -> Result<Self> {
        let mut united = self.clone();
        united.__include(other.clone())?;
        Ok(united)
    }

    /// Creates an empty catalog for macro-generated declaration code.
    #[doc(hidden)]
    pub fn __new() -> Self {
        Self::default()
    }

    /// Includes a generated child catalog with conflict rejection.
    #[doc(hidden)]
    pub fn __include(&mut self, child: Self) -> Result<()> {
        let Self {
            operations,
            states,
            capabilities,
            facts,
            adapters,
            side_effect_states,
            framework_states,
        } = child;
        for descriptor in operations.into_values() {
            self.insert_operation(descriptor)?;
        }
        for descriptor in states.into_values() {
            let framework = framework_states.contains(&descriptor.descriptor_id);
            self.insert_state(descriptor, framework)?;
        }
        for descriptor in capabilities.into_values() {
            self.insert_capability(descriptor)?;
        }
        for descriptor in facts.into_values() {
            self.insert_fact(descriptor)?;
        }
        for binding in adapters.into_values() {
            self.insert_adapter(binding)?;
        }
        self.side_effect_states.extend(side_effect_states);
        Ok(())
    }

    /// Adds one typed state and all semantic contracts derived from it.
    #[doc(hidden)]
    pub fn __register_state<S>(&mut self) -> Result<()>
    where
        S: program::StateSpec,
        S::Effect: program::EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = program::state_descriptor::<S>()?;
        for capability in &descriptor.capabilities().capabilities {
            self.insert_capability(capability.clone())?;
        }
        for fact in descriptor.emitted_fact_descriptors() {
            self.insert_fact(fact.clone())?;
        }
        for adapter in
            S::adapter_bindings().map_err(|error| CertifyError::Lowering(error.to_string()))?
        {
            self.insert_adapter(adapter)?;
        }
        if descriptor.runner() == program::RunnerKind::ApplySideEffect {
            self.side_effect_states
                .insert(descriptor.descriptor_id().clone());
        }
        self.insert_state(
            state_descriptor_identity_from_descriptor(&descriptor)?,
            false,
        )
    }

    /// Adds one typed operation descriptor.
    #[doc(hidden)]
    pub fn __register_operation<O>(&mut self) -> Result<()>
    where
        O: program::Operation,
    {
        let descriptor = program::operation_descriptor::<O>()?;
        self.insert_operation(operation_descriptor_identity_from_descriptor(&descriptor))
    }

    /// Adds one descriptor from the runtime's closed framework bootstrap.
    #[doc(hidden)]
    pub fn __register_framework_state_descriptor(
        &mut self,
        descriptor: spec::StateDescriptorIdentity,
    ) -> Result<()> {
        for capability in &descriptor.capabilities.capabilities {
            self.insert_capability(capability.clone())?;
        }
        for fact in &descriptor.emitted_fact_descriptors {
            self.insert_fact(fact.clone())?;
        }
        self.insert_state(descriptor, true)
    }

    fn insert_operation(&mut self, descriptor: spec::OperationDescriptorIdentity) -> Result<()> {
        if let Some(existing) = self.operations.values().find(|existing| {
            existing.operation_kind == descriptor.operation_kind
                && existing.operation_version == descriptor.operation_version
                && existing.descriptor_id != descriptor.descriptor_id
        }) {
            return Err(catalog_conflict(
                "operation kind/version",
                format!(
                    "{}@{} conflicts with {}",
                    descriptor.operation_kind, descriptor.operation_version, existing.descriptor_id
                ),
            ));
        }
        let key = descriptor.descriptor_id.clone();
        insert_exact(
            &mut self.operations,
            key,
            descriptor,
            "operation descriptor",
        )
    }

    fn insert_state(
        &mut self,
        descriptor: spec::StateDescriptorIdentity,
        framework: bool,
    ) -> Result<()> {
        if let Some(existing) = self.states.values().find(|existing| {
            existing.state_kind == descriptor.state_kind
                && existing.state_version == descriptor.state_version
                && existing.descriptor_id != descriptor.descriptor_id
        }) {
            return Err(catalog_conflict(
                "state kind/version",
                format!(
                    "{}@{} conflicts with {}",
                    descriptor.state_kind, descriptor.state_version, existing.descriptor_id
                ),
            ));
        }
        let key = descriptor.descriptor_id.clone();
        if self.states.contains_key(&key) && self.framework_states.contains(&key) != framework {
            return Err(catalog_conflict("state ownership", key.as_str()));
        }
        insert_exact(
            &mut self.states,
            key.clone(),
            descriptor,
            "state descriptor",
        )?;
        if framework {
            self.framework_states.insert(key);
        }
        Ok(())
    }

    fn insert_capability(
        &mut self,
        descriptor: mfm_capabilities::CapabilityDescriptor,
    ) -> Result<()> {
        let key = (descriptor.kind.clone(), descriptor.version.clone());
        insert_exact(
            &mut self.capabilities,
            key,
            descriptor,
            "capability descriptor",
        )
    }

    fn insert_fact(&mut self, descriptor: program::FactDescriptorRef) -> Result<()> {
        let key = descriptor.descriptor_hash.clone();
        insert_exact(&mut self.facts, key, descriptor, "fact descriptor")
    }

    fn insert_adapter(&mut self, binding: program::AdapterBindingSpec) -> Result<()> {
        let key = (
            binding.adapter_kind.clone(),
            binding.adapter_version.clone(),
        );
        insert_exact(&mut self.adapters, key, binding, "adapter binding")
    }
}

fn insert_exact<K, V>(
    values: &mut BTreeMap<K, V>,
    key: K,
    value: V,
    surface: &'static str,
) -> Result<()>
where
    K: Ord + fmt::Debug,
    V: PartialEq,
{
    match values.get(&key) {
        Some(existing) if existing == &value => Ok(()),
        Some(_) => Err(catalog_conflict(surface, format!("{key:?}"))),
        None => {
            values.insert(key, value);
            Ok(())
        }
    }
}

fn catalog_conflict(surface: &'static str, key: impl fmt::Display) -> CertifyError {
    problem(
        ProblemClass::InvalidSemanticTransition,
        format!("conflicting {surface} in authoring catalog for {key}"),
    )
}

/// Builds the framework-only catalog used by the runtime's sealed production bootstrap.
#[doc(hidden)]
pub fn __framework_authoring_catalog(
    public_output_schema_id: &SchemaId,
) -> Result<ProgramAuthoringCatalog> {
    let mut catalog = ProgramAuthoringCatalog::__new();
    let placeholder_node = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    );

    let render_config = spec::framework_config_ref("public_output_render", &placeholder_node)
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    catalog.__register_framework_state_descriptor(framework_render_descriptor(
        &spec::public_output_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?,
        &spec::public_output_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?,
        &render_config.schema_id,
        public_output_schema_id,
        spec::StateContextDescriptorSpec::no_context(),
    )?)?;

    let retention_config =
        spec::framework_config_ref("project_retention_manifest", &placeholder_node)
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
    catalog.__register_framework_state_descriptor(
        framework_project_retention_manifest_descriptor(
            &spec::retention_manifest_receipt_schema_id()
                .map_err(|error| CertifyError::Spec(error.to_string()))?,
            &spec::retention_manifest_receipt_semantic_type_id()
                .map_err(|error| CertifyError::Spec(error.to_string()))?,
            &retention_config.schema_id,
            &spec::public_output_receipt_schema_id()
                .map_err(|error| CertifyError::Spec(error.to_string()))?,
        )?,
    )?;

    let complete_config = spec::framework_config_ref("complete_run", &placeholder_node)
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    catalog.__register_framework_state_descriptor(framework_complete_run_descriptor(
        &spec::complete_run_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?,
        &spec::complete_run_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?,
        &complete_config.schema_id,
        &spec::retention_manifest_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?,
    )?)?;

    let resolve_config = spec::framework_config_ref("resolve_saga_terminal", &placeholder_node)
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    let resolve_input = spec::framework_lifecycle_unit_input_binding("resolve_saga_terminal")
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    catalog.__register_framework_state_descriptor(framework_resolve_saga_terminal_descriptor(
        &spec::resolve_saga_terminal_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?,
        &spec::resolve_saga_terminal_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?,
        &resolve_config.schema_id,
        &resolve_input.input_schema_id,
    )?)?;

    Ok(catalog)
}
