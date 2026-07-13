use super::*;

pub(super) fn checked_key(label: &str, value: &str) -> Result<CheckedStableAuthorKey> {
    CheckedStableAuthorKey::new(value)
        .map_err(|_| PlanError::Key(format!("{label} {value:?} must use stable key grammar")))
}

pub(super) fn checked_field_path(label: &str, value: impl AsRef<str>) -> Result<CheckedFieldPath> {
    let value = value.as_ref();
    CheckedFieldPath::new(value).map_err(|_| {
        PlanError::Key(format!(
            "{label} {value:?} must contain non-empty field segments"
        ))
    })
}

pub(super) fn checked_field_segment(label: &str, value: &str) -> Result<CheckedFieldSegment> {
    CheckedFieldSegment::new(value).map_err(|_| {
        PlanError::Key(format!(
            "{label} {value:?} must be a non-empty field segment"
        ))
    })
}

pub(super) fn canonical_digest(value: serde_json::Value) -> Result<ContentDigest> {
    let json =
        serde_json::to_string(&value).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(canonical.content_digest())
}

pub(super) fn canonical_digest_bytes(value: serde_json::Value) -> Result<DigestBytes> {
    Ok(*canonical_digest(value)?.digest())
}

pub(super) fn stable_domain_key_refs_json(
    domain_keys: &[StableDomainKeyRef],
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

pub(super) fn operation_lineage_json(lineage: &OperationLineage) -> serde_json::Value {
    serde_json::json!({
        "active_instances": lineage
            .active_instances
            .iter()
            .map(OperationInstanceId::as_str)
            .collect::<Vec<_>>(),
        "completed_frames": lineage
            .completed_frames
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>(),
        "digest": lineage.digest.as_str(),
    })
}

pub(super) fn scope_id(key: &ScopeKey) -> Result<ScopeId> {
    scope_id_from_parts(None, key, &OperationLineage::empty()?)
}

pub(super) fn scope_id_from_parts(
    parent_scope_id: Option<&ScopeId>,
    key: &ScopeKey,
    operation_lineage: &OperationLineage,
) -> Result<ScopeId> {
    Ok(ScopeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "local_scope_key": key.as_str(),
            "operation_lineage": operation_lineage.digest.as_str(),
            "parent_scope_id": parent_scope_id.map(ScopeId::as_str),
        }))?,
    ))
}

pub(super) fn child_scope_id(
    parent_scope_id: &ScopeId,
    key: &ScopeKey,
    operation_lineage: &OperationLineage,
) -> Result<ScopeId> {
    scope_id_from_parts(Some(parent_scope_id), key, operation_lineage)
}

pub(super) fn seed_id<T: MfmValue>(
    scope_id: &ScopeId,
    key: &SeedKey,
    seed: &CanonicalSeed<T>,
) -> Result<SeedId> {
    Ok(SeedId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": LOWERING_VERSION,
            "scope_id": scope_id.as_str(),
            "seed_key": key.as_str(),
            "semantic_type_id": seed.semantic_type_id.as_str(),
            "schema_id": seed.schema_id.as_str(),
        }))?,
    ))
}

pub(super) fn seed_cell_id<T: MfmValue>(
    scope_id: &ScopeId,
    seed_id: &SeedId,
    seed: &CanonicalSeed<T>,
) -> Result<CellId> {
    cell_id(
        scope_id,
        &CellProducer::Seed(seed_id.clone()),
        &seed.semantic_type_id,
        &seed.schema_id,
    )
}

pub(super) fn seed_value_lineage(
    scope_id: &ScopeId,
    seed_id: &SeedId,
    operation_lineage: &OperationLineage,
) -> Result<ValueLineage> {
    Ok(ValueLineage {
        scope_id: scope_id.clone(),
        producer: CellProducer::Seed(seed_id.clone()),
        input_cells: Vec::new(),
        config_ref_digest: None,
        operation_lineage: operation_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: LineageTransformPolicy::Source,
    })
}

pub(super) fn bridge_value_lineage(
    target_scope_id: &ScopeId,
    node_id: &NodeId,
    source_cell_id: &CellId,
    operation_lineage: &OperationLineage,
) -> Result<ValueLineage> {
    Ok(ValueLineage {
        scope_id: target_scope_id.clone(),
        producer: CellProducer::Node(node_id.clone()),
        input_cells: vec![source_cell_id.clone()],
        config_ref_digest: None,
        operation_lineage: operation_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: LineageTransformPolicy::SameValueBridge,
    })
}

pub(super) fn value_lineage_ref(lineage: &ValueLineage) -> Result<ValueLineageRef> {
    let mut input_cells = lineage.input_cells.clone();
    input_cells.sort();
    let mut domain_keys = lineage.domain_keys.clone();
    domain_keys.sort();
    let digest = canonical_digest(serde_json::json!({
        "config_ref_digest": lineage.config_ref_digest.as_ref().map(ContentDigest::as_str),
        "domain_keys": stable_domain_key_refs_json(&domain_keys),
        "input_cells": input_cells.iter().map(CellId::as_str).collect::<Vec<_>>(),
        "operation_lineage": operation_lineage_json(&lineage.operation_lineage),
        "producer": cell_producer_json(&lineage.producer),
        "scope_id": lineage.scope_id.as_str(),
        "transform_policy": lineage.transform_policy.as_str(),
    }))?;
    Ok(ValueLineageRef::new(digest))
}

pub(super) fn bridge_node_id(
    source_scope_id: &ScopeId,
    target_scope_id: &ScopeId,
    source_cell_id: &CellId,
    key: &BridgeKey,
    bridge_kind: BridgeKind,
    policy: BridgePolicy,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "bridge_kind": bridge_kind.as_str(),
            "local_node_key": key.as_str(),
            "lowering_version": LOWERING_VERSION,
            "policy": policy.as_str(),
            "source_cell_id": source_cell_id.as_str(),
            "source_scope_id": source_scope_id.as_str(),
            "target_scope_id": target_scope_id.as_str(),
        }))?,
    ))
}

pub(super) fn bridge_cell_id(
    target_scope_id: &ScopeId,
    node_id: &NodeId,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    cell_id(
        target_scope_id,
        &CellProducer::Node(node_id.clone()),
        semantic_type_id,
        schema_id,
    )
}

pub(super) fn bridge_session_token(
    parent_scope_id: &ScopeId,
    child_scope_id: &ScopeId,
    key: &ScopeKey,
) -> BridgeSessionToken {
    BridgeSessionToken(sha256_digest_bytes(
        format!(
            "mfm.program:bridge-session:{}:{}:{}",
            parent_scope_id.as_str(),
            child_scope_id.as_str(),
            key.as_str()
        )
        .as_bytes(),
    ))
}

pub(super) fn bridge_ref_key(bridge_ref: &BridgeRef) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        bridge_ref.source_scope_id.as_str(),
        bridge_ref.target_scope_id.as_str(),
        bridge_ref.source_cell_id.as_str(),
        bridge_ref.target_cell_id.as_str(),
        bridge_ref.semantic_type_id.as_str(),
        bridge_ref.schema_id.as_str(),
        bridge_ref.bridge_node_id.as_str()
    )
}

pub(super) fn canonical_config_binding<C: MfmConfig>(
    config: &ValidatedConfig<C>,
) -> Result<ConfigBindingSpec> {
    let json = serde_json::to_string(config.as_ref())
        .map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    let schema_id = C::schema_id().map_err(|error| PlanError::Value(error.to_string()))?;
    let content_digest = canonical.content_digest();
    let byte_len = canonical.as_bytes().len();
    let config_ref_digest = canonical_digest(serde_json::json!({
        "byte_len": byte_len,
        "content_digest": content_digest.as_str(),
        "schema_id": schema_id.as_str(),
    }))?;
    Ok(ConfigBindingSpec {
        schema_id,
        canonical_json: canonical,
        content_digest,
        config_ref_digest,
        byte_len,
    })
}

pub(super) fn context_descriptor_for<C: MfmContext>() -> Result<StateContextDescriptorSpec> {
    let descriptor = C::schema_descriptor().map_err(|error| PlanError::Value(error.to_string()))?;
    if descriptor.identity.schema_kind != SchemaKind::Value {
        return Err(PlanError::ContextContract(
            "context descriptors must use value schema kind".to_owned(),
        ));
    }
    if descriptor.identity.persisted_surface != PersistedSurfacePolicy::strict() {
        return Err(PlanError::ContextContract(
            "context descriptors must use strict no-secret/no-float persisted policy".to_owned(),
        ));
    }
    if descriptor.identity.persisted_surface.secrets != SecretPolicy::NoSecrets
        || descriptor.identity.persisted_surface.numbers != NumberPolicy::NoFloats
    {
        return Err(PlanError::ContextContract(
            "context descriptors must reject secrets and floats".to_owned(),
        ));
    }
    let schema_id = descriptor
        .schema_id()
        .map_err(|error| PlanError::Value(error.to_string()))?;
    let semantic_type_id = C::semantic_id().map_err(|error| PlanError::Value(error.to_string()))?;
    let canonicalizer_identity = C::canonicalizer_identity()?;
    let context_descriptor_id =
        context_descriptor_id(&schema_id, &semantic_type_id, &canonicalizer_identity)?;
    Ok(StateContextDescriptorSpec::Required(Box::new(
        StateContextDescriptorRequirementSpec {
            context_descriptor_id,
            schema_id,
            semantic_type_id,
            canonicalizer_identity,
        },
    )))
}

pub(super) fn context_validator_spec_for<C: MfmContext>() -> Result<ContextValidatorSpec> {
    let StateContextDescriptorSpec::Required(requirement) = context_descriptor_for::<C>()? else {
        return Err(PlanError::ContextContract(
            "semantic context resolved to no-context descriptor".to_owned(),
        ));
    };
    Ok(ContextValidatorSpec {
        requirement: *requirement,
        validate: validate_context_spec_for::<C>,
    })
}

pub(super) fn validate_context_spec_for<C: MfmContext>(spec: &CertifiedContextSpec) -> Result<()> {
    <C as StateContext>::materialize_certified(Some(spec)).map(|_| ())
}

pub(super) fn context_descriptor_id(
    schema_id: &SchemaId,
    semantic_type_id: &SemanticTypeId,
    canonicalizer_identity: &CanonicalizerIdentity,
) -> Result<ContextDescriptorId> {
    let digest = canonical_digest(serde_json::json!({
        "canonicalizer_identity": canonicalizer_identity.as_str(),
        "domain_separator": "mfm.state_context_descriptor.v1",
        "schema_id": schema_id.as_str(),
        "semantic_type_id": semantic_type_id.as_str(),
    }))?;
    Ok(ContextDescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    ))
}

pub(super) fn certified_context_spec<C: MfmContext>(value: C) -> Result<CertifiedContextSpec> {
    let StateContextDescriptorSpec::Required(requirement) = context_descriptor_for::<C>()? else {
        return Err(PlanError::ContextContract(
            "semantic context resolved to no-context descriptor".to_owned(),
        ));
    };
    let StateContextDescriptorRequirementSpec {
        context_descriptor_id,
        schema_id,
        semantic_type_id,
        canonicalizer_identity,
    } = *requirement;
    let json =
        serde_json::to_string(&value).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical_context = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    let context_ref = CertifiedContextSpec::derive_context_ref(
        &context_descriptor_id,
        &schema_id,
        &semantic_type_id,
        &canonicalizer_identity,
        &canonical_context,
    )
    .map_err(|error| PlanError::Value(error.to_string()))?;
    Ok(CertifiedContextSpec {
        context_ref,
        context_descriptor_id,
        schema_id,
        semantic_type_id,
        canonicalizer_identity,
        canonical_context_digest: canonical_context.content_digest(),
        canonical_context_byte_len: canonical_context.as_bytes().len() as u64,
        canonical_context,
    })
}

pub(super) struct StateDescriptorIdParts<'a> {
    pub(super) kind: &'a StateKind,
    pub(super) version: &'a StateVersion,
    pub(super) name: &'static str,
    pub(super) context: &'a StateContextDescriptorSpec,
    pub(super) input_context: &'a StateInputContextContractSpec,
    pub(super) output_context: &'a StateOutputContextContractSpec,
    pub(super) config_schema_id: &'a SchemaId,
    pub(super) input_schema_id: &'a SchemaId,
    pub(super) output_schema_id: &'a SchemaId,
    pub(super) output_semantic_type_id: &'a SemanticTypeId,
    pub(super) effect: &'a EffectDescriptor,
    pub(super) capabilities: &'a CapabilitySetDescriptor,
    pub(super) emitted_fact_descriptors: &'a [FactDescriptorRef],
    pub(super) side_effect_contract_digest: Option<&'a ContentDigest>,
    pub(super) runner: RunnerKind,
}

pub(super) fn canonical_fact_descriptor_refs(
    mut refs: Vec<FactDescriptorRef>,
) -> Result<Vec<FactDescriptorRef>> {
    refs.sort();
    for window in refs.windows(2) {
        if window[0] == window[1] {
            return Err(PlanError::Registry(format!(
                "duplicate fact descriptor ref {}",
                window[0].descriptor_hash
            )));
        }
    }
    Ok(refs)
}

pub(super) fn fact_descriptor_refs_json(refs: &[FactDescriptorRef]) -> Vec<serde_json::Value> {
    refs.iter()
        .map(|reference| {
            serde_json::json!({
                "descriptor_hash": reference.descriptor_hash.as_str(),
            })
        })
        .collect()
}

pub(super) fn state_context_descriptor_json(
    context: &StateContextDescriptorSpec,
) -> serde_json::Value {
    match context {
        StateContextDescriptorSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        StateContextDescriptorSpec::Required(requirement) => {
            let StateContextDescriptorRequirementSpec {
                context_descriptor_id,
                schema_id,
                semantic_type_id,
                canonicalizer_identity,
            } = requirement.as_ref();
            serde_json::json!({
                "canonicalizer_identity": canonicalizer_identity.as_str(),
                "context_descriptor_id": context_descriptor_id.as_str(),
                "kind": "required",
                "schema_id": schema_id.as_str(),
                "semantic_type_id": semantic_type_id.as_str(),
            })
        }
    }
}

pub(super) fn context_producer_json(producer: &ContextProducerSpec) -> serde_json::Value {
    serde_json::json!({
        "producer_descriptor_ids": producer
            .producer_descriptor_ids
            .iter()
            .map(DescriptorId::as_str)
            .collect::<Vec<_>>(),
        "seed_producers_allowed": producer.seed_producers_allowed,
    })
}

pub(super) fn state_input_context_contract_json(
    contract: &StateInputContextContractSpec,
) -> serde_json::Value {
    match contract {
        StateInputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        StateInputContextContractSpec::Required {
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "kind": "required",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

pub(super) fn state_output_context_contract_json(
    contract: &StateOutputContextContractSpec,
) -> serde_json::Value {
    match contract {
        StateOutputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        StateOutputContextContractSpec::Produces {
            resource_kind,
            stage,
        } => serde_json::json!({
            "kind": "produces",
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

pub(super) fn state_descriptor_id(parts: StateDescriptorIdParts<'_>) -> Result<DescriptorId> {
    let json = serde_json::json!({
        "capabilities": parts.capabilities
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
            .collect::<Vec<_>>(),
        "config_schema_id": parts.config_schema_id.as_str(),
        "context": state_context_descriptor_json(parts.context),
        "effect": {
            "class": parts.effect.class.as_str(),
            "kind": parts.effect.kind.as_str(),
            "name": parts.effect.name,
            "version": parts.effect.version.as_str(),
        },
        "emitted_fact_descriptors": fact_descriptor_refs_json(parts.emitted_fact_descriptors),
        "input_context": state_input_context_contract_json(parts.input_context),
        "input_schema_id": parts.input_schema_id.as_str(),
        "kind": parts.kind.as_str(),
        "name": parts.name,
        "output_context": state_output_context_contract_json(parts.output_context),
        "output_schema_id": parts.output_schema_id.as_str(),
        "output_semantic_type_id": parts.output_semantic_type_id.as_str(),
        "runner": parts.runner.as_str(),
        "side_effect_contract_digest": parts
            .side_effect_contract_digest
            .map(ContentDigest::as_str),
        "version": parts.version.as_str(),
    });
    let json =
        serde_json::to_string(&json).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(canonical.as_bytes()),
    ))
}

pub(super) struct OperationDescriptorIdParts<'a> {
    pub(super) kind: &'a OperationKind,
    pub(super) version: &'a OperationVersion,
    pub(super) name: &'static str,
    pub(super) config_schema_id: &'a SchemaId,
    pub(super) input_schema_id: &'a SchemaId,
    pub(super) output_schema_id: &'a SchemaId,
    pub(super) expansion_abi: &'static str,
}

pub(super) fn operation_descriptor_id(
    parts: OperationDescriptorIdParts<'_>,
) -> Result<DescriptorId> {
    let json = serde_json::json!({
        "config_schema_id": parts.config_schema_id.as_str(),
        "expansion_abi": parts.expansion_abi,
        "input_schema_id": parts.input_schema_id.as_str(),
        "kind": parts.kind.as_str(),
        "name": parts.name,
        "output_schema_id": parts.output_schema_id.as_str(),
        "version": parts.version.as_str(),
    });
    let json =
        serde_json::to_string(&json).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(canonical.as_bytes()),
    ))
}

pub(super) fn state_node_id(
    scope_id: &ScopeId,
    key: &StateKey,
    state_kind: &StateKind,
    state_version: &StateVersion,
    config_digest: &ContentDigest,
    input_digest: &ContentDigest,
    context: &NodeContextSpec,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": config_digest.as_str(),
            "context": node_context_json(context),
            "input_binding_digest": input_digest.as_str(),
            "local_node_key": key.as_str(),
            "lowering_version": LOWERING_VERSION,
            "scope_id": scope_id.as_str(),
            "state_kind": state_kind.as_str(),
            "state_version": state_version.as_str(),
        }))?,
    ))
}

pub(super) fn node_context_json(context: &NodeContextSpec) -> serde_json::Value {
    match context {
        NodeContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        NodeContextSpec::Required { context_ref } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "required",
        }),
    }
}

pub(super) fn state_output_cell_id(
    scope_id: &ScopeId,
    node_id: &NodeId,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    cell_id(
        scope_id,
        &CellProducer::Node(node_id.clone()),
        semantic_type_id,
        schema_id,
    )
}

pub(super) fn state_value_lineage(
    scope_id: &ScopeId,
    node_id: &NodeId,
    input_root: &InputBindingNode,
    config_ref_digest: &ContentDigest,
    operation_lineage: &OperationLineage,
    domain_keys: Vec<StableDomainKeyRef>,
) -> Result<ValueLineage> {
    let mut input_cells = Vec::new();
    collect_input_cell_ids(input_root, &mut input_cells);
    input_cells.sort();
    state_value_lineage_from_cells(
        scope_id,
        node_id,
        input_cells,
        Some(config_ref_digest.clone()),
        operation_lineage,
        domain_keys,
    )
}

pub(super) fn state_value_lineage_from_cells(
    scope_id: &ScopeId,
    node_id: &NodeId,
    mut input_cells: Vec<CellId>,
    config_ref_digest: Option<ContentDigest>,
    operation_lineage: &OperationLineage,
    domain_keys: Vec<StableDomainKeyRef>,
) -> Result<ValueLineage> {
    input_cells.sort();
    Ok(ValueLineage {
        scope_id: scope_id.clone(),
        producer: CellProducer::Node(node_id.clone()),
        input_cells,
        config_ref_digest,
        operation_lineage: operation_lineage.clone(),
        domain_keys,
        transform_policy: LineageTransformPolicy::StateOutput,
    })
}

pub(super) fn spec_config_ref_digest(
    config_ref: &mfm_spec::v1::ConfigRef,
) -> Result<ContentDigest> {
    canonical_digest(serde_json::json!({
        "byte_len": config_ref.byte_len,
        "content_digest": config_ref.digest.as_str(),
        "schema_id": config_ref.schema_id.as_str(),
    }))
}

pub(super) fn cell_id(
    scope_id: &ScopeId,
    producer: &CellProducer,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    Ok(CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": LOWERING_VERSION,
            "output_index": 0,
            "producer": cell_producer_json(producer),
            "schema_id": schema_id.as_str(),
            "scope_id": scope_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?,
    ))
}

pub(super) fn cell_producer_json(producer: &CellProducer) -> serde_json::Value {
    match producer {
        CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
        CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
    }
}

pub(super) struct OperationInstanceIdParts<'a> {
    pub(super) scope_id: &'a ScopeId,
    pub(super) key: &'a OperationKey,
    pub(super) descriptor: &'a OperationDescriptorIdentity,
    pub(super) parent_operation_lineage: &'a OperationLineage,
    pub(super) config_digest: &'a ContentDigest,
    pub(super) input_digest: &'a ContentDigest,
}

pub(super) fn operation_instance_id(
    parts: OperationInstanceIdParts<'_>,
) -> Result<OperationInstanceId> {
    Ok(OperationInstanceId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": parts.config_digest.as_str(),
            "input_binding_digest": parts.input_digest.as_str(),
            "operation_descriptor_id": parts.descriptor.descriptor_id().as_str(),
            "operation_key": parts.key.as_str(),
            "operation_kind": parts.descriptor.kind().as_str(),
            "operation_version": parts.descriptor.version().as_str(),
            "parent_operation_lineage": parts.parent_operation_lineage.digest.as_str(),
            "parent_scope_id": parts.scope_id.as_str(),
        }))?,
    ))
}

pub(super) struct OperationLineageFrameDigestParts<'a> {
    pub(super) scope_id: &'a ScopeId,
    pub(super) key: &'a OperationKey,
    pub(super) operation_instance_id: &'a OperationInstanceId,
    pub(super) descriptor: &'a OperationDescriptorIdentity,
    pub(super) parent_operation_lineage: &'a OperationLineage,
    pub(super) config_digest: &'a ContentDigest,
    pub(super) input_digest: &'a ContentDigest,
    pub(super) output_handles: &'a [TypedHandleRef],
}

pub(super) fn operation_lineage_frame_digest(
    parts: OperationLineageFrameDigestParts<'_>,
) -> Result<ContentDigest> {
    let json = serde_json::json!({
        "config_digest": parts.config_digest.as_str(),
        "expansion_abi": parts.descriptor.expansion_abi(),
        "input_digest": parts.input_digest.as_str(),
        "operation_descriptor_id": parts.descriptor.descriptor_id().as_str(),
        "operation_instance_id": parts.operation_instance_id.as_str(),
        "operation_key": parts.key.as_str(),
        "operation_kind": parts.descriptor.kind().as_str(),
        "operation_version": parts.descriptor.version().as_str(),
        "parent_operation_lineage": parts.parent_operation_lineage.digest.as_str(),
        "output_handles": parts.output_handles
            .iter()
            .map(|handle| {
                serde_json::json!({
                    "cell_id": handle.cell_id().as_str(),
                    "context": cell_context_json(handle.context()),
                    "schema_id": handle.schema_id().as_str(),
                    "scope_id": handle.scope_id().as_str(),
                    "semantic_type_id": handle.semantic_type_id().as_str(),
                    "value_lineage": handle.value_lineage().digest().as_str(),
                })
            })
            .collect::<Vec<_>>(),
        "scope_id": parts.scope_id.as_str(),
    });
    let json =
        serde_json::to_string(&json).map_err(|error| PlanError::Serialize(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| PlanError::Canonical(error.to_string()))?;
    Ok(canonical.content_digest())
}

pub(super) fn cell_context_json(context: &CellContextSpec) -> serde_json::Value {
    match context {
        CellContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        CellContextSpec::Bound {
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

#[path = "input_binding_ids.rs"]
mod input_binding_ids;
pub(super) use self::input_binding_ids::*;

pub(super) fn digest_only_id<I>(
    domain: &str,
    value: &str,
    construct: fn(DigestAlgorithm, DigestBytes) -> I,
) -> Result<I> {
    let digest = sha256_digest_bytes(format!("mfm.program:{domain}:{value}").as_bytes());
    Ok(construct(DigestAlgorithm::Sha256JcsV1, digest))
}
