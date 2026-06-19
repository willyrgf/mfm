use super::*;

pub(super) fn validate_state_node_contract(input: StateNodeContractInput<'_>) -> Result<()> {
    let StateNodeContractInput {
        node,
        descriptor,
        config_ref_digest,
        cells,
        lineages,
        label,
        id_derivation,
        enforce_framework_lineage_inputs,
    } = input;
    if descriptor.state_kind != node.state_kind
        || descriptor.state_version != node.state_version
        || descriptor.config_schema_id != node.config_ref.schema_id
        || descriptor.input_schema_id != node.input_bindings.input_schema_id
        || descriptor.effect_kind != node.effect_kind
        || descriptor.capabilities != node.capability_bindings
        || descriptor.side_effect_contract_digest
            != node
                .side_effect
                .as_ref()
                .map(|contract| contract.contract_digest.clone())
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("{label} {} descriptor metadata mismatch", node.node_id),
        ));
    }
    validate_effect_capabilities(&node.effect_kind, &node.capability_bindings)?;
    validate_side_effect_contract(node, descriptor)?;
    let input_cells = validate_input_binding(&node.input_bindings, cells)?;
    let expected_node_id = match id_derivation {
        StateNodeIdDerivation::FrameworkAware => match &node.framework {
            Some(spec::FrameworkNodeSpec::Bridge(bridge)) => {
                bridge_node_id_from_spec(node.stable_key.as_str(), bridge)?
            }
            Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => render_node_id(
                &node.scope_id,
                node.stable_key.as_str(),
                &render.output_spec_digest,
            )?,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
                project_retention_manifest_node_id_from_spec(
                    &node.scope_id,
                    node.stable_key.as_str(),
                    retention,
                )?
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(complete)) => {
                complete_run_node_id_from_spec(&node.scope_id, node.stable_key.as_str(), complete)?
            }
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(resolve)) => {
                resolve_saga_terminal_node_id_from_spec(
                    &node.scope_id,
                    node.stable_key.as_str(),
                    resolve,
                )?
            }
            None => state_node_id_from_spec(node, config_ref_digest)?,
        },
        StateNodeIdDerivation::StateOnly => state_node_id_from_spec(node, config_ref_digest)?,
    };
    if node.node_id != expected_node_id {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!("{label} {} is not stable-id derived", node.node_id),
        ));
    }
    let expected_predecessors = predecessor_nodes(&input_cells, cells)?;
    if expected_predecessors != node.deterministic_predecessors {
        return Err(problem(
        ProblemClass::InvalidTopology,
        format!(
            "{label} {} deterministic predecessors do not match input cells: expected {:?}, got {:?}",
            node.node_id, expected_predecessors, node.deterministic_predecessors
        ),
    ));
    }
    let output = cells.get(node.output_cell.as_str()).ok_or_else(|| {
        problem(
            ProblemClass::InvalidTopology,
            format!(
                "{label} {} output cell {} is missing",
                node.node_id, node.output_cell
            ),
        )
    })?;
    if output.producer != spec::CellProducer::Node(node.node_id.clone())
        || output.scope_id != node.scope_id
        || output.schema_id != descriptor.output_schema_id
        || output.semantic_type_id != descriptor.output_semantic_type_id
    {
        return Err(problem(
            ProblemClass::InvalidInterfaceWiring,
            format!("{label} {} output cell metadata mismatch", node.node_id),
        ));
    }
    let lineage = lineages
        .get(output.value_lineage.lineage_digest.as_str())
        .ok_or_else(|| {
            problem(
                ProblemClass::InvalidDataMeaning,
                format!("{label} {} output lineage is missing", node.node_id),
            )
        })?;
    if lineage.producer != spec::CellProducer::Node(node.node_id.clone()) {
        return Err(problem(
            ProblemClass::InvalidDataMeaning,
            format!("{label} {} output lineage producer mismatch", node.node_id),
        ));
    }
    let expected_lineage = expected_node_lineage(node, &input_cells, config_ref_digest)?;
    if enforce_framework_lineage_inputs
        && node.framework.is_some()
        && sorted_cell_ids(input_cells.clone())
            != sorted_cell_ids(expected_lineage.input_cells.clone())
    {
        return Err(problem(
            ProblemClass::InvalidInterfaceWiring,
            format!("framework node {} input binding mismatch", node.node_id),
        ));
    }
    if lineage.config_ref_digest != expected_lineage.config_ref_digest
        || sorted_cell_ids(lineage.input_cells.clone())
            != sorted_cell_ids(expected_lineage.input_cells)
        || lineage.transform_policy != expected_lineage.transform_policy
    {
        return Err(problem(
            ProblemClass::InvalidDataMeaning,
            format!(
                "{label} {} output lineage input/config mismatch",
                node.node_id
            ),
        ));
    }
    Ok(())
}
