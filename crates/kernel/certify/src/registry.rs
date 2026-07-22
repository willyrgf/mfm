use super::*;

/// Source that validated supplied typed config bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigValidationSource {
    /// A registered typed config validator decoded and validated the bytes.
    RegisteredValidator,
    /// A trusted draft config reference matched the bytes exactly.
    TrustedExactRef,
}

#[derive(Clone)]
pub(super) struct ConfigValidator {
    pub(super) schema_id: SchemaId,
    pub(super) validate: fn(&[u8]) -> Result<()>,
}

impl fmt::Debug for ConfigValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigValidator")
            .field("schema_id", &self.schema_id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ConfigValidator {
    fn eq(&self, other: &Self) -> bool {
        self.schema_id == other.schema_id
    }
}

impl Eq for ConfigValidator {}

#[derive(Clone)]
pub(super) struct ContextValidator {
    pub(super) requirement: spec::StateContextDescriptorRequirementSpec,
    pub(super) validate: fn(&spec::CertifiedContextSpec) -> program::Result<()>,
}

impl fmt::Debug for ContextValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextValidator")
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

impl PartialEq for ContextValidator {
    fn eq(&self, other: &Self) -> bool {
        self.requirement == other.requirement
    }
}

impl Eq for ContextValidator {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrustedConfigRef {
    schema_id: SchemaId,
    digest: ContentDigest,
    byte_len: u64,
}

/// Registry authority used when certifying an already-lowered typed spec.
///
/// Descriptor ids and descriptor kind/version pairs must both be unique. This matches typed
/// authoring registry admission, but applies to every certification descriptor insertion path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CertificationRegistry {
    pub(super) states: BTreeMap<String, spec::StateDescriptorIdentity>,
    pub(super) operations: BTreeMap<String, spec::OperationDescriptorIdentity>,
    fact_descriptor_artifacts: BTreeMap<String, FactDescriptorArtifact>,
    config_validators: BTreeMap<String, ConfigValidator>,
    pub(super) context_validators: BTreeMap<String, ContextValidator>,
    trusted_config_refs: BTreeMap<String, TrustedConfigRef>,
    pub(super) schema_roles: BTreeMap<String, BTreeSet<CertifiedSchemaRole>>,
    pub(super) manual_authorization_verifiers: BTreeSet<String>,
    pub(super) operator_authority_snapshots: BTreeMap<String, spec::OperatorAuthoritySnapshotSpec>,
}

impl CertificationRegistry {
    /// Creates an empty certification registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the deterministic digest of this registry authority.
    pub fn digest(&self) -> Result<ContentDigest> {
        let operations = self
            .operations
            .values()
            .map(operation_descriptor_ref_json)
            .collect::<Result<Vec<_>>>()?;
        let states = self
            .states
            .values()
            .map(state_descriptor_ref_json)
            .collect::<Result<Vec<_>>>()?;
        content_digest_json(serde_json::json!({
            "algorithm": REGISTRY_DIGEST_ALGORITHM,
            "operations": operations,
            "contexts": self
                .context_validators
                .values()
                .map(context_validator_ref_json)
                .collect::<Vec<_>>(),
            "manual_authorization_verifiers": self
                .manual_authorization_verifiers
                .iter()
                .collect::<Vec<_>>(),
            "operator_authority_snapshots": self
                .operator_authority_snapshots
                .values()
                .map(operator_authority_snapshot_json)
                .collect::<Vec<_>>(),
            "schema_role_grants": self
                .schema_roles
                .iter()
                .flat_map(|(schema_id, roles)| {
                    roles.iter().map(move |role| {
                        serde_json::json!({
                            "role": role.as_str(),
                            "schema_id": schema_id,
                        })
                    })
                })
                .collect::<Vec<_>>(),
            "states": states,
        }))
    }

    /// Adds a framework-validated state descriptor to this registry.
    pub fn register_state<S>(&mut self) -> Result<()>
    where
        S: program::StateSpec,
        S::Effect: program::EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = program::state_descriptor::<S>()?;
        self.insert_state(state_descriptor_identity_from_descriptor(&descriptor)?)?;
        for fact_descriptor in
            <S::Effect as program::EffectRunner<S>>::emitted_fact_descriptor_artifacts()
                .map_err(|error| problem(ProblemClass::InvalidDataShape, error.to_string()))?
        {
            self.insert_fact_descriptor(&fact_descriptor)?;
        }
        self.insert_config_validator(config_validator_for::<S::Config>()?)?;
        if let Some(validator) = context_validator_for::<S::Context>()? {
            self.insert_context_validator(validator)?;
        }
        Ok(())
    }

    /// Adds a framework-validated operation descriptor to this registry.
    pub fn register_operation<O>(&mut self) -> Result<()>
    where
        O: program::Operation,
    {
        let descriptor = program::operation_descriptor::<O>()?;
        self.insert_operation(operation_descriptor_identity_from_descriptor(&descriptor))?;
        self.insert_config_validator(config_validator_for::<O::Config>()?)
    }

    fn insert_fact_descriptor(
        &mut self,
        descriptor: &program::facts::FactDescriptor,
    ) -> Result<()> {
        let bytes = program::facts::canonical_fact_descriptor_bytes(descriptor)
            .map_err(|error| problem(ProblemClass::InvalidDataShape, error.to_string()))?;
        let descriptor_hash = program::facts::fact_descriptor_hash(descriptor)
            .map_err(|error| problem(ProblemClass::InvalidDataShape, error.to_string()))?;
        self.insert_fact_descriptor_artifact(FactDescriptorArtifact {
            descriptor_hash,
            bytes: bytes.as_bytes().to_vec(),
        })
    }

    /// Returns canonical fact descriptor artifact material by descriptor hash.
    pub fn fact_descriptor_artifact(
        &self,
        descriptor_hash: &ContentDigest,
    ) -> Option<&FactDescriptorArtifact> {
        self.fact_descriptor_artifacts.get(descriptor_hash.as_str())
    }

    /// Adds a trusted already-lowered state descriptor identity to this registry.
    ///
    /// This is for registry assembly code whose descriptor source is already trusted. Do not derive
    /// this identity from the same persisted spec that will be certified; persisted spec bytes are
    /// hostile until [`verify_persisted_spec_certificate`] succeeds.
    pub fn register_state_descriptor(
        &mut self,
        descriptor: spec::StateDescriptorIdentity,
    ) -> Result<()> {
        self.insert_state(descriptor)
    }

    /// Adds a trusted already-lowered operation descriptor identity to this registry.
    ///
    /// This is for registry assembly code whose descriptor source is already trusted. Do not derive
    /// this identity from the same persisted spec that will be certified; persisted spec bytes are
    /// hostile until [`verify_persisted_spec_certificate`] succeeds.
    pub fn register_operation_descriptor(
        &mut self,
        descriptor: spec::OperationDescriptorIdentity,
    ) -> Result<()> {
        self.insert_operation(descriptor)
    }

    /// Grants a schema id a certification role.
    pub fn register_schema_role(
        &mut self,
        schema_id: SchemaId,
        role: CertifiedSchemaRole,
    ) -> Result<()> {
        self.insert_schema_role(schema_id, role);
        Ok(())
    }

    /// Registers a manual authorization verifier identity.
    pub fn register_manual_authorization_verifier(
        &mut self,
        verifier_id: spec::ManualAuthorizationVerifierId,
    ) -> Result<()> {
        self.manual_authorization_verifiers
            .insert(verifier_id.as_str().to_owned());
        Ok(())
    }

    /// Registers an operator authority snapshot used by manual resolution certification.
    pub fn register_operator_authority_snapshot(
        &mut self,
        snapshot: spec::OperatorAuthoritySnapshotSpec,
    ) -> Result<()> {
        self.insert_operator_authority_snapshot(snapshot)
    }

    /// Builds the descriptor registry used by a framework-lowered program draft.
    pub fn from_program_draft(draft: &program::TypedProgramDraft) -> Result<Self> {
        let mut registry = Self::new();
        for node in draft.state_nodes() {
            registry.insert_state(state_descriptor_identity_from_program(node)?)?;
            registry.insert_trusted_config_binding(&node.config)?;
        }
        for validator in draft.context_validators() {
            registry.insert_context_validator(ContextValidator {
                requirement: validator.requirement().clone(),
                validate: validator.validate_fn(),
            })?;
        }
        for frame in draft.operation_lineage() {
            registry.insert_operation(operation_descriptor_identity_from_program(frame))?;
            registry.insert_trusted_config_binding(&frame.config)?;
        }
        Ok(registry)
    }

    /// Returns the trusted registry subset named by a parsed spec's descriptor identities.
    ///
    /// The parsed spec supplies only selector keys. Operation identities must be present in this
    /// trusted registry. State identities are copied when the trusted registry owns them; framework
    /// generated states remain subject to full verifier validation before authority can be minted.
    pub fn scoped_for_spec(&self, spec: &spec::TypedExecutionSpec) -> Result<Self> {
        let mut scoped = Self::new();
        for descriptor in &spec.descriptor_identities {
            match descriptor {
                spec::DescriptorIdentity::State(identity) => {
                    if let Some(trusted) = self.states.get(identity.descriptor_id.as_str()) {
                        if trusted != identity.as_ref() {
                            return Err(certificate(format!(
                                "state descriptor {} does not match the trusted registry",
                                identity.descriptor_id
                            )));
                        }
                        scoped.insert_state(trusted.clone())?;
                        if let Some(validator) = self
                            .config_validators
                            .get(trusted.config_schema_id.as_str())
                        {
                            scoped.insert_config_validator(validator.clone())?;
                        }
                        if let spec::StateContextDescriptorSpec::Required(requirement) =
                            &trusted.context
                        {
                            if let Some(validator) = self
                                .context_validators
                                .get(requirement.context_descriptor_id.as_str())
                            {
                                scoped.insert_context_validator(validator.clone())?;
                            }
                        }
                    }
                }
                spec::DescriptorIdentity::Operation(identity) => {
                    let Some(trusted) = self.operations.get(identity.descriptor_id.as_str()) else {
                        return Err(certificate(format!(
                            "operation descriptor {} is not present in the trusted registry",
                            identity.descriptor_id
                        )));
                    };
                    if trusted != identity.as_ref() {
                        return Err(certificate(format!(
                            "operation descriptor {} does not match the trusted registry",
                            identity.descriptor_id
                        )));
                    }
                    scoped.insert_operation(trusted.clone())?;
                    if let Some(validator) = self
                        .config_validators
                        .get(trusted.config_schema_id.as_str())
                    {
                        scoped.insert_config_validator(validator.clone())?;
                    }
                }
                spec::DescriptorIdentity::Renderer(_) => {}
            }
        }
        for descriptor_hash in fact_descriptor_hashes_for_spec(spec) {
            if let Some(artifact) = self.fact_descriptor_artifact(&descriptor_hash) {
                scoped.insert_fact_descriptor_artifact(artifact.clone())?;
            }
        }
        for config_ref in &spec.config_refs {
            let key = config_ref_key(config_ref);
            if let Some(trusted) = self.trusted_config_refs.get(&key) {
                if trusted.schema_id != config_ref.schema_id
                    || trusted.digest != config_ref.digest
                    || trusted.byte_len != config_ref.byte_len
                {
                    return Err(certificate(format!(
                        "trusted config ref {} does not match parsed spec",
                        config_ref.digest
                    )));
                }
                scoped.trusted_config_refs.insert(key, trusted.clone());
            }
        }
        for context in &spec.contexts {
            let Some(validator) = self
                .context_validators
                .get(context.context_descriptor_id.as_str())
            else {
                return Err(certificate(format!(
                    "context descriptor {} is not present in the trusted registry",
                    context.context_descriptor_id
                )));
            };
            scoped.insert_context_validator(validator.clone())?;
        }
        for manual in manual_resolution_specs(spec) {
            scoped.copy_schema_role_from(
                self,
                &manual.evidence_schema,
                CertifiedSchemaRole::ManualResolutionEvidence,
            )?;
            scoped
                .copy_manual_authorization_verifier_from(self, &manual.authorization.verifier_id)?;
            scoped.copy_operator_authority_snapshot_from(
                self,
                &manual.authorization.authority.authority_id,
            )?;
        }
        Ok(scoped)
    }

    /// Validates supplied config bytes for a certified config reference when this registry owns
    /// either a typed config validator or an exact trusted draft reference.
    pub fn validate_config_ref_bytes(
        &self,
        config_ref: &spec::ConfigRef,
        bytes: &[u8],
    ) -> Result<Option<ConfigValidationSource>> {
        let canonical = canonical_config_bytes_for_ref(config_ref, bytes)?;
        if let Some(validator) = self.config_validators.get(config_ref.schema_id.as_str()) {
            (validator.validate)(canonical.as_bytes())?;
            return Ok(Some(ConfigValidationSource::RegisteredValidator));
        }
        let key = config_ref_key(config_ref);
        if self.trusted_config_refs.contains_key(&key) {
            return Ok(Some(ConfigValidationSource::TrustedExactRef));
        }
        Ok(None)
    }

    fn insert_state(&mut self, descriptor: spec::StateDescriptorIdentity) -> Result<()> {
        let key = descriptor.descriptor_id.as_str().to_owned();
        if let Some(existing) = self.states.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered state descriptor {key}"),
                ));
            }
            return Ok(());
        }
        if let Some(existing) = self.states.values().find(|existing| {
            existing.state_kind == descriptor.state_kind
                && existing.state_version == descriptor.state_version
        }) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "conflicting registered state kind/version {}@{}: descriptor {} conflicts with {}",
                    descriptor.state_kind.as_str(),
                    descriptor.state_version.as_str(),
                    descriptor.descriptor_id.as_str(),
                    existing.descriptor_id.as_str()
                ),
            ));
        }
        self.states.insert(key, descriptor);
        Ok(())
    }

    fn insert_operation(&mut self, descriptor: spec::OperationDescriptorIdentity) -> Result<()> {
        let key = descriptor.descriptor_id.as_str().to_owned();
        if let Some(existing) = self.operations.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered operation descriptor {key}"),
                ));
            }
            return Ok(());
        }
        if let Some(existing) = self.operations.values().find(|existing| {
            existing.operation_kind == descriptor.operation_kind
                && existing.operation_version == descriptor.operation_version
        }) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "conflicting registered operation kind/version {}@{}: descriptor {} conflicts with {}",
                    descriptor.operation_kind.as_str(),
                    descriptor.operation_version.as_str(),
                    descriptor.descriptor_id.as_str(),
                    existing.descriptor_id.as_str()
                ),
            ));
        }
        self.operations.insert(key, descriptor);
        Ok(())
    }

    fn insert_fact_descriptor_artifact(&mut self, artifact: FactDescriptorArtifact) -> Result<()> {
        let key = artifact.descriptor_hash.as_str().to_owned();
        if let Some(existing) = self.fact_descriptor_artifacts.get(&key) {
            if existing != &artifact {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting fact descriptor artifact {key}"),
                ));
            }
        } else {
            self.fact_descriptor_artifacts.insert(key, artifact);
        }
        Ok(())
    }

    pub(super) fn insert_config_validator(&mut self, validator: ConfigValidator) -> Result<()> {
        let key = validator.schema_id.as_str().to_owned();
        if let Some(existing) = self.config_validators.get(&key) {
            if existing != &validator {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered config validator {key}"),
                ));
            }
        } else {
            self.config_validators.insert(key, validator);
        }
        Ok(())
    }

    fn insert_context_validator(&mut self, validator: ContextValidator) -> Result<()> {
        let key = validator
            .requirement
            .context_descriptor_id
            .as_str()
            .to_owned();
        if let Some(existing) = self.context_validators.get(&key) {
            if existing != &validator {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered context validator {key}"),
                ));
            }
        } else {
            self.context_validators.insert(key, validator);
        }
        Ok(())
    }

    fn insert_schema_role(&mut self, schema_id: SchemaId, role: CertifiedSchemaRole) {
        self.schema_roles
            .entry(schema_id.as_str().to_owned())
            .or_default()
            .insert(role);
    }

    fn copy_schema_role_from(
        &mut self,
        trusted: &CertificationRegistry,
        schema_id: &SchemaId,
        role: CertifiedSchemaRole,
    ) -> Result<()> {
        let Some(roles) = trusted.schema_roles.get(schema_id.as_str()) else {
            return Err(certificate(format!(
                "manual schema {} is not present in the trusted registry",
                schema_id
            )));
        };
        if !roles.contains(&role) {
            return Err(certificate(format!(
                "manual schema {} lacks certified role {}",
                schema_id,
                role.as_str()
            )));
        }
        self.insert_schema_role(schema_id.clone(), role);
        Ok(())
    }

    fn copy_manual_authorization_verifier_from(
        &mut self,
        trusted: &CertificationRegistry,
        verifier_id: &spec::ManualAuthorizationVerifierId,
    ) -> Result<()> {
        if !trusted
            .manual_authorization_verifiers
            .contains(verifier_id.as_str())
        {
            return Err(certificate(format!(
                "manual authorization verifier {} is not present in the trusted registry",
                verifier_id
            )));
        }
        self.manual_authorization_verifiers
            .insert(verifier_id.as_str().to_owned());
        Ok(())
    }

    fn copy_operator_authority_snapshot_from(
        &mut self,
        trusted: &CertificationRegistry,
        authority_id: &spec::OperatorAuthorityId,
    ) -> Result<()> {
        let Some(snapshot) = trusted
            .operator_authority_snapshots
            .get(authority_id.as_str())
        else {
            return Err(certificate(format!(
                "operator authority snapshot {} is not present in the trusted registry",
                authority_id
            )));
        };
        self.insert_operator_authority_snapshot(snapshot.clone())
    }

    fn insert_operator_authority_snapshot(
        &mut self,
        snapshot: spec::OperatorAuthoritySnapshotSpec,
    ) -> Result<()> {
        let key = snapshot.authority_id.as_str().to_owned();
        if let Some(existing) = self.operator_authority_snapshots.get(&key) {
            if existing != &snapshot {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting operator authority snapshot {key}"),
                ));
            }
        } else {
            self.operator_authority_snapshots.insert(key, snapshot);
        }
        Ok(())
    }

    fn insert_trusted_config_binding(
        &mut self,
        binding: &program::ConfigBindingSpec,
    ) -> Result<()> {
        let trusted = TrustedConfigRef {
            schema_id: binding.schema_id.clone(),
            digest: binding.content_digest.clone(),
            byte_len: binding.byte_len as u64,
        };
        let key = format!("{}:{}", trusted.schema_id, trusted.digest);
        if let Some(existing) = self.trusted_config_refs.get(&key) {
            if existing != &trusted {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    format!("conflicting trusted config ref {key}"),
                ));
            }
        } else {
            self.trusted_config_refs.insert(key, trusted);
        }
        Ok(())
    }
}
