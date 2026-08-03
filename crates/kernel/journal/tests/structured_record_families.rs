use mfm_ids::{
    AccessAttemptId, ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, InvocationIdentity,
    JournalRecordHash, OccurrenceId, RequestDigest, RunId, RunSemanticStateDigest, SchemaId,
    SemanticCallId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    AccessKind, AdmissionMaterialRefs, CertifiedProgramAuditRefs, ExternalAccessAuthorized,
    ExternalAccessObserved, LexicalValueRef, ObservationOutcome, RecordRef, RunAdmitted, RunClosed,
    RunRecord, SemanticHead, StateOutcomeRef, StateTransitionCommitted, TypedValueRef,
};

fn digest(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn run_id() -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(1))
}

fn content_ref(byte: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            &format!("mfm.test.value-{byte}"),
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest(byte),
        )
        .expect("schema id"),
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, digest(byte.wrapping_add(1))),
    )
    .expect("content ref")
}

fn typed(byte: u8) -> TypedValueRef {
    TypedValueRef {
        contract_ref: content_ref(byte),
        value_ref: content_ref(byte.wrapping_add(16)),
    }
}

fn lexical(byte: u8) -> LexicalValueRef {
    LexicalValueRef {
        slot_ref: content_ref(byte.wrapping_add(32)),
        value: typed(byte),
        structural_origin: None,
    }
}

fn record_ref() -> RecordRef {
    RecordRef {
        run_id: run_id(),
        run_sequence: 1,
        ordinal: 0,
        record_hash: JournalRecordHash::from_digest(digest(2)),
    }
}

fn semantic_head() -> SemanticHead {
    SemanticHead::Genesis {
        admission_ref: record_ref(),
        semantic_state_digest: RunSemanticStateDigest::from_digest(digest(3)),
    }
}

fn admitted() -> RunRecord {
    let audit_ref = content_ref(4);
    RunRecord::RunAdmitted(RunAdmitted {
        store_scope_id: StoreScopeId::new("mfm.store_scope.v1:00000000000000000000000000000000")
            .expect("store scope"),
        store_epoch: StoreEpoch::new(1),
        run_id: run_id(),
        tenant_scope_id: TenantScopeId::new("mfm.tenant_scope.v1:11111111111111111111111111111111")
            .expect("tenant scope"),
        invocation_identity: InvocationIdentity::new("00000000-0000-4000-8000-000000000000")
            .expect("invocation"),
        entry_point_operation_id: StableId::new("mfm.test/operation").expect("operation"),
        certified_program_ref: content_ref(5),
        certified_program_root_ref: content_ref(6),
        qualified_entry_point_admission_policy_ref: content_ref(7),
        audit_refs: CertifiedProgramAuditRefs {
            authored_program_ref: audit_ref.clone(),
            expanded_program_ref: audit_ref.clone(),
            expansion_profile_ref: audit_ref.clone(),
            expansion_proof_ref: audit_ref.clone(),
            policy_coverage_proof_ref: audit_ref.clone(),
            component_manifest_ref: audit_ref.clone(),
            implementation_manifest_ref: audit_ref,
        },
        admission_material_refs: AdmissionMaterialRefs {
            configuration_ref: content_ref(8),
            context_manifest_ref: content_ref(9),
            prior_run_source_manifest_ref: content_ref(10),
            routing_policy_ref: content_ref(11),
            stable_resource_lineage_contract_refs: vec![content_ref(12)],
        },
        initial_bindings: vec![lexical(13)],
        genesis_semantic_state_digest: RunSemanticStateDigest::from_digest(digest(14)),
    })
}

fn transitioned() -> RunRecord {
    RunRecord::StateTransitionCommitted(StateTransitionCommitted {
        occurrence_id: OccurrenceId::from_digest(digest(20)),
        occurrence_path_ref: content_ref(21),
        semantic_call_id: SemanticCallId::from_digest(digest(22)),
        input: lexical(23),
        consumed_observation_ref: None,
        outcome_ref: content_ref(24),
        outcome: StateOutcomeRef::Success(lexical(25)),
        facts: Vec::new(),
        before_semantic_state_digest: RunSemanticStateDigest::from_digest(digest(26)),
        after_semantic_state_digest: RunSemanticStateDigest::from_digest(digest(27)),
    })
}

fn authorized() -> RunRecord {
    RunRecord::ExternalAccessAuthorized(ExternalAccessAuthorized {
        access_attempt_id: AccessAttemptId::from_digest(digest(30)),
        attempt_ordinal: 0,
        occurrence_id: OccurrenceId::from_digest(digest(31)),
        occurrence_path_ref: content_ref(32),
        semantic_call_id: SemanticCallId::from_digest(digest(33)),
        state_input_ref: lexical(34),
        access_kind: AccessKind::Read,
        semantic_head: semantic_head(),
        capability_contract_ref: content_ref(35),
        capability_implementation_ref: content_ref(36),
        adapter_contract_ref: content_ref(37),
        adapter_implementation_ref: content_ref(38),
        request: typed(39),
        request_digest: RequestDigest::from_digest(digest(40)),
        physical_binding_ref: content_ref(41),
        stable_resource_lineage_contract_ref: None,
    })
}

fn observed() -> RunRecord {
    RunRecord::ExternalAccessObserved(ExternalAccessObserved {
        authorization_ref: record_ref(),
        access_attempt_id: AccessAttemptId::from_digest(digest(42)),
        outcome: ObservationOutcome::Returned { value: typed(43) },
    })
}

fn closed() -> RunRecord {
    RunRecord::RunClosed(RunClosed {
        outcome_ref: content_ref(44),
    })
}

#[test]
fn the_wire_algebra_has_exactly_five_current_record_families() {
    let records = [
        admitted(),
        transitioned(),
        authorized(),
        observed(),
        closed(),
    ];
    let kinds = records
        .iter()
        .map(|record| {
            let value = serde_json::to_value(record).expect("record encodes");
            let decoded: RunRecord = serde_json::from_value(value.clone()).expect("record decodes");
            assert_eq!(&decoded, record);
            value["kind"].as_str().expect("record kind").to_owned()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        [
            "run_admitted",
            "state_transition_committed",
            "external_access_authorized",
            "external_access_observed",
            "run_closed",
        ]
    );
}

#[test]
fn retired_record_kinds_and_open_envelopes_are_rejected() {
    for kind in [
        "effect_requested",
        "effect_settled",
        "fact_selection",
        "dependency_skipped",
        "non_domain_failure",
    ] {
        let bytes = format!(r#"{{"kind":"{kind}","payload":{{}}}}"#);
        assert!(
            serde_json::from_str::<RunRecord>(&bytes).is_err(),
            "retired kind {kind} was accepted"
        );
    }

    let mut open = serde_json::to_value(closed()).expect("closed record");
    open.as_object_mut()
        .expect("record object")
        .insert("legacy".to_owned(), serde_json::Value::Bool(true));
    assert!(serde_json::from_value::<RunRecord>(open).is_err());
}
