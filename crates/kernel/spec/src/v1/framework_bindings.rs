use super::*;

/// Returns canonical JSON bytes for a framework-owned config artifact.
pub fn framework_config_canonical_json(
    kind: &str,
    node_id: &NodeId,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "framework": kind,
        "node_id": node_id.as_str(),
    }))
}

/// Returns the deterministic framework-owned config reference for a framework node.
pub fn framework_config_ref(kind: &str, node_id: &NodeId) -> Result<ConfigRef> {
    let bytes = framework_config_canonical_json(kind, node_id)?;
    let digest = bytes.content_digest();
    let schema_digest = content_digest(serde_json::json!({ "framework": kind }))?;
    Ok(ConfigRef {
        schema_id: SchemaId::new(
            "mfm.framework.config",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            *schema_digest.digest(),
        )?,
        artifact_id: ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: MediaType::new("application/json")?,
    })
}

/// Derives the certified side-effect pair id for one submit node and contract.
pub fn side_effect_pair_id(
    submit_node_id: &NodeId,
    submit_output_cell_id: &CellId,
    contract: &SideEffectContractSpec,
) -> Result<SideEffectPairId> {
    Ok(SideEffectPairId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *content_digest(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": LOWERING_VERSION,
            "side_effect_contract": contract.json(),
            "submit_node_id": submit_node_id.as_str(),
            "submit_output_cell_id": submit_output_cell_id.as_str(),
        }))?
        .digest(),
    ))
}

/// Derives the framework verify node id for a certified side-effect pair.
pub fn side_effect_verify_node_id(
    submit_node_id: &NodeId,
    pair_id: &SideEffectPairId,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *content_digest(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "framework": "side_effect_verify",
            "lowering_version": LOWERING_VERSION,
            "pair_id": pair_id.as_str(),
            "submit_node_id": submit_node_id.as_str(),
        }))?
        .digest(),
    ))
}

/// Derives the framework verify stable key for a side-effect submit key.
pub fn side_effect_verify_stable_key(submit_key: &StableAuthorKey) -> Result<StableAuthorKey> {
    StableAuthorKey::new(format!(
        "framework/side-effect-verify/{}",
        submit_key.as_str()
    ))
}

/// Returns the deterministic unit input binding for a lifecycle framework node.
pub fn framework_lifecycle_unit_input_binding(kind: &str) -> Result<InputBindingSpec> {
    let root = InputBindingNodeSpec::Unit;
    Ok(InputBindingSpec {
        input_schema_id: framework_lifecycle_unit_input_schema_id(kind)?,
        input_descriptor_id: framework_lifecycle_input_descriptor_id(
            kind,
            serde_json::json!({ "input": "unit" }),
        )?,
        digest: framework_lifecycle_input_digest(&root)?,
        root,
    })
}

/// Returns the deterministic receipt-cell input binding for a lifecycle framework node.
pub fn framework_lifecycle_receipt_input_binding(
    kind: &str,
    field_path: &str,
    cell: &CellSpec,
) -> Result<InputBindingSpec> {
    framework_lifecycle_cell_input_binding(
        kind,
        field_path,
        cell,
        RequiredTerminal::ProducedOnly,
        "receipt_cell",
    )
}

/// Returns the deterministic maybe-skipped cell input binding for a lifecycle framework node.
pub fn framework_lifecycle_maybe_skipped_cell_input_binding(
    kind: &str,
    field_path: &str,
    cell: &CellSpec,
) -> Result<InputBindingSpec> {
    framework_lifecycle_cell_input_binding(
        kind,
        field_path,
        cell,
        RequiredTerminal::MaybeSkipped,
        "maybe_skipped_cell",
    )
}

fn framework_lifecycle_cell_input_binding(
    kind: &str,
    field_path: &str,
    cell: &CellSpec,
    required_terminal: RequiredTerminal,
    input_kind: &str,
) -> Result<InputBindingSpec> {
    let field_path = PublicFieldPath::new(field_path)?;
    let root = InputBindingNodeSpec::Cell(Box::new(InputBindingCellSpec {
        field_path: field_path.clone(),
        cell_id: cell.cell_id.clone(),
        semantic_type_id: cell.semantic_type_id.clone(),
        schema_id: cell.schema_id.clone(),
        required_terminal,
        value_lineage: cell.value_lineage.clone(),
        context: input_context_from_cell_context(&cell.context),
    }));
    Ok(InputBindingSpec {
        input_schema_id: cell.schema_id.clone(),
        input_descriptor_id: framework_lifecycle_input_descriptor_id(
            kind,
            serde_json::json!({
                "cell_id": cell.cell_id.as_str(),
                "context": input_context_from_cell_context(&cell.context).json(),
                "field_path": field_path.as_str(),
                "input": input_kind,
                "schema_id": cell.schema_id.as_str(),
                "semantic_type_id": cell.semantic_type_id.as_str(),
            }),
        )?,
        digest: framework_lifecycle_input_digest(&root)?,
        root,
    })
}

fn framework_lifecycle_input_digest(root: &InputBindingNodeSpec) -> Result<ContentDigest> {
    content_digest(framework_lifecycle_input_digest_json(root))
}

fn input_context_from_cell_context(context: &CellContextSpec) -> InputContextSpec {
    match context {
        CellContextSpec::NoContext => InputContextSpec::NoContext,
        CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => InputContextSpec::Required {
            context_ref: context_ref.clone(),
            resource_kind: resource_kind.clone(),
            stage: stage.clone(),
            producer: producer.clone(),
        },
    }
}

fn framework_lifecycle_input_digest_json(root: &InputBindingNodeSpec) -> serde_json::Value {
    match root {
        InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
        InputBindingNodeSpec::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "context": cell.context.json(),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": cell.required_terminal.as_str(),
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.lineage_digest.as_str(),
        }),
        InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
            "elements": elements
                .iter()
                .map(framework_lifecycle_input_digest_json)
                .collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        InputBindingNodeSpec::Struct(fields) => serde_json::json!({
            "fields": fields
                .iter()
                .map(|field| serde_json::json!({
                    "field_path": field.field_path.as_str(),
                    "node": framework_lifecycle_input_digest_json(&field.node),
                }))
                .collect::<Vec<_>>(),
            "kind": "struct",
        }),
        InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys
                .iter()
                .map(|key| serde_json::json!({
                    "content_digest": key.content_digest.as_str(),
                    "schema_id": key.schema_id.as_str(),
                }))
                .collect::<Vec<_>>(),
            "elements": elements
                .iter()
                .map(framework_lifecycle_input_digest_json)
                .collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering.as_str(),
        }),
        InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys
                .iter()
                .map(|key| serde_json::json!({
                    "content_digest": key.content_digest.as_str(),
                    "schema_id": key.schema_id.as_str(),
                }))
                .collect::<Vec<_>>(),
            "elements": elements
                .iter()
                .map(framework_lifecycle_input_digest_json)
                .collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering.as_str(),
        }),
    }
}

fn framework_lifecycle_unit_input_schema_id(kind: &str) -> Result<SchemaId> {
    let digest = content_digest(serde_json::json!({
        "framework": kind,
        "input": "unit",
    }))?;
    Ok(SchemaId::new(
        "mfm.framework.lifecycle_unit_input",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

fn framework_lifecycle_input_descriptor_id(
    kind: &str,
    input: serde_json::Value,
) -> Result<DescriptorId> {
    Ok(DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *content_digest(serde_json::json!({
            "framework": kind,
            "input": input,
        }))?
        .digest(),
    ))
}
