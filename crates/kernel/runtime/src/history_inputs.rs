use super::*;

pub(crate) fn materialize_inputs<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<MaterializedInputs>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
    Ok(MaterializedInputs {
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        root: materialize_input_node(runtime_spec, node, &node.input_bindings.root, lifecycle)?,
    })
}

fn materialize_input_node<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    input: &spec::InputBindingNodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<MaterializedInputNode>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(MaterializedInputNode::Unit),
        spec::InputBindingNodeSpec::Cell(cell) => Ok(MaterializedInputNode::Cell(Box::new(
            materialize_cell(runtime_spec, node, cell, lifecycle)?,
        ))),
        spec::InputBindingNodeSpec::Tuple(elements) => Ok(MaterializedInputNode::Tuple(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, node, element, lifecycle))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Struct(fields) => Ok(MaterializedInputNode::Struct(
            fields
                .iter()
                .map(|field| {
                    Ok(NamedMaterializedInput {
                        field_path: field.field_path.clone(),
                        node: materialize_input_node(runtime_spec, node, &field.node, lifecycle)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Vec { elements, .. } => Ok(MaterializedInputNode::Vec(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, node, element, lifecycle))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            Ok(MaterializedInputNode::NonEmptyVec(
                elements
                    .iter()
                    .map(|element| materialize_input_node(runtime_spec, node, element, lifecycle))
                    .collect::<Result<Vec<_>>>()?,
            ))
        }
    }
}

fn materialize_cell<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    cell: &spec::InputBindingCellSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<MaterializedCell>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
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
            let seed = lifecycle.seed(&cell.cell_id)?.ok_or_else(|| {
                RuntimeError::InputMaterialization(format!(
                    "seed cell {} has no RunAdmitted evidence",
                    cell.cell_id
                ))
            })?;
            let seed = seed.evidence();
            let seed_artifact = committed_input_artifact(
                lifecycle,
                &cell.cell_id,
                &seed.seed_artifact.artifact_id,
                &seed.seed_artifact.evidence_hash,
                events::ArtifactRole::SeedInput,
            )?;
            let artifact_evidence = seed_artifact.evidence();
            if artifact_evidence.digest != seed.digest
                || artifact_evidence.byte_len != seed.seed_artifact.byte_len
                || artifact_evidence.media_type != seed.seed_artifact.media_type
                || artifact_evidence.schema_id.as_ref() != Some(&seed.schema_id)
                || artifact_evidence.semantic_type_id.as_ref() != Some(&seed.semantic_type_id)
                || artifact_evidence.producer_node_id.is_some()
                || artifact_evidence.producer_seed_id.as_ref() != Some(seed_id)
                || artifact_evidence.artifact_role != events::ArtifactRole::SeedInput
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
            let projection = lifecycle.cell(&cell.cell_id).ok_or_else(|| {
                RuntimeError::InputMaterialization(format!(
                    "input cell {} is not terminal",
                    cell.cell_id
                ))
            })?;
            if let Some(produced) = projection.produced() {
                if certified.producer != spec::CellProducer::Node(produced.node_id().clone())
                    || produced.schema_id() != &cell.schema_id
                    || produced.semantic_type_id() != &cell.semantic_type_id
                {
                    return Err(RuntimeError::InputMaterialization(format!(
                        "produced cell {} projection metadata mismatch",
                        cell.cell_id
                    )));
                }
                match lifecycle.attempt(produced.node_id(), produced.attempt_id()) {
                    Some(attempt)
                        if matches!(
                            attempt.status(),
                            store::current_lifecycle::CurrentAttemptStatusRef::Completed {
                                output_cell_id
                            } if output_cell_id == &cell.cell_id
                        ) => {}
                    _ => {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} is not backed by a completed producer attempt",
                            cell.cell_id
                        )));
                    }
                }
                let artifact = committed_input_artifact(
                    lifecycle,
                    &cell.cell_id,
                    produced.artifact_id(),
                    produced.evidence_hash(),
                    events::ArtifactRole::StateOutput,
                )?;
                let artifact_evidence = artifact.evidence();
                if artifact_evidence.digest != *produced.content_digest()
                    || artifact_evidence.evidence_hash()? != *produced.evidence_hash()
                    || artifact_evidence.schema_id.as_ref() != Some(produced.schema_id())
                    || artifact_evidence.semantic_type_id.as_ref()
                        != Some(produced.semantic_type_id())
                    || artifact_evidence.producer_node_id.as_ref() != Some(produced.node_id())
                    || artifact_evidence.producer_seed_id.is_some()
                    || artifact_evidence.artifact_role != events::ArtifactRole::StateOutput
                    || artifact.attempt_id() != Some(produced.attempt_id())
                {
                    return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} committed artifact evidence does not match terminal projection",
                            cell.cell_id
                        )));
                }
                MaterializedCellTerminal::Produced {
                    producer_node_id: produced.node_id().clone(),
                    artifact_id: produced.artifact_id().clone(),
                    content_digest: produced.content_digest().clone(),
                    evidence_hash: produced.evidence_hash().clone(),
                }
            } else if let Some(skipped) = projection.skipped() {
                if cell.required_terminal == spec::RequiredTerminal::ProducedOnly {
                    return Err(RuntimeError::InputMaterialization(format!(
                        "input cell {} requires produced terminal but was skipped",
                        cell.cell_id
                    )));
                }
                if certified.producer != spec::CellProducer::Node(skipped.node_id().clone())
                    || skipped.schema_id() != &cell.schema_id
                    || skipped.semantic_type_id() != &cell.semantic_type_id
                {
                    return Err(RuntimeError::InputMaterialization(format!(
                        "skipped cell {} projection metadata mismatch",
                        cell.cell_id
                    )));
                }
                match lifecycle.attempt(skipped.node_id(), skipped.attempt_id()) {
                    Some(attempt)
                        if matches!(
                            attempt.status(),
                            store::current_lifecycle::CurrentAttemptStatusRef::Completed {
                                output_cell_id
                            } if output_cell_id == &cell.cell_id
                        ) => {}
                    _ => {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "skipped cell {} is not backed by a completed producer attempt",
                            cell.cell_id
                        )));
                    }
                }
                MaterializedCellTerminal::Skipped {
                    skip_reason: skipped.skip_reason().clone(),
                }
            } else {
                return Err(RuntimeError::InputMaterialization(format!(
                    "input cell {} has an unsupported terminal state",
                    cell.cell_id
                )));
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

pub(crate) fn validate_seed_cells<S>(
    runtime_spec: &S,
    seed_cells: &[events::SeedCellRef],
) -> Result<BTreeMap<CellId, events::SeedCellRef>>
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
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
