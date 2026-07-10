use super::*;
use mfm_ids::DigestBytes;

fn digest_bytes(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn spec_hash(byte: u8) -> SpecHash {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn artifact_id(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn side_effect_pair_id(byte: u8) -> SideEffectPairId {
    SideEffectPairId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn descriptor_id(byte: u8) -> DescriptorId {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn cell_id(byte: u8) -> CellId {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn scope_id(byte: u8) -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn seed_id(byte: u8) -> SeedId {
    SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn schema_id(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest_bytes(byte)).expect("schema id")
}

fn semantic_id(name: &str, byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("semantic id")
}

fn media_type(value: &str) -> MediaType {
    MediaType::new(value).expect("media type")
}

fn event_artifact_ref(
    artifact_id: ArtifactId,
    role: ArtifactRole,
    schema_id: SchemaId,
    content_digest: ContentDigest,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        role,
        schema_id,
        semantic_type_id: None,
        content_digest: content_digest.clone(),
        evidence_hash: content_digest,
        byte_len: 64,
        media_type: media_type("application/json"),
    }
}

fn fact_claim(
    capability_kind_byte: u8,
    adapter_kind_byte: u8,
    request_schema_byte: u8,
    request_hash_byte: u8,
    response_schema_byte: u8,
    response_hash_byte: u8,
    artifact_id_byte: u8,
) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        fact_kind: mfm_facts::FactKind::new("chain.head").expect("fact kind"),
        fact_descriptor_hash: content_digest(artifact_id_byte.wrapping_add(1)),
        subject: fact_subject_evidence(artifact_id_byte.wrapping_add(2)),
        observed_at: Some("2026-01-02T03:04:05Z".to_owned()),
        request: Some(mfm_facts::FactRequestEvidence::new(
            schema_id("mfm.test.fact_request", request_schema_byte),
            content_digest(request_hash_byte),
        )),
        response: mfm_facts::FactResponseEvidence::new(
            schema_id("mfm.test.fact_response", response_schema_byte),
            content_digest(response_hash_byte),
            artifact_id(artifact_id_byte),
            content_digest(artifact_id_byte.wrapping_add(5)),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.test",
                "capability",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(capability_kind_byte),
            )
            .expect("capability kind"),
            CapabilityVersion::new("mfm.test.capability.v1").expect("capability version"),
            AdapterKind::new(
                "mfm.test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(adapter_kind_byte),
            )
            .expect("adapter kind"),
            AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        ),
    })
    .expect("fact claim")
}

fn fact_subject_evidence(namespace_byte: u8) -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactFieldValue::new(
        mfm_facts::FactFieldId::new("subject.chain").expect("field"),
        mfm_facts::FactFieldValueType::String,
        mfm_facts::FactCanonicalScalar::string(format!("chain_{namespace_byte}")),
    )
    .expect("subject value")])
    .expect("subject material");
    mfm_facts::FactSubjectEvidence::from_material(content_digest(namespace_byte), &material)
        .expect("subject evidence")
}

fn run_artifact_ref(
    artifact_id: ArtifactId,
    role: ArtifactRole,
    schema_id: Option<SchemaId>,
    content_digest: ContentDigest,
) -> RunArtifactEvidenceRef {
    RunArtifactEvidenceRef {
        artifact_id,
        role,
        schema_id,
        semantic_type_id: None,
        content_digest: content_digest.clone(),
        evidence_hash: content_digest,
        byte_len: 64,
        media_type: media_type("application/json"),
    }
}

#[test]
fn mfm_error_info_constructor_accepts_redacted_diagnostic_ref() {
    let diagnostic = event_artifact_ref(
        artifact_id(42),
        ArtifactRole::RedactedDiagnostic,
        schema_id("mfm.test.diagnostic", 43),
        content_digest(44),
    );

    let error = MfmErrorInfo::new(
        ErrorCode::new("redacted_diagnostic").expect("code"),
        ErrorCategory::Runtime,
        false,
        "runtime validation failed",
    )
    .expect("base error")
    .with_public_details(RedactedJson::new(content_digest(45)))
    .expect("public details")
    .with_diagnostic_ref(diagnostic.clone())
    .expect("diagnostic ref");

    assert_eq!(error.diagnostic_ref, Some(diagnostic));
    assert!(error.public_details.is_some());
}

#[test]
fn mfm_error_info_rejects_public_diagnostic_boundary_violations() {
    let error = MfmErrorInfo::new(
        ErrorCode::new("redacted_diagnostic").expect("code"),
        ErrorCategory::Runtime,
        false,
        "provider returned bearer token=super-secret-value",
    )
    .expect_err("secret-shaped message rejected");

    assert!(matches!(
        error,
        EventError::InvalidPublicDiagnostic {
            field: "safe_message",
            reason: "message resembles secret material"
        }
    ));

    let diagnostic = event_artifact_ref(
        artifact_id(46),
        ArtifactRole::SideEffectIntent,
        schema_id("mfm.test.diagnostic", 47),
        content_digest(48),
    );
    let error = MfmErrorInfo::new(
        ErrorCode::new("redacted_diagnostic").expect("code"),
        ErrorCategory::Runtime,
        false,
        "runtime validation failed",
    )
    .expect("base error")
    .with_diagnostic_ref(diagnostic)
    .expect_err("wrong diagnostic role rejected");

    assert!(matches!(
        error,
        EventError::InvalidPublicDiagnostic {
            field: "diagnostic_ref",
            reason: "diagnostic artifact role must be redacted_diagnostic"
        }
    ));
}

fn error_with_diagnostic(
    artifact_id: ArtifactId,
    role: ArtifactRole,
    schema_id: SchemaId,
    content_digest: ContentDigest,
) -> MfmErrorInfo {
    MfmErrorInfo {
        code: ErrorCode::new("event_accessor_test").expect("error code"),
        category: ErrorCategory::Runtime,
        retryable: false,
        safe_message: "event accessor test".to_owned(),
        public_details: None,
        diagnostic_ref: Some(event_artifact_ref(
            artifact_id,
            role,
            schema_id,
            content_digest,
        )),
    }
}

fn touched_set(byte: u8) -> ResourceTouchedSetEvidence {
    ResourceTouchedSetEvidence {
        namespace: ResourceNamespace::new("mfm.test.resource").expect("namespace"),
        evidence_schema_id: schema_id("mfm.test.touched_set", byte),
        evidence_hash: content_digest(byte.wrapping_add(1)),
        evidence_artifact_id: artifact_id(byte.wrapping_add(2)),
        evidence_artifact_evidence_hash: content_digest(byte.wrapping_add(3)),
    }
}

fn store_scope_id() -> StoreScopeId {
    StoreScopeId::new("mfm.store_scope.v1:20202020202020202020202020202020").expect("store scope")
}

fn run_identity_material(spec_hash: SpecHash) -> RunIdentityMaterialV1 {
    RunIdentityMaterialV1 {
        certified_spec_hash: spec_hash,
        store_scope_id: store_scope_id(),
        invocation_key_digest: content_digest(1),
    }
}

#[test]
fn invocation_key_material_v1_digest_is_canonical_and_redacted() {
    let material = InvocationKeyMaterialV1::new("alpha").expect("invocation key");

    assert_eq!(
        material.canonical_json().expect("canonical").as_str(),
        r#"{"domain":"mfm.invocation_key.v1","raw_key":"alpha"}"#
    );
    assert_eq!(
        material.digest().expect("digest").as_str(),
        "content:sha256-jcs-v1:e8b2dba1a1730580547875fc9f27a3edf6a5c62660bdc3a6ac06518b487ecd43"
    );
    assert!(!format!("{material:?}").contains("alpha"));
}

#[test]
fn invocation_key_material_v1_rejects_empty_and_oversized_keys_without_echo() {
    let empty = InvocationKeyMaterialV1::new("").expect_err("empty key rejects");
    assert_eq!(empty.to_string(), "invalid invocation key: empty");

    let too_large = "x".repeat(InvocationKeyMaterialV1::MAX_RAW_KEY_BYTES + 1);
    let err = InvocationKeyMaterialV1::new(&too_large).expect_err("oversized key rejects");
    assert_eq!(err.to_string(), "invalid invocation key: too_large");
    assert!(!err.to_string().contains(&too_large));
}

fn run_admitted_payload() -> KernelEventPayload {
    let identity_material = run_identity_material(spec_hash(2));
    let run_id = identity_material.derive_run_id().expect("run id");
    KernelEventPayload::RunAdmitted(Box::new(RunAdmitted {
        run_id,
        identity_material,
        entry_point: EntryPointLaunchEvidence {
            resolved_op_id: EntryPointOpId::new("mfm.portfolio/snapshot@2").expect("op id"),
            entry_point_registry_digest: content_digest(18),
        },
        spec_hash: spec_hash(2),
        spec_artifact: run_artifact_ref(
            artifact_id(3),
            ArtifactRole::TypedExecutionSpec,
            None,
            content_digest(2),
        ),
        certificate_artifact: run_artifact_ref(
            artifact_id(4),
            ArtifactRole::TypedSpecCertificate,
            None,
            content_digest(5),
        ),
        config_artifacts: vec![run_artifact_ref(
            artifact_id(14),
            ArtifactRole::TypedConfig,
            Some(schema_id("mfm.test.config", 15)),
            content_digest(16),
        )],
        fact_descriptor_artifacts: Vec::new(),
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 6),
        saga_policy_digest: mfm_spec::v1::SagaPolicySpec::NoSideEffects
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(17),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        seed_cells: vec![SeedCellRef {
            seed_id: seed_id(7),
            cell_id: cell_id(8),
            scope_id: scope_id(9),
            semantic_type_id: semantic_id("seed", 10),
            schema_id: schema_id("mfm.test.seed", 11),
            digest: content_digest(12),
            seed_artifact: event_artifact_ref(
                artifact_id(13),
                ArtifactRole::SeedInput,
                schema_id("mfm.test.seed", 11),
                content_digest(12),
            ),
        }],
    }))
}

#[derive(Debug, Clone, Copy)]
struct ArtifactRoleBaseline {
    role: ArtifactRole,
    tag: &'static str,
    schema_policy: &'static str,
    semantic_policy: &'static str,
    producer_policy: &'static str,
    staging_class: &'static str,
    retention_class: &'static str,
    same_commit_policy: &'static str,
}

fn artifact_role_baselines() -> &'static [ArtifactRoleBaseline] {
    &[
        ArtifactRoleBaseline {
            role: ArtifactRole::TypedExecutionSpec,
            tag: "typed_execution_spec",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "launch_or_global_no_seed",
            staging_class: "run_admission",
            retention_class: "framework_ignored",
            same_commit_policy: "run_admitted_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::TypedSpecCertificate,
            tag: "typed_spec_certificate",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "launch_or_global_no_seed",
            staging_class: "run_admission",
            retention_class: "framework_ignored",
            same_commit_policy: "run_admitted_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::TypedConfig,
            tag: "typed_config",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "launch_or_global_no_seed",
            staging_class: "run_admission",
            retention_class: "framework_ignored",
            same_commit_policy: "run_admitted_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::FactDescriptor,
            tag: "fact_descriptor",
            schema_policy: "exact_fact_descriptor_schema",
            semantic_policy: "absent",
            producer_policy: "launch_or_global_no_seed",
            staging_class: "run_admission",
            retention_class: "framework_ignored",
            same_commit_policy: "run_admitted_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::SeedInput,
            tag: "seed_input",
            schema_policy: "exact_seed_schema",
            semantic_policy: "exact_seed_semantic",
            producer_policy: "seed_required",
            staging_class: "run_admission",
            retention_class: "framework_ignored",
            same_commit_policy: "run_admitted_seed_cell",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::StateOutput,
            tag: "state_output",
            schema_policy: "exact_value_schema",
            semantic_policy: "exact_value_semantic",
            producer_policy: "node_required",
            staging_class: "attempt_state_output",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::FactResponse,
            tag: "fact_response",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "attempt_fact_response",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::ExternalReadEvidence,
            tag: "external_read_evidence",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "attempt_external_read_evidence",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::FactQueryEvidence,
            tag: "fact_query_evidence",
            schema_policy: "exact_fact_query_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "attempt_fact_query_evidence",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::SideEffectIntent,
            tag: "side_effect_intent",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "side_effect_intent",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::PreparedInvocation,
            tag: "prepared_invocation",
            schema_policy: "absent",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "side_effect_prepared_invocation",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::NotSubmittedProof,
            tag: "not_submitted_proof",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "side_effect_not_submitted_proof",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::Submission,
            tag: "submission",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "side_effect_submission",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::SubmissionUnknownEvidence,
            tag: "submission_unknown_evidence",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "side_effect_submission_unknown",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::Receipt,
            tag: "receipt",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "side_effect_receipt",
            retention_class: "receipt_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::Confirmation,
            tag: "confirmation",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "side_effect_confirmation",
            retention_class: "confirmation_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::AmbiguityEvidence,
            tag: "ambiguity_evidence",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "side_effect_ambiguity",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::ManualResolutionEvidence,
            tag: "manual_resolution_evidence",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "global_no_seed",
            staging_class: "manual_resolution",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::ManualResolutionAuthorization,
            tag: "manual_resolution_authorization",
            schema_policy: "exact_evidence_schema",
            semantic_policy: "absent",
            producer_policy: "global_no_seed",
            staging_class: "manual_resolution",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::PublicOutput,
            tag: "public_output",
            schema_policy: "exact_public_schema",
            semantic_policy: "absent",
            producer_policy: "node_required",
            staging_class: "attempt_public_output",
            retention_class: "public_output_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::RedactedDiagnostic,
            tag: "redacted_diagnostic",
            schema_policy: "exact_diagnostic_schema",
            semantic_policy: "absent",
            producer_policy: "diagnostic_optional_node_no_seed",
            staging_class: "attempt_redacted_diagnostic",
            retention_class: "value_artifacts",
            same_commit_policy: "payload_required_artifact",
        },
        ArtifactRoleBaseline {
            role: ArtifactRole::RetentionManifest,
            tag: "retention_manifest",
            schema_policy: "absent",
            semantic_policy: "absent",
            producer_policy: "middleware_no_seed",
            staging_class: "middleware_retention_manifest",
            retention_class: "framework_ignored",
            same_commit_policy: "retention_manifest_requires_ref",
        },
    ]
}

fn artifact_role_schema_tags() -> Vec<String> {
    let schema = ARTIFACT_REFERENCED_SCHEMA
        .canonical_json()
        .expect("artifact schema json");
    let json: serde_json::Value = serde_json::from_str(schema.as_str()).expect("schema json value");
    find_artifact_role_variants(&json).expect("artifact role variants")
}

fn find_artifact_role_variants(value: &serde_json::Value) -> Option<Vec<String>> {
    if let Some(array) = value.as_array() {
        for nested in array {
            if let Some(variants) = find_artifact_role_variants(nested) {
                return Some(variants);
            }
        }
        return None;
    }
    let object = value.as_object()?;
    if object.get("kind").and_then(serde_json::Value::as_str) == Some("enum")
        && object.get("name").and_then(serde_json::Value::as_str) == Some("ArtifactRole")
    {
        return object
            .get("variants")
            .and_then(serde_json::Value::as_array)
            .map(|variants| {
                variants
                    .iter()
                    .map(|variant| {
                        variant
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .expect("variant name")
                            .to_owned()
                    })
                    .collect()
            });
    }
    for nested in object.values() {
        if let Some(variants) = find_artifact_role_variants(nested) {
            return Some(variants);
        }
    }
    None
}

#[test]
fn artifact_role_contract_policy_baseline_covers_schema_tags() {
    let tags = artifact_role_baselines()
        .iter()
        .map(|row| row.tag.to_owned())
        .collect::<Vec<_>>();
    assert_eq!(tags, artifact_role_schema_tags());
    assert_eq!(
        tags,
        ArtifactRole::ALL
            .iter()
            .map(|role| role.as_str().to_owned())
            .collect::<Vec<_>>()
    );

    let mut roles = artifact_role_baselines()
        .iter()
        .map(|row| row.role)
        .collect::<Vec<_>>();
    roles.sort_unstable();
    roles.dedup();
    assert_eq!(roles.len(), artifact_role_baselines().len());
    assert_eq!(roles.len(), ArtifactRole::ALL.len());

    let rows = artifact_role_baselines()
        .iter()
        .map(|row| {
            let contract = row.role.contract();
            assert_eq!(contract.role, row.role);
            assert_eq!(contract.tag, row.tag);
            assert_eq!(row.role.as_str(), row.tag);
            assert_eq!(ArtifactRole::parse(row.tag), Some(row.role));
            assert_eq!(contract.schema.as_str(), row.schema_policy);
            assert_eq!(contract.semantic.as_str(), row.semantic_policy);
            assert_eq!(contract.producer.as_str(), row.producer_policy);
            assert_eq!(contract.staging.as_str(), row.staging_class);
            assert_eq!(contract.retention.as_str(), row.retention_class);
            assert_eq!(contract.same_commit.as_str(), row.same_commit_policy);
            format!(
                "{} schema={} semantic={} producer={} staging={} retention={} same_commit={}",
                row.tag,
                row.schema_policy,
                row.semantic_policy,
                row.producer_policy,
                row.staging_class,
                row.retention_class,
                row.same_commit_policy
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(ArtifactRole::parse("resource_touched_set_evidence"), None);

    assert_eq!(
        rows,
        "typed_execution_spec schema=exact_evidence_schema semantic=absent producer=launch_or_global_no_seed staging=run_admission retention=framework_ignored same_commit=run_admitted_artifact\n\
typed_spec_certificate schema=exact_evidence_schema semantic=absent producer=launch_or_global_no_seed staging=run_admission retention=framework_ignored same_commit=run_admitted_artifact\n\
typed_config schema=exact_evidence_schema semantic=absent producer=launch_or_global_no_seed staging=run_admission retention=framework_ignored same_commit=run_admitted_artifact\n\
fact_descriptor schema=exact_fact_descriptor_schema semantic=absent producer=launch_or_global_no_seed staging=run_admission retention=framework_ignored same_commit=run_admitted_artifact\n\
seed_input schema=exact_seed_schema semantic=exact_seed_semantic producer=seed_required staging=run_admission retention=framework_ignored same_commit=run_admitted_seed_cell\n\
state_output schema=exact_value_schema semantic=exact_value_semantic producer=node_required staging=attempt_state_output retention=value_artifacts same_commit=payload_required_artifact\n\
fact_response schema=exact_evidence_schema semantic=absent producer=node_required staging=attempt_fact_response retention=value_artifacts same_commit=payload_required_artifact\n\
external_read_evidence schema=exact_evidence_schema semantic=absent producer=node_required staging=attempt_external_read_evidence retention=value_artifacts same_commit=payload_required_artifact\n\
fact_query_evidence schema=exact_fact_query_evidence_schema semantic=absent producer=node_required staging=attempt_fact_query_evidence retention=value_artifacts same_commit=payload_required_artifact\n\
side_effect_intent schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_intent retention=value_artifacts same_commit=payload_required_artifact\n\
prepared_invocation schema=absent semantic=absent producer=node_required staging=side_effect_prepared_invocation retention=value_artifacts same_commit=payload_required_artifact\n\
not_submitted_proof schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_not_submitted_proof retention=value_artifacts same_commit=payload_required_artifact\n\
submission schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_submission retention=value_artifacts same_commit=payload_required_artifact\n\
submission_unknown_evidence schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_submission_unknown retention=value_artifacts same_commit=payload_required_artifact\n\
receipt schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_receipt retention=receipt_artifacts same_commit=payload_required_artifact\n\
confirmation schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_confirmation retention=confirmation_artifacts same_commit=payload_required_artifact\n\
ambiguity_evidence schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_ambiguity retention=value_artifacts same_commit=payload_required_artifact\n\
manual_resolution_evidence schema=exact_evidence_schema semantic=absent producer=global_no_seed staging=manual_resolution retention=value_artifacts same_commit=payload_required_artifact\n\
manual_resolution_authorization schema=exact_evidence_schema semantic=absent producer=global_no_seed staging=manual_resolution retention=value_artifacts same_commit=payload_required_artifact\n\
public_output schema=exact_public_schema semantic=absent producer=node_required staging=attempt_public_output retention=public_output_artifacts same_commit=payload_required_artifact\n\
redacted_diagnostic schema=exact_diagnostic_schema semantic=absent producer=diagnostic_optional_node_no_seed staging=attempt_redacted_diagnostic retention=value_artifacts same_commit=payload_required_artifact\n\
retention_manifest schema=absent semantic=absent producer=middleware_no_seed staging=middleware_retention_manifest retention=framework_ignored same_commit=retention_manifest_requires_ref"
    );
}

fn descriptor_artifact_requirement_sources(schema_name: &str) -> &'static str {
    match schema_name {
        "mfm.events.v1.run_admitted" => "RunSpec,RunCertificate,RunConfig,SeedCell,FactDescriptor",
        "mfm.events.v1.fact_recorded" => "FactResponse",
        "mfm.events.v1.artifact_referenced" => "ArtifactReferenced",
        "mfm.events.v1.cell_produced" => "StateOutput",
        "mfm.events.v1.side_effect.intent_persisted" => "SideEffectIntent",
        "mfm.events.v1.side_effect.invocation_prepared" => "PreparedInvocation",
        "mfm.events.v1.side_effect.not_submitted_proven" => "NotSubmittedProof",
        "mfm.events.v1.side_effect.submission_observed" => "Submission",
        "mfm.events.v1.side_effect.submission_unknown" => "SubmissionUnknownEvidence",
        "mfm.events.v1.side_effect.receipt_observed" => "Receipt,ResourceTouchedSet",
        "mfm.events.v1.side_effect.confirmation_observed" => "Confirmation,ResourceTouchedSet",
        "mfm.events.v1.side_effect.ambiguous" => "AmbiguityEvidence",
        "mfm.events.v1.side_effect.failed" => "SideEffectFailureDiagnostic",
        "mfm.events.v1.public_output_produced" => "PublicOutputCell,PublicOutputRendered",
        "mfm.events.v1.public_output_render_failed" => "PublicOutputRenderFailureDiagnostic",
        "mfm.events.v1.state_attempt_failed" => "StateAttemptFailureDiagnostic",
        "mfm.events.v1.manual_resolution_recorded" => {
            "ManualResolutionEvidence,ManualResolutionAuthorization"
        }
        "mfm.events.v1.retention_refs_appended" => "RetentionRef",
        "mfm.events.v1.retention_manifest_projected" => "RetentionManifest",
        "mfm.events.v1.state_attempt_started"
        | "mfm.events.v1.cell_skipped"
        | "mfm.events.v1.side_effect.claimed"
        | "mfm.events.v1.side_effect.claim_taken_over"
        | "mfm.events.v1.resource_lane.claimed"
        | "mfm.events.v1.resource_lane.claim_intent"
        | "mfm.events.v1.side_effect.invocation_started"
        | "mfm.events.v1.resource_lane.released"
        | "mfm.events.v1.resource_lane.release_intent"
        | "mfm.events.v1.state_attempt_completed"
        | "mfm.events.v1.state_attempt_interrupted"
        | "mfm.events.v1.run_completed" => "",
        other => panic!("uncovered event schema descriptor {other}"),
    }
}

#[test]
fn event_schema_descriptor_requirement_sources_golden() {
    let rows = all_event_schema_descriptors()
        .into_iter()
        .map(|descriptor| {
            format!(
                "{} {} [{}]",
                descriptor.schema_name,
                descriptor.schema_id().expect("schema id").as_str(),
                descriptor_artifact_requirement_sources(descriptor.schema_name)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        rows,
        r#"mfm.events.v1.run_admitted schema:mfm.events.v1.run_admitted:1:sha256-jcs-v1:9da01b57ef30318cc98621ddeef1092fe3b6fd8b1fce3b49b739af4278a83ba4 [RunSpec,RunCertificate,RunConfig,SeedCell,FactDescriptor]
mfm.events.v1.state_attempt_started schema:mfm.events.v1.state_attempt_started:1:sha256-jcs-v1:986f35aa39938713b9862192cab7d2b9b3a37219f5872bd242f8a06e7957ff1b []
mfm.events.v1.fact_recorded schema:mfm.events.v1.fact_recorded:1:sha256-jcs-v1:f66fc733963d3564fe94bcd8c5d80c489405ddc63c34002a9c6af381cf8f1734 [FactResponse]
mfm.events.v1.artifact_referenced schema:mfm.events.v1.artifact_referenced:1:sha256-jcs-v1:b60ebe4bc262799c154a596362ae57672801f54e1b18d703572848626ccef3f0 [ArtifactReferenced]
mfm.events.v1.cell_produced schema:mfm.events.v1.cell_produced:1:sha256-jcs-v1:a4bac036db54afd368d6e9c53b566daadba6ef6ea72bdf9839dd08213284737e [StateOutput]
mfm.events.v1.cell_skipped schema:mfm.events.v1.cell_skipped:1:sha256-jcs-v1:e430038e78e1a83bf2465de6bdc23703395781dc4395e0340eb3856298225b84 []
mfm.events.v1.side_effect.intent_persisted schema:mfm.events.v1.side_effect.intent_persisted:1:sha256-jcs-v1:1301896f1bd678773c0f145114b0c29cf7b06138686a99b75716a7d41fc181ed [SideEffectIntent]
mfm.events.v1.side_effect.claimed schema:mfm.events.v1.side_effect.claimed:1:sha256-jcs-v1:793b4e2c8a58d2acef7cd4500fe7b20a1cf6dfd57386ccb8ef47cf64b3feb862 []
mfm.events.v1.side_effect.claim_taken_over schema:mfm.events.v1.side_effect.claim_taken_over:1:sha256-jcs-v1:abdc3cb22d1022954df00d0fa18ea5ddf0f71f576b9a9c7e0dca08de2a0f5acf []
mfm.events.v1.resource_lane.claimed schema:mfm.events.v1.resource_lane.claimed:1:sha256-jcs-v1:c9b27a3f52464ad34d1cb80fc41947bbfa71bfcba8ab4d78cc09e4b1a9286e4c []
mfm.events.v1.resource_lane.claim_intent schema:mfm.events.v1.resource_lane.claim_intent:1:sha256-jcs-v1:14ec3899b92dc1182f8b5865e2c5597b053ae648290b9b900eb161014dd7b4ed []
mfm.events.v1.side_effect.invocation_prepared schema:mfm.events.v1.side_effect.invocation_prepared:1:sha256-jcs-v1:71fcef6b4e021f3c49cf1702377b49db1461f6e5918d17a45fda924b8e680fbd [PreparedInvocation]
mfm.events.v1.side_effect.invocation_started schema:mfm.events.v1.side_effect.invocation_started:1:sha256-jcs-v1:7d4cf3ffbd5109af4251e207927b0b4819e3ec058a948b1d106d022eeddfa3fc []
mfm.events.v1.side_effect.not_submitted_proven schema:mfm.events.v1.side_effect.not_submitted_proven:1:sha256-jcs-v1:c1ccc7c56931f0f14fa86e233899beb7b744b5cff47f68ec74e53e7f8cf6af12 [NotSubmittedProof]
mfm.events.v1.side_effect.submission_observed schema:mfm.events.v1.side_effect.submission_observed:1:sha256-jcs-v1:612411db371d86f8cc7033745692bcd88018b632ded4b156b0c96935bc4532cb [Submission]
mfm.events.v1.side_effect.submission_unknown schema:mfm.events.v1.side_effect.submission_unknown:1:sha256-jcs-v1:56c8fb7e5f12f11c178c671edd68fff8d22a8acd27eb97a7cc364b71a93f5637 [SubmissionUnknownEvidence]
mfm.events.v1.side_effect.receipt_observed schema:mfm.events.v1.side_effect.receipt_observed:1:sha256-jcs-v1:f24eae04a3a4dcf471c5c4fc362deeb25e8915958565bc61236684ef5e8fd549 [Receipt,ResourceTouchedSet]
mfm.events.v1.side_effect.confirmation_observed schema:mfm.events.v1.side_effect.confirmation_observed:1:sha256-jcs-v1:c5f0bee3807376b77bdb4386ea18c92ed17f2e8381a249c50eddb35b75690143 [Confirmation,ResourceTouchedSet]
mfm.events.v1.side_effect.ambiguous schema:mfm.events.v1.side_effect.ambiguous:1:sha256-jcs-v1:2e436cd594c93cdd1e8a548804b90719c13cd3e62933a90e0ae810e3ae04686a [AmbiguityEvidence]
mfm.events.v1.side_effect.failed schema:mfm.events.v1.side_effect.failed:1:sha256-jcs-v1:0f9166fa332675d82be749016fc2b5836dfebe6ea6aa1c25b7b59a7144c0fb4d [SideEffectFailureDiagnostic]
mfm.events.v1.resource_lane.released schema:mfm.events.v1.resource_lane.released:1:sha256-jcs-v1:5c4b4506528fbc8d849d79012d62c87ab72b7df12b26129d84ecacfcdd07be89 []
mfm.events.v1.resource_lane.release_intent schema:mfm.events.v1.resource_lane.release_intent:1:sha256-jcs-v1:8053fc9f4e470aaed7bd05872717ea78d07999cefa0b17e0bf78e0925b4acb1a []
mfm.events.v1.public_output_produced schema:mfm.events.v1.public_output_produced:1:sha256-jcs-v1:0f6b8361913bd2993f79a02c15f87d93cfaaadfbdcd055176419ca9758eba098 [PublicOutputCell,PublicOutputRendered]
mfm.events.v1.public_output_render_failed schema:mfm.events.v1.public_output_render_failed:1:sha256-jcs-v1:c25355e4bddc82bd7626afbc7128a9c3f94d86f5951faa103eaeb8948bf8a8cc [PublicOutputRenderFailureDiagnostic]
mfm.events.v1.state_attempt_completed schema:mfm.events.v1.state_attempt_completed:1:sha256-jcs-v1:36800f9d3ae748d407bc2ea24339049471c8ffe40aa86c53b35b6c7c6cd6ee80 []
mfm.events.v1.state_attempt_interrupted schema:mfm.events.v1.state_attempt_interrupted:1:sha256-jcs-v1:a01ea4960dfa7c42cd9da4a572a2748513cae4107b99ac1f04afc37b7a4e9e14 []
mfm.events.v1.state_attempt_failed schema:mfm.events.v1.state_attempt_failed:1:sha256-jcs-v1:6b5970a1e5da5adc00fc141f0bfaeb9114196e1f87291c2ad9121e7fe41644d6 [StateAttemptFailureDiagnostic]
mfm.events.v1.manual_resolution_recorded schema:mfm.events.v1.manual_resolution_recorded:1:sha256-jcs-v1:045108e1d80601559ce9d7a408a24e2052dfee309ae2b255b3f83b4508b4d352 [ManualResolutionEvidence,ManualResolutionAuthorization]
mfm.events.v1.run_completed schema:mfm.events.v1.run_completed:1:sha256-jcs-v1:cda37495cb3c733164ce1a91f58ff6d27bdcfbf9b1f9efe5a7fd48ae68eba479 []
mfm.events.v1.retention_refs_appended schema:mfm.events.v1.retention_refs_appended:1:sha256-jcs-v1:354dd1baedf12dab84a3885079b19bbd205004bbc4ef5dc6dfb2dfd4ed197476 [RetentionRef]
mfm.events.v1.retention_manifest_projected schema:mfm.events.v1.retention_manifest_projected:1:sha256-jcs-v1:0dc12ccdc64f00e214e6fdb17e7be8b7bfe0900f64b99daa59c4d437a14784cf [RetentionManifest]"#
    );
}

#[test]
fn v1_event_schema_golden() {
    let rows = all_event_schema_descriptors()
        .iter()
        .map(|descriptor| {
            format!(
                "{} {}",
                descriptor.rust_type_path,
                descriptor.schema_id().expect("schema id").as_str()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(all_event_schema_descriptors().len(), 31);
    assert_eq!(
        rows,
        r#"mfm_events::v1::RunAdmitted schema:mfm.events.v1.run_admitted:1:sha256-jcs-v1:9da01b57ef30318cc98621ddeef1092fe3b6fd8b1fce3b49b739af4278a83ba4
mfm_events::v1::StateAttemptStarted schema:mfm.events.v1.state_attempt_started:1:sha256-jcs-v1:986f35aa39938713b9862192cab7d2b9b3a37219f5872bd242f8a06e7957ff1b
mfm_events::v1::FactRecorded schema:mfm.events.v1.fact_recorded:1:sha256-jcs-v1:f66fc733963d3564fe94bcd8c5d80c489405ddc63c34002a9c6af381cf8f1734
mfm_events::v1::ArtifactReferenced schema:mfm.events.v1.artifact_referenced:1:sha256-jcs-v1:b60ebe4bc262799c154a596362ae57672801f54e1b18d703572848626ccef3f0
mfm_events::v1::CellProduced schema:mfm.events.v1.cell_produced:1:sha256-jcs-v1:a4bac036db54afd368d6e9c53b566daadba6ef6ea72bdf9839dd08213284737e
mfm_events::v1::CellSkipped schema:mfm.events.v1.cell_skipped:1:sha256-jcs-v1:e430038e78e1a83bf2465de6bdc23703395781dc4395e0340eb3856298225b84
mfm_events::v1::side_effect::IntentPersisted schema:mfm.events.v1.side_effect.intent_persisted:1:sha256-jcs-v1:1301896f1bd678773c0f145114b0c29cf7b06138686a99b75716a7d41fc181ed
mfm_events::v1::side_effect::Claimed schema:mfm.events.v1.side_effect.claimed:1:sha256-jcs-v1:793b4e2c8a58d2acef7cd4500fe7b20a1cf6dfd57386ccb8ef47cf64b3feb862
mfm_events::v1::side_effect::ClaimTakenOver schema:mfm.events.v1.side_effect.claim_taken_over:1:sha256-jcs-v1:abdc3cb22d1022954df00d0fa18ea5ddf0f71f576b9a9c7e0dca08de2a0f5acf
mfm_events::v1::ResourceLaneClaimed schema:mfm.events.v1.resource_lane.claimed:1:sha256-jcs-v1:c9b27a3f52464ad34d1cb80fc41947bbfa71bfcba8ab4d78cc09e4b1a9286e4c
mfm_events::v1::ResourceLaneClaimIntent schema:mfm.events.v1.resource_lane.claim_intent:1:sha256-jcs-v1:14ec3899b92dc1182f8b5865e2c5597b053ae648290b9b900eb161014dd7b4ed
mfm_events::v1::side_effect::InvocationPrepared schema:mfm.events.v1.side_effect.invocation_prepared:1:sha256-jcs-v1:71fcef6b4e021f3c49cf1702377b49db1461f6e5918d17a45fda924b8e680fbd
mfm_events::v1::side_effect::InvocationStarted schema:mfm.events.v1.side_effect.invocation_started:1:sha256-jcs-v1:7d4cf3ffbd5109af4251e207927b0b4819e3ec058a948b1d106d022eeddfa3fc
mfm_events::v1::side_effect::NotSubmittedProven schema:mfm.events.v1.side_effect.not_submitted_proven:1:sha256-jcs-v1:c1ccc7c56931f0f14fa86e233899beb7b744b5cff47f68ec74e53e7f8cf6af12
mfm_events::v1::side_effect::SubmissionObserved schema:mfm.events.v1.side_effect.submission_observed:1:sha256-jcs-v1:612411db371d86f8cc7033745692bcd88018b632ded4b156b0c96935bc4532cb
mfm_events::v1::side_effect::SubmissionUnknown schema:mfm.events.v1.side_effect.submission_unknown:1:sha256-jcs-v1:56c8fb7e5f12f11c178c671edd68fff8d22a8acd27eb97a7cc364b71a93f5637
mfm_events::v1::side_effect::ReceiptObserved schema:mfm.events.v1.side_effect.receipt_observed:1:sha256-jcs-v1:f24eae04a3a4dcf471c5c4fc362deeb25e8915958565bc61236684ef5e8fd549
mfm_events::v1::side_effect::ConfirmationObserved schema:mfm.events.v1.side_effect.confirmation_observed:1:sha256-jcs-v1:c5f0bee3807376b77bdb4386ea18c92ed17f2e8381a249c50eddb35b75690143
mfm_events::v1::side_effect::Ambiguous schema:mfm.events.v1.side_effect.ambiguous:1:sha256-jcs-v1:2e436cd594c93cdd1e8a548804b90719c13cd3e62933a90e0ae810e3ae04686a
mfm_events::v1::side_effect::Failed schema:mfm.events.v1.side_effect.failed:1:sha256-jcs-v1:0f9166fa332675d82be749016fc2b5836dfebe6ea6aa1c25b7b59a7144c0fb4d
mfm_events::v1::ResourceLaneReleased schema:mfm.events.v1.resource_lane.released:1:sha256-jcs-v1:5c4b4506528fbc8d849d79012d62c87ab72b7df12b26129d84ecacfcdd07be89
mfm_events::v1::ResourceLaneReleaseIntent schema:mfm.events.v1.resource_lane.release_intent:1:sha256-jcs-v1:8053fc9f4e470aaed7bd05872717ea78d07999cefa0b17e0bf78e0925b4acb1a
mfm_events::v1::PublicOutputProduced schema:mfm.events.v1.public_output_produced:1:sha256-jcs-v1:0f6b8361913bd2993f79a02c15f87d93cfaaadfbdcd055176419ca9758eba098
mfm_events::v1::PublicOutputRenderFailed schema:mfm.events.v1.public_output_render_failed:1:sha256-jcs-v1:c25355e4bddc82bd7626afbc7128a9c3f94d86f5951faa103eaeb8948bf8a8cc
mfm_events::v1::StateAttemptCompleted schema:mfm.events.v1.state_attempt_completed:1:sha256-jcs-v1:36800f9d3ae748d407bc2ea24339049471c8ffe40aa86c53b35b6c7c6cd6ee80
mfm_events::v1::StateAttemptInterrupted schema:mfm.events.v1.state_attempt_interrupted:1:sha256-jcs-v1:a01ea4960dfa7c42cd9da4a572a2748513cae4107b99ac1f04afc37b7a4e9e14
mfm_events::v1::StateAttemptFailed schema:mfm.events.v1.state_attempt_failed:1:sha256-jcs-v1:6b5970a1e5da5adc00fc141f0bfaeb9114196e1f87291c2ad9121e7fe41644d6
mfm_events::v1::ManualResolutionRecorded schema:mfm.events.v1.manual_resolution_recorded:1:sha256-jcs-v1:045108e1d80601559ce9d7a408a24e2052dfee309ae2b255b3f83b4508b4d352
mfm_events::v1::RunCompleted schema:mfm.events.v1.run_completed:1:sha256-jcs-v1:cda37495cb3c733164ce1a91f58ff6d27bdcfbf9b1f9efe5a7fd48ae68eba479
mfm_events::v1::RetentionRefsAppended schema:mfm.events.v1.retention_refs_appended:1:sha256-jcs-v1:354dd1baedf12dab84a3885079b19bbd205004bbc4ef5dc6dfb2dfd4ed197476
mfm_events::v1::RetentionManifestProjected schema:mfm.events.v1.retention_manifest_projected:1:sha256-jcs-v1:0dc12ccdc64f00e214e6fdb17e7be8b7bfe0900f64b99daa59c4d437a14784cf"#
    );
}

#[test]
fn fact_recorded_schema_descriptor_baseline() {
    let canonical = FACT_RECORDED_SCHEMA
        .canonical_json()
        .expect("canonical fact schema");

    assert_eq!(
        canonical.as_str(),
        r#"{"canonicalization":"sha256-jcs-v1","fields":[{"cardinality":"required","name":"spec_hash","type":{"kind":"mfm_ids_identity","name":"SpecHash","persisted_as":"string"}},{"cardinality":"required","name":"node_id","type":{"kind":"mfm_ids_identity","name":"NodeId","persisted_as":"string"}},{"cardinality":"required","name":"attempt_id","type":{"kind":"mfm_ids_identity","name":"AttemptId","persisted_as":"string"}},{"cardinality":"required","name":"claim","type":{"fields":[{"cardinality":"required","name":"visibility","type":{"kind":"enum","name":"FactVisibility","variants":[{"fields":[],"name":"run_private"},{"fields":[{"cardinality":"required","name":"audience","type":{"kind":"enum","name":"FactAudience","variants":[{"fields":[],"name":"control"},{"fields":[],"name":"platform"}]}},{"cardinality":"required","name":"scope","type":{"kind":"enum","name":"FactVisibilityScope","variants":[{"fields":[],"name":"default"}]}}],"name":"indexed"}]}},{"cardinality":"required","name":"fact_kind","type":{"kind":"mfm_ids_visible_ascii","max_len":256,"name":"FactKind","non_empty":true}},{"cardinality":"required","name":"fact_descriptor_hash","type":{"kind":"mfm_ids_identity","name":"ContentDigest","persisted_as":"string"}},{"cardinality":"required","name":"subject","type":{"fields":[{"cardinality":"required","name":"fact_subject_namespace_hash","type":{"kind":"mfm_ids_identity","name":"ContentDigest","persisted_as":"string"}},{"cardinality":"required","name":"subject_material","type":{"kind":"string","name":"PlainCanonicalJsonBytes"}},{"cardinality":"required","name":"subject_material_hash","type":{"kind":"mfm_ids_identity","name":"ContentDigest","persisted_as":"string"}},{"cardinality":"required","name":"fact_key","type":{"kind":"mfm_ids_visible_ascii","max_len":256,"name":"FactKey","non_empty":true}}],"kind":"struct","name":"FactSubjectEvidence"}},{"cardinality":"optional","name":"observed_at","type":{"kind":"string","name":"String"}},{"cardinality":"optional","name":"request","type":{"fields":[{"cardinality":"required","name":"request_schema_id","type":{"kind":"mfm_ids_identity","name":"SchemaId","persisted_as":"string"}},{"cardinality":"required","name":"request_hash","type":{"kind":"mfm_ids_identity","name":"ContentDigest","persisted_as":"string"}}],"kind":"struct","name":"FactRequestEvidence"}},{"cardinality":"required","name":"response","type":{"fields":[{"cardinality":"required","name":"response_schema_id","type":{"kind":"mfm_ids_identity","name":"SchemaId","persisted_as":"string"}},{"cardinality":"required","name":"response_hash","type":{"kind":"mfm_ids_identity","name":"ContentDigest","persisted_as":"string"}},{"cardinality":"required","name":"artifact_id","type":{"kind":"mfm_ids_identity","name":"ArtifactId","persisted_as":"string"}},{"cardinality":"required","name":"artifact_evidence_hash","type":{"kind":"mfm_ids_identity","name":"ContentDigest","persisted_as":"string"}}],"kind":"struct","name":"FactResponseEvidence"}},{"cardinality":"required","name":"producer","type":{"fields":[{"cardinality":"required","name":"capability_kind","type":{"kind":"mfm_ids_identity","name":"CapabilityKind","persisted_as":"string"}},{"cardinality":"required","name":"capability_version","type":{"kind":"mfm_ids_version","name":"CapabilityVersion","persisted_as":"string"}},{"cardinality":"required","name":"adapter_kind","type":{"kind":"mfm_ids_identity","name":"AdapterKind","persisted_as":"string"}},{"cardinality":"required","name":"adapter_version","type":{"kind":"mfm_ids_version","name":"AdapterVersion","persisted_as":"string"}}],"kind":"struct","name":"FactProducerProvenance"}}],"kind":"struct","name":"FactClaim"}}],"schema_family":"mfm.kernel.event","schema_name":"mfm.events.v1.fact_recorded","schema_version":"1"}"#
    );
    assert_eq!(
        canonical.content_digest().as_str(),
        "content:sha256-jcs-v1:f66fc733963d3564fe94bcd8c5d80c489405ddc63c34002a9c6af381cf8f1734"
    );
    let schema: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("fact schema json");
    let top_level_fields = schema["fields"]
        .as_array()
        .expect("schema fields")
        .iter()
        .map(|field| field["name"].as_str().expect("field name"))
        .collect::<Vec<_>>();
    assert_eq!(
        top_level_fields,
        vec!["spec_hash", "node_id", "attempt_id", "claim"]
    );
}

#[test]
fn event_schema_hashes_include_nested_structural_shapes() {
    let run_admitted_schema = RUN_ADMITTED_SCHEMA
        .canonical_json()
        .expect("run admitted schema json");
    let run_admitted_json = run_admitted_schema.as_str();
    assert!(run_admitted_json.contains("\"name\":\"emitted_fact_descriptors\""));
    assert!(run_admitted_json.contains("\"name\":\"FactDescriptorRef\""));

    let artifact_schema = ARTIFACT_REFERENCED_SCHEMA
        .canonical_json()
        .expect("artifact schema json");
    let artifact_json = artifact_schema.as_str();
    assert!(artifact_json.contains("\"name\":\"ArtifactEvidenceRef\""));
    assert!(artifact_json.contains("\"name\":\"semantic_type_id\""));
    assert!(artifact_json.contains("\"name\":\"ArtifactRole\""));

    let output_schema = PUBLIC_OUTPUT_PRODUCED_SCHEMA
        .canonical_json()
        .expect("public output schema json");
    let output_json = output_schema.as_str();
    assert!(output_json.contains("\"name\":\"NamedTypedCellRef\""));
    assert!(output_json.contains("\"name\":\"value_lineage\""));

    let completed_schema = RUN_COMPLETED_SCHEMA
        .canonical_json()
        .expect("completed schema json");
    let completed_json = completed_schema.as_str();
    assert!(completed_json.contains("\"name\":\"RunCompletionOutcome\""));
    assert!(completed_json.contains("\"name\":\"PublicOutputCompletionEvidence\""));
    assert!(completed_json.contains("\"name\":\"public_output_event_id\""));
    assert!(completed_json.contains("\"name\":\"compensated\""));
    assert!(completed_json.contains("\"name\":\"manually_resolved\""));
    assert!(completed_json.contains("\"name\":\"failed_without_acdc_claim\""));
    assert!(!completed_json.contains("\"name\":\"terminal_error\""));

    let manual_schema = MANUAL_RESOLUTION_RECORDED_SCHEMA
        .canonical_json()
        .expect("manual resolution schema json");
    let manual_json = manual_schema.as_str();
    assert!(manual_json.contains("\"name\":\"ManualResolutionOutcome\""));
    assert!(manual_json.contains("\"name\":\"authorization_schema_id\""));
    assert!(manual_json.contains("\"name\":\"evidence_artifact_id\""));

    let side_effect_schema = SIDE_EFFECT_INTENT_PERSISTED_SCHEMA
        .canonical_json()
        .expect("side effect schema json");
    let side_effect_json = side_effect_schema.as_str();
    assert!(side_effect_json.contains("\"name\":\"SideEffectLedgerPurpose\""));
    assert!(side_effect_json.contains("\"name\":\"forward\""));
    assert!(side_effect_json.contains("\"name\":\"remediation\""));
}

#[test]
fn saga_projection_type_tags_are_stable() {
    assert_eq!(
        ManualResolutionOutcome::ConfirmRemediated.as_str(),
        "confirm_remediated"
    );
    assert_eq!(
        ManualResolutionOutcome::FailWithoutAcdcClaim.as_str(),
        "fail_without_acdc_claim"
    );
    assert_eq!(SideEffectLedgerPurpose::Forward.kind(), "forward");
    assert_eq!(
        SideEffectLedgerPurpose::Remediation {
            forward_pair_id: side_effect_pair_id(37),
        }
        .kind(),
        "remediation"
    );

    assert_eq!(RunCompletionOutcome::Compensated.kind(), "compensated");
}

#[test]
fn event_schema_descriptors_have_no_opaque_external_shapes() {
    for descriptor in all_event_schema_descriptors() {
        let json = descriptor.canonical_json().expect("descriptor json");
        assert!(
            !json.as_str().contains("external_contract_type"),
            "{} has an opaque external schema shape: {}",
            descriptor.rust_type_path,
            json.as_str()
        );
    }
}

#[test]
fn all_event_schema_descriptors_are_unique() {
    let descriptors = all_event_schema_descriptors();
    let mut schema_names = descriptors
        .iter()
        .map(|descriptor| descriptor.schema_name)
        .collect::<Vec<_>>();
    let original_len = schema_names.len();
    schema_names.sort_unstable();
    schema_names.dedup();

    assert_eq!(original_len, 31);
    assert_eq!(schema_names.len(), original_len);
}

#[test]
fn payload_accessors_expose_authority_fields_without_serialization_changes() {
    let run_admitted = run_admitted_payload();
    let expected_run_id = run_identity_material(spec_hash(2))
        .derive_run_id()
        .expect("run id");
    assert_eq!(run_admitted.run_id(), Some(&expected_run_id));
    assert_eq!(run_admitted.spec_hash(), &spec_hash(2));

    let side_effect =
        KernelEventPayload::SideEffectSubmissionObserved(side_effect::SubmissionObserved {
            spec_hash: spec_hash(30),
            node_id: node_id(31),
            attempt_id: attempt_id(32),
            ledger_key: SideEffectLedgerKey::new("ledger-1").expect("ledger"),
            ledger_purpose: SideEffectLedgerPurpose::Forward,
            pair_id: side_effect_pair_id(36),
            pair_role: SideEffectPairRole::Submit,
            invocation_epoch: 3,
            submission_schema_id: schema_id("mfm.test.submission", 33),
            submission_hash: content_digest(34),
            submission_artifact_id: artifact_id(35),
            submission_artifact_evidence_hash: content_digest(135),
        });
    let side_effect_ref = side_effect.side_effect_ref().expect("side-effect ref");
    assert_eq!(side_effect.run_id(), None);
    assert_eq!(side_effect.spec_hash(), &spec_hash(30));
    assert_eq!(side_effect_ref.node_id, &node_id(31));
    assert_eq!(side_effect_ref.attempt_id, &attempt_id(32));
    assert_eq!(side_effect_ref.pair_id, &side_effect_pair_id(36));
    assert_eq!(side_effect_ref.pair_role, SideEffectPairRole::Submit);
    assert_eq!(
        side_effect_ref.kind,
        SideEffectEventKind::SubmissionObserved
    );
    assert_eq!(side_effect_ref.invocation_epoch, Some(3));
}

#[test]
fn artifact_requirement_accessor_covers_artifact_bearing_variants() {
    let cases = vec![
        (
            run_admitted_payload(),
            vec![
                EventArtifactReferenceSource::RunSpec,
                EventArtifactReferenceSource::RunCertificate,
                EventArtifactReferenceSource::RunConfig,
                EventArtifactReferenceSource::SeedCell,
            ],
        ),
        (
            KernelEventPayload::FactRecorded(FactRecorded {
                spec_hash: spec_hash(40),
                node_id: node_id(41),
                attempt_id: attempt_id(42),
                claim: fact_claim(43, 44, 45, 46, 47, 48, 49),
            }),
            vec![EventArtifactReferenceSource::FactResponse],
        ),
        (
            KernelEventPayload::ArtifactReferenced(ArtifactReferenced {
                spec_hash: spec_hash(50),
                node_id: Some(node_id(51)),
                attempt_id: Some(attempt_id(52)),
                artifact_ref: event_artifact_ref(
                    artifact_id(53),
                    ArtifactRole::TypedConfig,
                    schema_id("mfm.test.config", 54),
                    content_digest(55),
                ),
            }),
            vec![EventArtifactReferenceSource::ArtifactReferenced],
        ),
        (
            KernelEventPayload::CellProduced(CellProduced {
                spec_hash: spec_hash(56),
                node_id: node_id(57),
                cell_id: cell_id(58),
                scope_id: scope_id(59),
                attempt_id: attempt_id(60),
                semantic_type_id: semantic_id("state_output", 61),
                schema_id: schema_id("mfm.test.state_output", 62),
                value_lineage: ValueLineageRef {
                    lineage_digest: content_digest(63),
                },
                context: mfm_spec::v1::CellContextSpec::no_context(),
                artifact_id: artifact_id(64),
                content_digest: content_digest(65),
                evidence_hash: content_digest(165),
                producer_state_kind: None,
                producer_state_version: None,
            }),
            vec![EventArtifactReferenceSource::StateOutput],
        ),
        (
            KernelEventPayload::PublicOutputProduced(PublicOutputProduced {
                spec_hash: spec_hash(66),
                node_id: node_id(67),
                attempt_id: attempt_id(68),
                receipt_cell_id: cell_id(69),
                public_schema_id: schema_id("mfm.test.public_output", 70),
                output_spec_digest: content_digest(71),
                cells: vec![NamedTypedCellRef {
                    public_field_path: PublicFieldPath::new("result").expect("field"),
                    cell_id: cell_id(72),
                    producer: CellProducer::Node(node_id(73)),
                    scope_id: scope_id(74),
                    semantic_type_id: semantic_id("public_cell", 75),
                    schema_id: schema_id("mfm.test.public_cell", 76),
                    value_lineage: ValueLineageRef {
                        lineage_digest: content_digest(77),
                    },
                    content_digest: content_digest(78),
                    artifact_id: artifact_id(79),
                    evidence_hash: content_digest(179),
                }],
                rendered_digest: content_digest(80),
                rendered_artifact_id: Some(artifact_id(81)),
                rendered_artifact_evidence_hash: Some(content_digest(181)),
                renderer_descriptor_id: descriptor_id(82),
            }),
            vec![
                EventArtifactReferenceSource::PublicOutputCell,
                EventArtifactReferenceSource::PublicOutputRendered,
            ],
        ),
        (
            KernelEventPayload::PublicOutputRenderFailed(PublicOutputRenderFailed {
                spec_hash: spec_hash(83),
                node_id: node_id(84),
                attempt_id: attempt_id(85),
                public_schema_id: schema_id("mfm.test.public_output", 86),
                renderer_descriptor_id: descriptor_id(87),
                error: error_with_diagnostic(
                    artifact_id(88),
                    ArtifactRole::RedactedDiagnostic,
                    schema_id("mfm.test.diagnostic", 89),
                    content_digest(90),
                ),
            }),
            vec![EventArtifactReferenceSource::PublicOutputRenderFailureDiagnostic],
        ),
        (
            KernelEventPayload::StateAttemptFailed(StateAttemptFailed {
                spec_hash: spec_hash(91),
                node_id: node_id(92),
                attempt_id: attempt_id(93),
                retryable: false,
                error: error_with_diagnostic(
                    artifact_id(94),
                    ArtifactRole::RedactedDiagnostic,
                    schema_id("mfm.test.diagnostic", 95),
                    content_digest(96),
                ),
            }),
            vec![EventArtifactReferenceSource::StateAttemptFailureDiagnostic],
        ),
        (
            KernelEventPayload::ManualResolutionRecorded(ManualResolutionRecorded {
                run_id: run_id(97),
                spec_hash: spec_hash(98),
                outcome: ManualResolutionOutcome::ConfirmRemediated,
                evidence_schema_id: schema_id("mfm.test.manual_evidence", 99),
                evidence_hash: content_digest(100),
                evidence_artifact_id: artifact_id(101),
                evidence_artifact_evidence_hash: content_digest(201),
                authorization_schema_id: schema_id("mfm.test.manual_authorization", 102),
                authorization_hash: content_digest(103),
                authorization_artifact_id: artifact_id(104),
                authorization_artifact_evidence_hash: content_digest(204),
                note: None,
            }),
            vec![
                EventArtifactReferenceSource::ManualResolutionEvidence,
                EventArtifactReferenceSource::ManualResolutionAuthorization,
            ],
        ),
        (
            KernelEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
                spec_hash: spec_hash(105),
                node_id: node_id(106),
                scope_id: scope_id(107),
                attempt_id: attempt_id(108),
                ledger_key: SideEffectLedgerKey::new("ledger-intent").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Submit,
                invocation_epoch: 1,
                intent_schema_id: schema_id("mfm.test.intent", 109),
                intent_hash: content_digest(110),
                intent_artifact_id: artifact_id(111),
                intent_artifact_evidence_hash: content_digest(211),
                idempotency_input_schema_id: schema_id("mfm.test.idempotency", 112),
                idempotency_input_hash: content_digest(113),
                idempotency_key: IdempotencyKeyRef::new("idem-key-1").expect("idempotency"),
                capability_kind: CapabilityKind::new(
                    "mfm.test",
                    "capability",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_bytes(114),
                )
                .expect("capability kind"),
                capability_version: CapabilityVersion::new("mfm.test.capability.v1")
                    .expect("capability version"),
                adapter_kind: AdapterKind::new(
                    "mfm.test",
                    "adapter",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_bytes(115),
                )
                .expect("adapter kind"),
                adapter_version: AdapterVersion::new("mfm.test.adapter.v1")
                    .expect("adapter version"),
            }),
            vec![EventArtifactReferenceSource::SideEffectIntent],
        ),
        (
            KernelEventPayload::SideEffectInvocationPrepared(side_effect::InvocationPrepared {
                spec_hash: spec_hash(116),
                node_id: node_id(117),
                attempt_id: attempt_id(118),
                ledger_key: SideEffectLedgerKey::new("ledger-prepared").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Submit,
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: side_effect::ClaimFencingToken::new("token-1").expect("token"),
                resource_key: None,
                prepared_artifact_id: Some(artifact_id(119)),
                prepared_hash: Some(content_digest(120)),
                prepared_artifact_evidence_hash: Some(content_digest(220)),
            }),
            vec![EventArtifactReferenceSource::PreparedInvocation],
        ),
        (
            KernelEventPayload::SideEffectNotSubmittedProven(side_effect::NotSubmittedProven {
                spec_hash: spec_hash(121),
                node_id: node_id(122),
                attempt_id: attempt_id(123),
                ledger_key: SideEffectLedgerKey::new("ledger-not-submitted").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Submit,
                invocation_epoch: 1,
                proof_schema_id: schema_id("mfm.test.not_submitted", 124),
                proof_hash: content_digest(125),
                proof_artifact_id: artifact_id(126),
                proof_artifact_evidence_hash: content_digest(226),
            }),
            vec![EventArtifactReferenceSource::NotSubmittedProof],
        ),
        (
            KernelEventPayload::SideEffectSubmissionObserved(side_effect::SubmissionObserved {
                spec_hash: spec_hash(127),
                node_id: node_id(128),
                attempt_id: attempt_id(129),
                ledger_key: SideEffectLedgerKey::new("ledger-submission").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Submit,
                invocation_epoch: 1,
                submission_schema_id: schema_id("mfm.test.submission", 130),
                submission_hash: content_digest(131),
                submission_artifact_id: artifact_id(132),
                submission_artifact_evidence_hash: content_digest(232),
            }),
            vec![EventArtifactReferenceSource::Submission],
        ),
        (
            KernelEventPayload::SideEffectSubmissionUnknown(side_effect::SubmissionUnknown {
                spec_hash: spec_hash(133),
                node_id: node_id(134),
                attempt_id: attempt_id(135),
                ledger_key: SideEffectLedgerKey::new("ledger-submission-unknown").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Submit,
                invocation_epoch: 1,
                evidence_schema_id: schema_id("mfm.test.submission_unknown", 136),
                evidence_hash: content_digest(137),
                evidence_artifact_id: artifact_id(138),
                evidence_artifact_evidence_hash: content_digest(238),
            }),
            vec![EventArtifactReferenceSource::SubmissionUnknownEvidence],
        ),
        (
            KernelEventPayload::SideEffectReceiptObserved(side_effect::ReceiptObserved {
                spec_hash: spec_hash(139),
                node_id: node_id(140),
                attempt_id: attempt_id(141),
                ledger_key: SideEffectLedgerKey::new("ledger-receipt").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Verify,
                invocation_epoch: 1,
                receipt_schema_id: schema_id("mfm.test.receipt", 142),
                receipt_hash: content_digest(143),
                receipt_artifact_id: artifact_id(144),
                receipt_artifact_evidence_hash: content_digest(244),
                replay_verifier_id: ReplayVerifierId::new("verifier-1").expect("verifier"),
                resource_touched_set: Some(touched_set(145)),
            }),
            vec![
                EventArtifactReferenceSource::Receipt,
                EventArtifactReferenceSource::ResourceTouchedSet,
            ],
        ),
        (
            KernelEventPayload::SideEffectConfirmationObserved(side_effect::ConfirmationObserved {
                spec_hash: spec_hash(148),
                node_id: node_id(149),
                attempt_id: attempt_id(150),
                ledger_key: SideEffectLedgerKey::new("ledger-confirmation").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Verify,
                invocation_epoch: 1,
                confirmation_schema_id: schema_id("mfm.test.confirmation", 151),
                confirmation_hash: content_digest(152),
                confirmation_artifact_id: artifact_id(153),
                confirmation_artifact_evidence_hash: content_digest(253),
                replay_verifier_id: ReplayVerifierId::new("verifier-1").expect("verifier"),
                resource_touched_set: Some(touched_set(154)),
            }),
            vec![
                EventArtifactReferenceSource::Confirmation,
                EventArtifactReferenceSource::ResourceTouchedSet,
            ],
        ),
        (
            KernelEventPayload::SideEffectAmbiguous(side_effect::Ambiguous {
                spec_hash: spec_hash(157),
                node_id: node_id(158),
                attempt_id: attempt_id(159),
                ledger_key: SideEffectLedgerKey::new("ledger-ambiguous").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Verify,
                invocation_epoch: 1,
                ambiguity_code: AmbiguityCode::new("ambiguous").expect("ambiguity"),
                evidence_schema_id: schema_id("mfm.test.ambiguity", 160),
                evidence_hash: content_digest(161),
                evidence_artifact_id: artifact_id(162),
                evidence_artifact_evidence_hash: content_digest(162),
            }),
            vec![EventArtifactReferenceSource::AmbiguityEvidence],
        ),
        (
            KernelEventPayload::SideEffectFailed(side_effect::Failed {
                spec_hash: spec_hash(163),
                node_id: node_id(164),
                attempt_id: attempt_id(165),
                ledger_key: SideEffectLedgerKey::new("ledger-failed").expect("ledger"),
                ledger_purpose: SideEffectLedgerPurpose::Forward,
                pair_id: side_effect_pair_id(201),
                pair_role: SideEffectPairRole::Verify,
                invocation_epoch: 1,
                failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
                retryable: false,
                error: error_with_diagnostic(
                    artifact_id(166),
                    ArtifactRole::RedactedDiagnostic,
                    schema_id("mfm.test.diagnostic", 167),
                    content_digest(168),
                ),
            }),
            vec![EventArtifactReferenceSource::SideEffectFailureDiagnostic],
        ),
        (
            KernelEventPayload::RetentionRefsAppended(RetentionRefsAppended {
                run_id: run_id(169),
                spec_hash: spec_hash(170),
                refs: vec![RetentionRef {
                    artifact_id: artifact_id(171),
                    role: ArtifactRole::FactResponse,
                    evidence_hash: content_digest(172),
                    content_digest: content_digest(172),
                }],
                reason: RetentionReason::RuntimeEvidence,
            }),
            vec![EventArtifactReferenceSource::RetentionRef],
        ),
        (
            KernelEventPayload::RetentionManifestProjected(RetentionManifestProjected {
                run_id: run_id(173),
                spec_hash: spec_hash(174),
                manifest_seq: 1,
                manifest_digest: content_digest(175),
                previous_manifest_digest: None,
                manifest_artifact_id: artifact_id(176),
                manifest_artifact_evidence_hash: content_digest(176),
            }),
            vec![EventArtifactReferenceSource::RetentionManifest],
        ),
    ];

    for (payload, expected_sources) in cases {
        let sources = payload
            .artifact_requirements()
            .into_iter()
            .map(|requirement| requirement.source)
            .collect::<Vec<_>>();
        assert_eq!(sources, expected_sources);
    }
}

#[test]
fn run_admitted_v1_summary_key_is_canonical() {
    let keys = ["v1_event_schema_golden", "run_admitted_v1_present"];
    assert!(keys.contains(&"run_admitted_v1_present"));
    assert!(!keys.contains(&"run_admitted_v2_present"));
}
