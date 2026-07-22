use super::*;

pub(super) fn framework_config_ref(kind: &str, node_id: &NodeId) -> Result<spec::ConfigRef> {
    spec::framework_config_ref(kind, node_id).map_err(|error| CertifyError::Spec(error.to_string()))
}

pub(super) fn renderer_descriptor(
    public_schema_id: &SchemaId,
) -> Result<spec::RendererDescriptorIdentity> {
    let renderer_kind =
        spec::RendererKind::new("public-output/json").map_err(|error| lower(error.to_string()))?;
    let renderer_version = spec::RendererVersion::new("mfm.renderer.public_output_json.v1")
        .map_err(|error| lower(error.to_string()))?;
    let canonicalizer_identity = spec::CanonicalizerIdentity::new("sha256-jcs-v1")
        .map_err(|error| lower(error.to_string()))?;
    let descriptor_id = spec::RendererDescriptorIdentity {
        descriptor_id: DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        ),
        renderer_kind: renderer_kind.clone(),
        renderer_version: renderer_version.clone(),
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity: canonicalizer_identity.clone(),
    }
    .expected_descriptor_id()
    .map_err(|error| CertifyError::Spec(error.to_string()))?;
    Ok(spec::RendererDescriptorIdentity {
        descriptor_id,
        renderer_kind,
        renderer_version,
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity,
    })
}

pub(super) fn framework_bridge_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "bridge_same_value",
        serde_json::json!({ "framework": "bridge_same_value" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.bridge_same_value.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.bridge_same_value",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context: spec::StateContextDescriptorSpec::no_context(),
        effect_kind: Pure::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "pure",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

pub(super) fn framework_side_effect_verify_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
    context: spec::StateContextDescriptorSpec,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "side_effect_verify",
        serde_json::json!({ "framework": "side_effect_verify" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.side_effect_verify.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.side_effect_verify",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context,
        effect_kind: ReadExternal::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "read_external",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

pub(super) fn framework_render_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
    context: spec::StateContextDescriptorSpec,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "render_public_outputs",
        serde_json::json!({ "framework": "render_public_outputs" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.render_public_outputs.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.render_public_outputs",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context,
        effect_kind: Pure::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "pure",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

pub(super) fn framework_project_retention_manifest_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    framework_lifecycle_descriptor(
        "project_retention_manifest",
        "mfm.framework.project_retention_manifest",
        "mfm.framework.state.project_retention_manifest.v1",
        output_schema_id,
        output_semantic_type_id,
        config_schema_id,
        input_schema_id,
    )
}

pub(super) fn framework_complete_run_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    framework_lifecycle_descriptor(
        "complete_run",
        "mfm.framework.complete_run",
        "mfm.framework.state.complete_run.v1",
        output_schema_id,
        output_semantic_type_id,
        config_schema_id,
        input_schema_id,
    )
}

pub(super) fn framework_resolve_saga_terminal_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    framework_lifecycle_descriptor(
        "resolve_saga_terminal",
        "mfm.framework.resolve_saga_terminal",
        "mfm.framework.state.resolve_saga_terminal.v1",
        output_schema_id,
        output_semantic_type_id,
        config_schema_id,
        input_schema_id,
    )
}

fn framework_lifecycle_descriptor(
    kind: &'static str,
    name: &'static str,
    version: &'static str,
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(kind, serde_json::json!({ "framework": kind }))?;
    let state_version = StateVersion::new(version).map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name,
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context: spec::StateContextDescriptorSpec::no_context(),
        effect_kind: Pure::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "pure",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

struct FrameworkStateDescriptorParts<'a> {
    name: &'static str,
    state_kind: StateKind,
    state_version: StateVersion,
    config_schema_id: &'a SchemaId,
    input_schema_id: &'a SchemaId,
    output_schema_id: &'a SchemaId,
    output_semantic_type_id: &'a SemanticTypeId,
    context: spec::StateContextDescriptorSpec,
    effect_kind: EffectKind,
    runner: &'static str,
    capabilities: CapabilitySetDescriptor,
}

fn framework_state_descriptor(
    parts: FrameworkStateDescriptorParts<'_>,
) -> Result<spec::StateDescriptorIdentity> {
    let FrameworkStateDescriptorParts {
        name,
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context,
        effect_kind,
        runner,
        capabilities,
    } = parts;
    let effect = effect_descriptor_for_kind(&effect_kind)?;
    let effect_contract_digest = match effect.class {
        EffectClass::ReadExternal => Some(content_digest_json(serde_json::json!({
            "config_schema_id": config_schema_id.as_str(),
            "context": state_context_descriptor_json(&context),
            "contract_domain": "mfm.framework.side_effect_verify",
            "contract_version": 1,
            "effect_class": effect.class.as_str(),
            "input_schema_id": input_schema_id.as_str(),
            "output_schema_id": output_schema_id.as_str(),
            "output_semantic_type_id": output_semantic_type_id.as_str(),
        }))?),
        EffectClass::Pure | EffectClass::ApplySideEffect => None,
    };
    let descriptor_id = descriptor_id_json(serde_json::json!({
        "capabilities": capabilities.capabilities.iter().map(|capability| {
            serde_json::json!({
                "kind": capability.kind.as_str(),
                "name": capability.name.as_str(),
                "role": capability.role.as_str(),
                "version": capability.version.as_str(),
            })
        }).collect::<Vec<_>>(),
        "config_schema_id": config_schema_id.as_str(),
        "context": state_context_descriptor_json(&context),
        "effect": {
            "class": effect.class.as_str(),
            "kind": effect.kind.as_str(),
            "name": effect.name,
            "version": effect.version.as_str(),
        },
        "emitted_fact_descriptors": [],
        "input_context": state_input_context_contract_json(&spec::StateInputContextContractSpec::no_context()),
        "input_schema_id": input_schema_id.as_str(),
        "kind": state_kind.as_str(),
        "name": name,
        "output_context": state_output_context_contract_json(&spec::StateOutputContextContractSpec::no_context()),
        "output_schema_id": output_schema_id.as_str(),
        "output_semantic_type_id": output_semantic_type_id.as_str(),
        "runner": runner,
        "effect_contract_digest": effect_contract_digest.as_ref().map(ContentDigest::as_str),
        "version": state_version.as_str(),
    }))?;
    Ok(spec::StateDescriptorIdentity {
        descriptor_id,
        name: name.to_owned(),
        state_kind,
        state_version,
        context,
        input_context: spec::StateInputContextContractSpec::no_context(),
        output_context: spec::StateOutputContextContractSpec::no_context(),
        config_schema_id: config_schema_id.clone(),
        input_schema_id: input_schema_id.clone(),
        output_schema_id: output_schema_id.clone(),
        output_semantic_type_id: output_semantic_type_id.clone(),
        effect_kind,
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version,
        capabilities,
        emitted_fact_descriptors: Vec::new(),
        runner: runner.to_owned(),
        effect_contract_digest,
    })
}

pub(super) fn render_node_id(
    root_scope_id: &ScopeId,
    key: &str,
    output_spec_digest: &ContentDigest,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "framework": "public_output_render",
            "key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "output_spec_digest": output_spec_digest.as_str(),
            "scope_id": root_scope_id.as_str(),
        }))?,
    ))
}

pub(super) fn project_retention_manifest_node_id_from_spec(
    scope_id: &ScopeId,
    key: &str,
    retention: &spec::ProjectRetentionManifestNodeSpec,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "framework": "project_retention_manifest",
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "public_output_receipt_cell": retention.public_output_receipt_cell.as_str(),
            "public_schema_id": retention.public_schema_id.as_str(),
            "scope_id": scope_id.as_str(),
        }))?,
    ))
}

pub(super) fn complete_run_node_id_from_spec(
    scope_id: &ScopeId,
    key: &str,
    complete: &spec::CompleteRunNodeSpec,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "framework": "complete_run",
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "public_schema_id": complete.public_schema_id.as_str(),
            "retention_manifest_receipt_cell": complete.retention_manifest_receipt_cell.as_str(),
            "scope_id": scope_id.as_str(),
        }))?,
    ))
}

pub(super) fn resolve_saga_terminal_node_id_from_spec(
    scope_id: &ScopeId,
    key: &str,
    resolve: &spec::ResolveSagaTerminalNodeSpec,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "framework": "resolve_saga_terminal",
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "public_schema_id": resolve.public_schema_id.as_str(),
            "scope_id": scope_id.as_str(),
        }))?,
    ))
}

pub(super) fn state_node_id_from_spec(
    node: &spec::NodeSpec,
    config_ref_digest: &ContentDigest,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": config_ref_digest.as_str(),
            "context": node_context_json(&node.context),
            "input_binding_digest": node.input_bindings.digest.as_str(),
            "local_node_key": node.stable_key.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "scope_id": node.scope_id.as_str(),
            "state_kind": node.state_kind.as_str(),
            "state_version": node.state_version.as_str(),
        }))?,
    ))
}

pub(super) fn bridge_node_id_from_spec(key: &str, bridge: &spec::BridgeNodeSpec) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "bridge_kind": bridge_kind_json(bridge.bridge_kind),
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "policy": bridge_policy_json(bridge.policy),
            "source_cell_id": bridge.source_cell_id.as_str(),
            "source_scope_id": bridge.source_scope_id.as_str(),
            "target_scope_id": bridge.target_scope_id.as_str(),
        }))?,
    ))
}

pub(super) fn operation_instance_id_from_spec(
    frame: &spec::OperationLineageFrameSpec,
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<OperationInstanceId> {
    Ok(OperationInstanceId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": frame.config_ref_digest.as_str(),
            "input_binding_digest": frame.input_binding_digest.as_str(),
            "operation_descriptor_id": descriptor.descriptor_id.as_str(),
            "operation_key": frame.operation_key.as_str(),
            "operation_kind": descriptor.operation_kind.as_str(),
            "operation_version": descriptor.operation_version.as_str(),
            "parent_operation_lineage": frame.parent_planning_lineage.lineage_digest.as_str(),
            "parent_scope_id": frame.scope_id.as_str(),
        }))?,
    ))
}

pub(super) fn operation_lineage_frame_digest_from_spec(
    frame: &spec::OperationLineageFrameSpec,
    descriptor: &spec::OperationDescriptorIdentity,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "config_digest": frame.config_ref_digest.as_str(),
        "expansion_abi": descriptor.expansion_abi.as_str(),
        "input_digest": frame.input_binding_digest.as_str(),
        "operation_descriptor_id": descriptor.descriptor_id.as_str(),
        "operation_instance_id": frame.operation_instance_id.as_str(),
        "operation_key": frame.operation_key.as_str(),
        "operation_kind": descriptor.operation_kind.as_str(),
        "operation_version": descriptor.operation_version.as_str(),
        "parent_operation_lineage": frame.parent_planning_lineage.lineage_digest.as_str(),
        "output_handles": frame.output_cells
            .iter()
            .map(|cell_id| {
                let cell = cells.get(cell_id.as_str()).ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!("operation frame references missing output cell {cell_id}"),
                    )
                })?;
                Ok(serde_json::json!({
                    "cell_id": cell.cell_id.as_str(),
                    "context": cell_context_json(&cell.context),
                    "schema_id": cell.schema_id.as_str(),
                    "scope_id": cell.scope_id.as_str(),
                    "semantic_type_id": cell.semantic_type_id.as_str(),
                    "value_lineage": cell.value_lineage.lineage_digest.as_str(),
                }))
            })
            .collect::<Result<Vec<_>>>()?,
        "scope_id": frame.scope_id.as_str(),
    }))
}

pub(super) fn cell_context_json(context: &spec::CellContextSpec) -> serde_json::Value {
    match context {
        spec::CellContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "bound",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

pub(super) fn framework_cell_id(
    scope_id: &ScopeId,
    node_id: &NodeId,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    Ok(CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "output_index": 0,
            "producer": { "node_id": node_id.as_str(), "kind": "node" },
            "schema_id": schema_id.as_str(),
            "scope_id": scope_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?,
    ))
}

pub(super) fn cell_id_from_parts(
    scope_id: &ScopeId,
    producer: &spec::CellProducer,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    Ok(CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "output_index": 0,
            "producer": cell_producer_json(producer),
            "schema_id": schema_id.as_str(),
            "scope_id": scope_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?,
    ))
}

pub(super) fn render_value_lineage_ref(
    scope_id: &ScopeId,
    node_id: &NodeId,
    input_cells: &[CellId],
    planning_lineage: &spec::PlanningLineage,
    config_ref_digest: &ContentDigest,
) -> Result<spec::ValueLineageRef> {
    Ok(spec::ValueLineageRef {
        lineage_digest: content_digest_json(serde_json::json!({
            "config_ref_digest": config_ref_digest.as_str(),
            "domain_keys": [],
            "input_cells": sorted_cell_ids(input_cells.to_vec())
                .iter()
                .map(CellId::as_str)
                .collect::<Vec<_>>(),
            "operation_lineage": planning_lineage_json(planning_lineage),
            "producer": { "kind": "node", "node_id": node_id.as_str() },
            "scope_id": scope_id.as_str(),
            "transform_policy": "state_output",
        }))?,
    })
}

pub(super) fn effect_descriptor_for_kind(
    effect_kind: &EffectKind,
) -> Result<mfm_capabilities::EffectDescriptor> {
    let pure = Pure::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &pure.kind {
        return Ok(pure);
    }
    let side_effect = ApplySideEffect::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &side_effect.kind {
        return Ok(side_effect);
    }
    let read = ReadExternal::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &read.kind {
        return Ok(read);
    }
    Err(problem(
        ProblemClass::InvalidSemanticTransition,
        format!("unknown effect kind {effect_kind}"),
    ))
}

pub(super) fn effect_class_for_kind(
    effect_kind: &EffectKind,
) -> Result<(EffectClass, &'static str)> {
    let effect = effect_descriptor_for_kind(effect_kind)?;
    Ok((effect.class, effect.name))
}

pub(super) fn input_node_json(node: &spec::InputBindingNodeSpec) -> serde_json::Value {
    match node {
        spec::InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
        spec::InputBindingNodeSpec::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "context": input_context_json(&cell.context),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": match cell.required_terminal {
                spec::RequiredTerminal::ProducedOnly => "produced_only",
                spec::RequiredTerminal::MaybeSkipped => "maybe_skipped",
            },
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.lineage_digest.as_str(),
        }),
        spec::InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        spec::InputBindingNodeSpec::Struct(fields) => serde_json::json!({
            "fields": fields.iter().map(|field| {
                serde_json::json!({
                    "field_path": field.field_path.as_str(),
                    "node": input_node_json(&field.node),
                })
            }).collect::<Vec<_>>(),
            "kind": "struct",
        }),
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering_json(*ordering),
        }),
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering_json(*ordering),
        }),
    }
}

pub(super) fn node_context_json(context: &spec::NodeContextSpec) -> serde_json::Value {
    match context {
        spec::NodeContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::NodeContextSpec::Required { context_ref } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "required",
        }),
    }
}

pub(super) fn input_context_json(context: &spec::InputContextSpec) -> serde_json::Value {
    match context {
        spec::InputContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::InputContextSpec::Required {
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

pub(super) fn context_producer_json(producer: &spec::ContextProducerSpec) -> serde_json::Value {
    serde_json::json!({
        "producer_descriptor_ids": producer
            .producer_descriptor_ids
            .iter()
            .map(DescriptorId::as_str)
            .collect::<Vec<_>>(),
        "seed_producers_allowed": producer.seed_producers_allowed,
    })
}

pub(super) fn domain_key_refs_json(
    domain_keys: &[spec::StableDomainKeyRef],
) -> Vec<serde_json::Value> {
    domain_keys
        .iter()
        .map(|key| {
            serde_json::json!({
                "content_digest": key.content_digest.as_str(),
                "schema_id": key.schema_id.as_str(),
            })
        })
        .collect()
}

pub(super) fn capability_set_json(descriptor: &CapabilitySetDescriptor) -> Vec<serde_json::Value> {
    descriptor
        .capabilities
        .iter()
        .map(|capability| {
            serde_json::json!({
                "kind": capability.kind.as_str(),
                "name": capability.name.as_str(),
                "role": capability.role.as_str(),
                "version": capability.version.as_str(),
            })
        })
        .collect()
}

pub(super) fn fact_descriptor_refs_json(
    refs: &[spec::FactDescriptorRef],
) -> Vec<serde_json::Value> {
    refs.iter()
        .map(|reference| {
            serde_json::json!({
                "descriptor_hash": reference.descriptor_hash.as_str(),
            })
        })
        .collect()
}

pub(super) fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
        spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
    }
}

pub(super) fn planning_lineage_json(lineage: &spec::PlanningLineage) -> serde_json::Value {
    serde_json::json!({
        "active_instances": lineage
            .active_operation_instances
            .iter()
            .map(OperationInstanceId::as_str)
            .collect::<Vec<_>>(),
        "completed_frames": lineage
            .completed_operation_frames
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>(),
        "digest": lineage.lineage_digest.as_str(),
    })
}
