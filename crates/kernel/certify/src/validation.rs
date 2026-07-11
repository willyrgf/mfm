use super::*;

#[path = "framework_lifecycle.rs"]
mod framework_lifecycle;
#[path = "node_contract.rs"]
mod node_contract;

pub(super) fn validate_typed_spec(
    spec: spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<ValidatedTypedExecutionSpec> {
    let envelope = spec::HashedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    envelope
        .verify_hash()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    let spec = &envelope.spec;
    spec.spec_hash()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    validate_contract_header(spec)?;
    let descriptor_index = DescriptorIndex::new(&spec.descriptor_identities)?;
    validate_descriptor_authority(&descriptor_index, registry)?;
    let context_index = validate_contexts(&spec.contexts, registry)?;
    let scope_ids = validate_scopes(&spec.scopes)?;
    let config_refs = validate_config_refs(&spec.config_refs)?;
    let lineage_index = validate_value_lineages(&spec.value_lineages, &scope_ids)?;
    let cell_index = validate_cells(&spec.cells, &scope_ids, &lineage_index)?;
    validate_seeds(&spec.seeds, &scope_ids, &cell_index)?;
    let node_index = validate_nodes(
        &spec.nodes,
        NodeValidationContext {
            remediations: &spec.remediations,
            scope_ids: &scope_ids,
            descriptors: &descriptor_index,
            contexts: &context_index,
            config_refs: &config_refs,
            cells: &cell_index,
            lineages: &lineage_index,
            public_outputs: &spec.public_outputs,
        },
    )?;
    validate_saga_structure(SagaValidationContext {
        typed: spec,
        scope_ids: &scope_ids,
        descriptors: &descriptor_index,
        contexts: &context_index,
        config_refs: &config_refs,
        cells: &cell_index,
        lineages: &lineage_index,
        forward_nodes: &node_index,
        registry,
    })?;
    validate_operation_lineage(
        &spec.planning_lineage,
        &descriptor_index,
        &config_refs,
        &cell_index,
    )?;
    validate_planning_lineage_authority(spec)?;
    validate_public_outputs(
        &spec.public_outputs,
        &descriptor_index,
        &cell_index,
        &node_index,
    )?;
    validate_lineage_references(&spec.value_lineages, &cell_index, &config_refs)?;
    let graph = CertifiedSpecGraph::from_validated_parts(
        scope_ids,
        &descriptor_index,
        config_refs,
        lineage_index,
        cell_index,
        node_index,
        spec.remediations.clone(),
    );
    Ok(ValidatedTypedExecutionSpec::new(envelope, graph))
}

fn validate_contract_header(spec: &spec::TypedExecutionSpec) -> Result<()> {
    if spec.spec_version.as_str() != spec::SPEC_VERSION {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected spec version {}", spec.spec_version),
        ));
    }
    if spec.media_type.as_str() != spec::MEDIA_TYPE {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected spec media type {}", spec.media_type.as_str()),
        ));
    }
    if spec.canonicalization != DigestAlgorithm::Sha256JcsV1 {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "typed specs must use sha256-jcs-v1",
        ));
    }
    if spec.lowering_version.as_str() != spec::LOWERING_VERSION {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected lowering version {}", spec.lowering_version),
        ));
    }
    Ok(())
}

fn validate_scopes(scopes: &[spec::ScopeSpec]) -> Result<BTreeSet<String>> {
    if scopes.is_empty() {
        return Err(problem(
            ProblemClass::InvalidTopology,
            "typed spec must contain at least one scope",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut parents = BTreeMap::new();
    let mut root_count = 0_usize;
    for scope in scopes {
        validate_planning_lineage(&scope.planning_lineage)?;
        if !ids.insert(scope.scope_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate scope id {}", scope.scope_id),
            ));
        }
        if scope.parent_scope_id.is_none() {
            root_count += 1;
        }
        let expected = scope_id_from_spec(
            scope.parent_scope_id.as_ref(),
            scope.stable_key.as_str(),
            &scope.planning_lineage,
        )?;
        if scope.scope_id != expected {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("scope {} is not stable-id derived", scope.scope_id),
            ));
        }
        parents.insert(
            scope.scope_id.as_str().to_owned(),
            scope
                .parent_scope_id
                .as_ref()
                .map(|parent| parent.as_str().to_owned()),
        );
    }
    if root_count != 1 {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!("typed spec must contain exactly one root scope, found {root_count}"),
        ));
    }
    for scope in scopes {
        if let Some(parent) = &scope.parent_scope_id {
            if !ids.contains(parent.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "scope {} references unknown parent {parent}",
                        scope.scope_id
                    ),
                ));
            }
        }
        let mut seen = BTreeSet::new();
        let mut current = Some(scope.scope_id.as_str().to_owned());
        while let Some(scope_id) = current {
            if !seen.insert(scope_id.clone()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!("scope {} participates in a parent cycle", scope.scope_id),
                ));
            }
            current = parents.get(&scope_id).cloned().flatten();
        }
    }
    Ok(ids)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConfigIndex {
    pub(super) keys: BTreeSet<String>,
    ref_digests: BTreeSet<String>,
}

impl ConfigIndex {
    fn contains_ref(&self, config_ref: &spec::ConfigRef) -> bool {
        self.keys.contains(&config_ref_key(config_ref))
    }

    fn contains_ref_digest(&self, digest: &ContentDigest) -> bool {
        self.ref_digests.contains(digest.as_str())
    }
}

fn validate_config_refs(config_refs: &[spec::ConfigRef]) -> Result<ConfigIndex> {
    let mut keys = BTreeSet::new();
    let mut ref_digests = BTreeSet::new();
    for config in config_refs {
        if config.byte_len == 0 {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!("config ref {} has zero byte length", config.digest),
            ));
        }
        let expected_artifact =
            ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *config.digest.digest());
        if config.artifact_id != expected_artifact {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!("config ref {} artifact id mismatch", config.digest),
            ));
        }
        if !keys.insert(config_ref_key(config)) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "duplicate config ref schema {} digest {}",
                    config.schema_id, config.digest
                ),
            ));
        }
        ref_digests.insert(config_ref_digest(config)?.as_str().to_owned());
    }
    Ok(ConfigIndex { keys, ref_digests })
}

#[derive(Debug)]
struct ContextIndex<'a> {
    by_ref: BTreeMap<String, &'a spec::CertifiedContextSpec>,
}

impl<'a> ContextIndex<'a> {
    fn get(&self, context_ref: &mfm_ids::ContextRef) -> Result<&'a spec::CertifiedContextSpec> {
        self.by_ref
            .get(context_ref.as_str())
            .copied()
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("missing certified context {context_ref}"),
                )
            })
    }
}

fn validate_contexts<'a>(
    contexts: &'a [spec::CertifiedContextSpec],
    registry: &CertificationRegistry,
) -> Result<ContextIndex<'a>> {
    let mut by_ref = BTreeMap::new();
    for context in contexts {
        let validator = registry
            .context_validators
            .get(context.context_descriptor_id.as_str())
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "certified context {} uses unregistered context descriptor {}",
                        context.context_ref, context.context_descriptor_id
                    ),
                )
            })?;
        if validator.requirement.schema_id != context.schema_id
            || validator.requirement.semantic_type_id != context.semantic_type_id
            || validator.requirement.canonicalizer_identity != context.canonicalizer_identity
        {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "certified context {} descriptor metadata does not match registry authority",
                    context.context_ref
                ),
            ));
        }
        let digest = context.canonical_context.content_digest();
        if context.canonical_context_digest != digest {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "certified context {} digest mismatch: expected {}, recomputed {}",
                    context.context_ref, context.canonical_context_digest, digest
                ),
            ));
        }
        let byte_len = context.canonical_context.as_bytes().len() as u64;
        if context.canonical_context_byte_len != byte_len {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "certified context {} byte length mismatch: expected {}, recomputed {}",
                    context.context_ref, context.canonical_context_byte_len, byte_len
                ),
            ));
        }
        let value: serde_json::Value = serde_json::from_slice(context.canonical_context.as_bytes())
            .map_err(|error| {
                problem(
                    ProblemClass::InvalidDataShape,
                    format!(
                        "certified context {} JSON did not decode: {error}",
                        context.context_ref
                    ),
                )
            })?;
        let encoded = serde_json::to_string(&value).map_err(|error| {
            problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "certified context {} could not be serialized canonically: {error}",
                    context.context_ref
                ),
            )
        })?;
        let expected = PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|error| {
            problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "certified context {} canonical encoding was invalid: {error}",
                    context.context_ref
                ),
            )
        })?;
        if expected.as_bytes() != context.canonical_context.as_bytes() {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "certified context {} did not match registered canonical encoding",
                    context.context_ref
                ),
            ));
        }
        let derived = spec::CertifiedContextSpec::derive_context_ref(
            &context.context_descriptor_id,
            &context.schema_id,
            &context.semantic_type_id,
            &context.canonicalizer_identity,
            &context.canonical_context,
        )
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
        if context.context_ref != derived {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "certified context ref mismatch: expected {}, recomputed {}",
                    context.context_ref, derived
                ),
            ));
        }
        (validator.validate)(context).map_err(|error| {
            problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "certified context {} did not decode through registered descriptor: {error}",
                    context.context_ref
                ),
            )
        })?;
        if by_ref
            .insert(context.context_ref.as_str().to_owned(), context)
            .is_some()
        {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!("duplicate certified context {}", context.context_ref),
            ));
        }
    }
    Ok(ContextIndex { by_ref })
}

#[derive(Debug)]
pub(super) struct DescriptorIndex<'a> {
    pub(super) states: BTreeMap<String, &'a spec::StateDescriptorIdentity>,
    pub(super) operations: BTreeMap<String, &'a spec::OperationDescriptorIdentity>,
    pub(super) renderers: BTreeMap<String, &'a spec::RendererDescriptorIdentity>,
}

impl<'a> DescriptorIndex<'a> {
    fn new(descriptors: &'a [spec::DescriptorIdentity]) -> Result<Self> {
        let mut index = Self {
            states: BTreeMap::new(),
            operations: BTreeMap::new(),
            renderers: BTreeMap::new(),
        };
        let mut all = BTreeSet::new();
        for descriptor in descriptors {
            let id = descriptor.descriptor_id().as_str().to_owned();
            if !all.insert(id.clone()) {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("duplicate descriptor identity {id}"),
                ));
            }
            match descriptor {
                spec::DescriptorIdentity::State(state) => {
                    index.states.insert(id, state);
                }
                spec::DescriptorIdentity::Operation(operation) => {
                    index.operations.insert(id, operation);
                }
                spec::DescriptorIdentity::Renderer(renderer) => {
                    index.renderers.insert(id, renderer);
                }
            }
        }
        Ok(index)
    }

    fn state(&self, id: &DescriptorId) -> Result<&'a spec::StateDescriptorIdentity> {
        self.states.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("missing state descriptor identity {id}"),
            )
        })
    }

    fn operation(&self, id: &DescriptorId) -> Result<&'a spec::OperationDescriptorIdentity> {
        self.operations.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("missing operation descriptor identity {id}"),
            )
        })
    }

    fn renderer(&self, id: &DescriptorId) -> Result<&'a spec::RendererDescriptorIdentity> {
        self.renderers.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidTerminalShape,
                format!("missing renderer descriptor identity {id}"),
            )
        })
    }
}

fn validate_descriptor_authority(
    descriptors: &DescriptorIndex<'_>,
    registry: &CertificationRegistry,
) -> Result<()> {
    for descriptor in descriptors.states.values() {
        validate_state_descriptor_identity(descriptor)?;
        if validate_builtin_framework_state_descriptor(descriptor)? {
            continue;
        }
        let Some(registered) = registry.states.get(descriptor.descriptor_id.as_str()) else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "state descriptor {} is not registered",
                    descriptor.descriptor_id
                ),
            ));
        };
        if registered != *descriptor {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "state descriptor {} does not match registry authority",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    for descriptor in descriptors.operations.values() {
        validate_operation_descriptor_identity(descriptor)?;
        let Some(registered) = registry.operations.get(descriptor.descriptor_id.as_str()) else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation descriptor {} is not registered",
                    descriptor.descriptor_id
                ),
            ));
        };
        if registered != *descriptor {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation descriptor {} does not match registry authority",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    for descriptor in descriptors.renderers.values() {
        let expected = descriptor
            .expected_descriptor_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        if descriptor.descriptor_id != expected {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "renderer descriptor {} is not content addressed",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_state_descriptor_identity(descriptor: &spec::StateDescriptorIdentity) -> Result<()> {
    let effect = effect_descriptor_for_kind(&descriptor.effect_kind)?;
    if descriptor.effect_class != effect.class.as_str()
        || descriptor.effect_name != effect.name
        || descriptor.effect_version != effect.version
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} effect metadata mismatch",
                descriptor.descriptor_id
            ),
        ));
    }
    let expected_runner = match effect.class {
        EffectClass::Pure => "pure",
        EffectClass::ReadExternal => "read_external",
        EffectClass::ManagedPlatformWrite => "managed_platform_write",
        EffectClass::ApplySideEffect => "apply_side_effect",
    };
    if descriptor.runner != expected_runner {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} runner mismatch",
                descriptor.descriptor_id
            ),
        ));
    }
    validate_sorted_unique_fact_descriptor_refs(
        &descriptor.emitted_fact_descriptors,
        "state descriptor emitted fact descriptors",
    )?;
    match (effect.class, &descriptor.side_effect_contract_digest) {
        (EffectClass::ApplySideEffect, Some(_)) => {}
        (EffectClass::ApplySideEffect, None) => {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "side-effect state descriptor {} is missing contract digest",
                    descriptor.descriptor_id
                ),
            ));
        }
        (_, None) => {}
        (_, Some(_)) => {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "non-side-effect state descriptor {} carries contract digest",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    let expected = state_descriptor_id_from_spec(descriptor)?;
    if descriptor.descriptor_id != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} is not content addressed",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(())
}

fn validate_operation_descriptor_identity(
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<()> {
    let expected = operation_descriptor_id_from_spec(descriptor)?;
    if descriptor.descriptor_id != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "operation descriptor {} is not content addressed",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(())
}

fn validate_builtin_framework_state_descriptor(
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<bool> {
    let expected = match descriptor.name.as_str() {
        "mfm.framework.bridge_same_value" => Some(framework_bridge_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        "mfm.framework.side_effect_verify" => Some(framework_side_effect_verify_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
            descriptor.context.clone(),
        )?),
        "mfm.framework.render_public_outputs" => Some(framework_render_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
            descriptor.context.clone(),
        )?),
        "mfm.framework.project_retention_manifest" => {
            Some(framework_project_retention_manifest_descriptor(
                &descriptor.output_schema_id,
                &descriptor.output_semantic_type_id,
                &descriptor.config_schema_id,
                &descriptor.input_schema_id,
            )?)
        }
        "mfm.framework.complete_run" => Some(framework_complete_run_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        "mfm.framework.resolve_saga_terminal" => Some(framework_resolve_saga_terminal_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        _ => None,
    };
    let Some(expected) = expected else {
        return Ok(false);
    };
    if *descriptor != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "framework state descriptor {} does not match built-in authority",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(true)
}

fn validate_value_lineages(
    lineages: &[spec::ValueLineage],
    scope_ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, spec::ValueLineage>> {
    let mut index = BTreeMap::new();
    for lineage in lineages {
        if !scope_ids.contains(lineage.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("lineage references unknown scope {}", lineage.scope_id),
            ));
        }
        validate_planning_lineage(&lineage.planning_lineage)?;
        validate_domain_keys(&lineage.domain_keys)?;
        let expected = value_lineage_digest(lineage)?;
        if lineage.lineage_ref.lineage_digest != expected {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "value lineage {} is not content addressed",
                    lineage.lineage_ref.lineage_digest
                ),
            ));
        }
        let key = lineage.lineage_ref.lineage_digest.as_str().to_owned();
        if index.insert(key.clone(), lineage.clone()).is_some() {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("duplicate value lineage {key}"),
            ));
        }
    }
    Ok(index)
}

fn validate_cells(
    cells: &[spec::CellSpec],
    scope_ids: &BTreeSet<String>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
) -> Result<BTreeMap<String, spec::CellSpec>> {
    let mut index = BTreeMap::new();
    for cell in cells {
        if !scope_ids.contains(cell.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "cell {} references unknown scope {}",
                    cell.cell_id, cell.scope_id
                ),
            ));
        }
        let lineage = lineages
            .get(cell.value_lineage.lineage_digest.as_str())
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("cell {} references missing lineage", cell.cell_id),
                )
            })?;
        if lineage.scope_id != cell.scope_id || lineage.producer != cell.producer {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "cell {} lineage does not match producer/scope",
                    cell.cell_id
                ),
            ));
        }
        let expected_cell_id = cell_id_from_parts(
            &cell.scope_id,
            &cell.producer,
            &cell.semantic_type_id,
            &cell.schema_id,
        )?;
        if cell.cell_id != expected_cell_id {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("cell {} is not stable-id derived", cell.cell_id),
            ));
        }
        if index
            .insert(cell.cell_id.as_str().to_owned(), cell.clone())
            .is_some()
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate cell id {}", cell.cell_id),
            ));
        }
    }
    Ok(index)
}

fn validate_seeds(
    seeds: &[spec::SeedSpec],
    scope_ids: &BTreeSet<String>,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<()> {
    let mut seed_ids = BTreeSet::new();
    for seed in seeds {
        if !seed_ids.insert(seed.seed_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate seed id {}", seed.seed_id),
            ));
        }
        let expected_seed_id = seed_id_from_spec(seed)?;
        if seed.seed_id != expected_seed_id {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("seed {} is not stable-id derived", seed.seed_id),
            ));
        }
        if !scope_ids.contains(seed.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "seed {} references unknown scope {}",
                    seed.seed_id, seed.scope_id
                ),
            ));
        }
        let cell = cells.get(seed.cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!(
                    "seed {} references missing cell {}",
                    seed.seed_id, seed.cell_id
                ),
            )
        })?;
        if cell.producer != spec::CellProducer::Seed(seed.seed_id.clone())
            || cell.scope_id != seed.scope_id
            || cell.schema_id != seed.schema_id
            || cell.semantic_type_id != seed.semantic_type_id
        {
            return Err(problem(
                ProblemClass::InvalidInterfaceWiring,
                format!("seed {} cell metadata mismatch", seed.seed_id),
            ));
        }
        if let spec::CellContextSpec::Bound { producer, .. } = &cell.context {
            if !producer.seed_producers_allowed {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "seed {} is not authorized to produce context-bound cell {}",
                        seed.seed_id, seed.cell_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

enum StateNodeIdDerivation {
    FrameworkAware,
    StateOnly,
}

struct StateNodeContractInput<'a> {
    nodes: &'a [spec::NodeSpec],
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    contexts: &'a ContextIndex<'a>,
    config_ref_digest: &'a ContentDigest,
    cells: &'a BTreeMap<String, spec::CellSpec>,
    lineages: &'a BTreeMap<String, spec::ValueLineage>,
    remediations: Option<&'a BTreeMap<NodeId, spec::NodeSpec>>,
    label: &'static str,
    id_derivation: StateNodeIdDerivation,
    enforce_framework_lineage_inputs: bool,
}

struct NodeValidationContext<'a, 'd> {
    remediations: &'a BTreeMap<NodeId, spec::NodeSpec>,
    scope_ids: &'a BTreeSet<String>,
    descriptors: &'a DescriptorIndex<'d>,
    contexts: &'a ContextIndex<'a>,
    config_refs: &'a ConfigIndex,
    cells: &'a BTreeMap<String, spec::CellSpec>,
    lineages: &'a BTreeMap<String, spec::ValueLineage>,
    public_outputs: &'a spec::PublicOutputSpec,
}

fn validate_nodes(
    nodes: &[spec::NodeSpec],
    context: NodeValidationContext<'_, '_>,
) -> Result<BTreeMap<String, spec::NodeSpec>> {
    let NodeValidationContext {
        remediations,
        scope_ids,
        descriptors,
        contexts,
        config_refs,
        cells,
        lineages,
        public_outputs,
    } = context;
    let mut index = BTreeMap::new();
    for node in nodes {
        if !scope_ids.contains(node.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "node {} references unknown scope {}",
                    node.node_id, node.scope_id
                ),
            ));
        }
        if !config_refs.contains_ref(&node.config_ref) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "node {} references missing config {}",
                    node.node_id, node.config_ref.digest
                ),
            ));
        }
        let descriptor = descriptors.state(&node.descriptor_id)?;
        let config_ref_digest = config_ref_digest(&node.config_ref)?;
        framework_lifecycle::validate_framework_descriptor_variant(node, descriptor)?;
        validate_node_fact_descriptor_allowlist(node, descriptor)?;
        if let Some(framework) = &node.framework {
            framework_lifecycle::validate_framework_config_ref(node, framework.config_kind())?;
        }
        node_contract::validate_state_node_contract(StateNodeContractInput {
            nodes,
            node,
            descriptor,
            contexts,
            config_ref_digest: &config_ref_digest,
            cells,
            lineages,
            remediations: Some(remediations),
            label: "node",
            id_derivation: StateNodeIdDerivation::FrameworkAware,
            enforce_framework_lineage_inputs: true,
        })?;
        if index
            .insert(node.node_id.as_str().to_owned(), node.clone())
            .is_some()
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate node id {}", node.node_id),
            ));
        }
    }
    framework_lifecycle::validate_framework_nodes(
        nodes,
        remediations,
        cells,
        descriptors,
        public_outputs,
    )?;
    validate_node_graph_acyclic(&index)?;
    Ok(index)
}

struct SagaValidationContext<'a, 'd> {
    typed: &'a spec::TypedExecutionSpec,
    scope_ids: &'a BTreeSet<String>,
    descriptors: &'a DescriptorIndex<'d>,
    contexts: &'a ContextIndex<'a>,
    config_refs: &'a ConfigIndex,
    cells: &'a BTreeMap<String, spec::CellSpec>,
    lineages: &'a BTreeMap<String, spec::ValueLineage>,
    forward_nodes: &'a BTreeMap<String, spec::NodeSpec>,
    registry: &'a CertificationRegistry,
}

fn validate_saga_structure(context: SagaValidationContext<'_, '_>) -> Result<()> {
    let forward_side_effects = context
        .forward_nodes
        .values()
        .filter(|node| is_apply_side_effect_node(node).unwrap_or(false))
        .collect::<Vec<_>>();

    match &context.typed.saga {
        spec::SagaPolicySpec::NoSideEffects => {
            if !forward_side_effects.is_empty() {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    "NoSideEffects policy cannot certify forward side-effect nodes",
                ));
            }
        }
        spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::ManualResolution { .. }
        | spec::SagaPolicySpec::CompensateCompleted { .. } => {
            if forward_side_effects.is_empty() {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    "side-effecting saga policy requires at least one forward side-effect node",
                ));
            }
        }
    }

    if !matches!(
        context.typed.saga,
        spec::SagaPolicySpec::CompensateCompleted { .. }
    ) && !context.typed.remediations.is_empty()
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            "remediations are valid only under CompensateCompleted policy",
        ));
    }

    let forward_side_effect_ids = forward_side_effects
        .iter()
        .map(|node| node.node_id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let mut remediation_node_ids = BTreeSet::new();
    for (forward_node_id, remediation) in &context.typed.remediations {
        if context
            .forward_nodes
            .contains_key(remediation.node_id.as_str())
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "remediation node {} also appears in the forward node collection",
                    remediation.node_id
                ),
            ));
        }
        if !remediation_node_ids.insert(remediation.node_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate remediation node id {}", remediation.node_id),
            ));
        }
        let Some(forward) = context.forward_nodes.get(forward_node_id.as_str()) else {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("remediation key {forward_node_id} references missing forward node"),
            ));
        };
        if !forward_side_effect_ids.contains(forward_node_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation key {forward_node_id} does not reference a forward side-effect node"
                ),
            ));
        }
        validate_remediation_node_contract(
            remediation,
            RemediationNodeValidationContext {
                forward_nodes: &context.typed.nodes,
                scope_ids: context.scope_ids,
                descriptors: context.descriptors,
                contexts: context.contexts,
                config_refs: context.config_refs,
                cells: context.cells,
                lineages: context.lineages,
            },
        )?;
        validate_remediation_binding_scope(
            forward,
            remediation,
            context.cells,
            context.forward_nodes,
        )?;
    }

    if matches!(
        context.typed.saga,
        spec::SagaPolicySpec::CompensateCompleted { .. }
    ) {
        for node in forward_side_effects {
            if !context.typed.remediations.contains_key(&node.node_id) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "compensating policy left forward side-effect node {} without remediation",
                        node.node_id
                    ),
                ));
            }
        }
    }

    validate_manual_resolution_authority(context.typed, context.registry)?;

    Ok(())
}

fn validate_manual_resolution_authority(
    typed: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<()> {
    for manual in manual_resolution_specs(typed) {
        validate_manual_evidence_schema(&manual.evidence_schema, registry)?;
        validate_manual_authorization_spec(&manual.authorization, registry)?;
    }
    Ok(())
}

fn validate_manual_evidence_schema(
    schema_id: &SchemaId,
    registry: &CertificationRegistry,
) -> Result<()> {
    let Some(roles) = registry.schema_roles.get(schema_id.as_str()) else {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("unknown manual evidence schema {schema_id}"),
        ));
    };
    if !roles.contains(&CertifiedSchemaRole::ManualResolutionEvidence) {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("manual evidence schema {schema_id} has wrong certified role"),
        ));
    }
    Ok(())
}

fn validate_manual_authorization_spec(
    authorization: &spec::ManualResolutionAuthorizationSpec,
    registry: &CertificationRegistry,
) -> Result<()> {
    if authorization.signing_scheme.as_str() != MANUAL_RESOLUTION_SIGNING_SCHEME {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "unsupported manual authorization signing scheme {}",
                authorization.signing_scheme
            ),
        ));
    }
    if !registry
        .manual_authorization_verifiers
        .contains(authorization.verifier_id.as_str())
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "unknown manual authorization verifier {}",
                authorization.verifier_id
            ),
        ));
    }
    validate_operator_authority_snapshot(&authorization.authority)?;
    let required_signatures = authorization.quorum.required_signatures() as usize;
    if required_signatures > authorization.authority.operators.len() {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "manual authorization quorum {} exceeds operator authority size {}",
                authorization.quorum.required_signatures(),
                authorization.authority.operators.len()
            ),
        ));
    }
    let authority_id = &authorization.authority.authority_id;
    let Some(registered) = registry
        .operator_authority_snapshots
        .get(authority_id.as_str())
    else {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("missing operator authority snapshot {authority_id}"),
        ));
    };
    if registered != &authorization.authority {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("operator authority snapshot {authority_id} does not match registry"),
        ));
    }
    Ok(())
}

fn validate_operator_authority_snapshot(
    authority: &spec::OperatorAuthoritySnapshotSpec,
) -> Result<()> {
    if authority.operators.is_empty() {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "operator authority snapshot {} has no operators",
                authority.authority_id
            ),
        ));
    }
    let mut operator_ids = BTreeSet::new();
    let mut public_identities = BTreeSet::new();
    for operator in &authority.operators {
        if !operator_ids.insert(operator.operator_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operator authority snapshot {} contains duplicate operator {}",
                    authority.authority_id, operator.operator_id
                ),
            ));
        }
        if !public_identities.insert(operator.public_identity.as_str()) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operator authority snapshot {} contains duplicate public identity {}",
                    authority.authority_id, operator.public_identity
                ),
            ));
        }
    }
    Ok(())
}

struct RemediationNodeValidationContext<'a, 'd> {
    forward_nodes: &'a [spec::NodeSpec],
    scope_ids: &'a BTreeSet<String>,
    descriptors: &'a DescriptorIndex<'d>,
    contexts: &'a ContextIndex<'a>,
    config_refs: &'a ConfigIndex,
    cells: &'a BTreeMap<String, spec::CellSpec>,
    lineages: &'a BTreeMap<String, spec::ValueLineage>,
}

fn validate_remediation_node_contract(
    node: &spec::NodeSpec,
    context: RemediationNodeValidationContext<'_, '_>,
) -> Result<()> {
    let RemediationNodeValidationContext {
        forward_nodes,
        scope_ids,
        descriptors,
        contexts,
        config_refs,
        cells,
        lineages,
    } = context;
    if node.framework.is_some() {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "remediation node {} must not be a framework node",
                node.node_id
            ),
        ));
    }
    if !scope_ids.contains(node.scope_id.as_str()) {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!(
                "remediation node {} references unknown scope {}",
                node.node_id, node.scope_id
            ),
        ));
    }
    if !config_refs.contains_ref(&node.config_ref) {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!(
                "remediation node {} references missing config {}",
                node.node_id, node.config_ref.digest
            ),
        ));
    }
    let descriptor = descriptors.state(&node.descriptor_id)?;
    let config_ref_digest = config_ref_digest(&node.config_ref)?;
    if !is_apply_side_effect_node(node)? {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("remediation node {} is not side-effect-grade", node.node_id),
        ));
    }
    node_contract::validate_state_node_contract(StateNodeContractInput {
        nodes: forward_nodes,
        node,
        descriptor,
        contexts,
        config_ref_digest: &config_ref_digest,
        cells,
        lineages,
        remediations: None,
        label: "remediation node",
        id_derivation: StateNodeIdDerivation::StateOnly,
        enforce_framework_lineage_inputs: false,
    })?;
    Ok(())
}

fn validate_remediation_binding_scope(
    forward: &spec::NodeSpec,
    remediation: &spec::NodeSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    forward_nodes: &BTreeMap<String, spec::NodeSpec>,
) -> Result<()> {
    let mut allowed = BTreeSet::new();
    allowed.insert(forward.output_cell.clone());
    for node in forward_nodes.values() {
        if let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework {
            if verify.submit_node_id == forward.node_id {
                allowed.insert(node.output_cell.clone());
            }
        }
    }
    for input_cell in collect_input_cells(&forward.input_bindings.root) {
        collect_forward_ancestor_cells(input_cell, cells, forward_nodes, &mut allowed)?;
    }

    for input_cell in collect_input_cells(&remediation.input_bindings.root) {
        if !allowed.contains(&input_cell) {
            return Err(problem(
                ProblemClass::InvalidInterfaceWiring,
                format!(
                    "remediation node {} references cell {} outside linked forward node {} scope",
                    remediation.node_id, input_cell, forward.node_id
                ),
            ));
        }
    }
    Ok(())
}

fn collect_forward_ancestor_cells(
    cell_id: CellId,
    cells: &BTreeMap<String, spec::CellSpec>,
    forward_nodes: &BTreeMap<String, spec::NodeSpec>,
    allowed: &mut BTreeSet<CellId>,
) -> Result<()> {
    if !allowed.insert(cell_id.clone()) {
        return Ok(());
    }
    let cell = cells.get(cell_id.as_str()).ok_or_else(|| {
        problem(
            ProblemClass::InvalidTopology,
            format!("ancestor cell {cell_id} is missing"),
        )
    })?;
    if let spec::CellProducer::Node(node_id) = &cell.producer {
        let Some(node) = forward_nodes.get(node_id.as_str()) else {
            return Ok(());
        };
        for input in collect_input_cells(&node.input_bindings.root) {
            collect_forward_ancestor_cells(input, cells, forward_nodes, allowed)?;
        }
    }
    Ok(())
}

fn is_apply_side_effect_node(node: &spec::NodeSpec) -> Result<bool> {
    let (class, _) = effect_class_for_kind(&node.effect_kind)?;
    Ok(class == EffectClass::ApplySideEffect)
}

fn validate_operation_lineage(
    frames: &[spec::OperationLineageFrameSpec],
    descriptors: &DescriptorIndex<'_>,
    config_refs: &ConfigIndex,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    for frame in frames {
        if !ids.insert(frame.operation_instance_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "duplicate operation instance {}",
                    frame.operation_instance_id
                ),
            ));
        }
        validate_planning_lineage(&frame.parent_planning_lineage)?;
        let descriptor = descriptors.operation(&frame.operation_descriptor_id)?;
        if descriptor.input_schema_id != frame.input_bindings.input_schema_id {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation frame {} input schema does not match descriptor",
                    frame.operation_instance_id
                ),
            ));
        }
        if !config_refs.contains_ref_digest(&frame.config_ref_digest) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "operation frame {} references missing config {}",
                    frame.operation_instance_id, frame.config_ref_digest
                ),
            ));
        }
        validate_input_binding(&frame.input_bindings, cells)?;
        if frame.input_binding_digest != frame.input_bindings.digest {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "operation frame {} input binding digest mismatch",
                    frame.operation_instance_id
                ),
            ));
        }
        let expected_instance = operation_instance_id_from_spec(frame, descriptor)?;
        if frame.operation_instance_id != expected_instance {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "operation instance {} is not stable-id derived",
                    frame.operation_instance_id
                ),
            ));
        }
        for cell_id in &frame.output_cells {
            if !cells.contains_key(cell_id.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "operation frame {} returned missing cell {}",
                        frame.operation_instance_id, cell_id
                    ),
                ));
            }
        }
        let expected_lineage = operation_lineage_frame_digest_from_spec(frame, descriptor, cells)?;
        if frame.lineage_digest != expected_lineage {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "operation frame {} lineage is not content addressed",
                    frame.operation_instance_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_planning_lineage_authority(spec: &spec::TypedExecutionSpec) -> Result<()> {
    let frame_instances = spec
        .planning_lineage
        .iter()
        .map(|frame| frame.operation_instance_id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let frame_digests = spec
        .planning_lineage
        .iter()
        .map(|frame| frame.lineage_digest.as_str().to_owned())
        .collect::<BTreeSet<_>>();

    for scope in &spec.scopes {
        validate_planning_lineage_refs(&scope.planning_lineage, &frame_instances, &frame_digests)?;
    }
    for node in &spec.nodes {
        validate_planning_lineage_refs(&node.planning_lineage, &frame_instances, &frame_digests)?;
    }
    for node in spec.remediations.values() {
        validate_planning_lineage_refs(&node.planning_lineage, &frame_instances, &frame_digests)?;
    }
    for lineage in &spec.value_lineages {
        validate_planning_lineage_refs(
            &lineage.planning_lineage,
            &frame_instances,
            &frame_digests,
        )?;
    }
    for frame in &spec.planning_lineage {
        validate_planning_lineage_refs(
            &frame.parent_planning_lineage,
            &frame_instances,
            &frame_digests,
        )?;
    }
    Ok(())
}

fn validate_planning_lineage_refs(
    lineage: &spec::PlanningLineage,
    frame_instances: &BTreeSet<String>,
    frame_digests: &BTreeSet<String>,
) -> Result<()> {
    validate_planning_lineage(lineage)?;
    for instance in &lineage.active_operation_instances {
        if !frame_instances.contains(instance.as_str()) {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("planning lineage references missing operation instance {instance}"),
            ));
        }
    }
    for digest in &lineage.completed_operation_frames {
        if !frame_digests.contains(digest.as_str()) {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("planning lineage references missing operation frame {digest}"),
            ));
        }
    }
    Ok(())
}

fn validate_public_outputs(
    public_outputs: &spec::PublicOutputSpec,
    descriptors: &DescriptorIndex<'_>,
    cells: &BTreeMap<String, spec::CellSpec>,
    nodes: &BTreeMap<String, spec::NodeSpec>,
) -> Result<()> {
    if public_outputs.outputs.is_empty() {
        return Err(problem(
            ProblemClass::InvalidTerminalShape,
            "public output spec must declare at least one output",
        ));
    }
    descriptors.renderer(&public_outputs.renderer_descriptor.descriptor_id)?;
    let mut fields = BTreeSet::new();
    for output in &public_outputs.outputs {
        if !fields.insert(output.public_field_path.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "duplicate public output field {}",
                    output.public_field_path.as_str()
                ),
            ));
        }
        let cell = cells.get(output.cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTerminalShape,
                format!("public output cell {} is missing", output.cell_id),
            )
        })?;
        if cell.producer != output.producer
            || cell.scope_id != output.scope_id
            || cell.semantic_type_id != output.semantic_type_id
            || cell.schema_id != output.schema_id
            || cell.value_lineage != output.value_lineage
        {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "public output {} does not match certified cell",
                    output.public_field_path.as_str()
                ),
            ));
        }
    }
    let output_digest = public_outputs
        .digest()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    let render_nodes = nodes.values().filter_map(|node| match &node.framework {
        Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => Some((node, render)),
        _ => None,
    });
    let mut matching = 0_usize;
    for (node, render) in render_nodes {
        if render.public_schema_id == public_outputs.public_schema_id
            && render.output_spec_digest == output_digest
            && render.renderer_descriptor == public_outputs.renderer_descriptor
            && render.required_cells == public_outputs.outputs
        {
            matching += 1;
            let expected_predecessors = predecessor_nodes(
                &render
                    .required_cells
                    .iter()
                    .map(|cell| cell.cell_id.clone())
                    .collect::<Vec<_>>(),
                cells,
            )?;
            if node.deterministic_predecessors != expected_predecessors {
                return Err(problem(
                    ProblemClass::InvalidTerminalShape,
                    format!(
                        "render node {} predecessors do not match public outputs",
                        node.node_id
                    ),
                ));
            }
        }
    }
    if matching != 1 {
        return Err(problem(
            ProblemClass::InvalidTerminalShape,
            format!("expected exactly one public output render node, found {matching}"),
        ));
    }
    Ok(())
}

fn validate_lineage_references(
    lineages: &[spec::ValueLineage],
    cells: &BTreeMap<String, spec::CellSpec>,
    config_refs: &ConfigIndex,
) -> Result<()> {
    for lineage in lineages {
        if let Some(config_ref_digest) = &lineage.config_ref_digest {
            if !config_refs.contains_ref_digest(config_ref_digest) {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("lineage references missing config {config_ref_digest}"),
                ));
            }
        }
        for input in &lineage.input_cells {
            if !cells.contains_key(input.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("lineage references missing input cell {input}"),
                ));
            }
        }
    }
    Ok(())
}

fn side_effect_verify_pair_problem(error: mfm_spec::SpecError) -> CertifyError {
    let class = match &error {
        mfm_spec::SpecError::SideEffectVerifyPair {
            kind:
                SideEffectVerifyPairErrorKind::FrameworkSubmitNode
                | SideEffectVerifyPairErrorKind::NonSideEffectSubmitNode,
            ..
        } => ProblemClass::InvalidSemanticTransition,
        _ => ProblemClass::InvalidTopology,
    };
    problem(class, error.to_string())
}

struct ExpectedNodeLineage {
    input_cells: Vec<CellId>,
    config_ref_digest: Option<ContentDigest>,
    transform_policy: spec::LineageTransformPolicy,
}

fn expected_node_lineage(
    node: &spec::NodeSpec,
    input_cells: &[CellId],
    config_ref_digest: &ContentDigest,
    nodes: &[spec::NodeSpec],
    remediations: Option<&BTreeMap<NodeId, spec::NodeSpec>>,
) -> Result<ExpectedNodeLineage> {
    match &node.framework {
        Some(spec::FrameworkNodeSpec::Bridge(bridge)) => Ok(ExpectedNodeLineage {
            input_cells: vec![bridge.source_cell_id.clone()],
            config_ref_digest: None,
            transform_policy: spec::LineageTransformPolicy::SameValueBridge,
        }),
        Some(spec::FrameworkNodeSpec::SideEffectVerify(_)) => {
            let remediations = remediations.ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "side-effect verify node {} requires remediation index",
                        node.node_id
                    ),
                )
            })?;
            let pair = spec::resolve_side_effect_verify_pair(nodes, remediations, node)
                .map_err(side_effect_verify_pair_problem)?;
            Ok(ExpectedNodeLineage {
                input_cells: vec![pair.submit_output_cell.clone()],
                config_ref_digest: Some(config_ref_digest.clone()),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            })
        }
        Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => Ok(ExpectedNodeLineage {
            input_cells: render
                .required_cells
                .iter()
                .map(|cell| cell.cell_id.clone())
                .collect(),
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
            Ok(ExpectedNodeLineage {
                input_cells: vec![retention.public_output_receipt_cell.clone()],
                config_ref_digest: Some(config_ref_digest.clone()),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            })
        }
        Some(spec::FrameworkNodeSpec::CompleteRun(complete)) => Ok(ExpectedNodeLineage {
            input_cells: vec![complete.retention_manifest_receipt_cell.clone()],
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) => Ok(ExpectedNodeLineage {
            input_cells: Vec::new(),
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
        None => Ok(ExpectedNodeLineage {
            input_cells: input_cells.to_vec(),
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
    }
}

fn validate_node_graph_acyclic(nodes: &BTreeMap<String, spec::NodeSpec>) -> Result<()> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Done,
    }

    fn visit(
        node_id: &str,
        nodes: &BTreeMap<String, spec::NodeSpec>,
        marks: &mut BTreeMap<String, Mark>,
    ) -> Result<()> {
        match marks.get(node_id) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Visiting) => {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!("node graph contains a cycle at {node_id}"),
                ));
            }
            None => {}
        }
        let node = nodes.get(node_id).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!("node graph references missing predecessor {node_id}"),
            )
        })?;
        marks.insert(node_id.to_owned(), Mark::Visiting);
        for predecessor in &node.deterministic_predecessors {
            visit(predecessor.as_str(), nodes, marks)?;
        }
        marks.insert(node_id.to_owned(), Mark::Done);
        Ok(())
    }

    let mut marks = BTreeMap::new();
    for node_id in nodes.keys() {
        visit(node_id, nodes, &mut marks)?;
    }
    Ok(())
}

fn validate_effect_capabilities(
    effect_kind: &EffectKind,
    capabilities: &CapabilitySetDescriptor,
) -> Result<()> {
    let (class, name) = effect_class_for_kind(effect_kind)?;
    capabilities
        .validate_for_effect_class(class, name)
        .map_err(|error| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("invalid capability set for effect {effect_kind}: {error}"),
            )
        })
}

fn validate_sorted_unique_fact_descriptor_refs(
    refs: &[spec::FactDescriptorRef],
    owner: &str,
) -> Result<()> {
    let mut previous: Option<&spec::FactDescriptorRef> = None;
    for reference in refs {
        if let Some(previous) = previous {
            if previous == reference {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    format!(
                        "{owner} repeats fact descriptor {}",
                        reference.descriptor_hash
                    ),
                ));
            }
            if previous > reference {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    format!("{owner} fact descriptor refs are not sorted"),
                ));
            }
        }
        previous = Some(reference);
    }
    Ok(())
}

fn validate_node_fact_descriptor_allowlist(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<()> {
    validate_sorted_unique_fact_descriptor_refs(
        &node.fact_descriptor_allowlist,
        "node fact descriptor allow-list",
    )?;
    let emitted = descriptor
        .emitted_fact_descriptors
        .iter()
        .map(|reference| reference.descriptor_hash.as_str())
        .collect::<BTreeSet<_>>();
    for reference in &node.fact_descriptor_allowlist {
        if !emitted.contains(reference.descriptor_hash.as_str()) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "node {} allows fact descriptor {} outside state descriptor {}",
                    node.node_id, reference.descriptor_hash, descriptor.descriptor_id
                ),
            ));
        }
    }
    Ok(())
}

pub(super) fn fact_descriptor_hashes_for_spec(
    spec: &spec::TypedExecutionSpec,
) -> BTreeSet<ContentDigest> {
    spec.nodes
        .iter()
        .chain(spec.remediations.values())
        .flat_map(|node| {
            node.fact_descriptor_allowlist
                .iter()
                .map(|reference| reference.descriptor_hash.clone())
        })
        .collect()
}

fn validate_side_effect_contract(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<()> {
    let (class, _) = effect_class_for_kind(&node.effect_kind)?;
    match (class, &node.side_effect) {
        (EffectClass::ApplySideEffect, Some(contract))
            if descriptor.side_effect_contract_digest.as_ref()
                == Some(&contract.contract_digest) =>
        {
            Ok(())
        }
        (EffectClass::ApplySideEffect, Some(_)) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "side-effect node {} contract digest does not match descriptor",
                node.node_id
            ),
        )),
        (EffectClass::ApplySideEffect, None) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "side-effect node {} is missing side-effect contract",
                node.node_id
            ),
        )),
        (_, Some(_)) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "non-side-effect node {} carries side-effect contract",
                node.node_id
            ),
        )),
        _ => Ok(()),
    }
}

fn validate_input_binding(
    binding: &spec::InputBindingSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<Vec<CellId>> {
    let expected_digest = content_digest_json(input_node_json(&binding.root))?;
    if binding.digest != expected_digest {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!(
                "input binding {} is not content addressed",
                binding.input_descriptor_id
            ),
        ));
    }
    let mut input_cells = Vec::new();
    validate_input_node(&binding.root, cells, &mut input_cells)?;
    Ok(input_cells)
}

fn validate_input_node(
    node: &spec::InputBindingNodeSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    input_cells: &mut Vec<CellId>,
) -> Result<()> {
    match node {
        spec::InputBindingNodeSpec::Unit => Ok(()),
        spec::InputBindingNodeSpec::Cell(cell) => {
            let produced = cells.get(cell.cell_id.as_str()).ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!("input binding references missing cell {}", cell.cell_id),
                )
            })?;
            if produced.semantic_type_id != cell.semantic_type_id
                || produced.schema_id != cell.schema_id
                || produced.value_lineage != cell.value_lineage
            {
                return Err(problem(
                    ProblemClass::InvalidInterfaceWiring,
                    format!(
                        "input binding for cell {} does not match cell metadata",
                        cell.cell_id
                    ),
                ));
            }
            let expected_context = input_context_from_cell_context(&produced.context);
            if cell.context != expected_context {
                return Err(problem(
                    ProblemClass::InvalidInterfaceWiring,
                    format!(
                        "input binding for cell {} does not match cell context",
                        cell.cell_id
                    ),
                ));
            }
            input_cells.push(cell.cell_id.clone());
            Ok(())
        }
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            let mut seen = BTreeSet::new();
            for field in fields {
                if !seen.insert(field.field_path.as_str().to_owned()) {
                    return Err(problem(
                        ProblemClass::InvalidInterfaceWiring,
                        format!("duplicate input field {}", field.field_path.as_str()),
                    ));
                }
                validate_input_node(&field.node, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => {
            validate_collection_ordering(false, elements, *ordering, domain_keys)?;
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => {
            validate_collection_ordering(true, elements, *ordering, domain_keys)?;
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
    }
}

fn validate_collection_ordering(
    non_empty: bool,
    elements: &[spec::InputBindingNodeSpec],
    ordering: spec::OrderingEvidence,
    domain_keys: &[spec::StableDomainKeyRef],
) -> Result<()> {
    if non_empty && elements.is_empty() {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "non-empty collection binding contains no elements",
        ));
    }
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => {
            if !domain_keys.is_empty() {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    "explicit-order vectors must not carry domain keys",
                ));
            }
        }
        spec::OrderingEvidence::StableDomainKey => {
            if domain_keys.len() != elements.len() {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    "domain-key ordered vectors must carry one key per element",
                ));
            }
            validate_domain_keys(domain_keys)?;
        }
    }
    Ok(())
}

fn validate_domain_keys(domain_keys: &[spec::StableDomainKeyRef]) -> Result<()> {
    let mut sorted = domain_keys.to_vec();
    sorted.sort();
    if sorted != domain_keys {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "stable domain keys are not canonical sorted",
        ));
    }
    let mut seen = BTreeSet::new();
    for key in domain_keys {
        let rendered = format!("{}:{}", key.schema_id, key.content_digest);
        if !seen.insert(rendered) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                "duplicate stable domain key in collection",
            ));
        }
    }
    Ok(())
}
