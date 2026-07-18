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
        rows,
        r#"mfm.events.v1.run_admitted schema:mfm.events.v1.run_admitted:1:sha256-jcs-v1:e6f2a19c3f35479e6ba58a4e497469e0f1d34d983dfcf4129de74f4099ec63b4 [RunSpec,RunCertificate,RunConfig,SeedCell,FactDescriptor]
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
mfm.events.v1.side_effect.invocation_prepared schema:mfm.events.v1.side_effect.invocation_prepared:1:sha256-jcs-v1:727f477650e620a3526593d80d0a9afbc75359009b19003722bde42c1325c309 [PreparedInvocation]
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
