use super::*;

#[test]
fn certification_rejects_side_effect_contract_mismatch() {
    assert_side_effect_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let node = spec
            .nodes
            .iter_mut()
            .find(|node| node.side_effect.is_some())
            .expect("side-effect node");
        node.side_effect = Some(spec::SideEffectContractSpec {
            contract_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x63),
            ),
            resource_claim: spec::ResourceClaimSpec::ManualOnly,
            verification: spec::SideEffectVerificationSpec::Receipt,
        });
    });
}

#[test]
fn certification_lowers_side_effect_submit_verify_pair() {
    let (_registry, typed) = side_effect_registry_and_spec();
    let submit = side_effect_submit_node(&typed);
    let (verify_node, verify) = side_effect_verify_node(&typed);
    let contract = submit.side_effect.as_ref().expect("submit side effect");
    let expected_pair =
        spec::side_effect_pair_id(&submit.node_id, &submit.output_cell, contract).expect("pair id");
    let pair = typed
        .side_effect_verify_pair_for_pair_id(&verify.pair_id)
        .expect("side-effect pair");

    assert_eq!(verify.submit_node_id, submit.node_id);
    assert_eq!(pair.submit_output_cell, &submit.output_cell);
    assert_eq!(verify.pair_id, expected_pair);
    assert_eq!(
        verify_node.deterministic_predecessors,
        vec![submit.node_id.clone()]
    );
    assert_ne!(verify_node.output_cell, submit.output_cell);
    assert!(typed
        .public_outputs
        .outputs
        .iter()
        .any(|output| output.cell_id == verify_node.output_cell));
    assert!(!typed
        .public_outputs
        .outputs
        .iter()
        .any(|output| output.cell_id == submit.output_cell));
}

#[test]
fn test_local_manual_resolution_draft_certifies() {
    let (draft, manual) = manual_resolution_draft();
    let mut registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    registry
        .register_schema_role(
            manual.evidence_schema.clone(),
            CertifiedSchemaRole::ManualResolutionEvidence,
        )
        .expect("manual evidence role");
    registry
        .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
        .expect("manual verifier");
    registry
        .register_operator_authority_snapshot(manual.authorization.authority.clone())
        .expect("manual authority");

    let lowered = lower_program_draft(&draft).expect("lowered manual draft");
    let certified = certify_typed_spec(lowered, &registry).expect("certified manual draft");
    assert!(matches!(
        certified.validated_spec().spec().saga,
        spec::SagaPolicySpec::ManualResolution { .. }
    ));
    assert!(certified
        .validated_spec()
        .spec()
        .nodes
        .iter()
        .any(|node| matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        )));
}

#[test]
fn persisted_side_effect_verify_specs_reject_unknown_submit_output_cell_id() {
    let (_registry, typed) = side_effect_registry_and_spec();
    let submit_output_cell = side_effect_submit_node(&typed).output_cell.clone();
    let mut value: serde_json::Value =
        serde_json::from_str(typed.canonical_json().expect("canonical spec").as_str())
            .expect("typed spec JSON");
    let verify_node = value["nodes"]
        .as_array_mut()
        .expect("nodes")
        .iter_mut()
        .find(|node| node["framework"]["kind"] == "side_effect_verify")
        .expect("verify node");
    verify_node["framework"]["side_effect_verify"]
        .as_object_mut()
        .expect("verify object")
        .insert(
            "submit_output_cell_id".to_owned(),
            serde_json::json!(submit_output_cell.as_str()),
        );
    let input = serde_json::to_string(&value).expect("JSON");

    let error = spec::TypedExecutionSpec::from_json_str(&input)
        .expect_err("unknown submit output anchor must reject");

    assert!(error.to_string().contains("unknown or non-normalized"));
}

#[test]
fn certification_rejects_invalid_side_effect_verify_pair_shapes() {
    assert_rejection_cases!(
        side_effect_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes.retain(|node| {
                !matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
                )
            });
        },
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes.retain(|node| node.side_effect.is_none());
        },
        ProblemClass::InvalidTopology => |spec| {
            let duplicate = side_effect_verify_node(spec).0.clone();
            spec.nodes.push(duplicate);
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let submit_node_id = side_effect_submit_node(spec).node_id.clone();
            let verify_output_cell = side_effect_verify_node(spec).0.output_cell.clone();
            let cell = spec
                .cells
                .iter_mut()
                .find(|cell| cell.cell_id == verify_output_cell)
                .expect("verify output cell");
            cell.producer = spec::CellProducer::Node(submit_node_id);
        },
        ProblemClass::InvalidTerminalShape => |spec| {
            let submit_output_cell = side_effect_submit_node(spec).output_cell.clone();
            let submit_cell = spec
                .cells
                .iter()
                .find(|cell| cell.cell_id == submit_output_cell)
                .expect("submit output cell")
                .clone();
            let public_output = spec
                .public_outputs
                .outputs
                .first_mut()
                .expect("public output");
            public_output.cell_id = submit_cell.cell_id;
            public_output.producer = submit_cell.producer;
            public_output.scope_id = submit_cell.scope_id;
            public_output.semantic_type_id = submit_cell.semantic_type_id;
            public_output.schema_id = submit_cell.schema_id;
            public_output.value_lineage = submit_cell.value_lineage;
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let submit_output_cell = side_effect_submit_node(spec).output_cell.clone();
            let wrong_cell = spec
                .cells
                .iter()
                .find(|cell| cell.cell_id != submit_output_cell)
                .expect("wrong cell")
                .clone();
            let wrong_predecessors =
                predecessors_for_test_inputs(spec, std::slice::from_ref(&wrong_cell.cell_id));
            let verify_node = spec
                .nodes
                .iter_mut()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
                    )
                })
                .expect("verify node");
            verify_node.input_bindings = spec::framework_lifecycle_maybe_skipped_cell_input_binding(
                "side_effect_verify",
                "submit_output",
                &wrong_cell,
            )
            .expect("wrong verify input binding");
            verify_node.deterministic_predecessors = wrong_predecessors;
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let submit_output_cell = side_effect_submit_node(spec).output_cell.clone();
            let wrong_cell = spec
                .cells
                .iter()
                .find(|cell| cell.cell_id != submit_output_cell)
                .expect("wrong cell")
                .cell_id
                .clone();
            let verify_node = side_effect_verify_node(spec).0.clone();
            let lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| {
                    lineage.producer == spec::CellProducer::Node(verify_node.node_id.clone())
                })
                .expect("verify output lineage");
            lineage.input_cells = vec![wrong_cell];
            lineage.lineage_ref.lineage_digest =
                value_lineage_digest(lineage).expect("lineage digest");
            let lineage_ref = lineage.lineage_ref.clone();
            let output = spec
                .cells
                .iter_mut()
                .find(|cell| cell.cell_id == verify_node.output_cell)
                .expect("verify output cell");
            output.value_lineage = lineage_ref;
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            let submit_output_cell = side_effect_submit_node(spec).output_cell.clone();
            append_user_receipt_consumer(spec, submit_output_cell, "user/submit-output");
        },
    );
}

#[test]
fn side_effect_verification_policy_is_spec_authority_not_registry_authority() {
    let receipt = side_effect_draft_with_verification(program::SideEffectVerificationSpec::Receipt);
    let finalized_1 =
        side_effect_draft_with_verification(program::SideEffectVerificationSpec::Finalized {
            depth: 1,
        });
    let finalized_12 =
        side_effect_draft_with_verification(program::SideEffectVerificationSpec::Finalized {
            depth: 12,
        });

    let receipt_registry =
        CertificationRegistry::from_program_draft(&receipt).expect("receipt registry");
    let finalized_registry =
        CertificationRegistry::from_program_draft(&finalized_12).expect("finalized registry");
    assert_eq!(
        receipt_registry.digest().expect("receipt registry digest"),
        finalized_registry
            .digest()
            .expect("finalized registry digest"),
        "verification policy must not be resolved from registry authority"
    );

    let receipt_spec = certify_program_draft(&receipt)
        .expect("receipt certified")
        .validated_spec()
        .spec()
        .spec_hash()
        .expect("receipt hash");
    let finalized_1_spec = certify_program_draft(&finalized_1)
        .expect("finalized one certified")
        .validated_spec()
        .spec()
        .spec_hash()
        .expect("finalized one hash");
    let finalized_12_spec = certify_program_draft(&finalized_12)
        .expect("finalized twelve certified")
        .validated_spec()
        .spec()
        .spec_hash()
        .expect("finalized twelve hash");
    assert_ne!(receipt_spec, finalized_1_spec);
    assert_ne!(finalized_1_spec, finalized_12_spec);
}

#[test]
fn certifies_compensating_draft_with_separate_remediation_collection() {
    let draft = compensating_draft();
    let certified = certify_program_draft(&draft).expect("certified compensating draft");
    assert!(matches!(
        certified.validated_spec().spec().saga,
        spec::SagaPolicySpec::CompensateCompleted { .. }
    ));
    assert_eq!(certified.validated_spec().spec().remediations.len(), 1);
    let validated = certified.validated_spec().spec();
    let (forward_id, remediation) = validated.remediations.iter().next().expect("remediation");
    assert!(validated
        .nodes
        .iter()
        .any(|node| node.node_id == *forward_id));
    assert!(!validated
        .nodes
        .iter()
        .any(|node| node.node_id == remediation.node_id));
    let pair = validated
        .side_effect_verify_pair_for_submit_node(&remediation.node_id)
        .expect("remediation verify pair");
    assert_eq!(pair.submit_node.node_id, remediation.node_id);
    assert_eq!(pair.submit_output_cell, &remediation.output_cell);
}

#[test]
fn certified_side_effect_contract_validates_resource_evidence() {
    let (_, mut typed) = side_effect_spec_with_manual(manual_resolution_spec(0xc0));
    let node = typed
        .nodes
        .iter_mut()
        .find(|node| node.side_effect.is_some())
        .expect("side-effect node");
    let node_id = node.node_id.clone();
    let namespace = spec::ResourceNamespace::new("mfm.certify.test.wallet").expect("namespace");
    let key_schema = SchemaId::new(
        "mfm.certify.test.resource_key",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_byte(0xc1),
    )
    .expect("key schema");
    node.side_effect
        .as_mut()
        .expect("side effect")
        .resource_claim = spec::ResourceClaimSpec::Exclusive {
        namespace: namespace.clone(),
        key_schema: key_schema.clone(),
    };
    let contract =
        CertifiedSideEffectContract::for_node(&typed, &node_id).expect("certified contract");
    let key = events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema,
        key: events::ResourceKey::new("wallet-1").expect("resource key"),
    };
    assert!(contract.validate_resource_key(Some(&key)).is_ok());
    assert!(matches!(
        contract
            .validate_resource_key(None)
            .expect_err("missing key")
            .problem_class(),
        Some(ProblemClass::InvalidSemanticTransition)
    ));
    let touched_set = events::ResourceTouchedSetEvidence {
        namespace,
        evidence_schema_id: SchemaId::new(
            "mfm.certify.test.touched_set",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc2),
        )
        .expect("touched schema"),
        evidence_hash: ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xc3)),
        evidence_artifact_id: ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc3),
        ),
        evidence_artifact_evidence_hash: ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc4),
        ),
    };
    assert!(contract.validate_touched_set(Some(&touched_set)).is_err());
}

#[test]
fn certified_side_effect_contract_validates_remediation_links() {
    let (_, typed) = compensating_registry_and_spec();
    let (forward_node_id, remediation) = typed.remediations.iter().next().expect("remediation");
    let contract = CertifiedSideEffectContract::for_node(&typed, &remediation.node_id)
        .expect("remediation contract");
    let run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xc4));
    let purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: mfm_ids::SideEffectPairId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc5),
        ),
    };
    let forward_purpose = events::SideEffectLedgerPurpose::Forward;

    assert!(contract
        .validate_remediation_link(CertifiedRemediationLink {
            remediation_run_id: &run_id,
            ledger_purpose: &purpose,
            forward_run_id: None,
            forward_node_id: None,
            forward_ledger_purpose: None,
            forward_terminal: false,
        })
        .is_err());
    assert!(contract
        .validate_remediation_link(CertifiedRemediationLink {
            remediation_run_id: &run_id,
            ledger_purpose: &purpose,
            forward_run_id: Some(&run_id),
            forward_node_id: Some(forward_node_id),
            forward_ledger_purpose: Some(&forward_purpose),
            forward_terminal: true,
        })
        .is_ok());
}
