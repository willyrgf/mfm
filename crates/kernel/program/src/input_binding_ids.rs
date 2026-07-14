use super::*;

pub(crate) fn input_descriptor_id(input_schema_id: &SchemaId) -> Result<DescriptorId> {
    digest_only_id(
        "input-descriptor",
        input_schema_id.as_str(),
        DescriptorId::from_digest,
    )
}

pub(crate) fn collect_input_cell_ids(node: &InputBindingNode, output: &mut Vec<CellId>) {
    match &node.kind {
        InputBindingNodeKind::Unit => {}
        InputBindingNodeKind::Cell(cell) => output.push(cell.cell_id.clone()),
        InputBindingNodeKind::Tuple { elements }
        | InputBindingNodeKind::Vec { elements, .. }
        | InputBindingNodeKind::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cell_ids(element, output);
            }
        }
        InputBindingNodeKind::Struct { fields } => {
            for field in fields {
                collect_input_cell_ids(&field.node, output);
            }
        }
    }
}

pub(crate) fn validate_input_binding_node(node: &InputBindingNode) -> Result<()> {
    match &node.kind {
        InputBindingNodeKind::Unit | InputBindingNodeKind::Cell(_) => Ok(()),
        InputBindingNodeKind::Tuple { elements } | InputBindingNodeKind::Vec { elements, .. } => {
            for element in elements {
                validate_input_binding_node(element)?;
            }
            Ok(())
        }
        InputBindingNodeKind::Struct { fields } => {
            let mut seen = BTreeSet::new();
            for field in fields {
                if !seen.insert(field.field_path.as_str().to_owned()) {
                    return Err(PlanError::DuplicateInputFieldPath(
                        field.field_path.as_str().to_owned(),
                    ));
                }
                validate_input_binding_node(&field.node)?;
            }
            Ok(())
        }
        InputBindingNodeKind::NonEmptyVec { elements, .. } => {
            if elements.is_empty() {
                return Err(PlanError::EmptyNonEmptyInput);
            }
            for element in elements {
                validate_input_binding_node(element)?;
            }
            Ok(())
        }
    }
}

pub(crate) fn validate_input_binding_node_shape(
    node: &InputBindingNode,
    shape: &SchemaShape,
) -> Result<()> {
    match (&node.kind, shape) {
        (InputBindingNodeKind::Unit, SchemaShape::Unit) => Ok(()),
        (
            InputBindingNodeKind::Cell(cell),
            SchemaShape::ValueRef {
                schema_id: expected_schema_id,
                semantic_type_id: expected_semantic_type_id,
            },
        ) => {
            if &cell.schema_id != expected_schema_id
                || &cell.semantic_type_id != expected_semantic_type_id
            {
                return Err(PlanError::InputBindingShape(format!(
                    "cell {} expected ({}, {}) but got ({}, {})",
                    cell.field_path.as_str(),
                    expected_schema_id,
                    expected_semantic_type_id,
                    cell.schema_id,
                    cell.semantic_type_id
                )));
            }
            Ok(())
        }
        (InputBindingNodeKind::Tuple { elements }, SchemaShape::Tuple(expected)) => {
            if elements.len() != expected.len() {
                return Err(PlanError::InputBindingShape(format!(
                    "tuple expected {} elements but got {}",
                    expected.len(),
                    elements.len()
                )));
            }
            for (element, expected_shape) in elements.iter().zip(expected) {
                validate_input_binding_node_shape(element, expected_shape)?;
            }
            Ok(())
        }
        (InputBindingNodeKind::Struct { fields }, SchemaShape::Struct { fields: expected }) => {
            if fields.len() != expected.len() {
                return Err(PlanError::InputBindingShape(format!(
                    "struct expected {} fields but got {}",
                    expected.len(),
                    fields.len()
                )));
            }
            for (field, expected_field) in fields.iter().zip(expected) {
                if field.field_path.as_str() != expected_field.name {
                    return Err(PlanError::InputBindingShape(format!(
                        "struct expected field {} but got {}",
                        expected_field.name,
                        field.field_path.as_str()
                    )));
                }
                validate_input_binding_node_shape(&field.node, &expected_field.shape)?;
            }
            Ok(())
        }
        (InputBindingNodeKind::Vec { elements, .. }, SchemaShape::Vec(expected)) => {
            for element in elements {
                validate_input_binding_node_shape(element, expected)?;
            }
            Ok(())
        }
        (
            InputBindingNodeKind::NonEmptyVec { elements, .. },
            SchemaShape::NonEmptyVec(expected),
        ) => {
            for element in elements {
                validate_input_binding_node_shape(element, expected)?;
            }
            Ok(())
        }
        (node, shape) => Err(PlanError::InputBindingShape(format!(
            "expected {} but got {}",
            schema_shape_kind(shape),
            input_binding_node_kind_from_kind(node)
        ))),
    }
}

pub(crate) fn schema_shape_kind(shape: &SchemaShape) -> &'static str {
    match shape {
        SchemaShape::Unit => "unit",
        SchemaShape::Bool => "bool",
        SchemaShape::String => "string",
        SchemaShape::Bytes => "bytes",
        SchemaShape::SignedInteger { .. } => "signed_integer",
        SchemaShape::UnsignedInteger { .. } => "unsigned_integer",
        SchemaShape::DecimalString { .. } => "decimal_string",
        SchemaShape::Option(_) => "option",
        SchemaShape::Vec(_) => "vec",
        SchemaShape::NonEmptyVec(_) => "non_empty_vec",
        SchemaShape::Tuple(_) => "tuple",
        SchemaShape::Struct { .. } => "struct",
        SchemaShape::BTreeMapString { .. } => "btree_map_string",
        SchemaShape::Enum { .. } => "enum",
        SchemaShape::ValueRef { .. } => "value_ref",
        SchemaShape::Generic { .. } => "generic",
    }
}

pub(crate) fn input_binding_node_kind_from_kind(kind: &InputBindingNodeKind) -> &'static str {
    match kind {
        InputBindingNodeKind::Unit => "unit",
        InputBindingNodeKind::Cell(_) => "cell",
        InputBindingNodeKind::Tuple { .. } => "tuple",
        InputBindingNodeKind::Struct { .. } => "struct",
        InputBindingNodeKind::Vec { .. } => "vec",
        InputBindingNodeKind::NonEmptyVec { .. } => "non_empty_vec",
    }
}

pub(crate) fn input_binding_digest(root: &InputBindingNode) -> Result<ContentDigest> {
    let json = serde_json::to_string(&input_binding_node_json(root))
        .map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(canonical.content_digest())
}

pub(crate) fn input_binding_node_json(node: &InputBindingNode) -> serde_json::Value {
    match &node.kind {
        InputBindingNodeKind::Unit => serde_json::json!({
            "kind": "unit",
        }),
        InputBindingNodeKind::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "context": input_context_json(&cell.context),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": cell.required_terminal.as_str(),
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.digest().as_str(),
        }),
        InputBindingNodeKind::Tuple { elements } => serde_json::json!({
            "elements": elements.iter().map(input_binding_node_json).collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        InputBindingNodeKind::Struct { fields } => serde_json::json!({
            "fields": fields
                .iter()
                .map(|field| {
                    serde_json::json!({
                        "field_path": field.field_path.as_str(),
                        "node": input_binding_node_json(&field.node),
                    })
                })
                .collect::<Vec<_>>(),
            "kind": "struct",
        }),
        InputBindingNodeKind::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": stable_domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_binding_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering.as_str(),
        }),
        InputBindingNodeKind::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": stable_domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_binding_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering.as_str(),
        }),
    }
}

pub(crate) fn input_context_from_cell_context(context: &CellContextSpec) -> InputContextSpec {
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

pub(crate) fn input_context_json(context: &InputContextSpec) -> serde_json::Value {
    match context {
        InputContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        InputContextSpec::Required {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "required",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}
