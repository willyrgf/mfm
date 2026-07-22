use super::*;

#[cfg(test)]
pub(super) fn raw_stream_requires_artifact_byte_authority(
    stream: &[store::KernelEventEnvelope],
) -> bool {
    stream.iter().any(|event| match event.payload() {
        events::KernelEventPayload::RunAdmitted(payload) => {
            !payload.fact_descriptor_artifacts.is_empty()
        }
        events::KernelEventPayload::FactRecorded(_) => true,
        events::KernelEventPayload::ArtifactReferenced(payload) => matches!(
            payload.artifact_ref.role,
            events::ArtifactRole::FactDescriptor | events::ArtifactRole::ExternalReadEvidence
        ),
        _ => false,
    })
}

pub(crate) fn materialize_inputs(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputs> {
    Ok(MaterializedInputs {
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        root: materialize_input_node(runtime_spec, node, &node.input_bindings.root, view)?,
    })
}

fn materialize_input_node(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    input: &spec::InputBindingNodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputNode> {
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(MaterializedInputNode::Unit),
        spec::InputBindingNodeSpec::Cell(cell) => Ok(MaterializedInputNode::Cell(Box::new(
            materialize_cell(runtime_spec, node, cell, view)?,
        ))),
        spec::InputBindingNodeSpec::Tuple(elements) => Ok(MaterializedInputNode::Tuple(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, node, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Struct(fields) => Ok(MaterializedInputNode::Struct(
            fields
                .iter()
                .map(|field| {
                    Ok(NamedMaterializedInput {
                        field_path: field.field_path.clone(),
                        node: materialize_input_node(runtime_spec, node, &field.node, view)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Vec { elements, .. } => Ok(MaterializedInputNode::Vec(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, node, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            Ok(MaterializedInputNode::NonEmptyVec(
                elements
                    .iter()
                    .map(|element| materialize_input_node(runtime_spec, node, element, view))
                    .collect::<Result<Vec<_>>>()?,
            ))
        }
    }
}

fn materialize_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    cell: &spec::InputBindingCellSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedCell> {
    let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {} is not certified by the spec",
            cell.cell_id
        ))
    })?;
    if certified.schema_id != cell.schema_id
        || certified.semantic_type_id != cell.semantic_type_id
        || certified.value_lineage != cell.value_lineage
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "input cell {} metadata does not match certified cell",
            cell.cell_id
        )));
    }
    validate_materialized_input_context(node, cell, certified)?;
    let terminal = match &certified.producer {
        spec::CellProducer::Seed(seed_id) => {
            let seed = view.seed_cells.get(&cell.cell_id).ok_or_else(|| {
                RuntimeError::InputMaterialization(format!(
                    "seed cell {} has no RunAdmitted evidence",
                    cell.cell_id
                ))
            })?;
            let seed_artifact = committed_input_artifact(
                view,
                &cell.cell_id,
                &seed.seed_artifact.artifact_id,
                &seed.seed_artifact.evidence_hash,
            )?;
            if seed_artifact.evidence.digest != seed.digest
                || seed_artifact.evidence.byte_len != seed.seed_artifact.byte_len
                || seed_artifact.evidence.media_type != seed.seed_artifact.media_type
                || seed_artifact.evidence.schema_id.as_ref() != Some(&seed.schema_id)
                || seed_artifact.evidence.semantic_type_id.as_ref() != Some(&seed.semantic_type_id)
                || seed_artifact.evidence.producer_node_id.is_some()
                || seed_artifact.evidence.producer_seed_id.as_ref() != Some(seed_id)
                || seed_artifact.evidence.artifact_role != events::ArtifactRole::SeedInput
            {
                return Err(RuntimeError::InputMaterialization(format!(
                    "seed cell {} committed artifact evidence does not match certified seed",
                    cell.cell_id
                )));
            }
            MaterializedCellTerminal::Seed {
                seed_id: seed_id.clone(),
                artifact_id: seed.seed_artifact.artifact_id.clone(),
                content_digest: seed.digest.clone(),
                evidence_hash: seed.seed_artifact.evidence_hash.clone(),
            }
        }
        spec::CellProducer::Node(_) => {
            let projection = view
                .projections
                .cell_terminal_for_run(&view.run_admitted.run_id, &cell.cell_id)
                .ok_or_else(|| {
                    RuntimeError::InputMaterialization(format!(
                        "input cell {} is not terminal",
                        cell.cell_id
                    ))
                })?;
            match projection {
                store::CellTerminalProjection::Produced {
                    node_id,
                    attempt_id,
                    schema_id,
                    semantic_type_id,
                    artifact_id,
                    content_digest,
                    evidence_hash,
                    ..
                } => {
                    if certified.producer != spec::CellProducer::Node(node_id.clone())
                        || schema_id != &cell.schema_id
                        || semantic_type_id != &cell.semantic_type_id
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    match view.projections.attempt(node_id, attempt_id) {
                        Some(store::AttemptProjection {
                            status: store::AttemptStatus::Completed { output_cell_id },
                            ..
                        }) if output_cell_id == &cell.cell_id => {}
                        _ => {
                            return Err(RuntimeError::InputMaterialization(format!(
                                "produced cell {} is not backed by a completed producer attempt",
                                cell.cell_id
                            )));
                        }
                    }
                    let artifact =
                        committed_input_artifact(view, &cell.cell_id, artifact_id, evidence_hash)?;
                    if artifact.evidence.digest != *content_digest
                        || artifact.evidence.evidence_hash()? != *evidence_hash
                        || artifact.evidence.schema_id.as_ref() != Some(schema_id)
                        || artifact.evidence.semantic_type_id.as_ref() != Some(semantic_type_id)
                        || artifact.evidence.producer_node_id.as_ref() != Some(node_id)
                        || artifact.evidence.producer_seed_id.is_some()
                        || artifact.evidence.artifact_role != events::ArtifactRole::StateOutput
                        || artifact.attempt_id.as_ref() != Some(attempt_id)
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} committed artifact evidence does not match terminal projection",
                            cell.cell_id
                        )));
                    }
                    MaterializedCellTerminal::Produced {
                        producer_node_id: node_id.clone(),
                        artifact_id: artifact_id.clone(),
                        content_digest: content_digest.clone(),
                        evidence_hash: evidence_hash.clone(),
                    }
                }
                store::CellTerminalProjection::Skipped {
                    node_id,
                    attempt_id,
                    schema_id,
                    semantic_type_id,
                    skip_reason,
                    ..
                } => {
                    if cell.required_terminal == spec::RequiredTerminal::ProducedOnly {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "input cell {} requires produced terminal but was skipped",
                            cell.cell_id
                        )));
                    }
                    if certified.producer != spec::CellProducer::Node(node_id.clone())
                        || schema_id != &cell.schema_id
                        || semantic_type_id != &cell.semantic_type_id
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "skipped cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    match view.projections.attempt(node_id, attempt_id) {
                        Some(store::AttemptProjection {
                            status: store::AttemptStatus::Completed { output_cell_id },
                            ..
                        }) if output_cell_id == &cell.cell_id => {}
                        _ => {
                            return Err(RuntimeError::InputMaterialization(format!(
                                "skipped cell {} is not backed by a completed producer attempt",
                                cell.cell_id
                            )));
                        }
                    }
                    MaterializedCellTerminal::Skipped {
                        skip_reason: skip_reason.clone(),
                    }
                }
            }
        }
    };
    Ok(MaterializedCell {
        cell_id: cell.cell_id.clone(),
        schema_id: cell.schema_id.clone(),
        semantic_type_id: cell.semantic_type_id.clone(),
        value_lineage: cell.value_lineage.clone(),
        context: certified.context.clone(),
        terminal,
    })
}

fn validate_materialized_input_context(
    node: &spec::NodeSpec,
    input: &spec::InputBindingCellSpec,
    certified: &spec::CellSpec,
) -> Result<()> {
    if !input_context_matches_cell_context(&input.context, &certified.context) {
        return Err(RuntimeError::InputMaterialization(format!(
            "input cell {} context does not match certified cell context",
            input.cell_id
        )));
    }
    if let spec::InputContextSpec::Required { context_ref, .. } = &input.context {
        match &node.context {
            spec::NodeContextSpec::Required {
                context_ref: node_context_ref,
            } if node_context_ref == context_ref => {}
            _ => {
                return Err(RuntimeError::InputMaterialization(format!(
                    "input cell {} context does not match consuming node {} context",
                    input.cell_id, node.node_id
                )));
            }
        }
    }
    Ok(())
}

fn input_context_matches_cell_context(
    input: &spec::InputContextSpec,
    cell: &spec::CellContextSpec,
) -> bool {
    match (input, cell) {
        (spec::InputContextSpec::NoContext, spec::CellContextSpec::NoContext) => true,
        (
            spec::InputContextSpec::Required {
                context_ref,
                resource_kind,
                stage,
                producer,
            },
            spec::CellContextSpec::Bound {
                context_ref: cell_context_ref,
                resource_kind: cell_resource_kind,
                stage: cell_stage,
                producer: cell_producer,
            },
        ) => {
            context_ref == cell_context_ref
                && resource_kind == cell_resource_kind
                && stage == cell_stage
                && producer == cell_producer
        }
        _ => false,
    }
}

pub(crate) fn validate_seed_cells(
    runtime_spec: &CertifiedRuntimeSpec,
    seed_cells: &[events::SeedCellRef],
) -> Result<BTreeMap<CellId, events::SeedCellRef>> {
    let mut by_seed = BTreeMap::<_, _>::new();
    let mut by_cell = BTreeMap::<_, _>::new();
    for seed in seed_cells {
        if by_seed.insert(seed.seed_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed evidence for {}",
                seed.seed_id
            )));
        }
        if by_cell.insert(seed.cell_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed cell evidence for {}",
                seed.cell_id
            )));
        }
    }
    for declared in &runtime_spec.spec().seeds {
        let seed = by_seed.get(&declared.seed_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing RunAdmitted seed evidence for {}",
                declared.seed_id
            ))
        })?;
        if seed.cell_id != declared.cell_id
            || seed.scope_id != declared.scope_id
            || seed.schema_id != declared.schema_id
            || seed.semantic_type_id != declared.semantic_type_id
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunAdmitted seed {} metadata does not match certified seed",
                declared.seed_id
            )));
        }
        if let Some(required_digest) = &declared.required_digest {
            if &seed.digest != required_digest {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "RunAdmitted seed {} digest does not match certified required digest",
                    declared.seed_id
                )));
            }
        }
        if seed.seed_artifact.role != events::ArtifactRole::SeedInput
            || seed.seed_artifact.schema_id != declared.schema_id
            || seed.seed_artifact.semantic_type_id.as_ref() != Some(&declared.semantic_type_id)
            || seed.seed_artifact.content_digest != seed.digest
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunAdmitted seed {} artifact evidence does not match certified seed",
                declared.seed_id
            )));
        }
    }
    if by_seed.len() != runtime_spec.spec().seeds.len() {
        return Err(RuntimeError::InvalidRunStream(
            "RunAdmitted contains seed evidence not certified by the spec".to_owned(),
        ));
    }
    Ok(by_cell)
}
