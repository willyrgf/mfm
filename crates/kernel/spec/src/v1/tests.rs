use super::*;
use mfm_capabilities::CapabilitySetDescriptor;
use mfm_ids::{DigestBytes, SemanticTypeId};

const DIGEST_A: DigestBytes = DigestBytes::from_array([0x11; 32]);
const DIGEST_B: DigestBytes = DigestBytes::from_array([0x22; 32]);
const DIGEST_C: DigestBytes = DigestBytes::from_array([0x33; 32]);
const DIGEST_D: DigestBytes = DigestBytes::from_array([0x44; 32]);

fn digest(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn descriptor(byte: u8) -> DescriptorId {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn artifact(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn node(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn cell(byte: u8) -> CellId {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn scope(byte: u8) -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn seed(byte: u8) -> SeedId {
    SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn operation_instance(byte: u8) -> OperationInstanceId {
    OperationInstanceId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn schema(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest(byte)).expect("schema id")
}

fn semantic(name: &str, byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.spec.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest(byte),
    )
    .expect("semantic id")
}

fn test_spec() -> TypedExecutionSpec {
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
                runner: "pure".to_owned(),
                side_effect_contract_digest: None,
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
                runner: "pure".to_owned(),
                side_effect_contract_digest: None,
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
                runner: "managed_platform_write".to_owned(),
                side_effect_contract_digest: None,
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

#[test]
fn certified_spec_hash_golden() {
    let spec = test_spec();
    let canonical = spec.canonical_json().expect("canonical spec");
    assert_eq!(
        spec.public_outputs
            .digest()
            .expect("public output digest")
            .as_str(),
        "content:sha256-jcs-v1:e8d43ce660104518546731c9e7120d4df954e4f610e3c295996fceb377d27f4e"
    );
    assert_eq!(
        spec.spec_hash().expect("spec hash").as_str(),
        "spec:sha256-jcs-v1:87d706df0b869b002f44c3de373d2825969c828dcaa315b355997cc23ab32857"
    );
    assert!(canonical
        .as_str()
        .contains(r#""spec_version":"mfm.typed.execution_spec.v1""#));
    assert!(canonical.as_str().contains(r#""remediations":{}"#));
    assert!(canonical
        .as_str()
        .contains(r#""saga":{"kind":"no_side_effects"}"#));
    let audit = TypedExecutionSpecAudit {
        source_package_refs: vec![SourcePackageRef {
            name: "mfm-spec-test".to_owned(),
            version: "0.1.0".to_owned(),
            artifact_id: Some(artifact(0x90)),
        }],
        ..TypedExecutionSpecAudit::default()
    };
    let envelope = HashedSpecEnvelope::new(spec.clone(), audit).expect("env");
    assert_eq!(envelope.spec_hash, spec.spec_hash().expect("spec hash"));
    envelope.verify_hash().expect("hash verifies");

    let mut stale = envelope;
    stale.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0x99));
    assert!(matches!(
        stale.verify_hash(),
        Err(SpecError::HashMismatch { .. })
    ));
}

#[test]
fn saga_policy_and_remediations_are_hash_defining() {
    let base = test_spec();
    let base_hash = base.spec_hash().expect("base hash");
    let manual = ManualResolutionEvidenceSpec {
        evidence_schema: schema("mfm.spec.test.manual_evidence", 0x91),
        authorization: manual_authorization(0x92),
    };

    let mut manual_spec = base.clone();
    manual_spec.saga = SagaPolicySpec::ManualResolution {
        manual: manual.clone(),
    };
    assert_ne!(
        manual_spec.spec_hash().expect("manual hash"),
        base_hash,
        "run-level saga policy must be hash-defining"
    );

    let mut compensated = base.clone();
    let forward_node = compensated.nodes[0].node_id.clone();
    compensated.saga = SagaPolicySpec::CompensateCompleted {
        on_remediation_unresolved: RemediationUnresolvedSpec::ManualResolution {
            manual: Box::new(manual),
        },
    };
    compensated
        .remediations
        .insert(forward_node, compensated.nodes[0].clone());
    let canonical = compensated.canonical_json().expect("compensated canonical");
    let parsed = TypedExecutionSpec::from_json_slice(canonical.as_bytes())
        .expect("parse compensated saga spec");

    assert_eq!(parsed, compensated);
    assert_ne!(
        compensated.spec_hash().expect("compensated hash"),
        base_hash,
        "remediation node collection must be hash-defining"
    );
}

fn manual_authorization(byte: u8) -> ManualResolutionAuthorizationSpec {
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

#[test]
fn resource_claims_are_mandatory_and_hash_defining() {
    let mut manual = test_spec();
    manual.nodes[0].side_effect = Some(SideEffectContractSpec {
        contract_digest: content(0x91),
        resource_claim: ResourceClaimSpec::ManualOnly,
        verification: SideEffectVerificationSpec::Receipt,
    });
    let manual_hash = manual.spec_hash().expect("manual claim hash");

    let mut exclusive = manual.clone();
    exclusive.nodes[0]
        .side_effect
        .as_mut()
        .expect("side-effect")
        .resource_claim = ResourceClaimSpec::Exclusive {
        namespace: ResourceNamespace::new("mfm.spec.test.account_nonce").expect("namespace"),
        key_schema: schema("mfm.spec.test.resource_key", 0x92),
    };
    assert_ne!(
        exclusive.spec_hash().expect("exclusive claim hash"),
        manual_hash,
        "side-effect resource claim must be hash-defining"
    );

    let canonical = exclusive.canonical_json().expect("exclusive canonical");
    let parsed = TypedExecutionSpec::from_json_slice(canonical.as_bytes())
        .expect("parse exclusive resource claim");
    assert_eq!(parsed, exclusive);

    let mut missing: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    missing["nodes"][0]["side_effect"]
        .as_object_mut()
        .expect("side-effect object")
        .remove("resource_claim");
    let missing = serde_json::to_string(&missing).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&missing).expect_err("missing resource claim rejects");
    assert!(matches!(err, SpecError::Json(message) if message.contains("resource_claim")));

    let mut missing: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    missing["nodes"][0]["side_effect"]
        .as_object_mut()
        .expect("side-effect object")
        .remove("verification");
    let missing = serde_json::to_string(&missing).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&missing).expect_err("missing verification rejects");
    assert!(matches!(err, SpecError::Json(message) if message.contains("verification")));

    let mut finalized_1 = exclusive.clone();
    finalized_1.nodes[0]
        .side_effect
        .as_mut()
        .expect("side-effect")
        .verification = SideEffectVerificationSpec::Finalized { depth: 1 };
    let mut finalized_12 = exclusive.clone();
    finalized_12.nodes[0]
        .side_effect
        .as_mut()
        .expect("side-effect")
        .verification = SideEffectVerificationSpec::Finalized { depth: 12 };
    assert_ne!(
        finalized_1.spec_hash().expect("finalized one hash"),
        exclusive.spec_hash().expect("receipt hash"),
        "side-effect verification policy must be hash-defining"
    );
    assert_ne!(
        finalized_12.spec_hash().expect("finalized twelve hash"),
        finalized_1.spec_hash().expect("finalized one hash"),
        "side-effect verification depth must be hash-defining"
    );

    let mut unknown: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    unknown["nodes"][0]["side_effect"]["resource_claim"]["kind"] = serde_json::json!("optimistic");
    let unknown = serde_json::to_string(&unknown).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&unknown).expect_err("unknown resource claim rejects");
    assert!(matches!(
        err,
        SpecError::Json(message) if message.contains("unsupported resource claim kind")
    ));

    let mut unknown: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    unknown["nodes"][0]["side_effect"]["verification"]["kind"] =
        serde_json::json!("mutable_registry");
    let unknown = serde_json::to_string(&unknown).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&unknown).expect_err("unknown verification rejects");
    assert!(matches!(
        err,
        SpecError::Json(message) if message.contains("unsupported side-effect verification kind")
    ));

    let mut zero_depth: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    zero_depth["nodes"][0]["side_effect"]["verification"] = serde_json::json!({
        "finalized": {
            "depth": 0,
        },
        "kind": "finalized",
    });
    let zero_depth = serde_json::to_string(&zero_depth).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&zero_depth).expect_err("zero finalized depth rejects");
    assert!(matches!(
        err,
        SpecError::Json(message) if message.contains("depth must be positive")
    ));
}

#[test]
fn persisted_spec_json_round_trips_through_checked_parser() {
    let spec = test_spec();
    let canonical = spec.canonical_json().expect("canonical spec");
    let parsed =
        TypedExecutionSpec::from_json_slice(canonical.as_bytes()).expect("parse canonical spec");

    assert_eq!(parsed, spec);
    assert_eq!(
        parsed.canonical_json().expect("canonical parsed"),
        canonical
    );
    assert_eq!(
        parsed.spec_hash().expect("parsed hash"),
        spec.spec_hash().expect("spec hash")
    );
}

#[test]
fn persisted_node_descriptor_refs_are_checked_against_descriptor_table() {
    let spec = test_spec();
    let canonical = spec.canonical_json().expect("canonical spec");
    let value: serde_json::Value = serde_json::from_str(canonical.as_str()).expect("spec JSON");
    let first_node = &value["nodes"][0];

    assert!(first_node.get("descriptor_ref").is_some());
    assert!(first_node.get("descriptor_id").is_none());
    assert!(first_node.get("state_kind").is_none());
    assert!(first_node.get("state_version").is_none());
    assert!(first_node.get("effect_kind").is_none());
    assert!(first_node.get("capability_bindings").is_none());

    let mut wrong_digest = value.clone();
    wrong_digest["nodes"][0]["descriptor_ref"]["descriptor_digest"] =
        serde_json::json!(content(0xfe).as_str());
    let input = serde_json::to_string(&wrong_digest).expect("JSON");
    let err =
        TypedExecutionSpec::from_json_str(&input).expect_err("descriptor digest mismatch rejects");
    assert!(err.to_string().contains("descriptor ref mismatch"), "{err}");

    let mut wrong_family = value;
    wrong_family["nodes"][0]["descriptor_ref"]["descriptor_family"] =
        serde_json::json!("operation");
    let input = serde_json::to_string(&wrong_family).expect("JSON");
    let err =
        TypedExecutionSpec::from_json_str(&input).expect_err("descriptor family mismatch rejects");
    assert!(err.to_string().contains("expected state"), "{err}");
}

#[test]
fn lifecycle_framework_node_json_round_trips_through_checked_parser() {
    let mut variants = Vec::new();
    variants.push(FrameworkNodeSpec::ProjectRetentionManifest(
        ProjectRetentionManifestNodeSpec {
            public_schema_id: schema("mfm.spec.test.lifecycle_public", 0x70),
            public_output_receipt_cell: cell(0x71),
        },
    ));
    variants.push(FrameworkNodeSpec::CompleteRun(CompleteRunNodeSpec {
        public_schema_id: schema("mfm.spec.test.lifecycle_complete", 0x72),
        retention_manifest_receipt_cell: cell(0x73),
    }));
    variants.push(FrameworkNodeSpec::ResolveSagaTerminal(
        ResolveSagaTerminalNodeSpec {
            public_schema_id: schema("mfm.spec.test.lifecycle_resolve", 0x74),
        },
    ));

    for framework in variants {
        let mut spec = test_spec();
        spec.nodes[0].framework = Some(framework);
        let canonical = spec.canonical_json().expect("canonical spec");
        let parsed = TypedExecutionSpec::from_json_slice(canonical.as_bytes())
            .expect("parse lifecycle framework node");

        assert_eq!(parsed, spec);
    }
}

#[test]
fn persisted_spec_json_rejects_unknown_fields() {
    let spec = test_spec();
    let mut value: serde_json::Value =
        serde_json::from_str(spec.canonical_json().expect("canonical spec").as_str())
            .expect("spec JSON");
    value
        .as_object_mut()
        .expect("spec object")
        .insert("unknown_field".to_owned(), serde_json::json!(true));
    let input = serde_json::to_string(&value).expect("JSON");

    let err = TypedExecutionSpec::from_json_str(&input).expect_err("unknown field rejects");

    assert!(matches!(err, SpecError::Json(message) if message.contains("unknown")));
}
