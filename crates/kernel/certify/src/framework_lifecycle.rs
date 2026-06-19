use super::*;

pub(super) fn validate_framework_nodes(
    nodes: &[spec::NodeSpec],
    cells: &BTreeMap<String, spec::CellSpec>,
    descriptors: &DescriptorIndex<'_>,
    public_outputs: &spec::PublicOutputSpec,
) -> Result<()> {
    let mut retention_count = 0_usize;
    let mut completion_count = 0_usize;
    let mut resolve_count = 0_usize;
    let mut lifecycle_outputs = BTreeMap::<CellId, &spec::NodeSpec>::new();
    let mut all_input_cells = BTreeMap::<CellId, Vec<NodeId>>::new();
    let mut nodes_by_id = BTreeMap::<NodeId, &spec::NodeSpec>::new();
    for node in nodes {
        nodes_by_id.insert(node.node_id.clone(), node);
        for cell_id in validate_input_binding(&node.input_bindings, cells)? {
            all_input_cells
                .entry(cell_id)
                .or_default()
                .push(node.node_id.clone());
        }
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
                    | spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
            )
        ) {
            lifecycle_outputs.insert(node.output_cell.clone(), node);
        }
    }

    for node in nodes {
        if let Some(framework) = &node.framework {
            validate_framework_node_permissions(node)?;
            validate_framework_config_ref(node, framework.config_kind())?;
        }
        match &node.framework {
            Some(spec::FrameworkNodeSpec::Bridge(bridge)) => {
                let descriptor = descriptors.state(&node.descriptor_id)?;
                validate_framework_descriptor_name(
                    node,
                    descriptor,
                    "mfm.framework.bridge_same_value",
                )?;
                if bridge.target_cell_id != node.output_cell
                    || bridge.target_scope_id != node.scope_id
                    || bridge.schema_id
                        != cells
                            .get(node.output_cell.as_str())
                            .map(|cell| cell.schema_id.clone())
                            .ok_or_else(|| {
                                problem(
                                    ProblemClass::InvalidTopology,
                                    format!("bridge node {} missing target cell", node.node_id),
                                )
                            })?
                    || bridge.semantic_type_id
                        != cells
                            .get(node.output_cell.as_str())
                            .map(|cell| cell.semantic_type_id.clone())
                            .ok_or_else(|| {
                                problem(
                                    ProblemClass::InvalidTopology,
                                    format!("bridge node {} missing target cell", node.node_id),
                                )
                            })?
                {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!("bridge node {} metadata mismatch", node.node_id),
                    ));
                }
                if !cells.contains_key(bridge.source_cell_id.as_str()) {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!("bridge node {} source cell is missing", node.node_id),
                    ));
                }
            }
            Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => {
                let descriptor = descriptors.state(&node.descriptor_id)?;
                validate_framework_descriptor_name(
                    node,
                    descriptor,
                    "mfm.framework.render_public_outputs",
                )?;
                descriptors.renderer(&render.renderer_descriptor.descriptor_id)?;
                validate_framework_output_cell(
                    node,
                    cells,
                    &spec::public_output_receipt_schema_id()
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                    &spec::public_output_receipt_semantic_type_id()
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                    spec::StoragePolicy::PublicOutputArtifact,
                )?;
                validate_framework_receipt_consumers(
                    node,
                    &all_input_cells,
                    &nodes_by_id,
                    "project-retention-manifest lifecycle node",
                    |consumer| {
                        matches!(
                            &consumer.framework,
                            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention))
                                if retention.public_output_receipt_cell == node.output_cell
                        )
                    },
                )?;
            }
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
                retention_count += 1;
                let descriptor = descriptors.state(&node.descriptor_id)?;
                validate_framework_descriptor_name(
                    node,
                    descriptor,
                    "mfm.framework.project_retention_manifest",
                )?;
                if retention.public_schema_id != public_outputs.public_schema_id {
                    return Err(problem(
                        ProblemClass::InvalidTerminalShape,
                        format!(
                            "retention lifecycle node {} public schema mismatch",
                            node.node_id
                        ),
                    ));
                }
                validate_framework_output_cell(
                    node,
                    cells,
                    &spec::retention_manifest_receipt_schema_id()
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                    &spec::retention_manifest_receipt_semantic_type_id()
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                    spec::StoragePolicy::ContentAddressed,
                )?;
                let public_output_receipt_cell = cells
                    .get(retention.public_output_receipt_cell.as_str())
                    .ok_or_else(|| {
                        problem(
                            ProblemClass::InvalidTopology,
                            format!(
                                "retention lifecycle node {} missing public-output receipt cell",
                                node.node_id
                            ),
                        )
                    })?;
                validate_framework_input_binding(
                    node,
                    &spec::framework_lifecycle_receipt_input_binding(
                        "project_retention_manifest",
                        "public_output_receipt",
                        public_output_receipt_cell,
                    )
                    .map_err(|error| CertifyError::Spec(error.to_string()))?,
                )?;
                let input_cells = validate_input_binding(&node.input_bindings, cells)?;
                if input_cells != vec![retention.public_output_receipt_cell.clone()] {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!(
                        "retention lifecycle node {} must depend on the public-output receipt cell",
                        node.node_id
                    ),
                    ));
                }
                let render_node = nodes.iter().find(|candidate| {
                    candidate.output_cell == retention.public_output_receipt_cell
                        && matches!(
                            &candidate.framework,
                            Some(spec::FrameworkNodeSpec::PublicOutputRender(render))
                                if render.public_schema_id == retention.public_schema_id
                        )
                });
                if render_node.is_none() {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "retention lifecycle node {} is not ordered after public-output render",
                            node.node_id
                        ),
                    ));
                }
                validate_framework_receipt_consumers(
                    node,
                    &all_input_cells,
                    &nodes_by_id,
                    "complete-run lifecycle node",
                    |consumer| {
                        matches!(
                            &consumer.framework,
                            Some(spec::FrameworkNodeSpec::CompleteRun(complete))
                                if complete.retention_manifest_receipt_cell == node.output_cell
                        )
                    },
                )?;
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(complete)) => {
                completion_count += 1;
                let descriptor = descriptors.state(&node.descriptor_id)?;
                validate_framework_descriptor_name(node, descriptor, "mfm.framework.complete_run")?;
                if complete.public_schema_id != public_outputs.public_schema_id {
                    return Err(problem(
                        ProblemClass::InvalidTerminalShape,
                        format!(
                            "completion lifecycle node {} public schema mismatch",
                            node.node_id
                        ),
                    ));
                }
                validate_framework_output_cell(
                    node,
                    cells,
                    &spec::complete_run_receipt_schema_id()
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                    &spec::complete_run_receipt_semantic_type_id()
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                    spec::StoragePolicy::ContentAddressed,
                )?;
                let retention_manifest_receipt_cell = cells
                    .get(complete.retention_manifest_receipt_cell.as_str())
                    .ok_or_else(|| {
                        problem(
                            ProblemClass::InvalidTopology,
                            format!(
                                "completion lifecycle node {} missing retention receipt cell",
                                node.node_id
                            ),
                        )
                    })?;
                validate_framework_input_binding(
                    node,
                    &spec::framework_lifecycle_receipt_input_binding(
                        "complete_run",
                        "retention_manifest_receipt",
                        retention_manifest_receipt_cell,
                    )
                    .map_err(|error| CertifyError::Spec(error.to_string()))?,
                )?;
                let input_cells = validate_input_binding(&node.input_bindings, cells)?;
                if input_cells != vec![complete.retention_manifest_receipt_cell.clone()] {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!(
                        "completion lifecycle node {} must depend on the retention receipt cell",
                        node.node_id
                    ),
                    ));
                }
                let retention_node = lifecycle_outputs
                    .get(&complete.retention_manifest_receipt_cell)
                    .filter(|candidate| {
                        matches!(
                            &candidate.framework,
                            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention))
                                if retention.public_schema_id == complete.public_schema_id
                        )
                    });
                if retention_node.is_none() {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!(
                        "completion lifecycle node {} is not ordered after retention projection",
                        node.node_id
                    ),
                    ));
                }
                validate_no_framework_receipt_consumers(node, &all_input_cells)?;
            }
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(resolve)) => {
                resolve_count += 1;
                let descriptor = descriptors.state(&node.descriptor_id)?;
                validate_framework_descriptor_name(
                    node,
                    descriptor,
                    "mfm.framework.resolve_saga_terminal",
                )?;
                if resolve.public_schema_id != public_outputs.public_schema_id {
                    return Err(problem(
                        ProblemClass::InvalidTerminalShape,
                        format!(
                            "resolve-saga-terminal lifecycle node {} public schema mismatch",
                            node.node_id
                        ),
                    ));
                }
                validate_framework_output_cell(
                    node,
                    cells,
                    &spec::resolve_saga_terminal_receipt_schema_id()
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                    &spec::resolve_saga_terminal_receipt_semantic_type_id()
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                    spec::StoragePolicy::ContentAddressed,
                )?;
                validate_framework_input_binding(
                    node,
                    &spec::framework_lifecycle_unit_input_binding("resolve_saga_terminal")
                        .map_err(|error| CertifyError::Spec(error.to_string()))?,
                )?;
                let input_cells = validate_input_binding(&node.input_bindings, cells)?;
                if !input_cells.is_empty() || !node.deterministic_predecessors.is_empty() {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "resolve-saga-terminal lifecycle node {} must not have inputs",
                            node.node_id
                        ),
                    ));
                }
                validate_no_framework_receipt_consumers(node, &all_input_cells)?;
            }
            None => {}
        }
    }
    if retention_count != 1 {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!(
                "expected exactly one retention lifecycle framework node, found {retention_count}"
            ),
        ));
    }
    if completion_count != 1 {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!(
            "expected exactly one completion lifecycle framework node, found {completion_count}"
        ),
        ));
    }
    if resolve_count != 1 {
        return Err(problem(
        ProblemClass::InvalidTopology,
        format!(
            "expected exactly one resolve-saga-terminal lifecycle framework node, found {resolve_count}"
        ),
    ));
    }
    validate_lifecycle_tail_finality(nodes, &nodes_by_id)?;
    Ok(())
}

fn validate_lifecycle_tail_finality(
    nodes: &[spec::NodeSpec],
    nodes_by_id: &BTreeMap<NodeId, &spec::NodeSpec>,
) -> Result<()> {
    let render_node = nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                "missing public-output render node".to_owned(),
            )
        })?;
    let mut render_ancestors = BTreeSet::new();
    collect_deterministic_ancestors(render_node, nodes_by_id, &mut render_ancestors)?;
    for node in nodes {
        if node.node_id == render_node.node_id || render_ancestors.contains(&node.node_id) {
            continue;
        }
        match &node.framework {
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                | spec::FrameworkNodeSpec::CompleteRun(_)
                | spec::FrameworkNodeSpec::ResolveSagaTerminal(_),
            ) => {}
            _ => {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "node {} is outside the certified public-output lifecycle tail",
                        node.node_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn collect_deterministic_ancestors(
    node: &spec::NodeSpec,
    nodes_by_id: &BTreeMap<NodeId, &spec::NodeSpec>,
    ancestors: &mut BTreeSet<NodeId>,
) -> Result<()> {
    for predecessor_id in &node.deterministic_predecessors {
        if ancestors.insert(predecessor_id.clone()) {
            let predecessor = nodes_by_id.get(predecessor_id).ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!("node graph references missing predecessor {predecessor_id}"),
                )
            })?;
            collect_deterministic_ancestors(predecessor, nodes_by_id, ancestors)?;
        }
    }
    Ok(())
}

fn validate_framework_node_permissions(node: &spec::NodeSpec) -> Result<()> {
    if node.side_effect.is_some()
        || !node.adapter_bindings.is_empty()
        || !node.capability_bindings.capabilities.is_empty()
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "framework node {} carries user-controlled execution permissions",
                node.node_id
            ),
        ));
    }
    Ok(())
}

pub(super) fn validate_framework_config_ref(node: &spec::NodeSpec, kind: &str) -> Result<()> {
    let expected = framework_config_ref(kind, &node.node_id)?;
    if node.config_ref != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "framework node {} config ref is not deterministic for {kind}",
                node.node_id
            ),
        ));
    }
    Ok(())
}

fn validate_framework_input_binding(
    node: &spec::NodeSpec,
    expected: &spec::InputBindingSpec,
) -> Result<()> {
    if &node.input_bindings != expected {
        return Err(problem(
            ProblemClass::InvalidInterfaceWiring,
            format!(
                "framework node {} input binding is not deterministic",
                node.node_id
            ),
        ));
    }
    Ok(())
}

pub(super) fn validate_framework_descriptor_variant(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<()> {
    let matches_variant = match descriptor.name.as_str() {
        "mfm.framework.bridge_same_value" => {
            matches!(&node.framework, Some(spec::FrameworkNodeSpec::Bridge(_)))
        }
        "mfm.framework.render_public_outputs" => {
            matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        }
        "mfm.framework.project_retention_manifest" => {
            matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        }
        "mfm.framework.complete_run" => {
            matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::CompleteRun(_))
            )
        }
        "mfm.framework.resolve_saga_terminal" => {
            matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        }
        _ => return Ok(()),
    };
    if !matches_variant {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "node {} uses built-in framework descriptor {} without matching framework metadata",
                node.node_id, descriptor.name
            ),
        ));
    }
    Ok(())
}

fn validate_no_framework_receipt_consumers(
    node: &spec::NodeSpec,
    consumers_by_cell: &BTreeMap<CellId, Vec<NodeId>>,
) -> Result<()> {
    if consumers_by_cell
        .get(&node.output_cell)
        .map(|consumers| !consumers.is_empty())
        .unwrap_or(false)
    {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!(
                "framework receipt cell {} must not be consumed",
                node.output_cell
            ),
        ));
    }
    Ok(())
}

fn validate_framework_receipt_consumers(
    node: &spec::NodeSpec,
    consumers_by_cell: &BTreeMap<CellId, Vec<NodeId>>,
    nodes_by_id: &BTreeMap<NodeId, &spec::NodeSpec>,
    expected_consumer: &str,
    mut allowed: impl FnMut(&spec::NodeSpec) -> bool,
) -> Result<()> {
    for consumer_id in consumers_by_cell
        .get(&node.output_cell)
        .into_iter()
        .flatten()
    {
        let consumer = nodes_by_id.get(consumer_id).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!(
                    "framework receipt cell {} has missing consumer node {}",
                    node.output_cell, consumer_id
                ),
            )
        })?;
        if !allowed(consumer) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "framework receipt cell {} may only feed {expected_consumer}",
                    node.output_cell
                ),
            ));
        }
    }
    Ok(())
}

fn validate_framework_descriptor_name(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    expected_name: &str,
) -> Result<()> {
    if descriptor.name != expected_name {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "framework node {} descriptor is not {expected_name}",
                node.node_id
            ),
        ));
    }
    Ok(())
}

fn validate_framework_output_cell(
    node: &spec::NodeSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    expected_schema_id: &SchemaId,
    expected_semantic_type_id: &SemanticTypeId,
    expected_storage_policy: spec::StoragePolicy,
) -> Result<()> {
    let output = cells.get(node.output_cell.as_str()).ok_or_else(|| {
        problem(
            ProblemClass::InvalidTopology,
            format!("framework node {} missing output cell", node.node_id),
        )
    })?;
    if output.schema_id != *expected_schema_id
        || output.semantic_type_id != *expected_semantic_type_id
        || output.terminal_policy != spec::CellTerminalPolicy::ProducedOnly
        || output.storage_policy != expected_storage_policy
        || output.redaction_policy != spec::RedactionPolicy::Public
    {
        return Err(problem(
            ProblemClass::InvalidTerminalShape,
            format!(
                "framework node {} output cell contract mismatch",
                node.node_id
            ),
        ));
    }
    Ok(())
}
