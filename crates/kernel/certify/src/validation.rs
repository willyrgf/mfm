use super::*;

#[path = "validation_inputs.rs"]
mod validation_inputs;
use self::validation_inputs::{
    validate_cells, validate_config_refs, validate_contexts, validate_contract_header,
    validate_descriptor_authority, validate_scopes, validate_seeds, validate_value_lineages,
    ContextIndex,
};
pub(super) use self::validation_inputs::{ConfigIndex, DescriptorIndex};

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
