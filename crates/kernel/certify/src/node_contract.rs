use super::*;

pub(super) fn validate_state_node_contract(input: StateNodeContractInput<'_>) -> Result<()> {
    let StateNodeContractInput {
        nodes,
        node,
        descriptor,
        contexts,
        config_ref_digest,
        cells,
        lineages,
        remediations,
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
    validate_node_context_contract(node, descriptor, contexts, label)?;
    validate_effect_capabilities(&node.effect_kind, &node.capability_bindings)?;
    validate_side_effect_contract(node, descriptor)?;
    let input_cells = validate_input_binding(&node.input_bindings, cells)?;
    if node.framework.is_none() {
        validate_user_state_input_contexts(
            node,
            descriptor,
            &node.input_bindings.root,
            cells,
            lineages,
            nodes,
            remediations,
            label,
        )?;
    }
    let expected_node_id = match id_derivation {
        StateNodeIdDerivation::FrameworkAware => match &node.framework {
            Some(spec::FrameworkNodeSpec::Bridge(bridge)) => {
                bridge_node_id_from_spec(node.stable_key.as_str(), bridge)?
            }
            Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => {
                spec::side_effect_verify_node_id(&verify.submit_node_id, &verify.pair_id)
                    .map_err(|error| CertifyError::Spec(error.to_string()))?
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
    let remediation_verify_without_graph_predecessor = matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::SideEffectVerify(verify))
            if node.deterministic_predecessors.is_empty()
                && expected_predecessors == vec![verify.submit_node_id.clone()]
                && remediations
                    .is_some_and(|remediations| remediations
                        .values()
                        .any(|remediation| remediation.node_id == verify.submit_node_id))
    );
    if !remediation_verify_without_graph_predecessor
        && expected_predecessors != node.deterministic_predecessors
    {
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
    if node.framework.is_none() {
        validate_user_state_output_context(node, descriptor, output, label)?;
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
    let expected_lineage =
        expected_node_lineage(node, &input_cells, config_ref_digest, nodes, remediations)?;
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

fn validate_node_context_contract(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    contexts: &ContextIndex<'_>,
    label: &str,
) -> Result<()> {
    match (&descriptor.context, &node.context) {
        (spec::StateContextDescriptorSpec::NoContext, spec::NodeContextSpec::NoContext) => Ok(()),
        (
            spec::StateContextDescriptorSpec::NoContext,
            spec::NodeContextSpec::Required { context_ref },
        ) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "{label} {} declares context {} for a no-context state descriptor",
                node.node_id, context_ref
            ),
        )),
        (spec::StateContextDescriptorSpec::Required(_), spec::NodeContextSpec::NoContext) => {
            Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!("{label} {} is missing required state context", node.node_id),
            ))
        }
        (
            spec::StateContextDescriptorSpec::Required(requirement),
            spec::NodeContextSpec::Required { context_ref },
        ) => {
            let context = contexts.get(context_ref)?;
            if context.context_descriptor_id != requirement.context_descriptor_id
                || context.schema_id != requirement.schema_id
                || context.semantic_type_id != requirement.semantic_type_id
                || context.canonicalizer_identity != requirement.canonicalizer_identity
            {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "{label} {} context {} does not match registered state descriptor",
                        node.node_id, context_ref
                    ),
                ));
            }
            Ok(())
        }
    }
}

fn validate_user_state_output_context(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    output: &spec::CellSpec,
    label: &str,
) -> Result<()> {
    match (&descriptor.output_context, &node.context, &output.context) {
        (spec::StateOutputContextContractSpec::NoContext, _, spec::CellContextSpec::NoContext) => {
            Ok(())
        }
        (
            spec::StateOutputContextContractSpec::NoContext,
            _,
            spec::CellContextSpec::Bound { .. },
        ) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "{label} {} output cell is context-bound for a no-context output descriptor",
                node.node_id
            ),
        )),
        (
            spec::StateOutputContextContractSpec::Produces { .. },
            spec::NodeContextSpec::NoContext,
            _,
        ) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "{label} {} produces a context-bound output without node context",
                node.node_id
            ),
        )),
        (
            spec::StateOutputContextContractSpec::Produces {
                resource_kind,
                stage,
            },
            spec::NodeContextSpec::Required { context_ref },
            spec::CellContextSpec::Bound {
                context_ref: output_context_ref,
                resource_kind: output_resource_kind,
                stage: output_stage,
                producer,
            },
        ) => {
            if output_context_ref != context_ref
                || output_resource_kind != resource_kind
                || output_stage != stage
                || producer.producer_descriptor_ids.as_slice()
                    != std::slice::from_ref(&descriptor.descriptor_id)
                || producer.seed_producers_allowed
            {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "{label} {} output context does not match state descriptor contract",
                        node.node_id
                    ),
                ));
            }
            Ok(())
        }
        (
            spec::StateOutputContextContractSpec::Produces { .. },
            spec::NodeContextSpec::Required { .. },
            spec::CellContextSpec::NoContext,
        ) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "{label} {} output cell is missing required context binding",
                node.node_id
            ),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_user_state_input_contexts(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    root: &spec::InputBindingNodeSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
    nodes: &[spec::NodeSpec],
    remediations: Option<&BTreeMap<NodeId, spec::NodeSpec>>,
    label: &str,
) -> Result<()> {
    let mut cell_count = 0_usize;
    validate_user_state_input_context_node(
        node,
        descriptor,
        root,
        cells,
        lineages,
        nodes,
        remediations,
        label,
        &mut cell_count,
    )?;
    if matches!(
        descriptor.input_context,
        spec::StateInputContextContractSpec::Required { .. }
    ) && cell_count == 0
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "{label} {} requires context-bound input but has no input cells",
                node.node_id
            ),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_user_state_input_context_node(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    input: &spec::InputBindingNodeSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
    nodes: &[spec::NodeSpec],
    remediations: Option<&BTreeMap<NodeId, spec::NodeSpec>>,
    label: &str,
    cell_count: &mut usize,
) -> Result<()> {
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(()),
        spec::InputBindingNodeSpec::Cell(cell) => {
            *cell_count += 1;
            let produced = cells.get(cell.cell_id.as_str()).ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!("input binding references missing cell {}", cell.cell_id),
                )
            })?;
            validate_user_state_input_context_cell(
                node,
                descriptor,
                cell,
                produced,
                cells,
                lineages,
                nodes,
                remediations,
                label,
            )
        }
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                validate_user_state_input_context_node(
                    node,
                    descriptor,
                    element,
                    cells,
                    lineages,
                    nodes,
                    remediations,
                    label,
                    cell_count,
                )?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                validate_user_state_input_context_node(
                    node,
                    descriptor,
                    &field.node,
                    cells,
                    lineages,
                    nodes,
                    remediations,
                    label,
                    cell_count,
                )?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                validate_user_state_input_context_node(
                    node,
                    descriptor,
                    element,
                    cells,
                    lineages,
                    nodes,
                    remediations,
                    label,
                    cell_count,
                )?;
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_user_state_input_context_cell(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    input: &spec::InputBindingCellSpec,
    produced: &spec::CellSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
    nodes: &[spec::NodeSpec],
    remediations: Option<&BTreeMap<NodeId, spec::NodeSpec>>,
    label: &str,
) -> Result<()> {
    match (&descriptor.input_context, &node.context, &input.context) {
        (spec::StateInputContextContractSpec::NoContext, _, spec::InputContextSpec::NoContext) => {
            Ok(())
        }
        (
            spec::StateInputContextContractSpec::NoContext,
            _,
            spec::InputContextSpec::Required { .. },
        ) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "{label} {} consumes a context-bound input without a descriptor contract",
                node.node_id
            ),
        )),
        (
            spec::StateInputContextContractSpec::Required { .. },
            spec::NodeContextSpec::NoContext,
            _,
        ) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "{label} {} requires context-bound input without node context",
                node.node_id
            ),
        )),
        (
            spec::StateInputContextContractSpec::Required { .. },
            spec::NodeContextSpec::Required { .. },
            spec::InputContextSpec::NoContext,
        ) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "{label} {} input cell {} is missing required context binding",
                node.node_id, input.cell_id
            ),
        )),
        (
            spec::StateInputContextContractSpec::Required {
                resource_kind,
                stage,
                producer,
            },
            spec::NodeContextSpec::Required { context_ref },
            spec::InputContextSpec::Required {
                context_ref: input_context_ref,
                resource_kind: input_resource_kind,
                stage: input_stage,
                producer: input_producer,
            },
        ) => {
            if input_context_ref != context_ref
                || input_resource_kind != resource_kind
                || input_stage != stage
                || input_producer != producer
            {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "{label} {} input cell {} context does not match state descriptor contract",
                        node.node_id, input.cell_id
                    ),
                ));
            }
            validate_context_cell_producer(
                produced,
                input_producer,
                cells,
                lineages,
                nodes,
                remediations,
                label,
            )
        }
    }
}

fn validate_context_cell_producer(
    cell: &spec::CellSpec,
    producer: &spec::ContextProducerSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
    nodes: &[spec::NodeSpec],
    remediations: Option<&BTreeMap<NodeId, spec::NodeSpec>>,
    label: &str,
) -> Result<()> {
    validate_context_cell_producer_inner(
        cell,
        producer,
        cells,
        lineages,
        nodes,
        remediations,
        label,
        &mut BTreeSet::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_context_cell_producer_inner(
    cell: &spec::CellSpec,
    producer: &spec::ContextProducerSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
    nodes: &[spec::NodeSpec],
    remediations: Option<&BTreeMap<NodeId, spec::NodeSpec>>,
    label: &str,
    seen: &mut BTreeSet<CellId>,
) -> Result<()> {
    if !seen.insert(cell.cell_id.clone()) {
        return Err(problem(
            ProblemClass::InvalidDataMeaning,
            format!(
                "{label} context cell {} has cyclic bridge lineage",
                cell.cell_id
            ),
        ));
    }
    let lineage = lineages
        .get(cell.value_lineage.lineage_digest.as_str())
        .ok_or_else(|| {
            problem(
                ProblemClass::InvalidDataMeaning,
                format!("{label} context cell {} lineage is missing", cell.cell_id),
            )
        })?;
    if lineage.transform_policy == spec::LineageTransformPolicy::SameValueBridge {
        let [source_cell_id] = lineage.input_cells.as_slice() else {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "{label} context bridge cell {} must have exactly one source",
                    cell.cell_id
                ),
            ));
        };
        let source = cells.get(source_cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!("{label} context bridge source cell {source_cell_id} is missing"),
            )
        })?;
        if source.context != cell.context {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "{label} context bridge cell {} does not preserve source context",
                    cell.cell_id
                ),
            ));
        }
        return validate_context_cell_producer_inner(
            source,
            producer,
            cells,
            lineages,
            nodes,
            remediations,
            label,
            seen,
        );
    }
    match &cell.producer {
        spec::CellProducer::Seed(seed_id) => {
            if producer.seed_producers_allowed {
                Ok(())
            } else {
                Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "{label} context-bound cell {} was produced by unauthorized seed {}",
                        cell.cell_id, seed_id
                    ),
                ))
            }
        }
        spec::CellProducer::Node(node_id) => {
            let producer_node =
                find_producer_node(node_id, nodes, remediations).ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "{label} context-bound cell {} references missing producer node {}",
                            cell.cell_id, node_id
                        ),
                    )
                })?;
            if let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) =
                &producer_node.framework
            {
                let submit_node = find_producer_node(&verify.submit_node_id, nodes, remediations)
                    .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "{label} side-effect verify cell {} references missing submit node {}",
                            cell.cell_id, verify.submit_node_id
                        ),
                    )
                })?;
                let submit_output =
                    cells.get(submit_node.output_cell.as_str()).ok_or_else(|| {
                        problem(
                            ProblemClass::InvalidTopology,
                            format!(
                                "{label} side-effect submit output cell {} is missing",
                                submit_node.output_cell
                            ),
                        )
                    })?;
                if submit_output.context != cell.context {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "{label} side-effect verify cell {} does not preserve submit context",
                            cell.cell_id
                        ),
                    ));
                }
                return validate_context_cell_producer_inner(
                    submit_output,
                    producer,
                    cells,
                    lineages,
                    nodes,
                    remediations,
                    label,
                    seen,
                );
            }
            if producer.producer_descriptor_ids.is_empty() {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "{label} context-bound cell {} has no authorized producer descriptor",
                        cell.cell_id
                    ),
                ));
            }
            if !producer
                .producer_descriptor_ids
                .contains(&producer_node.descriptor_id)
            {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "{label} context-bound cell {} was produced by unapproved descriptor {}",
                        cell.cell_id, producer_node.descriptor_id
                    ),
                ));
            }
            Ok(())
        }
    }
}

fn find_producer_node<'a>(
    node_id: &NodeId,
    nodes: &'a [spec::NodeSpec],
    remediations: Option<&'a BTreeMap<NodeId, spec::NodeSpec>>,
) -> Option<&'a spec::NodeSpec> {
    nodes
        .iter()
        .chain(remediations.into_iter().flat_map(BTreeMap::values))
        .find(|candidate| candidate.node_id == *node_id)
}
