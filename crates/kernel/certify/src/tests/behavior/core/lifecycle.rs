use super::*;

#[test]
fn typed_spec_requires_registry_authority() {
    let spec = certify_program_draft(&reference_draft())
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    let error =
        certify_untrusted_typed_spec(spec, &CertificationRegistry::new()).expect_err("must reject");
    assert_eq!(
        error.problem_class(),
        Some(ProblemClass::InvalidSemanticTransition)
    );
}

#[test]
fn certification_rejects_node_fact_descriptor_outside_registered_descriptor() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        spec.nodes[0]
            .fact_descriptor_allowlist
            .push(spec::FactDescriptorRef {
                descriptor_hash: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0xa1),
                ),
            });
    });
}

#[test]
fn certification_carries_derive_fact_descriptor_allowlist_authority() {
    let draft = fact_emitting_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let certified = certify_program_draft(&draft).expect("certified");
    let expected_ref = mfm_program::fact_descriptor_ref::<ChainHeadFact>().expect("descriptor ref");
    let derived_descriptor = ChainHeadFact::descriptor().expect("derived descriptor");
    assert_eq!(
        mfm_program::facts::fact_descriptor_hash(&derived_descriptor).expect("descriptor hash"),
        expected_ref.descriptor_hash
    );

    let spec = certified.validated_spec().spec();
    let fact_node = spec
        .nodes
        .iter()
        .find(|node| {
            node.fact_descriptor_allowlist.as_slice() == std::slice::from_ref(&expected_ref)
        })
        .expect("fact-emitting node");
    let descriptor_identity = spec
        .descriptor_identities
        .iter()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == fact_node.descriptor_id =>
            {
                Some(state.as_ref())
            }
            _ => None,
        })
        .expect("fact-emitting descriptor identity");
    assert_eq!(
        descriptor_identity.emitted_fact_descriptors.as_slice(),
        std::slice::from_ref(&expected_ref)
    );

    let baseline_hash = certified.spec_hash().clone();
    let mut mutated = spec.clone();
    let mutated_node = mutated
        .nodes
        .iter_mut()
        .find(|node| node.descriptor_id == fact_node.descriptor_id)
        .expect("mutated fact node");
    mutated_node.fact_descriptor_allowlist.clear();
    let mutated_descriptor = mutated
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == fact_node.descriptor_id =>
            {
                Some(state.as_mut())
            }
            _ => None,
        })
        .expect("mutated descriptor identity");
    mutated_descriptor.emitted_fact_descriptors.clear();

    assert_ne!(
        mutated.spec_hash().expect("mutated spec hash"),
        baseline_hash
    );
    let error = certify_untrusted_typed_spec(mutated, &registry)
        .expect_err("tampered allow-list authority rejects");
    assert_eq!(
        error.problem_class(),
        Some(ProblemClass::InvalidSemanticTransition)
    );
}

#[test]
fn certification_rejects_forged_framework_descriptor_bypass() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let mut forged = spec
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                _ => None,
            })
            .expect("state descriptor");
        forged.name = "mfm.framework.forged".to_owned();
        forged.state_kind = state_kind_json("forged", serde_json::json!({ "framework": "forged" }))
            .expect("state kind");
        forged.state_version =
            StateVersion::new("mfm.framework.state.forged.v1").expect("state version");
        forged.descriptor_id = state_descriptor_id_from_spec(&forged).expect("descriptor id");
        spec.descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(forged)));
    });
}

#[test]
fn certification_accepts_valid_lifecycle_framework_node_shapes() {
    let (registry, spec) = reference_registry_and_spec();

    certify_untrusted_typed_spec(spec, &registry).expect("lifecycle framework nodes certify");
}

#[test]
fn certification_mints_descriptor_set_and_lifecycle_views() {
    let certified = certify_program_draft(&reference_draft()).expect("certified");
    let spec = certified.validated_spec().spec();
    let graph = certified.validated_spec().graph();
    let descriptors = certified.descriptor_set();
    let lifecycle = certified.framework_lifecycle();

    assert_eq!(graph.scope_count(), spec.scopes.len());
    assert_eq!(graph.config_ref_count(), spec.config_refs.len());
    assert_eq!(graph.value_lineage_count(), spec.value_lineages.len());
    assert_eq!(graph.cell_count(), spec.cells.len());
    assert_eq!(graph.forward_node_count(), spec.nodes.len());
    assert_eq!(graph.remediation_count(), spec.remediations.len());
    assert_eq!(graph.descriptors(), descriptors);
    assert!(descriptors.state_descriptors().count() > 0);
    assert!(descriptors
        .renderer(&spec.public_outputs.renderer_descriptor.descriptor_id)
        .is_some());
    assert!(descriptors
        .state(lifecycle.render().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.retention().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.complete().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.resolve().descriptor_id())
        .is_some());
    assert!(graph.cell(lifecycle.render().output_cell()).is_some());
    assert!(graph.forward_node(lifecycle.render().node_id()).is_some());
    assert_ne!(
        lifecycle.render().node_id(),
        lifecycle.retention().node_id()
    );
    assert_ne!(
        lifecycle.retention().output_cell(),
        lifecycle.complete().output_cell()
    );
}

#[test]
fn certification_rejects_missing_retention_lifecycle_node() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        spec.nodes.retain(|node| {
            !matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        });
    });
}

#[test]
fn certification_rejects_forged_lifecycle_framework_variant() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let forged = spec
            .nodes
            .iter()
            .find_map(|node| match &node.framework {
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
                    Some(retention.clone())
                }
                _ => None,
            })
            .expect("retention metadata");
        let node = spec
            .nodes
            .iter_mut()
            .find(|node| node.framework.is_none())
            .expect("user node");
        node.framework = Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(forged));
    });
}

#[test]
fn certification_rejects_lifecycle_descriptor_mismatch() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let mut forged = spec
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                _ => None,
            })
            .expect("state descriptor");
        forged.name = "mfm.framework.complete_run".to_owned();
        forged.state_kind = state_kind_json(
            "complete_run",
            serde_json::json!({ "framework": "complete_run" }),
        )
        .expect("complete state kind");
        forged.state_version = StateVersion::new("mfm.framework.state.complete_run.v1")
            .expect("complete state version");
        forged.runner = "forged".to_owned();
        forged.descriptor_id = state_descriptor_id_from_spec(&forged).expect("descriptor id");
        spec.descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(forged)));
    });
}

#[test]
fn certification_rejects_invalid_lifecycle_ordering() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        let public_cell = spec
            .public_outputs
            .outputs
            .first()
            .expect("public output")
            .cell_id
            .clone();
        append_retention_lifecycle_node(spec, public_cell);
    });
}

#[test]
fn certification_rejects_lifecycle_framework_permissions() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
        node.adapter_bindings.push(spec::AdapterBinding {
            adapter_kind: AdapterKind::new(
                "mfm.certify.test",
                "forged-adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0xa1),
            )
            .expect("adapter kind"),
            adapter_version: AdapterVersion::new("mfm.certify.test.adapter.v1")
                .expect("adapter version"),
            binding_digest: None,
        });
    });
}

#[test]
fn certification_rejects_user_consumers_of_lifecycle_receipts() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            let render_receipt = lifecycle_render_receipt_cell(spec);
            append_user_receipt_consumer(spec, render_receipt, "user/render-receipt");
        },
        ProblemClass::InvalidTopology => |spec| {
            let retention_receipt = lifecycle_retention_receipt_cell(spec);
            append_user_receipt_consumer(spec, retention_receipt, "user/retention-receipt");
        },
    );
}

#[test]
fn certification_rejects_executable_nodes_outside_lifecycle_tail() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        append_independent_user_node(spec, "user/outside-lifecycle-tail");
    });
}

#[test]
fn certification_rejects_forged_lifecycle_config_ref() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
        node.config_ref.byte_len += 1;
    });
}

#[test]
fn certification_rejects_builtin_lifecycle_descriptors_without_framework_metadata() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::ProjectRetentionManifestNodeSpec>(spec);
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::CompleteRunNodeSpec>(spec);
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::ResolveSagaTerminalNodeSpec>(spec);
        },
    );
}

#[test]
fn certification_rejects_forged_lifecycle_input_binding() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
            let current = match &node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
                _ => panic!("expected cell input"),
            };
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("public_output_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(current),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let node = find_lifecycle_node_mut::<spec::CompleteRunNodeSpec>(spec);
            let current = match &node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
                _ => panic!("expected cell input"),
            };
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(current),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let input_cell = spec.cells.first().expect("input cell").clone();
            let node = find_lifecycle_node_mut::<spec::ResolveSagaTerminalNodeSpec>(spec);
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                        field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                            .expect("field path"),
                        cell_id: input_cell.cell_id,
                        semantic_type_id: input_cell.semantic_type_id,
                        schema_id: input_cell.schema_id,
                        required_terminal: spec::RequiredTerminal::ProducedOnly,
                        value_lineage: input_cell.value_lineage,
                        context: spec::InputContextSpec::no_context(),
                    })),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
    );
}

#[test]
fn certification_rejects_operation_input_binding_tampering() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        let frame = spec
            .planning_lineage
            .first_mut()
            .expect("operation lineage frame");
        let input_cell = match &mut frame.input_bindings.root {
            spec::InputBindingNodeSpec::Cell(cell) => cell,
            _ => panic!("expected operation cell input"),
        };
        input_cell.cell_id = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x91));
        frame.input_bindings.digest =
            content_digest_json(input_node_json(&frame.input_bindings.root)).expect("input digest");
        frame.input_binding_digest = frame.input_bindings.digest.clone();
    });
}

#[test]
fn certification_rejects_stable_id_and_scope_tampering() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes[0].stable_key =
                spec::StableAuthorKey::new("renamed-node").expect("node key");
        },
        ProblemClass::InvalidTopology => |spec| {
            spec.scopes[0].parent_scope_id = Some(spec.scopes[0].scope_id.clone());
        },
    );
}

#[test]
fn certification_rejects_framework_lineage_tampering() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidDataMeaning => |spec| {
            spec.planning_lineage.clear();
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let lineage = spec
                .nodes
                .first_mut()
                .expect("node with operation planning lineage");
            lineage.planning_lineage.active_operation_instances = vec![
                OperationInstanceId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x92),
                ),
            ];
            lineage.planning_lineage.lineage_digest = planning_lineage_digest(
                &lineage.planning_lineage.active_operation_instances,
                &lineage.planning_lineage.completed_operation_frames,
            )
            .expect("planning lineage digest");
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let render_node_id = spec
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                    )
                })
                .expect("render node")
                .node_id
                .clone();
            let lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| {
                    lineage.producer == spec::CellProducer::Node(render_node_id.clone())
                })
                .expect("render lineage");
            lineage.input_cells.clear();
        },
    );
}
