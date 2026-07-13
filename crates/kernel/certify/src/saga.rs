use super::*;

pub(super) struct SagaValidationContext<'a, 'd> {
    pub(super) typed: &'a spec::TypedExecutionSpec,
    pub(super) scope_ids: &'a BTreeSet<String>,
    pub(super) descriptors: &'a DescriptorIndex<'d>,
    pub(super) contexts: &'a ContextIndex<'a>,
    pub(super) config_refs: &'a ConfigIndex,
    pub(super) cells: &'a BTreeMap<String, spec::CellSpec>,
    pub(super) lineages: &'a BTreeMap<String, spec::ValueLineage>,
    pub(super) forward_nodes: &'a BTreeMap<String, spec::NodeSpec>,
    pub(super) registry: &'a CertificationRegistry,
}

pub(super) fn validate_saga_structure(context: SagaValidationContext<'_, '_>) -> Result<()> {
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
