use super::*;

pub(super) fn capability_set_json(descriptor: &CapabilitySetDescriptor) -> serde_json::Value {
    serde_json::Value::Array(
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
            .collect(),
    )
}

pub(super) fn remediations_json(
    remediations: &BTreeMap<NodeId, NodeSpec>,
    descriptors: &DescriptorJsonIndex,
) -> Result<serde_json::Value> {
    let object = remediations
        .iter()
        .map(|(forward_node_id, node)| {
            Ok((forward_node_id.as_str().to_owned(), node.json(descriptors)?))
        })
        .collect::<Result<serde_json::Map<_, _>>>()?;
    Ok(serde_json::Value::Object(object))
}

pub(super) fn parse_typed_execution_spec(value: &serde_json::Value) -> Result<TypedExecutionSpec> {
    let object = object(value, "typed execution spec")?;
    let spec_version = parse_string::<SpecVersion>(required_str(object, "spec_version")?)?;
    if spec_version.as_str() != SPEC_VERSION {
        return Err(json_error(format!(
            "unsupported spec_version {}",
            spec_version.as_str()
        )));
    }
    let media_type = MediaType::new(required_str(object, "media_type")?)?;
    if media_type.as_str() != MEDIA_TYPE {
        return Err(json_error(format!(
            "unsupported media_type {}",
            media_type.as_str()
        )));
    }
    let canonicalization =
        parse_string::<DigestAlgorithm>(required_str(object, "canonicalization")?)?;
    if canonicalization != DigestAlgorithm::Sha256JcsV1 {
        return Err(json_error(format!(
            "unsupported canonicalization {}",
            canonicalization.as_str()
        )));
    }
    let lowering_version =
        parse_string::<LoweringVersion>(required_str(object, "lowering_version")?)?;
    if lowering_version.as_str() != LOWERING_VERSION {
        return Err(json_error(format!(
            "unsupported lowering_version {}",
            lowering_version.as_str()
        )));
    }

    let descriptor_identities = parse_vec(
        required(object, "descriptor_identities")?,
        parse_descriptor_identity,
    )?;
    let descriptors = DescriptorParseIndex::new(&descriptor_identities)?;
    let nodes = parse_vec(required(object, "nodes")?, |node| {
        parse_node_spec(node, &descriptors)
    })?;
    let remediations = parse_remediations(required(object, "remediations")?, &descriptors)?;
    let planning_lineage = parse_vec(required(object, "planning_lineage")?, |frame| {
        parse_operation_lineage_frame(frame, &descriptors)
    })?;
    let public_outputs =
        parse_public_output_spec(required(object, "public_outputs")?, &descriptors)?;
    drop(descriptors);

    TypedExecutionSpec::new(TypedExecutionSpecParts {
        authoring: parse_authoring(required(object, "authoring")?)?,
        saga: parse_saga_policy(required(object, "saga")?)?,
        contexts: parse_vec(required(object, "contexts")?, parse_certified_context_spec)?,
        scopes: parse_vec(required(object, "scopes")?, parse_scope_spec)?,
        seeds: parse_vec(required(object, "seeds")?, parse_seed_spec)?,
        descriptor_identities,
        config_refs: parse_vec(required(object, "config_refs")?, parse_config_ref)?,
        nodes,
        remediations,
        cells: parse_vec(required(object, "cells")?, parse_cell_spec)?,
        value_lineages: parse_vec(required(object, "value_lineages")?, parse_value_lineage)?,
        planning_lineage,
        public_outputs,
    })
}

fn parse_saga_policy(value: &serde_json::Value) -> Result<SagaPolicySpec> {
    let object = object(value, "saga policy")?;
    match required_str(object, "kind")? {
        "no_side_effects" => Ok(SagaPolicySpec::NoSideEffects),
        "fail_without_acdc_claim" => Ok(SagaPolicySpec::FailWithoutAcdcClaim),
        "manual_resolution" => Ok(SagaPolicySpec::ManualResolution {
            manual: Box::new(parse_manual_resolution_evidence(required(
                object, "manual",
            )?)?),
        }),
        "compensate_completed" => Ok(SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: parse_remediation_unresolved(required(
                object,
                "on_remediation_unresolved",
            )?)?,
        }),
        kind => Err(json_error(format!("unsupported saga policy kind {kind:?}"))),
    }
}

fn parse_remediation_unresolved(value: &serde_json::Value) -> Result<RemediationUnresolvedSpec> {
    let object = object(value, "remediation-unresolved directive")?;
    match required_str(object, "kind")? {
        "manual_resolution" => Ok(RemediationUnresolvedSpec::ManualResolution {
            manual: Box::new(parse_manual_resolution_evidence(required(
                object, "manual",
            )?)?),
        }),
        "fail_without_acdc_claim" => Ok(RemediationUnresolvedSpec::FailWithoutAcdcClaim),
        kind => Err(json_error(format!(
            "unsupported remediation-unresolved kind {kind:?}"
        ))),
    }
}

fn parse_manual_resolution_evidence(
    value: &serde_json::Value,
) -> Result<ManualResolutionEvidenceSpec> {
    let object = object(value, "manual-resolution evidence")?;
    Ok(ManualResolutionEvidenceSpec {
        evidence_schema: parse_string(required_str(object, "evidence_schema")?)?,
        authorization: parse_manual_resolution_authorization(required(object, "authorization")?)?,
    })
}

fn parse_manual_resolution_authorization(
    value: &serde_json::Value,
) -> Result<ManualResolutionAuthorizationSpec> {
    let object = object(value, "manual-resolution authorization")?;
    Ok(ManualResolutionAuthorizationSpec {
        verifier_id: ManualAuthorizationVerifierId::new(required_str(object, "verifier_id")?)?,
        signing_scheme: ManualSigningSchemeSpec::new(required_str(object, "signing_scheme")?)?,
        authority: parse_operator_authority_snapshot(required(object, "authority")?)?,
        quorum: parse_manual_authorization_quorum(required(object, "quorum")?)?,
    })
}

fn parse_operator_authority_snapshot(
    value: &serde_json::Value,
) -> Result<OperatorAuthoritySnapshotSpec> {
    let object = object(value, "operator authority snapshot")?;
    Ok(OperatorAuthoritySnapshotSpec {
        authority_id: OperatorAuthorityId::new(required_str(object, "authority_id")?)?,
        operators: parse_vec(
            required(object, "operators")?,
            parse_operator_authority_member,
        )?,
    })
}

fn parse_operator_authority_member(
    value: &serde_json::Value,
) -> Result<OperatorAuthorityMemberSpec> {
    let object = object(value, "operator authority member")?;
    Ok(OperatorAuthorityMemberSpec {
        operator_id: OperatorId::new(required_str(object, "operator_id")?)?,
        public_identity: OperatorPublicIdentity::new(required_str(object, "public_identity")?)?,
    })
}

fn parse_manual_authorization_quorum(
    value: &serde_json::Value,
) -> Result<ManualAuthorizationQuorumSpec> {
    let object = object(value, "manual authorization quorum")?;
    match required_str(object, "kind")? {
        "threshold" => {
            let required_signatures = required_u64(object, "required_signatures")?
                .try_into()
                .map_err(|_| {
                    json_error("manual authorization quorum exceeds supported u32 range")
                })?;
            ManualAuthorizationQuorumSpec::new(required_signatures)
        }
        kind => Err(json_error(format!(
            "unsupported manual authorization quorum kind {kind:?}"
        ))),
    }
}

fn parse_remediations(
    value: &serde_json::Value,
    descriptors: &DescriptorParseIndex<'_>,
) -> Result<BTreeMap<NodeId, NodeSpec>> {
    object(value, "remediations")?
        .iter()
        .map(|(forward_node_id, node)| {
            Ok((
                parse_string(forward_node_id.as_str())?,
                parse_node_spec(node, descriptors)?,
            ))
        })
        .collect()
}

fn parse_authoring(value: &serde_json::Value) -> Result<AuthoringProvenance> {
    let object = object(value, "authoring")?;
    match required_str(object, "kind")? {
        "operation_expansion" => Ok(AuthoringProvenance::OperationExpansion {
            operation_descriptor_id: parse_string(required_str(
                object,
                "operation_descriptor_id",
            )?)?,
            config_hash: parse_string(required_str(object, "config_hash")?)?,
        }),
        "state_composition" => Ok(AuthoringProvenance::StateComposition {
            descriptor: parse_composition_descriptor(required(object, "descriptor")?)?,
            config_hash: parse_string(required_str(object, "config_hash")?)?,
        }),
        "mixed_composition" => Ok(AuthoringProvenance::MixedComposition {
            descriptor: parse_composition_descriptor(required(object, "descriptor")?)?,
            config_hash: parse_string(required_str(object, "config_hash")?)?,
        }),
        kind => Err(json_error(format!("unsupported authoring kind {kind:?}"))),
    }
}

fn parse_composition_descriptor(value: &serde_json::Value) -> Result<CompositionDescriptor> {
    let object = object(value, "composition descriptor")?;
    Ok(CompositionDescriptor {
        descriptor_id: parse_string(required_str(object, "descriptor_id")?)?,
        name: required_str(object, "name")?.to_owned(),
        version: required_str(object, "version")?.to_owned(),
    })
}

fn parse_config_ref(value: &serde_json::Value) -> Result<ConfigRef> {
    let object = object(value, "config ref")?;
    Ok(ConfigRef {
        schema_id: parse_string(required_str(object, "schema_id")?)?,
        artifact_id: parse_string(required_str(object, "artifact_id")?)?,
        digest: parse_string(required_str(object, "digest")?)?,
        byte_len: required_u64(object, "byte_len")?,
        media_type: MediaType::new(required_str(object, "media_type")?)?,
    })
}

fn parse_certified_context_spec(value: &serde_json::Value) -> Result<CertifiedContextSpec> {
    let object = object(value, "certified context")?;
    let canonical_context = canonical_json(required(object, "canonical_context")?.clone())?;
    let context = CertifiedContextSpec {
        context_ref: parse_string(required_str(object, "context_ref")?)?,
        context_descriptor_id: parse_string(required_str(object, "context_descriptor_id")?)?,
        schema_id: parse_string(required_str(object, "schema_id")?)?,
        semantic_type_id: parse_string(required_str(object, "semantic_type_id")?)?,
        canonicalizer_identity: CanonicalizerIdentity::new(required_str(
            object,
            "canonicalizer_identity",
        )?)?,
        canonical_context,
        canonical_context_digest: parse_string(required_str(object, "canonical_context_digest")?)?,
        canonical_context_byte_len: required_u64(object, "canonical_context_byte_len")?,
    };
    context.validate_digest_and_ref()?;
    Ok(context)
}

fn parse_node_context(value: &serde_json::Value) -> Result<NodeContextSpec> {
    let object = object(value, "node context")?;
    match required_str(object, "kind")? {
        "no_context" => Ok(NodeContextSpec::NoContext),
        "required" => Ok(NodeContextSpec::Required {
            context_ref: parse_string(required_str(object, "context_ref")?)?,
        }),
        kind => Err(json_error(format!(
            "unsupported node context kind {kind:?}"
        ))),
    }
}

fn parse_state_context_descriptor(value: &serde_json::Value) -> Result<StateContextDescriptorSpec> {
    let object = object(value, "state context descriptor")?;
    match required_str(object, "kind")? {
        "no_context" => Ok(StateContextDescriptorSpec::NoContext),
        "required" => Ok(StateContextDescriptorSpec::Required(Box::new(
            StateContextDescriptorRequirementSpec {
                context_descriptor_id: parse_string(required_str(
                    object,
                    "context_descriptor_id",
                )?)?,
                schema_id: parse_string(required_str(object, "schema_id")?)?,
                semantic_type_id: parse_string(required_str(object, "semantic_type_id")?)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    object,
                    "canonicalizer_identity",
                )?)?,
            },
        ))),
        kind => Err(json_error(format!(
            "unsupported state context descriptor kind {kind:?}"
        ))),
    }
}

fn parse_context_producer(value: &serde_json::Value) -> Result<ContextProducerSpec> {
    let object = object(value, "context producer constraint")?;
    Ok(ContextProducerSpec {
        producer_descriptor_ids: parse_identity_vec(required(object, "producer_descriptor_ids")?)?,
        seed_producers_allowed: required_bool(object, "seed_producers_allowed")?,
    })
}

fn parse_state_input_context_contract(
    value: &serde_json::Value,
) -> Result<StateInputContextContractSpec> {
    let object = object(value, "state input context contract")?;
    match required_str(object, "kind")? {
        "no_context" => Ok(StateInputContextContractSpec::NoContext),
        "required" => Ok(StateInputContextContractSpec::Required {
            resource_kind: ContextResourceKind::new(required_str(object, "resource_kind")?)
                .map_err(|error| SpecError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(object, "stage")?)
                .map_err(|error| SpecError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required(object, "producer")?)?),
        }),
        kind => Err(json_error(format!(
            "unsupported state input context contract kind {kind:?}"
        ))),
    }
}

fn parse_state_output_context_contract(
    value: &serde_json::Value,
) -> Result<StateOutputContextContractSpec> {
    let object = object(value, "state output context contract")?;
    match required_str(object, "kind")? {
        "no_context" => Ok(StateOutputContextContractSpec::NoContext),
        "produces" => Ok(StateOutputContextContractSpec::Produces {
            resource_kind: ContextResourceKind::new(required_str(object, "resource_kind")?)
                .map_err(|error| SpecError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(object, "stage")?)
                .map_err(|error| SpecError::Identity(error.to_string()))?,
        }),
        kind => Err(json_error(format!(
            "unsupported state output context contract kind {kind:?}"
        ))),
    }
}

fn parse_cell_context(value: &serde_json::Value) -> Result<CellContextSpec> {
    let object = object(value, "cell context")?;
    match required_str(object, "kind")? {
        "no_context" => Ok(CellContextSpec::NoContext),
        "bound" => Ok(CellContextSpec::Bound {
            context_ref: parse_string(required_str(object, "context_ref")?)?,
            resource_kind: ContextResourceKind::new(required_str(object, "resource_kind")?)
                .map_err(|error| SpecError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(object, "stage")?)
                .map_err(|error| SpecError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required(object, "producer")?)?),
        }),
        kind => Err(json_error(format!(
            "unsupported cell context kind {kind:?}"
        ))),
    }
}

fn parse_input_context(value: &serde_json::Value) -> Result<InputContextSpec> {
    let object = object(value, "input context")?;
    match required_str(object, "kind")? {
        "no_context" => Ok(InputContextSpec::NoContext),
        "required" => Ok(InputContextSpec::Required {
            context_ref: parse_string(required_str(object, "context_ref")?)?,
            resource_kind: ContextResourceKind::new(required_str(object, "resource_kind")?)
                .map_err(|error| SpecError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(object, "stage")?)
                .map_err(|error| SpecError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required(object, "producer")?)?),
        }),
        kind => Err(json_error(format!(
            "unsupported input context kind {kind:?}"
        ))),
    }
}

fn parse_scope_spec(value: &serde_json::Value) -> Result<ScopeSpec> {
    let object = object(value, "scope")?;
    Ok(ScopeSpec {
        scope_id: parse_string(required_str(object, "scope_id")?)?,
        parent_scope_id: optional_identity(object, "parent_scope_id")?,
        stable_key: StableAuthorKey::new(required_str(object, "stable_key")?)?,
        planning_lineage: parse_planning_lineage(required(object, "planning_lineage")?)?,
    })
}

fn parse_seed_spec(value: &serde_json::Value) -> Result<SeedSpec> {
    let object = object(value, "seed")?;
    Ok(SeedSpec {
        seed_id: parse_string(required_str(object, "seed_id")?)?,
        seed_key: StableAuthorKey::new(required_str(object, "seed_key")?)?,
        cell_id: parse_string(required_str(object, "cell_id")?)?,
        scope_id: parse_string(required_str(object, "scope_id")?)?,
        semantic_type_id: parse_string(required_str(object, "semantic_type_id")?)?,
        schema_id: parse_string(required_str(object, "schema_id")?)?,
        required_digest: optional_identity(object, "required_digest")?,
    })
}

fn parse_descriptor_identity(value: &serde_json::Value) -> Result<DescriptorIdentity> {
    let object = object(value, "descriptor identity")?;
    match required_str(object, "descriptor_family")? {
        "state" => Ok(DescriptorIdentity::State(Box::new(
            parse_state_descriptor_identity(value)?,
        ))),
        "operation" => Ok(DescriptorIdentity::Operation(Box::new(
            parse_operation_descriptor_identity(value)?,
        ))),
        "renderer" => Ok(DescriptorIdentity::Renderer(Box::new(
            parse_renderer_descriptor_identity(value)?,
        ))),
        family => Err(json_error(format!(
            "unsupported descriptor_family {family:?}"
        ))),
    }
}

struct DescriptorParseIndex<'a> {
    states: BTreeMap<String, &'a StateDescriptorIdentity>,
    operations: BTreeMap<String, &'a OperationDescriptorIdentity>,
    renderers: BTreeMap<String, &'a RendererDescriptorIdentity>,
    refs: BTreeMap<String, DescriptorRef>,
}

impl<'a> DescriptorParseIndex<'a> {
    fn new(descriptors: &'a [DescriptorIdentity]) -> Result<Self> {
        let mut index = Self {
            states: BTreeMap::new(),
            operations: BTreeMap::new(),
            renderers: BTreeMap::new(),
            refs: BTreeMap::new(),
        };
        for descriptor in descriptors {
            let reference = descriptor.descriptor_ref()?;
            let key = reference.descriptor_id.as_str().to_owned();
            if index.refs.insert(key.clone(), reference).is_some() {
                return Err(json_error(format!("duplicate descriptor identity {key}")));
            }
            match descriptor {
                DescriptorIdentity::State(state) => {
                    index.states.insert(key, state);
                }
                DescriptorIdentity::Operation(operation) => {
                    index.operations.insert(key, operation);
                }
                DescriptorIdentity::Renderer(renderer) => {
                    index.renderers.insert(key, renderer);
                }
            }
        }
        Ok(index)
    }

    fn state(&self, reference: &DescriptorRef) -> Result<&'a StateDescriptorIdentity> {
        self.require_ref(reference, DescriptorFamily::State)?;
        self.states
            .get(reference.descriptor_id.as_str())
            .copied()
            .ok_or_else(|| {
                json_error(format!(
                    "missing state descriptor identity {}",
                    reference.descriptor_id
                ))
            })
    }

    fn operation(&self, reference: &DescriptorRef) -> Result<&'a OperationDescriptorIdentity> {
        self.require_ref(reference, DescriptorFamily::Operation)?;
        self.operations
            .get(reference.descriptor_id.as_str())
            .copied()
            .ok_or_else(|| {
                json_error(format!(
                    "missing operation descriptor identity {}",
                    reference.descriptor_id
                ))
            })
    }

    fn renderer(&self, reference: &DescriptorRef) -> Result<&'a RendererDescriptorIdentity> {
        self.require_ref(reference, DescriptorFamily::Renderer)?;
        self.renderers
            .get(reference.descriptor_id.as_str())
            .copied()
            .ok_or_else(|| {
                json_error(format!(
                    "missing renderer descriptor identity {}",
                    reference.descriptor_id
                ))
            })
    }

    fn require_ref(&self, reference: &DescriptorRef, family: DescriptorFamily) -> Result<()> {
        if reference.family != family {
            return Err(json_error(format!(
                "descriptor {} is {}, expected {}",
                reference.descriptor_id,
                reference.family.as_str(),
                family.as_str()
            )));
        }
        let Some(expected) = self.refs.get(reference.descriptor_id.as_str()) else {
            return Err(json_error(format!(
                "missing descriptor identity {}",
                reference.descriptor_id
            )));
        };
        if expected != reference {
            return Err(json_error(format!(
                "descriptor ref mismatch for {}",
                reference.descriptor_id
            )));
        }
        Ok(())
    }
}

fn parse_descriptor_ref(value: &serde_json::Value) -> Result<DescriptorRef> {
    let object = object(value, "descriptor ref")?;
    Ok(DescriptorRef {
        family: DescriptorFamily::parse(required_str(object, "descriptor_family")?)?,
        descriptor_id: parse_string(required_str(object, "descriptor_id")?)?,
        descriptor_digest: parse_string(required_str(object, "descriptor_digest")?)?,
    })
}

fn parse_state_descriptor_identity(value: &serde_json::Value) -> Result<StateDescriptorIdentity> {
    let object = object(value, "state descriptor identity")?;
    Ok(StateDescriptorIdentity {
        descriptor_id: parse_string(required_str(object, "descriptor_id")?)?,
        name: required_str(object, "name")?.to_owned(),
        state_kind: parse_string(required_str(object, "state_kind")?)?,
        state_version: parse_string(required_str(object, "state_version")?)?,
        context: parse_state_context_descriptor(required(object, "context")?)?,
        input_context: parse_state_input_context_contract(required(object, "input_context")?)?,
        output_context: parse_state_output_context_contract(required(object, "output_context")?)?,
        config_schema_id: parse_string(required_str(object, "config_schema_id")?)?,
        input_schema_id: parse_string(required_str(object, "input_schema_id")?)?,
        output_schema_id: parse_string(required_str(object, "output_schema_id")?)?,
        output_semantic_type_id: parse_string(required_str(object, "output_semantic_type_id")?)?,
        effect_kind: parse_string(required_str(object, "effect_kind")?)?,
        effect_class: required_str(object, "effect_class")?.to_owned(),
        effect_name: required_str(object, "effect_name")?.to_owned(),
        effect_version: parse_string(required_str(object, "effect_version")?)?,
        capabilities: parse_capability_set(required(object, "capabilities")?)?,
        emitted_fact_descriptors: parse_fact_descriptor_refs(required(
            object,
            "emitted_fact_descriptors",
        )?)?,
        runner: required_str(object, "runner")?.to_owned(),
        effect_contract_digest: optional_identity(object, "effect_contract_digest")?,
    })
}

fn parse_operation_descriptor_identity(
    value: &serde_json::Value,
) -> Result<OperationDescriptorIdentity> {
    let object = object(value, "operation descriptor identity")?;
    Ok(OperationDescriptorIdentity {
        descriptor_id: parse_string(required_str(object, "descriptor_id")?)?,
        name: required_str(object, "name")?.to_owned(),
        operation_kind: parse_string(required_str(object, "operation_kind")?)?,
        operation_version: parse_string(required_str(object, "operation_version")?)?,
        config_schema_id: parse_string(required_str(object, "config_schema_id")?)?,
        input_schema_id: parse_string(required_str(object, "input_schema_id")?)?,
        output_schema_id: parse_string(required_str(object, "output_schema_id")?)?,
        expansion_abi: required_str(object, "expansion_abi")?.to_owned(),
    })
}

fn parse_renderer_descriptor_identity(
    value: &serde_json::Value,
) -> Result<RendererDescriptorIdentity> {
    let object = object(value, "renderer descriptor identity")?;
    Ok(RendererDescriptorIdentity {
        descriptor_id: parse_string(required_str(object, "descriptor_id")?)?,
        renderer_kind: RendererKind::new(required_str(object, "renderer_kind")?)?,
        renderer_version: RendererVersion::new(required_str(object, "renderer_version")?)?,
        public_schema_id: parse_string(required_str(object, "public_schema_id")?)?,
        canonicalizer_identity: CanonicalizerIdentity::new(required_str(
            object,
            "canonicalizer_identity",
        )?)?,
    })
}

fn parse_node_spec(
    value: &serde_json::Value,
    descriptors: &DescriptorParseIndex<'_>,
) -> Result<NodeSpec> {
    let object = object(value, "node")?;
    let descriptor_ref = parse_descriptor_ref(required(object, "descriptor_ref")?)?;
    let descriptor = descriptors.state(&descriptor_ref)?;
    Ok(NodeSpec {
        node_id: parse_string(required_str(object, "node_id")?)?,
        stable_key: StableAuthorKey::new(required_str(object, "stable_key")?)?,
        scope_id: parse_string(required_str(object, "scope_id")?)?,
        state_kind: descriptor.state_kind.clone(),
        state_version: descriptor.state_version.clone(),
        descriptor_id: descriptor.descriptor_id.clone(),
        context: parse_node_context(required(object, "context")?)?,
        config_ref: parse_config_ref(required(object, "config_ref")?)?,
        input_bindings: parse_input_binding_spec(required(object, "input_bindings")?)?,
        output_cell: parse_string(required_str(object, "output_cell")?)?,
        effect_kind: descriptor.effect_kind.clone(),
        capability_bindings: descriptor.capabilities.clone(),
        adapter_bindings: parse_vec(required(object, "adapter_bindings")?, parse_adapter_binding)?,
        fact_descriptor_allowlist: parse_fact_descriptor_refs(required(
            object,
            "fact_descriptor_allowlist",
        )?)?,
        side_effect: optional_parse(object, "side_effect", parse_side_effect_contract)?,
        framework: optional_parse(object, "framework", |framework| {
            parse_framework_node(framework, descriptors)
        })?,
        planning_lineage: parse_planning_lineage(required(object, "planning_lineage")?)?,
        deterministic_predecessors: parse_identity_vec(required(
            object,
            "deterministic_predecessors",
        )?)?,
    })
}

fn parse_fact_descriptor_refs(value: &serde_json::Value) -> Result<Vec<FactDescriptorRef>> {
    parse_vec(value, parse_fact_descriptor_ref)
}

fn parse_fact_descriptor_ref(value: &serde_json::Value) -> Result<FactDescriptorRef> {
    let object = object(value, "fact descriptor ref")?;
    Ok(FactDescriptorRef {
        descriptor_hash: parse_string(required_str(object, "descriptor_hash")?)?,
    })
}

fn parse_adapter_binding(value: &serde_json::Value) -> Result<AdapterBinding> {
    let object = object(value, "adapter binding")?;
    Ok(AdapterBinding {
        adapter_kind: parse_string(required_str(object, "adapter_kind")?)?,
        adapter_version: parse_string(required_str(object, "adapter_version")?)?,
        binding_digest: optional_identity(object, "binding_digest")?,
    })
}

fn parse_side_effect_contract(value: &serde_json::Value) -> Result<SideEffectContractSpec> {
    let object = object(value, "side-effect contract")?;
    Ok(SideEffectContractSpec {
        contract_digest: parse_string(required_str(object, "contract_digest")?)?,
        resource_claim: parse_resource_claim(required(object, "resource_claim")?)?,
        verification: parse_side_effect_verification(required(object, "verification")?)?,
    })
}

fn parse_side_effect_verification(value: &serde_json::Value) -> Result<SideEffectVerificationSpec> {
    let verification = object(value, "side-effect verification")?;
    match required_str(verification, "kind")? {
        "receipt" => Ok(SideEffectVerificationSpec::Receipt),
        "finalized" => {
            let finalized = object(
                required(verification, "finalized")?,
                "finalized side-effect verification",
            )?;
            let depth = required_u64(finalized, "depth")?;
            if depth == 0 {
                return Err(json_error(
                    "finalized side-effect verification depth must be positive",
                ));
            }
            Ok(SideEffectVerificationSpec::Finalized { depth })
        }
        kind => Err(json_error(format!(
            "unsupported side-effect verification kind {kind:?}"
        ))),
    }
}

fn parse_resource_claim(value: &serde_json::Value) -> Result<ResourceClaimSpec> {
    let claim = object(value, "resource claim")?;
    match required_str(claim, "kind")? {
        "exclusive" => {
            let exclusive = object(required(claim, "exclusive")?, "exclusive resource claim")?;
            Ok(ResourceClaimSpec::Exclusive {
                namespace: ResourceNamespace::new(required_str(exclusive, "namespace")?)?,
                key_schema: parse_string(required_str(exclusive, "key_schema")?)?,
            })
        }
        "exact_touched_set" => {
            let touched_set = object(
                required(claim, "exact_touched_set")?,
                "exact-touched-set resource claim",
            )?;
            Ok(ResourceClaimSpec::ExactTouchedSet {
                namespace: ResourceNamespace::new(required_str(touched_set, "namespace")?)?,
                evidence_schema: parse_string(required_str(touched_set, "evidence_schema")?)?,
            })
        }
        "manual_only" => Ok(ResourceClaimSpec::ManualOnly),
        kind => Err(json_error(format!(
            "unsupported resource claim kind {kind:?}"
        ))),
    }
}

fn parse_framework_node(
    value: &serde_json::Value,
    descriptors: &DescriptorParseIndex<'_>,
) -> Result<FrameworkNodeSpec> {
    let object = object(value, "framework node")?;
    match required_str(object, "kind")? {
        "bridge" => Ok(FrameworkNodeSpec::Bridge(parse_bridge_node(required(
            object, "bridge",
        )?)?)),
        "side_effect_verify" => Ok(FrameworkNodeSpec::SideEffectVerify(
            parse_side_effect_verify_node(required(object, "side_effect_verify")?)?,
        )),
        "public_output_render" => Ok(FrameworkNodeSpec::PublicOutputRender(
            parse_public_output_render_node(
                required(object, "public_output_render")?,
                descriptors,
            )?,
        )),
        "project_retention_manifest" => Ok(FrameworkNodeSpec::ProjectRetentionManifest(
            parse_project_retention_manifest_node(required(object, "project_retention_manifest")?)?,
        )),
        "complete_run" => Ok(FrameworkNodeSpec::CompleteRun(parse_complete_run_node(
            required(object, "complete_run")?,
        )?)),
        "resolve_saga_terminal" => Ok(FrameworkNodeSpec::ResolveSagaTerminal(
            parse_resolve_saga_terminal_node(required(object, "resolve_saga_terminal")?)?,
        )),
        kind => Err(json_error(format!("unsupported framework kind {kind:?}"))),
    }
}

fn parse_side_effect_verify_node(value: &serde_json::Value) -> Result<SideEffectVerifyNodeSpec> {
    let object = object(value, "side-effect verify node")?;
    Ok(SideEffectVerifyNodeSpec {
        pair_id: parse_string(required_str(object, "pair_id")?)?,
        submit_node_id: parse_string(required_str(object, "submit_node_id")?)?,
    })
}

fn parse_bridge_node(value: &serde_json::Value) -> Result<BridgeNodeSpec> {
    let object = object(value, "bridge node")?;
    Ok(BridgeNodeSpec {
        bridge_kind: parse_bridge_kind(required_str(object, "bridge_kind")?)?,
        source_scope_id: parse_string(required_str(object, "source_scope_id")?)?,
        target_scope_id: parse_string(required_str(object, "target_scope_id")?)?,
        source_cell_id: parse_string(required_str(object, "source_cell_id")?)?,
        target_cell_id: parse_string(required_str(object, "target_cell_id")?)?,
        semantic_type_id: parse_string(required_str(object, "semantic_type_id")?)?,
        schema_id: parse_string(required_str(object, "schema_id")?)?,
        policy: parse_bridge_policy(required_str(object, "policy")?)?,
        provenance: parse_bridge_provenance(required_str(object, "provenance")?)?,
    })
}

fn parse_public_output_render_node(
    value: &serde_json::Value,
    descriptors: &DescriptorParseIndex<'_>,
) -> Result<PublicOutputRenderNodeSpec> {
    let object = object(value, "public-output render node")?;
    let renderer_ref = parse_descriptor_ref(required(object, "renderer_descriptor_ref")?)?;
    let renderer_descriptor = descriptors.renderer(&renderer_ref)?.clone();
    Ok(PublicOutputRenderNodeSpec {
        public_schema_id: parse_string(required_str(object, "public_schema_id")?)?,
        output_spec_digest: parse_string(required_str(object, "output_spec_digest")?)?,
        renderer_descriptor,
        required_cells: parse_vec(required(object, "required_cells")?, parse_public_cell)?,
    })
}

fn parse_project_retention_manifest_node(
    value: &serde_json::Value,
) -> Result<ProjectRetentionManifestNodeSpec> {
    let object = object(value, "project-retention-manifest node")?;
    Ok(ProjectRetentionManifestNodeSpec {
        public_schema_id: parse_string(required_str(object, "public_schema_id")?)?,
        public_output_receipt_cell: parse_string(required_str(
            object,
            "public_output_receipt_cell",
        )?)?,
    })
}

fn parse_complete_run_node(value: &serde_json::Value) -> Result<CompleteRunNodeSpec> {
    let object = object(value, "complete-run node")?;
    Ok(CompleteRunNodeSpec {
        public_schema_id: parse_string(required_str(object, "public_schema_id")?)?,
        retention_manifest_receipt_cell: parse_string(required_str(
            object,
            "retention_manifest_receipt_cell",
        )?)?,
    })
}

fn parse_resolve_saga_terminal_node(
    value: &serde_json::Value,
) -> Result<ResolveSagaTerminalNodeSpec> {
    let object = object(value, "resolve-saga-terminal node")?;
    Ok(ResolveSagaTerminalNodeSpec {
        public_schema_id: parse_string(required_str(object, "public_schema_id")?)?,
    })
}

fn parse_input_binding_spec(value: &serde_json::Value) -> Result<InputBindingSpec> {
    let object = object(value, "input binding")?;
    Ok(InputBindingSpec {
        input_schema_id: parse_string(required_str(object, "input_schema_id")?)?,
        input_descriptor_id: parse_string(required_str(object, "input_descriptor_id")?)?,
        root: parse_input_binding_node(required(object, "root")?)?,
        digest: parse_string(required_str(object, "digest")?)?,
    })
}

fn parse_input_binding_node(value: &serde_json::Value) -> Result<InputBindingNodeSpec> {
    let object = object(value, "input binding node")?;
    match required_str(object, "kind")? {
        "unit" => Ok(InputBindingNodeSpec::Unit),
        "cell" => Ok(InputBindingNodeSpec::Cell(Box::new(
            parse_input_binding_cell(value)?,
        ))),
        "tuple" => Ok(InputBindingNodeSpec::Tuple(parse_vec(
            required(object, "elements")?,
            parse_input_binding_node,
        )?)),
        "struct" => Ok(InputBindingNodeSpec::Struct(parse_vec(
            required(object, "fields")?,
            parse_named_input_binding,
        )?)),
        "vec" => Ok(InputBindingNodeSpec::Vec {
            elements: parse_vec(required(object, "elements")?, parse_input_binding_node)?,
            ordering: parse_ordering(required_str(object, "ordering")?)?,
            domain_keys: parse_vec(required(object, "domain_keys")?, parse_domain_key_ref)?,
        }),
        "non_empty_vec" => Ok(InputBindingNodeSpec::NonEmptyVec {
            elements: parse_vec(required(object, "elements")?, parse_input_binding_node)?,
            ordering: parse_ordering(required_str(object, "ordering")?)?,
            domain_keys: parse_vec(required(object, "domain_keys")?, parse_domain_key_ref)?,
        }),
        kind => Err(json_error(format!(
            "unsupported input binding kind {kind:?}"
        ))),
    }
}

pub(super) fn validate_input_context_refs(
    node: &InputBindingNodeSpec,
    refs: &BTreeMap<String, ()>,
) -> Result<()> {
    match node {
        InputBindingNodeSpec::Unit => {}
        InputBindingNodeSpec::Cell(cell) => cell.context.validate_known_ref(refs)?,
        InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                validate_input_context_refs(element, refs)?;
            }
        }
        InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                validate_input_context_refs(&field.node, refs)?;
            }
        }
        InputBindingNodeSpec::Vec { elements, .. }
        | InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                validate_input_context_refs(element, refs)?;
            }
        }
    }
    Ok(())
}

pub(super) fn require_context_ref(
    refs: &BTreeMap<String, ()>,
    context_ref: &ContextRef,
) -> Result<()> {
    if refs.contains_key(context_ref.as_str()) {
        Ok(())
    } else {
        Err(json_error(format!(
            "unknown certified context ref {context_ref}"
        )))
    }
}

fn parse_input_binding_cell(value: &serde_json::Value) -> Result<InputBindingCellSpec> {
    let object = object(value, "input binding cell")?;
    Ok(InputBindingCellSpec {
        field_path: PublicFieldPath::new(required_str(object, "field_path")?)?,
        cell_id: parse_string(required_str(object, "cell_id")?)?,
        semantic_type_id: parse_string(required_str(object, "semantic_type_id")?)?,
        schema_id: parse_string(required_str(object, "schema_id")?)?,
        required_terminal: parse_required_terminal(required_str(object, "required_terminal")?)?,
        value_lineage: parse_value_lineage_ref(required(object, "value_lineage")?)?,
        context: parse_input_context(required(object, "context")?)?,
    })
}

fn parse_named_input_binding(value: &serde_json::Value) -> Result<NamedInputBindingSpec> {
    let object = object(value, "named input binding")?;
    Ok(NamedInputBindingSpec {
        field_path: PublicFieldPath::new(required_str(object, "field_path")?)?,
        node: parse_input_binding_node(required(object, "node")?)?,
    })
}

fn parse_domain_key_ref(value: &serde_json::Value) -> Result<StableDomainKeyRef> {
    let object = object(value, "stable domain key")?;
    Ok(StableDomainKeyRef {
        schema_id: parse_string(required_str(object, "schema_id")?)?,
        content_digest: parse_string(required_str(object, "content_digest")?)?,
    })
}

fn parse_cell_producer(value: &serde_json::Value) -> Result<CellProducer> {
    let object = object(value, "cell producer")?;
    match required_str(object, "kind")? {
        "node" => Ok(CellProducer::Node(parse_string(required_str(
            object, "node_id",
        )?)?)),
        "seed" => Ok(CellProducer::Seed(parse_string(required_str(
            object, "seed_id",
        )?)?)),
        kind => Err(json_error(format!(
            "unsupported cell producer kind {kind:?}"
        ))),
    }
}

fn parse_cell_spec(value: &serde_json::Value) -> Result<CellSpec> {
    let object = object(value, "cell")?;
    Ok(CellSpec {
        cell_id: parse_string(required_str(object, "cell_id")?)?,
        producer: parse_cell_producer(required(object, "producer")?)?,
        scope_id: parse_string(required_str(object, "scope_id")?)?,
        semantic_type_id: parse_string(required_str(object, "semantic_type_id")?)?,
        schema_id: parse_string(required_str(object, "schema_id")?)?,
        value_lineage: parse_value_lineage_ref(required(object, "value_lineage")?)?,
        terminal_policy: parse_cell_terminal_policy(required_str(object, "terminal_policy")?)?,
        storage_policy: parse_storage_policy(required_str(object, "storage_policy")?)?,
        redaction_policy: parse_redaction_policy(required_str(object, "redaction_policy")?)?,
        context: parse_cell_context(required(object, "context")?)?,
    })
}

fn parse_value_lineage_ref(value: &serde_json::Value) -> Result<ValueLineageRef> {
    let object = object(value, "value lineage ref")?;
    Ok(ValueLineageRef {
        lineage_digest: parse_string(required_str(object, "lineage_digest")?)?,
    })
}

fn parse_value_lineage(value: &serde_json::Value) -> Result<ValueLineage> {
    let object = object(value, "value lineage")?;
    Ok(ValueLineage {
        lineage_ref: parse_value_lineage_ref(required(object, "lineage_ref")?)?,
        scope_id: parse_string(required_str(object, "scope_id")?)?,
        producer: parse_cell_producer(required(object, "producer")?)?,
        input_cells: parse_identity_vec(required(object, "input_cells")?)?,
        config_ref_digest: optional_identity(object, "config_ref_digest")?,
        planning_lineage: parse_planning_lineage(required(object, "planning_lineage")?)?,
        domain_keys: parse_vec(required(object, "domain_keys")?, parse_domain_key_ref)?,
        transform_policy: parse_transform_policy(required_str(object, "transform_policy")?)?,
    })
}

fn parse_planning_lineage(value: &serde_json::Value) -> Result<PlanningLineage> {
    let object = object(value, "planning lineage")?;
    Ok(PlanningLineage {
        active_operation_instances: parse_identity_vec(required(
            object,
            "active_operation_instances",
        )?)?,
        completed_operation_frames: parse_identity_vec(required(
            object,
            "completed_operation_frames",
        )?)?,
        lineage_digest: parse_string(required_str(object, "lineage_digest")?)?,
    })
}

fn parse_operation_lineage_frame(
    value: &serde_json::Value,
    descriptors: &DescriptorParseIndex<'_>,
) -> Result<OperationLineageFrameSpec> {
    let object = object(value, "operation lineage frame")?;
    let descriptor_ref = parse_descriptor_ref(required(object, "operation_descriptor_ref")?)?;
    let descriptor = descriptors.operation(&descriptor_ref)?;
    Ok(OperationLineageFrameSpec {
        operation_instance_id: parse_string(required_str(object, "operation_instance_id")?)?,
        operation_key: StableAuthorKey::new(required_str(object, "operation_key")?)?,
        scope_id: parse_string(required_str(object, "scope_id")?)?,
        operation_descriptor_id: descriptor.descriptor_id.clone(),
        config_ref_digest: parse_string(required_str(object, "config_ref_digest")?)?,
        input_bindings: parse_input_binding_spec(required(object, "input_bindings")?)?,
        input_binding_digest: parse_string(required_str(object, "input_binding_digest")?)?,
        parent_planning_lineage: parse_planning_lineage(required(
            object,
            "parent_planning_lineage",
        )?)?,
        output_cells: parse_identity_vec(required(object, "output_cells")?)?,
        lineage_digest: parse_string(required_str(object, "lineage_digest")?)?,
    })
}

fn parse_public_output_spec(
    value: &serde_json::Value,
    descriptors: &DescriptorParseIndex<'_>,
) -> Result<PublicOutputSpec> {
    let object = object(value, "public output spec")?;
    let renderer_ref = parse_descriptor_ref(required(object, "renderer_descriptor_ref")?)?;
    let renderer_descriptor = descriptors.renderer(&renderer_ref)?.clone();
    Ok(PublicOutputSpec {
        public_schema_id: parse_string(required_str(object, "public_schema_id")?)?,
        outputs: parse_vec(required(object, "outputs")?, parse_public_cell)?,
        renderer_descriptor,
    })
}

fn parse_public_cell(value: &serde_json::Value) -> Result<PublicOutputCell> {
    let object = object(value, "public output cell")?;
    Ok(PublicOutputCell {
        public_field_path: PublicFieldPath::new(required_str(object, "public_field_path")?)?,
        cell_id: parse_string(required_str(object, "cell_id")?)?,
        producer: parse_cell_producer(required(object, "producer")?)?,
        scope_id: parse_string(required_str(object, "scope_id")?)?,
        semantic_type_id: parse_string(required_str(object, "semantic_type_id")?)?,
        schema_id: parse_string(required_str(object, "schema_id")?)?,
        value_lineage: parse_value_lineage_ref(required(object, "value_lineage")?)?,
        required_terminal: parse_required_terminal(required_str(object, "required_terminal")?)?,
    })
}

fn parse_capability_set(value: &serde_json::Value) -> Result<CapabilitySetDescriptor> {
    let capabilities = array(value, "capabilities")?
        .iter()
        .map(parse_capability_descriptor)
        .collect::<Result<Vec<_>>>()?;
    Ok(CapabilitySetDescriptor::new(capabilities)?)
}

fn parse_capability_descriptor(value: &serde_json::Value) -> Result<CapabilityDescriptor> {
    let object = object(value, "capability descriptor")?;
    Ok(CapabilityDescriptor::new(
        parse_string(required_str(object, "kind")?)?,
        parse_string(required_str(object, "version")?)?,
        parse_capability_role(required_str(object, "role")?)?,
        required_str(object, "name")?,
    )?)
}

fn parse_capability_role(value: &str) -> Result<CapabilityRole> {
    match value {
        "read_external" => Ok(CapabilityRole::ReadExternal),
        "support" => Ok(CapabilityRole::Support),
        "external_mutation_authority" => Ok(CapabilityRole::ExternalMutationAuthority),
        role => Err(json_error(format!("unsupported capability role {role:?}"))),
    }
}

fn parse_bridge_kind(value: &str) -> Result<BridgeKind> {
    match value {
        "import_from_parent" => Ok(BridgeKind::ImportFromParent),
        "export_to_parent" => Ok(BridgeKind::ExportToParent),
        kind => Err(json_error(format!("unsupported bridge kind {kind:?}"))),
    }
}

fn parse_bridge_policy(value: &str) -> Result<BridgePolicy> {
    match value {
        "same_run_same_value" => Ok(BridgePolicy::SameRunSameValue),
        policy => Err(json_error(format!("unsupported bridge policy {policy:?}"))),
    }
}

fn parse_bridge_provenance(value: &str) -> Result<BridgeProvenance> {
    match value {
        "framework_child_scope_v1" => Ok(BridgeProvenance::FrameworkChildScopeV1),
        provenance => Err(json_error(format!(
            "unsupported bridge provenance {provenance:?}"
        ))),
    }
}

fn parse_ordering(value: &str) -> Result<OrderingEvidence> {
    match value {
        "explicit_author_order" => Ok(OrderingEvidence::ExplicitAuthorOrder),
        "stable_domain_key" => Ok(OrderingEvidence::StableDomainKey),
        ordering => Err(json_error(format!("unsupported ordering {ordering:?}"))),
    }
}

fn parse_cell_terminal_policy(value: &str) -> Result<CellTerminalPolicy> {
    match value {
        "produced_only" => Ok(CellTerminalPolicy::ProducedOnly),
        "maybe_skipped" => Ok(CellTerminalPolicy::MaybeSkipped),
        policy => Err(json_error(format!(
            "unsupported cell terminal policy {policy:?}"
        ))),
    }
}

fn parse_storage_policy(value: &str) -> Result<StoragePolicy> {
    match value {
        "content_addressed" => Ok(StoragePolicy::ContentAddressed),
        "artifact_reference" => Ok(StoragePolicy::ArtifactReference),
        "public_output_artifact" => Ok(StoragePolicy::PublicOutputArtifact),
        policy => Err(json_error(format!("unsupported storage policy {policy:?}"))),
    }
}

fn parse_redaction_policy(value: &str) -> Result<RedactionPolicy> {
    match value {
        "public" => Ok(RedactionPolicy::Public),
        "redacted" => Ok(RedactionPolicy::Redacted),
        policy => Err(json_error(format!(
            "unsupported redaction policy {policy:?}"
        ))),
    }
}

fn parse_transform_policy(value: &str) -> Result<LineageTransformPolicy> {
    match value {
        "source" => Ok(LineageTransformPolicy::Source),
        "state_output" => Ok(LineageTransformPolicy::StateOutput),
        "same_value_bridge" => Ok(LineageTransformPolicy::SameValueBridge),
        policy => Err(json_error(format!(
            "unsupported lineage transform policy {policy:?}"
        ))),
    }
}

fn parse_required_terminal(value: &str) -> Result<RequiredTerminal> {
    match value {
        "produced_only" => Ok(RequiredTerminal::ProducedOnly),
        "maybe_skipped" => Ok(RequiredTerminal::MaybeSkipped),
        terminal => Err(json_error(format!(
            "unsupported required terminal {terminal:?}"
        ))),
    }
}

#[path = "parser_helpers.rs"]
mod parser_helpers;
use self::parser_helpers::*;

pub(super) fn json_error(message: impl Into<String>) -> SpecError {
    SpecError::Json(message.into())
}
