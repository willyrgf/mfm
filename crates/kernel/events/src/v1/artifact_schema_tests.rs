use super::*;

pub(super) fn run_admitted_payload() -> KernelEventPayload {
    let identity_material = run_identity_material(spec_hash(2));
    let run_id = identity_material.derive_run_id().expect("run id");
    KernelEventPayload::RunAdmitted(Box::new(RunAdmitted {
        run_id,
        identity_material,
        entry_point: EntryPointLaunchEvidence::new("mfm.test/example@1", Vec::new())
            .expect("entry point evidence"),
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
        capability_implementations: vec![CapabilityImplementationIdentity {
            capability_kind: CapabilityKind::new(
                "mfm.test",
                "capability",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(18),
            )
            .expect("capability kind"),
            capability_version: CapabilityVersion::new("mfm.test.capability.v1")
                .expect("capability version"),
            implementation_id: RuntimeBindingId::new("mfm.test.capability.runtime.v1")
                .expect("implementation id"),
        }],
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
pub(super) struct ArtifactRoleBaseline {
    role: ArtifactRole,
    tag: &'static str,
    schema_policy: &'static str,
    semantic_policy: &'static str,
    producer_policy: &'static str,
    staging_class: &'static str,
    retention_class: &'static str,
    same_commit_policy: &'static str,
}

pub(super) fn artifact_role_baselines() -> &'static [ArtifactRoleBaseline] {
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
            schema_policy: "exact_evidence_schema",
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

pub(super) fn artifact_role_schema_tags() -> Vec<String> {
    let schema = ARTIFACT_REFERENCED_SCHEMA
        .canonical_json()
        .expect("artifact schema json");
    let json: serde_json::Value = serde_json::from_str(schema.as_str()).expect("schema json value");
    find_artifact_role_variants(&json).expect("artifact role variants")
}

pub(super) fn find_artifact_role_variants(value: &serde_json::Value) -> Option<Vec<String>> {
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
pub(super) fn artifact_role_contract_policy_baseline_covers_schema_tags() {
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
prepared_invocation schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_prepared_invocation retention=value_artifacts same_commit=payload_required_artifact\n\
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

pub(super) fn descriptor_artifact_requirement_sources(schema_name: &str) -> &'static str {
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
pub(super) fn event_schema_descriptor_requirement_sources_golden() {
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
        mfm_canonical::sha256_digest_bytes(rows.as_bytes()).to_string(),
        "9ec5cb07f5938944f2bde4052ebc11ab1a40b36a3a6af8ee038893af3e017887"
    );
}
