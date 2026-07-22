use super::*;

#[test]
fn authoring_catalog_derives_state_capability_fact_and_operation_contracts() {
    let mut catalog = ProgramAuthoringCatalog::__new();
    catalog
        .__register_state::<FactEmittingState>()
        .expect("fact-emitting state catalog");
    catalog
        .__register_operation::<MultiplyOperation>()
        .expect("operation catalog");

    assert_eq!(catalog.state_descriptors().len(), 1);
    assert_eq!(catalog.operation_descriptors().len(), 1);
    assert_eq!(catalog.capability_descriptors().len(), 1);
    assert_eq!(catalog.emitted_fact_descriptors().len(), 1);
    assert_eq!(catalog.adapter_bindings().len(), 0);
    assert_eq!(catalog.side_effect_state_descriptor_ids().len(), 0);
    assert_eq!(
        catalog
            .capability_descriptors()
            .next()
            .expect("capability")
            .kind,
        FactReadCap::kind().expect("fact-read capability kind")
    );
    assert_eq!(
        catalog
            .emitted_fact_descriptors()
            .next()
            .expect("fact descriptor"),
        &mfm_program::fact_descriptor_ref::<ChainHeadFact>().expect("chain-head descriptor")
    );
}

#[test]
fn authoring_catalog_union_rejects_conflicting_state_semantic_keys() {
    let mut first = ProgramAuthoringCatalog::__new();
    first
        .__register_state::<MultiplyState>()
        .expect("first state catalog");
    let mut conflicting = ProgramAuthoringCatalog::__new();
    conflicting
        .__register_state::<ConflictingMultiplyState>()
        .expect("conflicting state is independently valid");

    let error = first
        .union(&conflicting)
        .expect_err("conflicting state kind/version must reject");
    assert_invalid_semantic_contains(error, "authoring catalog");
}

#[test]
fn certification_registry_requires_exact_catalog_descriptors_and_fact_artifacts() {
    let mut domain = ProgramAuthoringCatalog::__new();
    domain
        .__register_state::<FactEmittingState>()
        .expect("fact state catalog");
    domain
        .__register_operation::<MultiplyOperation>()
        .expect("operation catalog");
    let public_schema = <TestPublicOutputs<'static, 'static> as mfm_program::PublicOutputs<
        'static,
        'static,
    >>::public_schema_id()
    .expect("public schema");
    let expected = domain
        .union(&__framework_authoring_catalog(&public_schema).expect("sealed framework catalog"))
        .expect("combined catalog");

    let mut exact = CertificationRegistry::new();
    exact
        .register_state::<FactEmittingState>()
        .expect("fact state registry");
    exact
        .register_operation::<MultiplyOperation>()
        .expect("operation registry");
    exact
        .validate_authoring_catalog(&expected)
        .expect("exact catalog coverage");

    let missing = CertificationRegistry::new()
        .validate_authoring_catalog(&expected)
        .expect_err("missing descriptors must reject");
    assert_invalid_semantic_contains(missing, "authoring catalog");

    exact
        .register_state::<MultiplyState>()
        .expect("extra state registry");
    let extra = exact
        .validate_authoring_catalog(&expected)
        .expect_err("extra descriptor must reject");
    assert_invalid_semantic_contains(extra, "authoring catalog");
}

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
        "content:sha256-jcs-v1:6334096a68af2e287a95d217632d879358bb7d9f328997f4faf509046bdeabe7"
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
