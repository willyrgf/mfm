use super::*;

#[test]
fn certification_rejects_saga_policy_graph_disagreement() {
    let (registry, side_effecting) = side_effect_registry_and_spec();

    assert_rejects(
        &registry,
        &side_effecting,
        ProblemClass::InvalidSemanticTransition,
        |spec| {
            spec.saga = spec::SagaPolicySpec::NoSideEffects;
        },
    );

    let (registry, pure) = reference_registry_and_spec();
    assert_rejects(
        &registry,
        &pure,
        ProblemClass::InvalidSemanticTransition,
        |spec| {
            spec.saga = spec::SagaPolicySpec::FailWithoutAcdcClaim;
        },
    );
}

#[test]
fn certification_rejects_invalid_remediation_keys_and_coverage() {
    assert_rejection_cases!(
        compensating_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            let (_, mut remediation) = spec.remediations.pop_first().expect("remediation");
            remediation.node_id = node_id(0xb1);
            spec.remediations.insert(node_id(0xb0), remediation);
        },
        ProblemClass::InvalidTopology => |spec| {
            let remediation = spec
                .remediations
                .values()
                .next()
                .expect("remediation")
                .clone();
            let mut cloned = remediation;
            cloned.node_id = node_id(0xb2);
            let lifecycle_node = find_lifecycle_node_mut::<spec::PublicOutputRenderNodeSpec>(spec)
                .node_id
                .clone();
            spec.remediations.insert(lifecycle_node, cloned);
        },
        ProblemClass::InvalidTopology => |spec| {
            spec.remediations.clear();
        },
    );
}

#[test]
fn certification_rejects_invalid_remediation_node_shapes() {
    assert_rejection_cases!(
        compensating_registry_and_spec,
        ProblemClass::InvalidSemanticTransition => |spec| {
            spec.saga = spec::SagaPolicySpec::FailWithoutAcdcClaim;
        },
        ProblemClass::InvalidTopology => |spec| {
            let forward_id = spec
                .remediations
                .keys()
                .next()
                .expect("forward key")
                .clone();
            spec.remediations
                .get_mut(&forward_id)
                .expect("remediation")
                .node_id = forward_id.clone();
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            spec.remediations
                .values_mut()
                .next()
                .expect("remediation")
                .side_effect = None;
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            spec.remediations
                .values_mut()
                .next()
                .expect("remediation")
                .state_version =
                StateVersion::new("mfm.certify.test.forged.v1").expect("state version");
        },
    );
}

#[test]
fn certification_rejects_out_of_scope_remediation_bindings() {
    assert_compensating_rejects(ProblemClass::InvalidTopology, |spec| {
        let render_receipt = lifecycle_render_receipt_cell(spec);
        retarget_first_remediation_input(spec, render_receipt);
    });
}

#[test]
fn certification_rejects_manual_policy_without_schemas_at_decode_boundary() {
    let (_, typed) = side_effect_spec_with_manual(manual_resolution_spec(0xc1));
    let mut value: serde_json::Value =
        serde_json::from_str(typed.canonical_json().expect("canonical spec").as_str())
            .expect("spec JSON");
    value["saga"]["manual"]
        .as_object_mut()
        .expect("manual object")
        .remove("evidence_schema");
    let input = serde_json::to_string(&value).expect("JSON");
    let error = spec::TypedExecutionSpec::from_json_str(&input)
        .expect_err("missing manual evidence schema rejects");
    assert!(error.to_string().contains("evidence_schema"), "{error}");
}

#[test]
fn certification_records_manual_authority_evidence() {
    let manual = manual_resolution_spec(0xc1);
    let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
    register_manual_authority(&mut registry, &manual);

    let certified = certify_untrusted_typed_spec(typed, &registry).expect("certified manual spec");

    assert_eq!(
        certified
            .certificate()
            .evidence
            .schema_role_grants
            .as_slice(),
        &[CertifiedSchemaRoleGrantEvidence {
            schema_id: manual.evidence_schema.clone(),
            role: CertifiedSchemaRole::ManualResolutionEvidence,
        }]
    );
    assert_eq!(
        certified
            .certificate()
            .evidence
            .manual_authorization_verifiers
            .as_slice(),
        &[CertifiedManualAuthorizationVerifierEvidence {
            verifier_id: manual.authorization.verifier_id.clone(),
        }]
    );
    assert_eq!(
        certified
            .certificate()
            .evidence
            .operator_authority_snapshots
            .len(),
        1
    );
    assert_eq!(
        certified
            .certificate()
            .evidence
            .operator_authority_snapshots[0]
            .authority_id,
        manual.authorization.authority.authority_id
    );
}

#[test]
fn certification_rejects_invalid_manual_authority_cases() {
    #[derive(Clone, Copy)]
    enum Case {
        UnknownEvidenceSchema,
        WrongSchemaRole,
        UnknownVerifier,
        AuthoritySnapshotMismatch,
        EmptyAuthority,
        UnsupportedSigningScheme,
        QuorumExceeded,
    }

    for (case, byte, expected) in [
        (
            Case::UnknownEvidenceSchema,
            0xc2,
            "unknown manual evidence schema",
        ),
        (Case::WrongSchemaRole, 0xc3, "wrong certified role"),
        (
            Case::UnknownVerifier,
            0xc4,
            "unknown manual authorization verifier",
        ),
        (
            Case::AuthoritySnapshotMismatch,
            0xc5,
            "does not match registry",
        ),
        (Case::EmptyAuthority, 0xc6, "has no operators"),
        (
            Case::UnsupportedSigningScheme,
            0xc7,
            "unsupported manual authorization signing scheme",
        ),
        (Case::QuorumExceeded, 0xc8, "quorum 2 exceeds"),
    ] {
        let mut manual = manual_resolution_spec(byte);
        match case {
            Case::EmptyAuthority => manual.authorization.authority.operators.clear(),
            Case::UnsupportedSigningScheme => {
                manual.authorization.signing_scheme =
                    spec::ManualSigningSchemeSpec::new("mfm.certify.test.unsupported-signing.v1")
                        .expect("signing scheme");
            }
            Case::QuorumExceeded => {
                manual.authorization.quorum =
                    spec::ManualAuthorizationQuorumSpec::new(2).expect("quorum");
            }
            Case::UnknownEvidenceSchema
            | Case::WrongSchemaRole
            | Case::UnknownVerifier
            | Case::AuthoritySnapshotMismatch => {}
        }
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        match case {
            Case::UnknownEvidenceSchema => {
                register_manual_verifier(&mut registry, &manual);
                register_manual_authority_snapshot(
                    &mut registry,
                    manual.authorization.authority.clone(),
                );
            }
            Case::WrongSchemaRole => {
                register_manual_schema_role(
                    &mut registry,
                    &manual,
                    CertifiedSchemaRole::ManualResolutionAuthorization,
                );
                register_manual_verifier(&mut registry, &manual);
                register_manual_authority_snapshot(
                    &mut registry,
                    manual.authorization.authority.clone(),
                );
            }
            Case::UnknownVerifier => {
                register_manual_evidence_schema(&mut registry, &manual);
                register_manual_authority_snapshot(
                    &mut registry,
                    manual.authorization.authority.clone(),
                );
            }
            Case::AuthoritySnapshotMismatch => {
                register_manual_evidence_schema(&mut registry, &manual);
                register_manual_verifier(&mut registry, &manual);
                let mut mismatched = manual.authorization.authority.clone();
                mismatched.operators[0].public_identity =
                    spec::OperatorPublicIdentity::new("operator-certify-public-mismatch")
                        .expect("operator public identity");
                register_manual_authority_snapshot(&mut registry, mismatched);
            }
            Case::EmptyAuthority | Case::UnsupportedSigningScheme | Case::QuorumExceeded => {
                register_manual_authority(&mut registry, &manual);
            }
        }

        assert_manual_certification_rejects(typed, &registry, expected);
    }
}

#[test]
fn certification_rejects_missing_or_duplicate_resolve_saga_terminal_node() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes.retain(|node| {
                !matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
                )
            });
        },
        ProblemClass::InvalidTopology => |spec| {
            let duplicate = spec
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
                    )
                })
                .expect("resolve node")
                .clone();
            spec.nodes.push(duplicate);
        },
    );
}
