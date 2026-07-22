use super::*;

#[test]
fn certification_registry_rejects_duplicate_state_kind_version() {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<MultiplyState>()
        .expect("first state descriptor registers");

    let error = registry
        .register_state::<ConflictingMultiplyState>()
        .expect_err("conflicting state kind/version rejects");

    assert_invalid_semantic_contains(error, "state kind/version");
}

#[test]
fn certification_registry_rejects_duplicate_operation_kind_version() {
    let mut registry = CertificationRegistry::new();
    registry
        .register_operation::<MultiplyOperation>()
        .expect("first operation descriptor registers");

    let error = registry
        .register_operation::<ConflictingMultiplyOperation>()
        .expect_err("conflicting operation kind/version rejects");

    assert_invalid_semantic_contains(error, "operation kind/version");
}

#[test]
fn registered_config_validator_rejects_invalid_config_bytes() {
    enum Case {
        SchemaShapeMismatch,
        NoncanonicalBytes,
        NonAuthoritativeCanonicalEncoding,
    }

    for case in [
        Case::SchemaShapeMismatch,
        Case::NoncanonicalBytes,
        Case::NonAuthoritativeCanonicalEncoding,
    ] {
        match case {
            Case::SchemaShapeMismatch => {
                let registry = registered_multiply_certification_registry();
                let invalid = PlainCanonicalJsonBytes::from_json_str(r#"{"multiplier":"bad"}"#)
                    .expect("canonical invalid config");
                let config_ref = config_ref_for_bytes::<TestConfig>(&invalid);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, invalid.as_bytes())
                    .expect_err("invalid typed config shape rejects");
                assert!(err
                    .to_string()
                    .contains("typed config did not match registered schema"));
            }
            Case::NoncanonicalBytes => {
                let registry = registered_multiply_certification_registry();
                let canonical = PlainCanonicalJsonBytes::from_json_str(r#"{"multiplier":2}"#)
                    .expect("canonical config");
                let config_ref = config_ref_for_bytes::<TestConfig>(&canonical);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, br#"{ "multiplier": 2 }"#)
                    .expect_err("noncanonical config rejects");
                let rendered = err.to_string();
                assert!(rendered.contains("typed config"));
                assert!(rendered.contains("not normalized canonical JSON"));
            }
            Case::NonAuthoritativeCanonicalEncoding => {
                let mut registry = CertificationRegistry::new();
                registry
                    .insert_config_validator(
                        config_validator_for::<DefaultedConfig>().expect("validator"),
                    )
                    .expect("insert validator");
                let supplied = PlainCanonicalJsonBytes::from_json_str(r#"{}"#)
                    .expect("canonical but not authoritative config");
                let config_ref = config_ref_for_bytes::<DefaultedConfig>(&supplied);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, supplied.as_bytes())
                    .expect_err("non-authoritative canonical config rejects");
                assert!(err
                    .to_string()
                    .contains("did not match registered canonical encoding"));
            }
        }
    }
}

#[test]
fn trusted_draft_config_ref_accepts_exact_bytes_without_descriptor_validator() {
    let draft = reference_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("draft registry");
    let config = draft
        .state_nodes()
        .first()
        .expect("state node")
        .config
        .clone();
    let config_ref = spec::ConfigRef {
        schema_id: config.schema_id.clone(),
        artifact_id: ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *config.content_digest.digest(),
        ),
        digest: config.content_digest.clone(),
        byte_len: config.byte_len as u64,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    };

    let source = registry
        .validate_config_ref_bytes(&config_ref, config.canonical_json.as_bytes())
        .expect("trusted exact config validates");
    assert_eq!(source, Some(ConfigValidationSource::TrustedExactRef));
}

#[test]
fn certifies_reference_program_draft() {
    let draft = reference_draft();
    let expected_public_schema = draft.public_output_spec().public_schema_id().clone();
    let certified = certify_program_draft(&draft).expect("certified");
    certified.envelope().verify_hash().expect("hash verifies");
    certified
        .certificate()
        .verify_hash()
        .expect("certificate hash verifies");
    assert_eq!(
        certified.certificate().evidence.certifier_algorithm,
        CERTIFIER_ALGORITHM
    );
    assert_eq!(
        certified.certificate().evidence.spec_hash,
        *certified.spec_hash()
    );
    assert_eq!(
        certified.certificate_hash().as_str(),
        "content:sha256-jcs-v1:66ef9b89906af1b355e2ac0dfb1fae2544e8272534cb1cb0352ac4c6398a3289"
    );
    assert_eq!(
        certified.envelope().spec.public_outputs.public_schema_id,
        expected_public_schema
    );
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    )));
}
