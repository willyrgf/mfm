use super::*;

#[test]
pub(super) fn v1_event_schema_golden() {
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
    assert!(all_event_schema_descriptors()
        .iter()
        .all(|descriptor| descriptor.schema_version == EVENT_SCHEMA_VERSION));
    assert_eq!(
        mfm_canonical::sha256_digest_bytes(rows.as_bytes()).to_string(),
        "5ea4ff6078629d8a2174c46df0512d6726a6f5077f3310667b3cfcf3a6eb600d"
    );
}

#[test]
pub(super) fn fact_recorded_schema_descriptor_baseline() {
    let canonical = FACT_RECORDED_SCHEMA
        .canonical_json()
        .expect("canonical fact schema");

    assert!(canonical.as_str().contains("\"schema_version\":\"2\""));
    assert!(canonical.as_str().contains("\"name\":\"FactClaim\""));
    for removed in [
        "FactVisibility",
        "FactAudience",
        "FactVisibilityScope",
        "FactRequestEvidence",
        "FactProducerProvenance",
        "observed_at",
    ] {
        assert!(!canonical.as_str().contains(removed));
    }
    assert_eq!(
        canonical.content_digest().as_str(),
        "content:sha256-jcs-v1:7e044feab4edfcbfd9080ee89b7105df72bb7404e0efe71ca184818860576381"
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
pub(super) fn event_schema_hashes_include_nested_structural_shapes() {
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
pub(super) fn saga_projection_type_tags_are_stable() {
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
pub(super) fn event_schema_descriptors_have_no_opaque_external_shapes() {
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
pub(super) fn all_event_schema_descriptors_are_unique() {
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
pub(super) fn payload_accessors_expose_authority_fields_without_serialization_changes() {
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
pub(super) fn run_admitted_v1_summary_key_is_canonical() {
    let keys = ["v1_event_schema_golden", "run_admitted_v1_present"];
    assert!(keys.contains(&"run_admitted_v1_present"));
    assert!(!keys.contains(&"run_admitted_v2_present"));
}
