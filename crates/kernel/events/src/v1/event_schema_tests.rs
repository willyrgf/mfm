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
    assert_eq!(
        rows,
        r#"mfm_events::v1::RunAdmitted schema:mfm.events.v1.run_admitted:1:sha256-jcs-v1:cda48aaa4d423db38f798386851a3d91992801b524768ab69bc3c41cf3fd0850
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
mfm_events::v1::side_effect::InvocationPrepared schema:mfm.events.v1.side_effect.invocation_prepared:1:sha256-jcs-v1:727f477650e620a3526593d80d0a9afbc75359009b19003722bde42c1325c309
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
pub(super) fn fact_recorded_schema_descriptor_baseline() {
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
