use super::*;

pub(super) fn predecessor_nodes(
    input_cells: &[CellId],
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<Vec<NodeId>> {
    let mut predecessors = BTreeSet::new();
    for cell_id in input_cells {
        let cell = cells.get(cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!("predecessor input cell {cell_id} is missing"),
            )
        })?;
        if let spec::CellProducer::Node(node_id) = &cell.producer {
            predecessors.insert(node_id.clone());
        }
    }
    Ok(predecessors.into_iter().collect())
}

pub(super) fn lower_input_binding(
    binding: &program::InputBindingSpec,
) -> Result<spec::InputBindingSpec> {
    lower_input_binding_parts(
        &binding.input_schema_id,
        &binding.input_descriptor_id,
        &binding.root,
        &binding.digest,
    )
}

pub(super) fn lower_saga_policy(policy: &program::SagaPolicy) -> spec::SagaPolicySpec {
    match policy {
        program::SagaPolicy::NoSideEffects => spec::SagaPolicySpec::NoSideEffects,
        program::SagaPolicy::FailWithoutAcdcClaim => spec::SagaPolicySpec::FailWithoutAcdcClaim,
        program::SagaPolicy::ManualResolution { manual } => {
            spec::SagaPolicySpec::ManualResolution {
                manual: lower_manual_resolution_evidence(manual),
            }
        }
        program::SagaPolicy::CompensateCompleted {
            on_remediation_unresolved,
        } => spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: lower_remediation_unresolved(on_remediation_unresolved),
        },
    }
}

pub(super) fn lower_remediation_unresolved(
    directive: &program::RemediationUnresolved,
) -> spec::RemediationUnresolvedSpec {
    match directive {
        program::RemediationUnresolved::ManualResolution { manual } => {
            spec::RemediationUnresolvedSpec::ManualResolution {
                manual: Box::new(lower_manual_resolution_evidence(manual)),
            }
        }
        program::RemediationUnresolved::FailWithoutAcdcClaim => {
            spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim
        }
    }
}

pub(super) fn lower_manual_resolution_evidence(
    manual: &program::ManualResolutionPolicyDraft,
) -> spec::ManualResolutionEvidenceSpec {
    manual.to_spec()
}

pub(super) fn lower_operation_input_binding(
    binding: &program::OperationInputBindingSpec,
) -> Result<spec::InputBindingSpec> {
    lower_input_binding_parts(
        &binding.input_schema_id,
        &binding.input_descriptor_id,
        &binding.root,
        &binding.digest,
    )
}

fn lower_input_binding_parts(
    input_schema_id: &SchemaId,
    input_descriptor_id: &DescriptorId,
    root: &program::InputBindingNode,
    digest: &ContentDigest,
) -> Result<spec::InputBindingSpec> {
    Ok(spec::InputBindingSpec {
        input_schema_id: input_schema_id.clone(),
        input_descriptor_id: input_descriptor_id.clone(),
        root: lower_input_node(root)?,
        digest: digest.clone(),
    })
}

pub(super) fn lower_input_node(
    node: &program::InputBindingNode,
) -> Result<spec::InputBindingNodeSpec> {
    match node.as_ref() {
        program::InputBindingNodeRef::Unit => Ok(spec::InputBindingNodeSpec::Unit),
        program::InputBindingNodeRef::Cell(cell) => Ok(spec::InputBindingNodeSpec::Cell(Box::new(
            spec::InputBindingCellSpec {
                field_path: input_field_path(cell.field_path().as_str())?,
                cell_id: cell.cell_id().clone(),
                semantic_type_id: cell.semantic_type_id().clone(),
                schema_id: cell.schema_id().clone(),
                required_terminal: lower_required_terminal(cell.required_terminal()),
                value_lineage: lineage_ref(cell.value_lineage().digest()),
                context: cell.context().clone(),
            },
        ))),
        program::InputBindingNodeRef::Tuple(elements) => elements
            .iter()
            .map(lower_input_node)
            .collect::<Result<Vec<_>>>()
            .map(spec::InputBindingNodeSpec::Tuple),
        program::InputBindingNodeRef::Struct(fields) => fields
            .iter()
            .map(|field| {
                Ok(spec::NamedInputBindingSpec {
                    field_path: input_field_path(field.field_path.as_str())?,
                    node: lower_input_node(&field.node)?,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(spec::InputBindingNodeSpec::Struct),
        program::InputBindingNodeRef::Vec {
            elements,
            ordering,
            domain_keys,
        } => Ok(spec::InputBindingNodeSpec::Vec {
            elements: elements
                .iter()
                .map(lower_input_node)
                .collect::<Result<Vec<_>>>()?,
            ordering: lower_ordering_evidence(ordering),
            domain_keys: domain_keys.iter().map(lower_domain_key_ref).collect(),
        }),
        program::InputBindingNodeRef::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => Ok(spec::InputBindingNodeSpec::NonEmptyVec {
            elements: elements
                .iter()
                .map(lower_input_node)
                .collect::<Result<Vec<_>>>()?,
            ordering: lower_ordering_evidence(ordering),
            domain_keys: domain_keys.iter().map(lower_domain_key_ref).collect(),
        }),
    }
}

pub(super) fn single_cell_input_binding(
    source: CellInfo,
    cell_id: CellId,
    field_path: &str,
) -> Result<spec::InputBindingSpec> {
    let node = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
        field_path: input_field_path(field_path)?,
        cell_id,
        semantic_type_id: source.semantic_type_id.clone(),
        schema_id: source.schema_id.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
        value_lineage: source.value_lineage.clone(),
        context: input_context_from_cell_context(&source.context),
    }));
    let digest = content_digest_json(input_node_json(&node))?;
    Ok(spec::InputBindingSpec {
        input_schema_id: source.schema_id,
        input_descriptor_id: descriptor_id_json(serde_json::json!({
            "framework": "single_cell_input",
            "semantic_type_id": source.semantic_type_id.as_str(),
        }))?,
        root: node,
        digest,
    })
}

pub(super) fn lower_required_terminal(value: program::RequiredTerminal) -> spec::RequiredTerminal {
    match value {
        program::RequiredTerminal::ProducedOnly => spec::RequiredTerminal::ProducedOnly,
        program::RequiredTerminal::MaybeSkipped => spec::RequiredTerminal::MaybeSkipped,
    }
}

pub(super) fn input_context_from_cell_context(
    context: &spec::CellContextSpec,
) -> spec::InputContextSpec {
    match context {
        spec::CellContextSpec::NoContext => spec::InputContextSpec::NoContext,
        spec::CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => spec::InputContextSpec::Required {
            context_ref: context_ref.clone(),
            resource_kind: resource_kind.clone(),
            stage: stage.clone(),
            producer: producer.clone(),
        },
    }
}

pub(super) fn node_context_from_cell_context(
    context: &spec::CellContextSpec,
) -> spec::NodeContextSpec {
    match context {
        spec::CellContextSpec::NoContext => spec::NodeContextSpec::no_context(),
        spec::CellContextSpec::Bound { context_ref, .. } => spec::NodeContextSpec::Required {
            context_ref: context_ref.clone(),
        },
    }
}

pub(super) fn state_context_descriptor_from_cell_context(
    context: &spec::CellContextSpec,
    contexts: &[spec::CertifiedContextSpec],
) -> Result<spec::StateContextDescriptorSpec> {
    match context {
        spec::CellContextSpec::NoContext => Ok(spec::StateContextDescriptorSpec::no_context()),
        spec::CellContextSpec::Bound { context_ref, .. } => {
            let context = contexts
                .iter()
                .find(|context| context.context_ref == *context_ref)
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "context-bound side-effect verify output references missing context {}",
                            context_ref
                        ),
                    )
                })?;
            Ok(spec::StateContextDescriptorSpec::Required(Box::new(
                spec::StateContextDescriptorRequirementSpec {
                    context_descriptor_id: context.context_descriptor_id.clone(),
                    schema_id: context.schema_id.clone(),
                    semantic_type_id: context.semantic_type_id.clone(),
                    canonicalizer_identity: context.canonicalizer_identity.clone(),
                },
            )))
        }
    }
}

pub(super) fn render_context_contract_for_public_outputs(
    required_cells: &[spec::PublicOutputCell],
    cells: &[spec::CellSpec],
    contexts: &[spec::CertifiedContextSpec],
) -> Result<(spec::NodeContextSpec, spec::StateContextDescriptorSpec)> {
    let mut context = None::<spec::CellContextSpec>;
    for output in required_cells {
        let cell = cells
            .iter()
            .find(|cell| cell.cell_id == output.cell_id)
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!("public output cell {} is missing", output.cell_id),
                )
            })?;
        let spec::CellContextSpec::Bound { context_ref, .. } = &cell.context else {
            continue;
        };
        match &context {
            Some(spec::CellContextSpec::Bound {
                context_ref: existing,
                ..
            }) if existing != context_ref => {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "public output render node cannot consume multiple context refs: {} and {}",
                        existing, context_ref
                    ),
                ));
            }
            Some(_) => {}
            None => context = Some(cell.context.clone()),
        }
    }
    let Some(context) = context else {
        return Ok((
            spec::NodeContextSpec::no_context(),
            spec::StateContextDescriptorSpec::no_context(),
        ));
    };
    Ok((
        node_context_from_cell_context(&context),
        state_context_descriptor_from_cell_context(&context, contexts)?,
    ))
}

pub(super) fn lower_ordering_evidence(value: &program::OrderingEvidence) -> spec::OrderingEvidence {
    match value {
        program::OrderingEvidence::ExplicitAuthorOrder => {
            spec::OrderingEvidence::ExplicitAuthorOrder
        }
        program::OrderingEvidence::StableDomainKey => spec::OrderingEvidence::StableDomainKey,
    }
}

pub(super) fn lower_domain_key_ref(
    value: &program::StableDomainKeyRef,
) -> spec::StableDomainKeyRef {
    spec::StableDomainKeyRef {
        schema_id: value.schema_id.clone(),
        content_digest: value.content_digest.clone(),
    }
}

pub(super) fn lower_bridge_kind(value: program::BridgeKind) -> spec::BridgeKind {
    match value {
        program::BridgeKind::ImportFromParent => spec::BridgeKind::ImportFromParent,
        program::BridgeKind::ExportToParent => spec::BridgeKind::ExportToParent,
    }
}

pub(super) fn bridge_kind_json(value: spec::BridgeKind) -> &'static str {
    match value {
        spec::BridgeKind::ImportFromParent => "import-from-parent",
        spec::BridgeKind::ExportToParent => "export-to-parent",
    }
}

pub(super) fn lower_bridge_policy(value: program::BridgePolicy) -> spec::BridgePolicy {
    match value {
        program::BridgePolicy::SameRunSameValueV1 => spec::BridgePolicy::SameRunSameValue,
    }
}

pub(super) fn bridge_policy_json(value: spec::BridgePolicy) -> &'static str {
    match value {
        spec::BridgePolicy::SameRunSameValue => "same-run-same-value-v1",
    }
}

pub(super) fn state_descriptor_identity_from_program(
    node: &program::StateNodeSpec,
) -> Result<spec::StateDescriptorIdentity> {
    let effect = effect_descriptor_for_kind(&node.effect_kind)?;
    Ok(spec::StateDescriptorIdentity {
        descriptor_id: node.state_descriptor_id.clone(),
        name: node.state_descriptor_name.clone(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        context: node.context_descriptor.clone(),
        input_context: node.input_context_contract.clone(),
        output_context: node.output_context_contract.clone(),
        config_schema_id: node.config.schema_id.clone(),
        input_schema_id: node.input.input_schema_id.clone(),
        output_schema_id: node.output_schema_id.clone(),
        output_semantic_type_id: node.output_semantic_type_id.clone(),
        effect_kind: node.effect_kind.clone(),
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version,
        capabilities: node.capability_bindings.clone(),
        emitted_fact_descriptors: node.fact_descriptor_allowlist.clone(),
        runner: runner_kind_name(node.runner).to_owned(),
        effect_contract_digest: node.effect_contract_digest.clone(),
    })
}

pub(super) fn state_descriptor_identity_from_descriptor(
    descriptor: &program::StateDescriptorIdentity,
) -> Result<spec::StateDescriptorIdentity> {
    let effect = descriptor.effect();
    Ok(spec::StateDescriptorIdentity {
        descriptor_id: descriptor.descriptor_id().clone(),
        name: descriptor.name().to_owned(),
        state_kind: descriptor.kind().clone(),
        state_version: descriptor.version().clone(),
        context: descriptor.context().clone(),
        input_context: descriptor.input_context().clone(),
        output_context: descriptor.output_context().clone(),
        config_schema_id: descriptor.config_schema_id().clone(),
        input_schema_id: descriptor.input_schema_id().clone(),
        output_schema_id: descriptor.output_schema_id().clone(),
        output_semantic_type_id: descriptor.output_semantic_type_id().clone(),
        effect_kind: effect.kind.clone(),
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version.clone(),
        capabilities: descriptor.capabilities().clone(),
        emitted_fact_descriptors: descriptor.emitted_fact_descriptors().to_vec(),
        runner: runner_kind_name(descriptor.runner()).to_owned(),
        effect_contract_digest: descriptor.effect_contract_digest().cloned(),
    })
}

pub(super) fn operation_descriptor_identity_from_program(
    frame: &program::OperationLineageFrameSpec,
) -> spec::OperationDescriptorIdentity {
    spec::OperationDescriptorIdentity {
        descriptor_id: frame.operation_descriptor_id.clone(),
        name: frame.operation_name.clone(),
        operation_kind: frame.operation_kind.clone(),
        operation_version: frame.operation_version.clone(),
        config_schema_id: frame.config.schema_id.clone(),
        input_schema_id: frame.input.input_schema_id.clone(),
        output_schema_id: frame.output_schema_id.clone(),
        expansion_abi: frame.expansion_abi.to_owned(),
    }
}

pub(super) fn operation_descriptor_identity_from_descriptor(
    descriptor: &program::OperationDescriptorIdentity,
) -> spec::OperationDescriptorIdentity {
    spec::OperationDescriptorIdentity {
        descriptor_id: descriptor.descriptor_id().clone(),
        name: descriptor.name().to_owned(),
        operation_kind: descriptor.kind().clone(),
        operation_version: descriptor.version().clone(),
        config_schema_id: descriptor.config_schema_id().clone(),
        input_schema_id: descriptor.input_schema_id().clone(),
        output_schema_id: descriptor.output_schema_id().clone(),
        expansion_abi: descriptor.expansion_abi().to_owned(),
    }
}

pub(super) fn runner_kind_name(runner: program::RunnerKind) -> &'static str {
    match runner {
        program::RunnerKind::Pure => "pure",
        program::RunnerKind::ReadExternal => "read_external",
        program::RunnerKind::ManagedPlatformWrite => "managed_platform_write",
        program::RunnerKind::ApplySideEffect => "apply_side_effect",
    }
}

pub(super) fn lower_planning_lineage(lineage: &program::OperationLineage) -> spec::PlanningLineage {
    spec::PlanningLineage {
        active_operation_instances: lineage.active_instances.clone(),
        completed_operation_frames: lineage.completed_frames.clone(),
        lineage_digest: lineage.digest.clone(),
    }
}

pub(super) fn empty_planning_lineage() -> Result<spec::PlanningLineage> {
    planning_lineage_from_parts(Vec::new(), Vec::new())
}

pub(super) fn final_planning_lineage(
    frames: &[program::OperationLineageFrameSpec],
) -> Result<spec::PlanningLineage> {
    planning_lineage_from_parts(
        Vec::new(),
        frames
            .iter()
            .map(|frame| frame.lineage_digest.clone())
            .collect(),
    )
}

pub(super) fn planning_lineage_from_parts(
    active_operation_instances: Vec<OperationInstanceId>,
    completed_operation_frames: Vec<ContentDigest>,
) -> Result<spec::PlanningLineage> {
    let lineage_digest =
        planning_lineage_digest(&active_operation_instances, &completed_operation_frames)?;
    Ok(spec::PlanningLineage {
        active_operation_instances,
        completed_operation_frames,
        lineage_digest,
    })
}

pub(super) fn validate_planning_lineage(lineage: &spec::PlanningLineage) -> Result<()> {
    let expected = planning_lineage_digest(
        &lineage.active_operation_instances,
        &lineage.completed_operation_frames,
    )?;
    if lineage.lineage_digest != expected {
        return Err(problem(
            ProblemClass::InvalidDataMeaning,
            format!(
                "planning lineage {} is not content addressed",
                lineage.lineage_digest
            ),
        ));
    }
    Ok(())
}

pub(super) fn planning_lineage_digest(
    active_operation_instances: &[OperationInstanceId],
    completed_operation_frames: &[ContentDigest],
) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "active_instances": active_operation_instances
            .iter()
            .map(OperationInstanceId::as_str)
            .collect::<Vec<_>>(),
        "completed_frames": completed_operation_frames
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>(),
    }))
}

pub(super) fn scope_id_from_spec(
    parent_scope_id: Option<&ScopeId>,
    key: &str,
    operation_lineage: &spec::PlanningLineage,
) -> Result<ScopeId> {
    Ok(ScopeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "local_scope_key": key,
            "operation_lineage": operation_lineage.lineage_digest.as_str(),
            "parent_scope_id": parent_scope_id.map(ScopeId::as_str),
        }))?,
    ))
}

pub(super) fn seed_id_from_spec(seed: &spec::SeedSpec) -> Result<mfm_ids::SeedId> {
    Ok(mfm_ids::SeedId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "scope_id": seed.scope_id.as_str(),
            "seed_key": seed.seed_key.as_str(),
            "semantic_type_id": seed.semantic_type_id.as_str(),
            "schema_id": seed.schema_id.as_str(),
        }))?,
    ))
}

pub(super) fn stable_author_key(value: &str) -> Result<spec::StableAuthorKey> {
    spec::StableAuthorKey::new(value).map_err(|error| lower(error.to_string()))
}

pub(super) fn input_field_path(value: &str) -> Result<spec::PublicFieldPath> {
    let value = if value.is_empty() { "root" } else { value };
    spec::PublicFieldPath::new(value).map_err(|error| lower(error.to_string()))
}

pub(super) fn lineage_ref(digest: &ContentDigest) -> spec::ValueLineageRef {
    spec::ValueLineageRef {
        lineage_digest: digest.clone(),
    }
}

pub(super) fn value_lineage_digest(lineage: &spec::ValueLineage) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "config_ref_digest": lineage.config_ref_digest.as_ref().map(ContentDigest::as_str),
        "domain_keys": domain_key_refs_json(&sorted_domain_keys(lineage.domain_keys.clone())),
        "input_cells": sorted_cell_ids(lineage.input_cells.clone())
            .iter()
            .map(CellId::as_str)
            .collect::<Vec<_>>(),
        "operation_lineage": planning_lineage_json(&lineage.planning_lineage),
        "producer": cell_producer_json(&lineage.producer),
        "scope_id": lineage.scope_id.as_str(),
        "transform_policy": lineage_transform_policy_json(lineage.transform_policy),
    }))
}

pub(super) fn sorted_cell_ids(mut values: Vec<CellId>) -> Vec<CellId> {
    values.sort();
    values
}

pub(super) fn sorted_domain_keys(
    mut values: Vec<spec::StableDomainKeyRef>,
) -> Vec<spec::StableDomainKeyRef> {
    values.sort();
    values
}

pub(super) fn cell_producer_json(producer: &spec::CellProducer) -> serde_json::Value {
    match producer {
        spec::CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
        spec::CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
    }
}

pub(super) fn lineage_transform_policy_json(policy: spec::LineageTransformPolicy) -> &'static str {
    match policy {
        spec::LineageTransformPolicy::Source => "source",
        spec::LineageTransformPolicy::StateOutput => "state_output",
        spec::LineageTransformPolicy::SameValueBridge => "same_value_bridge",
    }
}

pub(super) fn config_ref_key(config_ref: &spec::ConfigRef) -> String {
    format!("{}:{}", config_ref.schema_id, config_ref.digest)
}

pub(super) fn config_ref_digest(config_ref: &spec::ConfigRef) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "byte_len": config_ref.byte_len,
        "content_digest": config_ref.digest.as_str(),
        "schema_id": config_ref.schema_id.as_str(),
    }))
}

pub(super) fn collect_input_cells(root: &spec::InputBindingNodeSpec) -> Vec<CellId> {
    let mut cells = Vec::new();
    collect_input_cells_into(root, &mut cells);
    cells
}

pub(super) fn collect_input_cells_into(
    root: &spec::InputBindingNodeSpec,
    output: &mut Vec<CellId>,
) {
    match root {
        spec::InputBindingNodeSpec::Unit => {}
        spec::InputBindingNodeSpec::Cell(cell) => output.push(cell.cell_id.clone()),
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                collect_input_cells_into(element, output);
            }
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                collect_input_cells_into(&field.node, output);
            }
        }
        spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells_into(element, output);
            }
        }
    }
}

pub(super) fn descriptor_ref_json(reference: spec::DescriptorRef) -> serde_json::Value {
    serde_json::json!({
        "descriptor_digest": reference.descriptor_digest.as_str(),
        "descriptor_family": reference.family.as_str(),
        "descriptor_id": reference.descriptor_id.as_str(),
    })
}

pub(super) fn context_validator_ref_json(validator: &ContextValidator) -> serde_json::Value {
    let requirement = &validator.requirement;
    serde_json::json!({
        "canonicalizer_identity": requirement.canonicalizer_identity.as_str(),
        "context_descriptor_id": requirement.context_descriptor_id.as_str(),
        "schema_id": requirement.schema_id.as_str(),
        "semantic_type_id": requirement.semantic_type_id.as_str(),
    })
}

pub(super) fn state_descriptor_ref_json(
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<serde_json::Value> {
    let reference = spec::DescriptorIdentity::State(Box::new(descriptor.clone()))
        .descriptor_ref()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    Ok(descriptor_ref_json(reference))
}

pub(super) fn operation_descriptor_ref_json(
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<serde_json::Value> {
    let reference = spec::DescriptorIdentity::Operation(Box::new(descriptor.clone()))
        .descriptor_ref()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    Ok(descriptor_ref_json(reference))
}

pub(super) fn canonical_json_bytes(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(&value).map_err(|error| canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|error| canonical(error.to_string()))
}

pub(super) fn content_digest_json(value: serde_json::Value) -> Result<ContentDigest> {
    Ok(canonical_json_bytes(value)?.content_digest())
}

pub(super) fn digest_bytes_json(value: serde_json::Value) -> Result<DigestBytes> {
    Ok(*content_digest_json(value)?.digest())
}
