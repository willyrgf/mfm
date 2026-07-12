use super::*;

pub(super) trait TestPreparedCommitExt {
    fn append_prepared_commit(
        &mut self,
        request: CommitRequest,
    ) -> mfm_store::v1::Result<CommitOutcome>;

    fn append_prepared_commit_with_artifacts(
        &mut self,
        request: CommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    ) -> mfm_store::v1::Result<CommitOutcome>;
}

impl TestPreparedCommitExt for StoreContractRunStore {
    fn append_prepared_commit(
        &mut self,
        request: CommitRequest,
    ) -> mfm_store::v1::Result<CommitOutcome> {
        let admitted_artifacts = request.required_artifacts().to_vec();
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_test_commit_plan(plan)
    }

    fn append_prepared_commit_with_artifacts(
        &mut self,
        request: CommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    ) -> mfm_store::v1::Result<CommitOutcome> {
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_test_commit_plan(plan)
    }
}

pub(super) fn append_async_prepared_commit(
    store: &AsyncInMemoryRunStore,
    request: CommitRequest,
) -> mfm_store::v1::Result<CommitOutcome> {
    let plan = test_prepared_commit_plan(request.clone(), request.required_artifacts().to_vec())?;
    store.seed_artifact_evidence_for_test(plan.admitted_artifacts())?;
    let bundle = test_bundle_from_plan(plan)?;
    poll_ready_store_future(store.append_prepared_commit_bundle(bundle))
}

pub(super) fn async_expected_next_seq(store: &AsyncInMemoryRunStore, run_id: &RunId) -> StreamSeq {
    poll_ready_store_future(store.expected_next_seq(run_id)).expect("async expected next seq")
}

pub(super) fn run_start_request(run_id: RunId, commit_key: &str) -> CommitRequest {
    run_start_request_with_saga_policy(run_id, commit_key, &SagaPolicySpec::NoSideEffects)
}

pub(super) fn fact_run_start_request_with_node(
    run_id: RunId,
    commit_key: &str,
    fact_node_id: NodeId,
) -> CommitRequest {
    let authority_spec = fact_authority_spec_with_node(fact_node_id.clone());
    let descriptor_artifact = fact_descriptor_artifact_ref();
    CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(commit_key).expect("commit key"),
        vec![run_admitted_with_fact_descriptor_for_node(
            run_id.clone(),
            fact_node_id,
        )],
        vec![
            spec_artifact_ref(),
            certificate_artifact_ref(),
            descriptor_artifact,
        ],
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            certified_run_authority: Some(
                CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                    .expect("certified run authority"),
            ),
            ..CommitPreconditions::default()
        },
    )
    .expect("fact run start request")
}

pub(super) fn fact_run_start_bundle_with_node(
    run_id: RunId,
    commit_key: &str,
    fact_node_id: NodeId,
) -> PreparedCommitBundle {
    let request = fact_run_start_request_with_node(run_id, commit_key, fact_node_id);
    let descriptor_artifact = fact_descriptor_artifact_ref();
    let artifacts = CommitArtifactEvidenceSet::new(
        request.required_artifacts().to_vec(),
        request.required_artifacts().to_vec(),
    )
    .expect("artifact evidence set");
    let plan = PreparedCommitPlan::from(
        PreparedCommit::<RunAdmission>::new(request, artifacts).expect("fact run admission"),
    );
    PreparedCommitBundle::new(
        plan,
        vec![
            PreparedArtifactBytes::new(fact_descriptor_bytes(), descriptor_artifact.clone())
                .expect("descriptor bytes"),
        ],
        vec![
            existing_artifact_admission(&spec_artifact_ref()),
            existing_artifact_admission(&certificate_artifact_ref()),
        ],
    )
    .expect("fact run start bundle")
}

pub(super) fn existing_artifact_admission(
    evidence: &ArtifactEvidenceRef,
) -> ExistingArtifactAdmission {
    ExistingArtifactAdmission::new(
        evidence.artifact_id.clone(),
        evidence.evidence_hash().expect("evidence hash"),
    )
}

pub(super) fn run_start_request_with_saga_policy(
    run_id: RunId,
    commit_key: &str,
    saga_policy: &SagaPolicySpec,
) -> CommitRequest {
    let authority_spec = saga_authority_spec(saga_policy.clone());
    CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(commit_key).expect("commit key"),
        vec![run_admitted_with_saga_policy(run_id.clone(), saga_policy)],
        vec![spec_artifact_ref(), certificate_artifact_ref()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            certified_run_authority: Some(
                CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                    .expect("certified run authority"),
            ),
            ..CommitPreconditions::default()
        },
    )
    .expect("run start request")
}

pub(super) fn ensure_test_run_admitted(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) {
    if store.expected_next_seq(run_id) == StreamSeq::FIRST {
        store
            .append_prepared_commit(run_start_request(run_id.clone(), commit_key))
            .expect("append fixture run admission");
    }
}

pub(super) fn admitted_store(run_id: &RunId, commit_key: &str) -> StoreContractRunStore {
    let mut store = StoreContractRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), commit_key))
        .expect("append run start");
    store
}

pub(super) fn admitted_fact_store(run_id: &RunId, commit_key: &str) -> StoreContractRunStore {
    admitted_fact_store_with_node(run_id, commit_key, node_id(90))
}

pub(super) fn admitted_fact_store_with_node(
    run_id: &RunId,
    commit_key: &str,
    fact_node_id: NodeId,
) -> StoreContractRunStore {
    let mut store = StoreContractRunStore::new();
    store
        .append_test_commit_bundle(fact_run_start_bundle_with_node(
            run_id.clone(),
            commit_key,
            fact_node_id,
        ))
        .expect("append fact run start");
    store
}

pub(super) fn admitted_store_with_saga_policy(
    run_id: &RunId,
    commit_key: &str,
    saga_policy: &SagaPolicySpec,
) -> StoreContractRunStore {
    let mut store = StoreContractRunStore::new();
    append_run_start_with_saga_policy(&mut store, run_id, commit_key, saga_policy);
    store
}

pub(super) fn append_run_start_with_saga_policy(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    saga_policy: &SagaPolicySpec,
) {
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            commit_key,
            saga_policy,
        ))
        .expect("append run start");
}

pub(super) fn append_side_effect_prepare(store: &mut StoreContractRunStore, run_id: &RunId) {
    ensure_test_run_admitted(store, run_id, "sidefx-fixture-run-start");
    let artifact_id = artifact_id(81);
    let artifact_digest = content_digest(82);
    let intent_evidence = intent_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    append_certified_side_effect_commit(
        store,
        run_id,
        "sidefx-attempt-start",
        vec![side_effect_attempt_started()],
        Vec::new(),
    )
    .expect("append sidefx attempt start");
    append_certified_side_effect_commit(
        store,
        run_id,
        "sidefx-prepare",
        vec![
            side_effect_intent(artifact_id, artifact_digest),
            side_effect_claim(),
            side_effect_prepared(1, "token-1"),
        ],
        vec![intent_evidence],
    )
    .expect("append sidefx prepare");
}

pub(super) fn append_side_effect_prepare_for_ledger(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    ledger_key: events::SideEffectLedgerKey,
    resource_key: events::ResourceKeyEvidence,
    artifact_byte: u8,
    start_attempt: bool,
) {
    append_side_effect_prepare_for_ledger_on_attempt(
        store,
        run_id,
        commit_key,
        ledger_key,
        resource_key,
        artifact_byte,
        start_attempt,
        node_id(70),
        attempt_id(72),
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_side_effect_prepare_for_ledger_on_attempt(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    ledger_key: events::SideEffectLedgerKey,
    resource_key: events::ResourceKeyEvidence,
    artifact_byte: u8,
    start_attempt: bool,
    node_id: NodeId,
    attempt_id: AttemptId,
) {
    ensure_test_run_admitted(store, run_id, "sidefx-ledger-fixture-run-start");
    if start_attempt {
        append_certified_side_effect_commit(
            store,
            run_id,
            &format!("{commit_key}-attempt-start"),
            vec![side_effect_attempt_started_for(
                node_id.clone(),
                attempt_id.clone(),
            )],
            Vec::new(),
        )
        .expect("append sidefx attempt start");
    }

    let fixture = side_effect_prepare_fixture_for_ledger(
        ledger_key,
        resource_key,
        artifact_byte,
        node_id,
        attempt_id,
    );
    append_certified_side_effect_commit(
        store,
        run_id,
        commit_key,
        fixture.payloads,
        fixture.required_artifacts,
    )
    .expect("append sidefx prepare");
}

pub(super) fn append_side_effect_started(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_certified_side_effect_commit(
        store,
        run_id,
        "sidefx-started",
        vec![side_effect_started("owner-1", 1, "token-1")],
        Vec::new(),
    )
    .expect("append sidefx started");
}

pub(super) fn append_side_effect_verify_attempt_started(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
) {
    append_certified_side_effect_commit(
        store,
        run_id,
        "sidefx-verify-attempt-start",
        vec![side_effect_verify_attempt_started()],
        Vec::new(),
    )
    .expect("append sidefx verify attempt start");
}

pub(super) fn append_side_effect_observation_setup(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
) {
    append_side_effect_prepare(store, run_id);
    append_side_effect_started(store, run_id);
    append_side_effect_verify_attempt_started(store, run_id);
}

pub(super) fn append_side_effect_failure_setup(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_side_effect_prepare(store, run_id);
    append_side_effect_verify_attempt_started(store, run_id);
}

pub(super) fn append_remediation_attempts_started(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) {
    for (suffix, payload) in [
        (
            "submit",
            side_effect_attempt_started_for(
                remediation_submit_node_id(),
                remediation_submit_attempt_id(),
            ),
        ),
        (
            "verify",
            side_effect_attempt_started_for(
                remediation_verify_node_id(),
                remediation_verify_attempt_id(),
            ),
        ),
    ] {
        append_certified_side_effect_commit(
            store,
            run_id,
            &format!("{commit_key}-{suffix}"),
            vec![payload],
            Vec::new(),
        )
        .expect("append remediation side-effect attempt start");
    }
}

pub(super) fn append_certified_side_effect_commit(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    mut payloads: Vec<KernelEventPayload>,
    required_artifacts: Vec<ArtifactEvidenceRef>,
) -> std::result::Result<CommitOutcome, StoreError> {
    store.certify_payloads_for_run(run_id, &mut payloads);
    store.append_prepared_commit(typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
        commit_key: CommitKey::new(commit_key).expect("commit key"),
        payloads: payloads,
        required_artifacts: required_artifacts,
        preconditions: store.certified_preconditions(run_id),
    })
}

pub(super) fn append_fact_recorded_commit(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> std::result::Result<CommitOutcome, StoreError> {
    append_fact_recorded_commit_for_height(store, run_id, commit_key, 850000)
}

pub(super) fn append_fact_recorded_commit_for_height(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    height: u64,
) -> std::result::Result<CommitOutcome, StoreError> {
    let response = fact_artifact_ref_for_height(height);
    let mut payloads = vec![fact_recorded(&response)];
    store.certify_payloads_for_run(run_id, &mut payloads);
    let mut preconditions = store.certified_preconditions(run_id);
    preconditions.required_run_state = RequiredRunState::Started;
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
        commit_key: CommitKey::new(commit_key).expect("commit key"),
        payloads: payloads,
        required_artifacts: vec![response.clone()],
        preconditions: preconditions,
    };
    let plan = test_prepared_commit_plan(request, vec![response.clone()])?;
    let bundle = PreparedCommitBundle::new(
        plan,
        vec![
            PreparedArtifactBytes::new(fact_response_bytes_for_height(height), response)
                .expect("fact response bytes"),
        ],
        Vec::new(),
    )?;
    store.append_test_commit_bundle(bundle)
}

pub(super) fn append_generic_nonretryable_failure(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    key_prefix: &str,
) {
    ensure_test_run_admitted(store, run_id, &format!("{key_prefix}-fixture-run-start"));
    store
        .append_prepared_commit(typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new(format!("{key_prefix}-attempt-start")).expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: store.certified_preconditions(run_id),
        })
        .expect("append generic attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new(format!("{key_prefix}-attempt-failed")).expect("commit key"),
            payloads: vec![fact_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: store.certified_preconditions(run_id),
        })
        .expect("append generic attempt failure");
}

pub(super) fn append_confirmed_side_effect_attempt_failures(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
) {
    let mut submit_failure = side_effect_attempt_failed(false);
    set_attempt_failure_node_attempt(&mut submit_failure, submit_node_id(), submit_attempt_id());
    append_certified_side_effect_commit(
        store,
        run_id,
        commit_key,
        vec![submit_failure, side_effect_attempt_failed(false)],
        Vec::new(),
    )
    .expect("append confirmed side-effect attempt failures");
}

pub(super) fn append_manual_blocked_forward_failure(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    failure_commit_key: &str,
) {
    append_forward_confirmation(store, run_id);
    append_confirmed_side_effect_attempt_failures(store, run_id, failure_commit_key);
}

pub(super) fn append_forward_confirmation(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_side_effect_observation_setup(store, run_id);
    let submission_artifact_id = artifact_id(84);
    let submission_digest = content_digest(85);
    let receipt_artifact_id = artifact_id(86);
    let receipt_digest = content_digest(87);
    let confirmation_artifact_id = artifact_id(88);
    let confirmation_digest = content_digest(89);
    append_certified_side_effect_commit(
        store,
        run_id,
        "forward-confirmation",
        vec![
            side_effect_submission_observed(
                submission_artifact_id.clone(),
                submission_digest.clone(),
            ),
            side_effect_receipt(receipt_artifact_id.clone(), receipt_digest.clone()),
            side_effect_confirmation(
                confirmation_artifact_id.clone(),
                confirmation_digest.clone(),
            ),
        ],
        vec![
            side_effect_evidence(
                submission_artifact_id,
                submission_digest,
                submission_schema(),
                ArtifactRole::Submission,
            ),
            side_effect_evidence(
                receipt_artifact_id,
                receipt_digest,
                receipt_schema(),
                ArtifactRole::Receipt,
            ),
            side_effect_evidence(
                confirmation_artifact_id,
                confirmation_digest,
                confirmation_schema(),
                ArtifactRole::Confirmation,
            ),
        ],
    )
    .expect("append forward confirmation");
}

pub(super) fn append_forward_receipt(store: &mut StoreContractRunStore, run_id: &RunId) {
    append_side_effect_observation_setup(store, run_id);
    let submission_artifact_id = artifact_id(84);
    let submission_digest = content_digest(85);
    let receipt_artifact_id = artifact_id(86);
    let receipt_digest = content_digest(87);
    append_certified_side_effect_commit(
        store,
        run_id,
        "forward-receipt",
        vec![
            side_effect_submission_observed(
                submission_artifact_id.clone(),
                submission_digest.clone(),
            ),
            side_effect_receipt(receipt_artifact_id.clone(), receipt_digest.clone()),
        ],
        vec![
            side_effect_evidence(
                submission_artifact_id,
                submission_digest,
                submission_schema(),
                ArtifactRole::Submission,
            ),
            side_effect_evidence(
                receipt_artifact_id,
                receipt_digest,
                receipt_schema(),
                ArtifactRole::Receipt,
            ),
        ],
    )
    .expect("append forward receipt");
}

pub(super) fn append_remediation_confirmation(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    ledger_key: events::SideEffectLedgerKey,
) {
    append_remediation_attempts_started(store, run_id, "remediation-attempts-started");
    let intent_artifact_id = artifact_id(101);
    let intent_digest = content_digest(102);
    let submission_artifact_id = artifact_id(103);
    let submission_digest = content_digest(104);
    let receipt_artifact_id = artifact_id(105);
    let receipt_digest = content_digest(106);
    let confirmation_artifact_id = artifact_id(107);
    let confirmation_digest = content_digest(108);

    let mut intent = side_effect_intent(intent_artifact_id.clone(), intent_digest.clone());
    let mut claim = side_effect_claim();
    let mut prepared = side_effect_prepared(1, "token-1");
    let mut started = side_effect_started("owner-1", 1, "token-1");
    let mut submission =
        side_effect_submission_observed(submission_artifact_id.clone(), submission_digest.clone());
    let mut receipt = side_effect_receipt(receipt_artifact_id.clone(), receipt_digest.clone());
    let mut confirmation = side_effect_confirmation(
        confirmation_artifact_id.clone(),
        confirmation_digest.clone(),
    );
    for payload in [
        &mut intent,
        &mut claim,
        &mut prepared,
        &mut started,
        &mut submission,
        &mut receipt,
        &mut confirmation,
    ] {
        set_remediation_purpose(payload, ledger_key.clone());
    }
    let payloads = vec![
        intent,
        claim,
        prepared,
        started,
        submission,
        receipt,
        confirmation,
    ];
    append_certified_side_effect_commit(
        store,
        run_id,
        "remediation-confirmation",
        payloads,
        vec![
            remediation_intent_artifact_ref(intent_artifact_id, intent_digest),
            remediation_side_effect_evidence(
                submission_artifact_id,
                submission_digest,
                submission_schema(),
                ArtifactRole::Submission,
            ),
            remediation_side_effect_evidence(
                receipt_artifact_id,
                receipt_digest,
                receipt_schema(),
                ArtifactRole::Receipt,
            ),
            remediation_side_effect_evidence(
                confirmation_artifact_id,
                confirmation_digest,
                confirmation_schema(),
                ArtifactRole::Confirmation,
            ),
        ],
    )
    .expect("append remediation confirmation");
}

pub(super) fn side_effect_evidence(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    role: ArtifactRole,
) -> ArtifactEvidenceRef {
    side_effect_artifact_ref(artifact_id, digest, schema_id, role)
}
