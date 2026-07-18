use super::*;
const DIGEST_A: DigestBytes = DigestBytes::from_array([0x11; 32]);
const DIGEST_B: DigestBytes = DigestBytes::from_array([0x22; 32]);
const DIGEST_C: DigestBytes = DigestBytes::from_array([0x33; 32]);
const DIGEST_D: DigestBytes = DigestBytes::from_array([0x44; 32]);

pub(super) fn digest(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

pub(super) fn content(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn descriptor(byte: u8) -> DescriptorId {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn artifact(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn node(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn cell(byte: u8) -> CellId {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn scope(byte: u8) -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn seed(byte: u8) -> SeedId {
    SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn operation_instance(byte: u8) -> OperationInstanceId {
    OperationInstanceId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn schema(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest(byte)).expect("schema id")
}

pub(super) fn semantic(name: &str, byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.spec.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest(byte),
    )
    .expect("semantic id")
}

pub(super) fn context_descriptor(byte: u8) -> ContextDescriptorId {
    ContextDescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

pub(super) fn test_context() -> CertifiedContextSpec {
    let context_descriptor_id = context_descriptor(0xa0);
    let schema_id = schema("mfm.spec.test.context", 0xa1);
    let semantic_type_id = semantic("context", 0xa2);
    let canonicalizer_identity =
        CanonicalizerIdentity::new("sha256-jcs-v1").expect("canonicalizer");
    let canonical_context = PlainCanonicalJsonBytes::from_json_str(
        r#"{"expected_chain_id":31337,"network_id":"local"}"#,
    )
    .expect("canonical context");
    let context_ref = CertifiedContextSpec::derive_context_ref(
        &context_descriptor_id,
        &schema_id,
        &semantic_type_id,
        &canonicalizer_identity,
        &canonical_context,
    )
    .expect("context ref");
    CertifiedContextSpec {
        context_ref,
        context_descriptor_id,
        schema_id,
        semantic_type_id,
        canonicalizer_identity,
        canonical_context_digest: canonical_context.content_digest(),
        canonical_context_byte_len: canonical_context.as_bytes().len() as u64,
        canonical_context,
    }
}

pub(super) fn context_producer(descriptor_id: DescriptorId) -> ContextProducerSpec {
    ContextProducerSpec {
        producer_descriptor_ids: vec![descriptor_id],
        seed_producers_allowed: false,
    }
}

pub(super) fn test_spec() -> TypedExecutionSpec {
    let scope_id = scope(0x01);
    let seed_id = seed(0x02);
    let seed_cell = cell(0x03);
    let state_node = node(0x04);
    let output_cell = cell(0x05);
    let render_node = node(0x06);
    let receipt_cell = cell(0x07);
    let bridge_node = node(0x0f);
    let bridged_cell = cell(0x10);
    let value_schema = schema("mfm.spec.test.launch_value", 0x08);
    let input_schema = schema("mfm.spec.test.launch_input", 0x09);
    let config_schema = schema("mfm.spec.test.launch_config", 0x0a);
    let public_schema = schema("mfm.spec.test.public_outputs", 0x0b);
    let semantic_id = semantic("launch_value", 0x0c);
    let receipt_semantic_id = SemanticTypeId::new(
        "mfm.framework",
        "public_output_receipt",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest(0x70),
    )
    .expect("receipt semantic");
    let state_descriptor = descriptor(0x0d);
    let render_state_descriptor = descriptor(0x12);
    let bridge_state_descriptor = descriptor(0x13);
    let operation_descriptor_id = descriptor(0x81);
    let renderer_descriptor = RendererDescriptorIdentity {
        descriptor_id: descriptor(0x0e),
        renderer_kind: RendererKind::new("public-output/json").expect("renderer kind"),
        renderer_version: RendererVersion::new("mfm.renderer.public_output_json.v1")
            .expect("renderer version"),
        public_schema_id: public_schema.clone(),
        canonicalizer_identity: CanonicalizerIdentity::new("sha256-jcs-v1").expect("canonicalizer"),
    };
    let public_cell = PublicOutputCell {
        public_field_path: PublicFieldPath::new("result").expect("field path"),
        cell_id: output_cell.clone(),
        producer: CellProducer::Node(state_node.clone()),
        scope_id: scope_id.clone(),
        semantic_type_id: semantic_id.clone(),
        schema_id: value_schema.clone(),
        value_lineage: ValueLineageRef {
            lineage_digest: content(0x21),
        },
        required_terminal: RequiredTerminal::ProducedOnly,
    };
    let empty_planning_lineage = PlanningLineage {
        active_operation_instances: Vec::new(),
        completed_operation_frames: Vec::new(),
        lineage_digest: content(0x60),
    };
    let public_outputs = PublicOutputSpec {
        public_schema_id: public_schema.clone(),
        outputs: vec![public_cell.clone()],
        renderer_descriptor: renderer_descriptor.clone(),
    };
    let output_spec_digest = public_outputs.digest().expect("public output digest");
    let config_ref = ConfigRef {
        schema_id: config_schema.clone(),
        artifact_id: artifact(0x31),
        digest: content(0x32),
        byte_len: 18,
        media_type: MediaType::new("application/json").expect("media type"),
    };
    let input_binding = InputBindingSpec {
        input_schema_id: input_schema.clone(),
        input_descriptor_id: descriptor(0x41),
        root: InputBindingNodeSpec::Cell(Box::new(InputBindingCellSpec {
            field_path: PublicFieldPath::new("input").expect("input path"),
            cell_id: seed_cell.clone(),
            semantic_type_id: semantic_id.clone(),
            schema_id: value_schema.clone(),
            required_terminal: RequiredTerminal::ProducedOnly,
            value_lineage: ValueLineageRef {
                lineage_digest: content(0x20),
            },
            context: InputContextSpec::no_context(),
        })),
        digest: content(0x42),
    };
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    TypedExecutionSpec::new(TypedExecutionSpecParts {
        authoring: AuthoringProvenance::StateComposition {
            descriptor: CompositionDescriptor {
                descriptor_id: descriptor(0x50),
                name: "test_state_composition".to_owned(),
                version: "mfm.spec.test.composition.v1".to_owned(),
            },
            config_hash: content(0x51),
        },
        saga: SagaPolicySpec::NoSideEffects,
        contexts: Vec::new(),
        scopes: vec![ScopeSpec {
            scope_id: scope_id.clone(),
            parent_scope_id: None,
            stable_key: StableAuthorKey::new("root").expect("root key"),
            planning_lineage: empty_planning_lineage.clone(),
        }],
        seeds: vec![SeedSpec {
            seed_id: seed_id.clone(),
            seed_key: StableAuthorKey::new("launch-input").expect("seed key"),
            cell_id: seed_cell.clone(),
            scope_id: scope_id.clone(),
            semantic_type_id: semantic_id.clone(),
            schema_id: value_schema.clone(),
            required_digest: Some(content(0x52)),
        }],
        descriptor_identities: vec![
            DescriptorIdentity::State(Box::new(StateDescriptorIdentity {
                descriptor_id: state_descriptor.clone(),
                name: "mfm.spec.test.state.multiply".to_owned(),
                state_kind: StateKind::new(
                    "mfm.spec.test.state",
                    "multiply",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_A,
                )
                .expect("state kind"),
                state_version: StateVersion::new("mfm.spec.test.state.multiply.v1")
                    .expect("state version"),
                context: StateContextDescriptorSpec::no_context(),
                input_context: StateInputContextContractSpec::no_context(),
                output_context: StateOutputContextContractSpec::no_context(),
                config_schema_id: config_schema.clone(),
                input_schema_id: input_schema.clone(),
                output_schema_id: value_schema.clone(),
                output_semantic_type_id: semantic_id.clone(),
                effect_kind: EffectKind::new(
                    "mfm.kernel.effect",
                    "pure",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_B,
                )
                .expect("effect kind"),
                effect_class: "pure".to_owned(),
                effect_name: "pure".to_owned(),
                effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
                capabilities: no_caps.clone(),
                emitted_fact_descriptors: Vec::new(),
                runner: "pure".to_owned(),
                effect_contract_digest: None,
            })),
            DescriptorIdentity::State(Box::new(StateDescriptorIdentity {
                descriptor_id: bridge_state_descriptor.clone(),
                name: "mfm.framework.bridge_same_value".to_owned(),
                state_kind: StateKind::new(
                    "mfm.framework.state",
                    "same_value_bridge",
                    DigestAlgorithm::Sha256JcsV1,
                    digest(0x55),
                )
                .expect("bridge state kind"),
                state_version: StateVersion::new("mfm.framework.state.same_value_bridge.v1")
                    .expect("bridge state version"),
                context: StateContextDescriptorSpec::no_context(),
                input_context: StateInputContextContractSpec::no_context(),
                output_context: StateOutputContextContractSpec::no_context(),
                config_schema_id: config_schema.clone(),
                input_schema_id: value_schema.clone(),
                output_schema_id: value_schema.clone(),
                output_semantic_type_id: semantic_id.clone(),
                effect_kind: EffectKind::new(
                    "mfm.kernel.effect",
                    "pure",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_B,
                )
                .expect("effect kind"),
                effect_class: "pure".to_owned(),
                effect_name: "pure".to_owned(),
                effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
                capabilities: no_caps.clone(),
                emitted_fact_descriptors: Vec::new(),
                runner: "pure".to_owned(),
                effect_contract_digest: None,
            })),
            DescriptorIdentity::State(Box::new(StateDescriptorIdentity {
                descriptor_id: render_state_descriptor.clone(),
                name: "mfm.framework.render_public_outputs".to_owned(),
                state_kind: StateKind::new(
                    "mfm.framework.state",
                    "render_public_outputs",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_C,
                )
                .expect("render state kind"),
                state_version: StateVersion::new("mfm.framework.state.render_public_outputs.v1")
                    .expect("render state version"),
                context: StateContextDescriptorSpec::no_context(),
                input_context: StateInputContextContractSpec::no_context(),
                output_context: StateOutputContextContractSpec::no_context(),
                config_schema_id: config_schema.clone(),
                input_schema_id: public_schema.clone(),
                output_schema_id: public_schema.clone(),
                output_semantic_type_id: receipt_semantic_id.clone(),
                effect_kind: EffectKind::new(
                    "mfm.kernel.effect",
                    "managed_platform_write",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_D,
                )
                .expect("managed effect kind"),
                effect_class: "managed_platform_write".to_owned(),
                effect_name: "managed_platform_write".to_owned(),
                effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
                capabilities: no_caps.clone(),
                emitted_fact_descriptors: Vec::new(),
                runner: "managed_platform_write".to_owned(),
                effect_contract_digest: None,
            })),
            DescriptorIdentity::Operation(Box::new(OperationDescriptorIdentity {
                descriptor_id: operation_descriptor_id.clone(),
                name: "mfm.spec.test.operation.multiply".to_owned(),
                operation_kind: OperationKind::new(
                    "mfm.spec.test.operation",
                    "multiply",
                    DigestAlgorithm::Sha256JcsV1,
                    digest(0x71),
                )
                .expect("operation kind"),
                operation_version: OperationVersion::new("mfm.spec.test.operation.multiply.v1")
                    .expect("operation version"),
                config_schema_id: config_schema.clone(),
                input_schema_id: input_schema.clone(),
                output_schema_id: value_schema.clone(),
                expansion_abi: "mfm.typed.operation.expansion.v1".to_owned(),
            })),
            DescriptorIdentity::Renderer(Box::new(renderer_descriptor.clone())),
        ],
        config_refs: vec![config_ref.clone()],
        nodes: vec![
            NodeSpec {
                node_id: state_node.clone(),
                stable_key: StableAuthorKey::new("multiply").expect("node key"),
                scope_id: scope_id.clone(),
                state_kind: StateKind::new(
                    "mfm.spec.test.state",
                    "multiply",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_A,
                )
                .expect("state kind"),
                state_version: StateVersion::new("mfm.spec.test.state.multiply.v1")
                    .expect("state version"),
                descriptor_id: state_descriptor,
                context: NodeContextSpec::no_context(),
                config_ref: config_ref.clone(),
                input_bindings: input_binding.clone(),
                output_cell: output_cell.clone(),
                effect_kind: EffectKind::new(
                    "mfm.kernel.effect",
                    "pure",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_B,
                )
                .expect("effect kind"),
                capability_bindings: no_caps.clone(),
                adapter_bindings: Vec::new(),
                fact_descriptor_allowlist: Vec::new(),
                side_effect: None,
                framework: None,
                planning_lineage: empty_planning_lineage.clone(),
                deterministic_predecessors: Vec::new(),
            },
            NodeSpec {
                node_id: bridge_node.clone(),
                stable_key: StableAuthorKey::new("bridge/output-to-root").expect("bridge key"),
                scope_id: scope_id.clone(),
                state_kind: StateKind::new(
                    "mfm.framework.state",
                    "same_value_bridge",
                    DigestAlgorithm::Sha256JcsV1,
                    digest(0x55),
                )
                .expect("bridge state kind"),
                state_version: StateVersion::new("mfm.framework.state.same_value_bridge.v1")
                    .expect("bridge state version"),
                descriptor_id: bridge_state_descriptor,
                context: NodeContextSpec::no_context(),
                config_ref: config_ref.clone(),
                input_bindings: InputBindingSpec {
                    input_schema_id: value_schema.clone(),
                    input_descriptor_id: descriptor(0x45),
                    root: InputBindingNodeSpec::Cell(Box::new(InputBindingCellSpec {
                        field_path: PublicFieldPath::new("value").expect("bridge input"),
                        cell_id: output_cell.clone(),
                        semantic_type_id: semantic_id.clone(),
                        schema_id: value_schema.clone(),
                        required_terminal: RequiredTerminal::ProducedOnly,
                        value_lineage: ValueLineageRef {
                            lineage_digest: content(0x21),
                        },
                        context: InputContextSpec::no_context(),
                    })),
                    digest: content(0x46),
                },
                output_cell: bridged_cell.clone(),
                effect_kind: EffectKind::new(
                    "mfm.kernel.effect",
                    "pure",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_B,
                )
                .expect("effect kind"),
                capability_bindings: no_caps.clone(),
                adapter_bindings: Vec::new(),
                fact_descriptor_allowlist: Vec::new(),
                side_effect: None,
                framework: Some(FrameworkNodeSpec::Bridge(BridgeNodeSpec {
                    bridge_kind: BridgeKind::ExportToParent,
                    source_scope_id: scope_id.clone(),
                    target_scope_id: scope_id.clone(),
                    source_cell_id: output_cell.clone(),
                    target_cell_id: bridged_cell.clone(),
                    semantic_type_id: semantic_id.clone(),
                    schema_id: value_schema.clone(),
                    policy: BridgePolicy::SameRunSameValue,
                    provenance: BridgeProvenance::FrameworkChildScopeV1,
                })),
                planning_lineage: empty_planning_lineage.clone(),
                deterministic_predecessors: vec![state_node.clone()],
            },
            NodeSpec {
                node_id: render_node.clone(),
                stable_key: StableAuthorKey::new("public-output/render").expect("render key"),
                scope_id: scope_id.clone(),
                state_kind: StateKind::new(
                    "mfm.framework.state",
                    "render_public_outputs",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_C,
                )
                .expect("render state kind"),
                state_version: StateVersion::new("mfm.framework.state.render_public_outputs.v1")
                    .expect("render state version"),
                descriptor_id: render_state_descriptor,
                context: NodeContextSpec::no_context(),
                config_ref: config_ref.clone(),
                input_bindings: InputBindingSpec {
                    input_schema_id: public_schema.clone(),
                    input_descriptor_id: descriptor(0x43),
                    root: InputBindingNodeSpec::Unit,
                    digest: content(0x44),
                },
                output_cell: receipt_cell.clone(),
                effect_kind: EffectKind::new(
                    "mfm.kernel.effect",
                    "managed_platform_write",
                    DigestAlgorithm::Sha256JcsV1,
                    DIGEST_D,
                )
                .expect("managed effect kind"),
                capability_bindings: no_caps.clone(),
                adapter_bindings: Vec::new(),
                fact_descriptor_allowlist: Vec::new(),
                side_effect: None,
                framework: Some(FrameworkNodeSpec::PublicOutputRender(
                    PublicOutputRenderNodeSpec {
                        public_schema_id: public_schema.clone(),
                        output_spec_digest,
                        renderer_descriptor,
                        required_cells: vec![public_cell],
                    },
                )),
                planning_lineage: empty_planning_lineage.clone(),
                deterministic_predecessors: vec![state_node.clone()],
            },
        ],
        remediations: BTreeMap::new(),
        cells: vec![
            CellSpec {
                cell_id: seed_cell.clone(),
                producer: CellProducer::Seed(seed_id.clone()),
                scope_id: scope_id.clone(),
                semantic_type_id: semantic_id.clone(),
                schema_id: value_schema.clone(),
                value_lineage: ValueLineageRef {
                    lineage_digest: content(0x20),
                },
                terminal_policy: CellTerminalPolicy::ProducedOnly,
                storage_policy: StoragePolicy::ContentAddressed,
                redaction_policy: RedactionPolicy::Public,
                context: CellContextSpec::no_context(),
            },
            CellSpec {
                cell_id: output_cell.clone(),
                producer: CellProducer::Node(state_node.clone()),
                scope_id: scope_id.clone(),
                semantic_type_id: semantic_id.clone(),
                schema_id: value_schema.clone(),
                value_lineage: ValueLineageRef {
                    lineage_digest: content(0x21),
                },
                terminal_policy: CellTerminalPolicy::ProducedOnly,
                storage_policy: StoragePolicy::ContentAddressed,
                redaction_policy: RedactionPolicy::Public,
                context: CellContextSpec::no_context(),
            },
            CellSpec {
                cell_id: bridged_cell.clone(),
                producer: CellProducer::Node(bridge_node.clone()),
                scope_id: scope_id.clone(),
                semantic_type_id: semantic_id.clone(),
                schema_id: value_schema.clone(),
                value_lineage: ValueLineageRef {
                    lineage_digest: content(0x23),
                },
                terminal_policy: CellTerminalPolicy::ProducedOnly,
                storage_policy: StoragePolicy::ContentAddressed,
                redaction_policy: RedactionPolicy::Public,
                context: CellContextSpec::no_context(),
            },
            CellSpec {
                cell_id: receipt_cell,
                producer: CellProducer::Node(render_node.clone()),
                scope_id: scope_id.clone(),
                semantic_type_id: receipt_semantic_id,
                schema_id: public_schema.clone(),
                value_lineage: ValueLineageRef {
                    lineage_digest: content(0x22),
                },
                terminal_policy: CellTerminalPolicy::ProducedOnly,
                storage_policy: StoragePolicy::PublicOutputArtifact,
                redaction_policy: RedactionPolicy::Public,
                context: CellContextSpec::no_context(),
            },
        ],
        value_lineages: vec![
            ValueLineage {
                lineage_ref: ValueLineageRef {
                    lineage_digest: content(0x20),
                },
                scope_id: scope_id.clone(),
                producer: CellProducer::Seed(seed_id),
                input_cells: Vec::new(),
                config_ref_digest: None,
                planning_lineage: empty_planning_lineage.clone(),
                domain_keys: Vec::new(),
                transform_policy: LineageTransformPolicy::Source,
            },
            ValueLineage {
                lineage_ref: ValueLineageRef {
                    lineage_digest: content(0x21),
                },
                scope_id: scope_id.clone(),
                producer: CellProducer::Node(state_node),
                input_cells: vec![seed_cell],
                config_ref_digest: Some(content(0x32)),
                planning_lineage: empty_planning_lineage.clone(),
                domain_keys: Vec::new(),
                transform_policy: LineageTransformPolicy::StateOutput,
            },
            ValueLineage {
                lineage_ref: ValueLineageRef {
                    lineage_digest: content(0x23),
                },
                scope_id: scope_id.clone(),
                producer: CellProducer::Node(bridge_node),
                input_cells: vec![output_cell.clone()],
                config_ref_digest: Some(content(0x32)),
                planning_lineage: empty_planning_lineage.clone(),
                domain_keys: Vec::new(),
                transform_policy: LineageTransformPolicy::SameValueBridge,
            },
            ValueLineage {
                lineage_ref: ValueLineageRef {
                    lineage_digest: content(0x22),
                },
                scope_id,
                producer: CellProducer::Node(render_node),
                input_cells: vec![output_cell.clone()],
                config_ref_digest: Some(content(0x32)),
                planning_lineage: empty_planning_lineage.clone(),
                domain_keys: Vec::new(),
                transform_policy: LineageTransformPolicy::StateOutput,
            },
        ],
        planning_lineage: vec![OperationLineageFrameSpec {
            operation_instance_id: operation_instance(0x80),
            operation_key: StableAuthorKey::new("multiply-operation").expect("operation key"),
            scope_id: scope(0x01),
            operation_descriptor_id,
            config_ref_digest: content(0x82),
            input_bindings: input_binding.clone(),
            input_binding_digest: content(0x83),
            parent_planning_lineage: empty_planning_lineage,
            output_cells: vec![output_cell],
            lineage_digest: content(0x84),
        }],
        public_outputs,
    })
    .expect("typed execution spec")
}

pub(super) fn manual_authorization(byte: u8) -> ManualResolutionAuthorizationSpec {
    ManualResolutionAuthorizationSpec {
        verifier_id: ManualAuthorizationVerifierId::new(format!("mfm.test.manual.verifier.{byte}"))
            .expect("verifier id"),
        signing_scheme: ManualSigningSchemeSpec::new("mfm.manual_resolution.digest_signature.v1")
            .expect("signing scheme"),
        authority: OperatorAuthoritySnapshotSpec {
            authority_id: OperatorAuthorityId::new(format!("mfm.test.manual.authority.{byte}"))
                .expect("authority id"),
            operators: vec![OperatorAuthorityMemberSpec {
                operator_id: OperatorId::new(format!("operator.{byte}")).expect("operator id"),
                public_identity: OperatorPublicIdentity::new(format!("operator-public-{byte}"))
                    .expect("operator public identity"),
            }],
        },
        quorum: ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}
