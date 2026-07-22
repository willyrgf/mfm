use super::*;

pub(super) fn run_start_request(run_id: RunId, key: &str) -> mfm_store::v1::CommitRequest {
    run_start_request_with_saga_policy(run_id, key, &SagaPolicySpec::NoSideEffects)
}

pub(super) fn run_start_request_with_saga_policy(
    run_id: RunId,
    key: &str,
    saga_policy: &SagaPolicySpec,
) -> mfm_store::v1::CommitRequest {
    let authority_spec = saga_authority_spec(saga_policy.clone());
    mfm_store::v1::CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(key).expect("commit key"),
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
    .expect("typed run start request")
}

pub(super) fn fact_run_start_request(run_id: RunId, key: &str) -> mfm_store::v1::CommitRequest {
    let authority_spec = fact_authority_spec();
    mfm_store::v1::CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(key).expect("commit key"),
        vec![run_admitted_with_fact_descriptor(run_id.clone())],
        vec![
            spec_artifact_ref(),
            certificate_artifact_ref(),
            fact_descriptor_artifact_ref(),
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
    .expect("typed fact run start request")
}

pub(super) async fn append_prepared(
    store: &PostgresStore,
    mut request: mfm_store::v1::CommitRequest,
    artifacts: Vec<ArtifactEvidenceRef>,
) -> Result<CommitOutcome> {
    if request.required_artifacts().is_empty() && !artifacts.is_empty() {
        request = request.with_required_artifacts(artifacts.clone());
    }
    let plan = test_prepared_commit_plan(request, artifacts)?;
    store
        .append_prepared_commit_bundle(test_prepared_commit_bundle(plan)?)
        .await
}

pub(super) fn retention_artifact_bundle(
    run_id: RunId,
    seq: u64,
    commit_key: &str,
    artifact: PreparedArtifactBytes,
    preconditions: CommitPreconditions,
) -> mfm_store::v1::Result<PreparedCommitBundle> {
    let evidence = artifact.evidence().clone();
    let request = mfm_store::v1::CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(commit_key).expect("commit key"),
        vec![retention_refs_appended_for_evidence(
            run_id,
            &evidence,
            events::RetentionReason::RuntimeEvidence,
        )],
        vec![evidence.clone()],
        preconditions,
    )?;
    let artifact_set =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), vec![evidence])?;
    let plan: PreparedCommitPlan = PreparedCommit::<Retention>::new(request, artifact_set)?.into();
    PreparedCommitBundle::new(plan, vec![artifact], Vec::new())
}

pub(super) fn test_prepared_commit_bundle(
    plan: PreparedCommitPlan,
) -> mfm_store::v1::Result<PreparedCommitBundle> {
    let artifact_bytes = plan
        .admitted_artifacts()
        .iter()
        .map(test_prepared_artifact_bytes)
        .collect::<mfm_store::v1::Result<Vec<_>>>()?;
    PreparedCommitBundle::new(plan, artifact_bytes, Vec::new())
}

pub(super) fn test_prepared_commit_bundle_with_artifact_bytes(
    plan: PreparedCommitPlan,
    exact_artifacts: Vec<PreparedArtifactBytes>,
) -> mfm_store::v1::Result<PreparedCommitBundle> {
    let mut artifact_bytes = Vec::new();
    for evidence in plan.admitted_artifacts() {
        if let Some(exact) = exact_artifacts
            .iter()
            .find(|artifact| artifact.evidence() == evidence)
        {
            artifact_bytes.push(exact.clone());
        } else {
            artifact_bytes.push(test_prepared_artifact_bytes(evidence)?);
        }
    }
    PreparedCommitBundle::new(plan, artifact_bytes, Vec::new())
}

pub(super) fn test_prepared_commit_bundle_with_existing_artifacts(
    plan: PreparedCommitPlan,
    evidence: &[ArtifactEvidenceRef],
) -> mfm_store::v1::Result<PreparedCommitBundle> {
    PreparedCommitBundle::new(
        plan,
        Vec::new(),
        evidence
            .iter()
            .map(|evidence| {
                Ok(ExistingArtifactAdmission::new(
                    evidence.artifact_id.clone(),
                    evidence.evidence_hash()?,
                ))
            })
            .collect::<mfm_store::v1::Result<Vec<_>>>()?,
    )
}

pub(super) async fn append_fact_run_start(
    store: &PostgresStore,
    run_id: RunId,
) -> Result<CommitOutcome> {
    let descriptor_ref = fact_descriptor_artifact_ref();
    let descriptor_bytes =
        PreparedArtifactBytes::new(fact_descriptor_bytes(), descriptor_ref.clone())?;
    let request = fact_run_start_request(run_id, "run-start");
    let plan = test_prepared_commit_plan(
        request,
        vec![
            spec_artifact_ref(),
            certificate_artifact_ref(),
            descriptor_ref,
        ],
    )?;
    store
        .append_prepared_commit_bundle(test_prepared_commit_bundle_with_artifact_bytes(
            plan,
            vec![descriptor_bytes],
        )?)
        .await
}

pub(super) async fn append_fact_attempt_start(
    store: &PostgresStore,
    run: RunId,
    commit_key: &str,
) -> Result<CommitOutcome> {
    append_prepared(
        store,
        certified_fact_request(run, 2, commit_key, vec![fact_attempt_started()]),
        Vec::new(),
    )
    .await
}

pub(super) fn fact_commit_request(
    run_id: RunId,
    seq: u64,
    key: &str,
    response: &ArtifactEvidenceRef,
) -> mfm_store::v1::CommitRequest {
    let output = fact_output_artifact_ref(response);
    let request = certified_fact_request(
        run_id,
        seq,
        key,
        vec![
            fact_recorded(response),
            fact_cell_produced(response),
            fact_attempt_completed(),
        ],
    );
    let mut preconditions = request.preconditions().clone();
    preconditions.required_run_state = RequiredRunState::Started;
    request
        .with_preconditions(preconditions)
        .with_required_artifacts(vec![response.clone(), output])
}

pub(super) async fn append_fact_commit(
    store: &PostgresStore,
    request: mfm_store::v1::CommitRequest,
    response: &ArtifactEvidenceRef,
) -> Result<CommitOutcome> {
    append_fact_commit_with_response_bytes(store, request, response, fact_response_bytes()).await
}

pub(super) async fn append_fact_commit_with_response_bytes(
    store: &PostgresStore,
    request: mfm_store::v1::CommitRequest,
    response: &ArtifactEvidenceRef,
    response_bytes: Vec<u8>,
) -> Result<CommitOutcome> {
    let response_bytes = PreparedArtifactBytes::new(response_bytes, response.clone())?;
    let output = fact_output_artifact_ref(response);
    let output_bytes = PreparedArtifactBytes::new(response_bytes.bytes().to_vec(), output.clone())?;
    let plan = test_prepared_commit_plan(request, vec![response.clone(), output])?;
    store
        .append_prepared_commit_bundle(test_prepared_commit_bundle_with_artifact_bytes(
            plan,
            vec![response_bytes, output_bytes],
        )?)
        .await
}

pub(super) async fn assert_empty_fact_query(
    store: &PostgresStore,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) {
    let query_result = store
        .execute_fact_query(plan)
        .await
        .expect("empty fact query execution");
    assert!(query_result.rows().is_empty());
    assert_fact_query_receipt(
        store,
        plan,
        &query_result,
        mfm_facts::QueryResultCardinality::Exact(0),
    );
}

pub(super) async fn assert_single_public_fact_query(
    store: &PostgresStore,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> mfm_facts::FactQueryResult {
    let query_result = store
        .execute_fact_query(plan)
        .await
        .expect("fact query execution");
    assert_eq!(query_result.rows().len(), 1);
    let row = &query_result.rows()[0];
    assert_eq!(row.fact_ref().fact_key(), &fact_key());
    assert_eq!(row.returned_fields().len(), 2);
    assert_eq!(
        row.returned_fields()[0].field_id().as_str(),
        "subject.chain"
    );
    assert_eq!(
        row.returned_fields()[0].value(),
        &mfm_facts::FactCanonicalScalar::string("postgres_test_chain")
    );
    assert_eq!(
        row.returned_fields()[1].field_id().as_str(),
        "result.height"
    );
    assert_eq!(
        row.returned_fields()[1].value(),
        &mfm_facts::FactCanonicalScalar::UnsignedInteger(12_345)
    );
    query_result
}

pub(super) async fn append_fact_commit_with_missing_existing_artifact(
    store: &PostgresStore,
    request: mfm_store::v1::CommitRequest,
    response: &ArtifactEvidenceRef,
) -> Result<CommitOutcome> {
    let output = fact_output_artifact_ref(response);
    let plan = test_prepared_commit_plan(request, vec![response.clone(), output.clone()])?;
    store
        .append_prepared_commit_bundle(test_prepared_commit_bundle_with_existing_artifacts(
            plan,
            &[response.clone(), output],
        )?)
        .await
}

pub(super) async fn append_run_start(
    store: &PostgresStore,
    run_id: &RunId,
    commit_key: &str,
) -> Result<CommitOutcome> {
    append_prepared(
        store,
        run_start_request(run_id.clone(), commit_key),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
}
