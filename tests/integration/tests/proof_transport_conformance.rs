use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, DigestBytes, EventId, RunId};
use mfm_op_proof::{
    certified_proof_spec, proof_program_draft, ProofApplyConfig, ProofConfirmation, ProofFact,
    ProofFactResponse, ProofIdempotencyInput, ProofIntent, ProofReadConfig, ProofReceipt,
    ProofReplayVerifier, ProofSubmission, ProofWorkflowConfig, RecordedProofFacts,
};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    CertifiedRuntimeSpec, RunLaunchArtifact, RunLaunchEvidence, RuntimeArtifactStageFuture,
    RuntimeArtifactStager, RuntimeArtifactStore, SchedulerStatus, SerialTypedScheduler,
};
use mfm_store::v1::{self as store, TypedRunEventStore};
use serde::Serialize;

type ProofArtifactRecord = (Vec<u8>, store::ArtifactEvidenceRef);
type ProofArtifactMap = BTreeMap<ArtifactId, ProofArtifactRecord>;

fn typed_commit_request(
    run_id: RunId,
    expected_next_seq: store::StreamSeq,
    commit_key: store::CommitKey,
    payloads: Vec<events::KernelEventPayload>,
    required_artifacts: Vec<store::ArtifactEvidenceRef>,
    preconditions: store::CommitPreconditions,
) -> store::TypedCommitRequest {
    store::TypedCommitRequest::from_payloads(
        run_id,
        expected_next_seq,
        commit_key,
        payloads,
        required_artifacts,
        preconditions,
    )
    .expect("typed commit request")
}

#[derive(Clone, Default)]
struct InMemoryProofArtifacts {
    artifacts: Arc<Mutex<ProofArtifactMap>>,
}

impl InMemoryProofArtifacts {
    async fn store_verified_artifact(
        &self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> mfm_runtime::Result<()> {
        let digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
        if evidence.digest != digest
            || evidence.byte_len != bytes.len() as u64
            || evidence.artifact_id != ArtifactId::from_digest(digest.algorithm(), *digest.digest())
        {
            return Err(mfm_runtime::RuntimeError::Store(
                "proof artifact bytes do not match typed evidence".to_owned(),
            ));
        }
        let mut artifacts = self.artifacts.lock().map_err(|_| {
            mfm_runtime::RuntimeError::Store("proof artifact store lock was poisoned".to_owned())
        })?;
        if let Some((existing_bytes, existing_evidence)) = artifacts.get(&evidence.artifact_id) {
            if existing_bytes != &bytes || existing_evidence != &evidence {
                return Err(mfm_runtime::RuntimeError::Store(format!(
                    "conflicting proof artifact evidence for {}",
                    evidence.artifact_id
                )));
            }
            return Ok(());
        }
        artifacts.insert(evidence.artifact_id.clone(), (bytes, evidence));
        Ok(())
    }
}

impl RuntimeArtifactStager for InMemoryProofArtifacts {
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a> {
        Box::pin(async move { self.store_verified_artifact(bytes, evidence).await })
    }
}

impl store::RetainedArtifactReadProvider for InMemoryProofArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let (bytes, evidence) = self
                .artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .get(&requirement.artifact_id)
                .cloned()
                .ok_or_else(|| store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                })?;
            store::VerifiedRunArtifactBytes::new(bytes, evidence, requirement)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProofImplementationConformanceSummary {
    implementation: String,
    facts_valid: bool,
    submission_valid: bool,
    receipt_valid: bool,
    confirmation_valid: bool,
    replay_valid: bool,
}

impl ProofImplementationConformanceSummary {
    fn to_summary_document(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": "proof-implementation-conformance",
            "version": 1,
            "implementation": self.implementation,
            "payload": {
                "facts_valid": self.facts_valid,
                "submission_valid": self.submission_valid,
                "receipt_valid": self.receipt_valid,
                "confirmation_valid": self.confirmation_valid,
                "replay_valid": self.replay_valid,
            }
        })
    }

    fn validate(&self) -> Result<(), String> {
        for (key, value) in [
            ("facts_valid", self.facts_valid),
            ("submission_valid", self.submission_valid),
            ("receipt_valid", self.receipt_valid),
            ("confirmation_valid", self.confirmation_valid),
            ("replay_valid", self.replay_valid),
        ] {
            if !value {
                return Err(format!(
                    "proof implementation conformance key is false: {key}"
                ));
            }
        }
        Ok(())
    }
}

#[tokio::test]
async fn deterministic_proof_transport_conforms_from_test_support_fixture() {
    let summary = proof_implementation_conformance_summary()
        .await
        .expect("proof conformance");
    assert_eq!(summary.implementation, "deterministic-proof");
    assert_eq!(
        summary.to_summary_document()["kind"],
        "proof-implementation-conformance"
    );
}

#[tokio::test]
async fn conformance_start_rejects_draft_not_bound_to_certified_spec() {
    let certified =
        certified_proof_spec(ProofWorkflowConfig::default()).expect("certified proof spec");
    let runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
    let mismatched_draft = proof_program_draft(ProofWorkflowConfig::new(
        ProofReadConfig { fact_n: 2 },
        ProofApplyConfig::new("reject").expect("non-empty proof action"),
    ))
    .expect("mismatched proof draft");
    let artifacts = InMemoryProofArtifacts::default();
    let artifact_stager: Arc<dyn RuntimeArtifactStore> = Arc::new(artifacts);
    let scheduler = SerialTypedScheduler::new(
        mfm_transports_proof::deterministic_proof_runner_registry().expect("runner registry"),
        artifact_stager,
    );
    let mut store = store::InMemoryTypedRunStore::new();
    let run_id = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x35; 32]),
    );

    let error = start_proof_run(
        &scheduler,
        &mut store,
        &runtime_spec,
        &mismatched_draft,
        run_id.clone(),
    )
    .await
    .expect_err("mismatched draft must not authorize conformance start");

    assert!(
        matches!(error, mfm_runtime::RuntimeError::InvalidSpec(_)),
        "expected InvalidSpec, got {error:?}"
    );
    assert!(
        store.load_run_stream(&run_id).is_empty(),
        "rejected conformance launch must not create a run stream"
    );
}

#[tokio::test]
async fn conformance_replay_rejects_completed_history_without_retention_projection() {
    let (runtime_spec, stream, artifacts) = conformance_stream().await;
    let corrupt = remove_retention_projection_commit(&stream);

    let error = verify_conformance_replay_stream(&runtime_spec, corrupt, &artifacts)
        .await
        .expect_err("completed history without retention projection must reject");

    assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
}

#[tokio::test]
async fn conformance_replay_rejects_standalone_retention_projection_history() {
    let (runtime_spec, stream, artifacts) = conformance_stream().await;
    let corrupt = standalone_retention_projection_history(&stream);

    let error = verify_conformance_replay_stream(&runtime_spec, corrupt, &artifacts)
        .await
        .expect_err("standalone retention projection history must reject");

    assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
}

#[tokio::test]
async fn conformance_replay_rejects_failed_completion_without_framework_authority() {
    let (runtime_spec, stream, artifacts) = conformance_stream().await;
    let corrupt = failed_completion_after_run_start(&stream);

    let error = verify_conformance_replay_stream(&runtime_spec, corrupt, &artifacts)
        .await
        .expect_err("failed completion without framework authority must reject");

    assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
}

#[tokio::test]
async fn conformance_replay_rejects_post_completion_retention_refs() {
    let (runtime_spec, stream, artifacts) = conformance_stream().await;
    let corrupt = append_post_completion_retention_refs(&stream);

    let error = verify_conformance_replay_stream(&runtime_spec, corrupt, &artifacts)
        .await
        .expect_err("post-completion retention refs must reject");

    assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
}

#[tokio::test]
async fn conformance_replay_rejects_post_projection_retention_refs_before_completion() {
    let (runtime_spec, stream, artifacts) = conformance_stream().await;
    let corrupt = append_post_projection_retention_refs_before_completion(&stream);

    let error = verify_conformance_replay_stream(&runtime_spec, corrupt, &artifacts)
        .await
        .expect_err("post-projection retention refs before completion must reject");

    assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
}

#[tokio::test]
async fn conformance_replay_rejects_extra_payload_in_retention_projection_commit() {
    let (runtime_spec, stream, artifacts) = conformance_stream().await;
    let corrupt = append_extra_retention_ref_to_projection_commit(&stream);

    let error = verify_conformance_replay_stream(&runtime_spec, corrupt, &artifacts)
        .await
        .expect_err("extra retention projection commit payload must reject");

    assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
}

#[tokio::test]
async fn conformance_replay_rejects_same_sequence_sidecar_retention_projection_payload() {
    let (runtime_spec, stream, artifacts) = conformance_stream().await;
    let corrupt = append_same_sequence_sidecar_to_retention_projection(&stream);

    let error = verify_conformance_replay_stream(&runtime_spec, corrupt, &artifacts)
        .await
        .expect_err("same-sequence sidecar retention payload must reject");

    assert_eq!(error.kind, replay::ReplayErrorKind::InvalidRunStream);
}

async fn proof_implementation_conformance_summary(
) -> Result<ProofImplementationConformanceSummary, String> {
    let config = ProofWorkflowConfig::default();
    let draft = proof_program_draft(config.clone()).map_err(|error| error.to_string())?;
    let certified = certified_proof_spec(config).map_err(|error| error.to_string())?;
    let runtime_spec =
        CertifiedRuntimeSpec::new(certified.clone()).map_err(|error| error.to_string())?;
    let artifacts = InMemoryProofArtifacts::default();
    let mut store = store::InMemoryTypedRunStore::new();
    let artifact_stager: Arc<dyn RuntimeArtifactStore> = Arc::new(artifacts.clone());
    let scheduler = SerialTypedScheduler::new(
        mfm_transports_proof::deterministic_proof_runner_registry()
            .map_err(|error| error.to_string())?,
        artifact_stager,
    );
    let run_id = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x34; 32]),
    );
    start_proof_run(
        &scheduler,
        &mut store,
        &runtime_spec,
        &draft,
        run_id.clone(),
    )
    .await
    .map_err(|error| error.to_string())?;

    for _ in 0..16 {
        match scheduler
            .drive_until_blocked(&mut store, &runtime_spec, &run_id)
            .await
            .map_err(|error| error.to_string())?
        {
            SchedulerStatus::PublicOutputProjected => break,
            SchedulerStatus::Blocked => break,
            SchedulerStatus::Advanced => continue,
        }
    }

    let stream = store.load_run_stream(&run_id);
    let verifier = mfm_transports_proof::DeterministicProofReplayVerifier::new()
        .map_err(|error| error.to_string())?;
    let facts = RecordedProofFacts { fact: proof_fact() };
    let mut facts_valid = false;
    let mut submission_valid = false;
    let mut receipt_valid = false;
    let mut confirmation_valid = false;
    for event in &stream {
        match event.payload() {
            events::KernelEventPayload::FactRecorded(payload) => {
                facts_valid = payload.response_hash
                    == digest_value(&ProofFactResponse { fact: proof_fact() })
                        .map_err(|error| error.to_string())?;
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                let submission = proof_submission().map_err(|error| error.to_string())?;
                submission_valid = payload.submission_hash
                    == digest_value(&submission).map_err(|error| error.to_string())?
                    && verifier
                        .verify_submission(&proof_intent(), &submission, &facts)
                        .is_ok();
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                let receipt = proof_receipt().map_err(|error| error.to_string())?;
                receipt_valid = payload.receipt_hash
                    == digest_value(&receipt).map_err(|error| error.to_string())?
                    && payload.replay_verifier_id
                        == replay_verifier_id().map_err(|error| error.to_string())?
                    && verifier
                        .verify_receipt(&proof_intent(), &receipt, &facts)
                        .is_ok();
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                let receipt = proof_receipt().map_err(|error| error.to_string())?;
                let confirmation = proof_confirmation().map_err(|error| error.to_string())?;
                confirmation_valid = payload.confirmation_hash
                    == digest_value(&confirmation).map_err(|error| error.to_string())?
                    && payload.replay_verifier_id
                        == replay_verifier_id().map_err(|error| error.to_string())?
                    && verifier
                        .verify_confirmation(&receipt, &confirmation, &facts)
                        .is_ok();
            }
            _ => {}
        }
    }
    let replay_valid = verify_conformance_replay(&runtime_spec, &run_id, &store, &artifacts)
        .await
        .map_err(|error| error.to_string())?;
    let summary = ProofImplementationConformanceSummary {
        implementation: "deterministic-proof".to_owned(),
        facts_valid,
        submission_valid,
        receipt_valid,
        confirmation_valid,
        replay_valid,
    };
    summary.validate()?;
    Ok(summary)
}

async fn verify_conformance_replay(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    store: &impl store::TypedRunEventStore,
    artifacts: &InMemoryProofArtifacts,
) -> replay::Result<bool> {
    let committed =
        store::CommittedRunStream::from_events(run_id.clone(), store.load_run_stream(run_id))
            .map_err(|error| {
                replay::ReplayError::new(
                    replay::ReplayErrorKind::InvalidRunStream,
                    error.to_string(),
                )
            })?;
    let retained_artifacts =
        store::VerifiedRunArtifactStore::from_committed_stream(&committed, artifacts)
            .await
            .map_err(|error| {
                replay::ReplayError::new(
                    replay::ReplayErrorKind::ArtifactMissing,
                    error.to_string(),
                )
            })?;
    let verified_history = mfm_runtime::VerifiedRunHistory::from_committed_stream(
        runtime_spec,
        committed,
        retained_artifacts,
    )
    .map_err(|error| {
        replay::ReplayError::new(replay::ReplayErrorKind::InvalidRunStream, error.to_string())
    })?;
    let run_started = verified_history
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .ok_or_else(|| {
            replay::ReplayError::new(
                replay::ReplayErrorKind::RunStartedMissing,
                "proof conformance stream has no run-start event",
            )
        })?;
    let projection = verified_history.projection_snapshot();
    let _retention = projection.retention(&run_started.run_id).ok_or_else(|| {
        replay::ReplayError::new(
            replay::ReplayErrorKind::ArtifactMissing,
            "proof conformance stream has no retained artifact evidence",
        )
    })?;
    let authority =
        replay::ReplayReadAuthority::from_verified_run_history(runtime_spec, &verified_history)?;
    let broker = replay::ReplayBroker::from_read_authority(authority)?;
    let proof_verified = mfm_transports_proof::verify_deterministic_proof_replay(&broker)?;
    Ok(proof_verified
        && broker.projection_snapshot().run_state(&run_started.run_id)
            == store::RunState::Completed)
}

fn conformance_config_launch_artifacts(
    draft: &mfm_program::TypedProgramDraft,
    typed_spec: &mfm_spec::v1::TypedExecutionSpec,
) -> mfm_runtime::Result<Vec<RunLaunchArtifact>> {
    let mut by_artifact = BTreeMap::new();
    for config in draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
    {
        let artifact_id = ArtifactId::from_digest(
            config.content_digest.algorithm(),
            *config.content_digest.digest(),
        );
        insert_conformance_config_launch_artifact(
            &mut by_artifact,
            RunLaunchArtifact {
                bytes: config.canonical_json.as_bytes().to_vec(),
                evidence: store::ArtifactEvidenceRef {
                    artifact_id,
                    digest: config.content_digest.clone(),
                    byte_len: config.byte_len as u64,
                    media_type: mfm_spec::v1::MediaType::new("application/json")?,
                    schema_id: Some(config.schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::TypedConfig,
                },
            },
        )?;
    }

    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes =
            mfm_spec::v1::framework_config_canonical_json(framework.config_kind(), &node.node_id)
                .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
        insert_conformance_config_launch_artifact(
            &mut by_artifact,
            RunLaunchArtifact {
                bytes: bytes.to_vec(),
                evidence: store::ArtifactEvidenceRef {
                    artifact_id: node.config_ref.artifact_id.clone(),
                    digest: node.config_ref.digest.clone(),
                    byte_len: node.config_ref.byte_len,
                    media_type: node.config_ref.media_type.clone(),
                    schema_id: Some(node.config_ref.schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::TypedConfig,
                },
            },
        )?;
    }

    let mut artifacts = Vec::with_capacity(typed_spec.config_refs.len());
    for config in &typed_spec.config_refs {
        let artifact = by_artifact.remove(&config.artifact_id).ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidSpec(format!(
                "missing conformance config bytes for {}",
                config.artifact_id
            ))
        })?;
        artifacts.push(artifact);
    }
    if !by_artifact.is_empty() {
        return Err(mfm_runtime::RuntimeError::InvalidSpec(
            "conformance config inputs contain entries not certified by the spec".to_owned(),
        ));
    }
    Ok(artifacts)
}

fn insert_conformance_config_launch_artifact(
    by_artifact: &mut BTreeMap<ArtifactId, RunLaunchArtifact>,
    artifact: RunLaunchArtifact,
) -> mfm_runtime::Result<()> {
    if let Some(existing) = by_artifact.get(&artifact.evidence.artifact_id) {
        if existing.bytes != artifact.bytes || existing.evidence != artifact.evidence {
            return Err(mfm_runtime::RuntimeError::InvalidSpec(format!(
                "conflicting conformance config bytes for {}",
                artifact.evidence.artifact_id
            )));
        }
        return Ok(());
    }
    by_artifact.insert(artifact.evidence.artifact_id.clone(), artifact);
    Ok(())
}

fn conformance_spec_launch_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
) -> mfm_runtime::Result<RunLaunchArtifact> {
    let spec_bytes = runtime_spec.spec().canonical_json()?;
    let spec_digest = spec_bytes.content_digest();
    Ok(RunLaunchArtifact {
        bytes: spec_bytes.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(spec_digest.algorithm(), *spec_digest.digest()),
            digest: spec_digest,
            byte_len: spec_bytes.as_bytes().len() as u64,
            media_type: runtime_spec.spec().media_type.clone(),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedExecutionSpec,
        },
    })
}

fn conformance_certificate_launch_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
) -> mfm_runtime::Result<RunLaunchArtifact> {
    let certificate_bytes = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    let certificate_digest = certificate_bytes.content_digest();
    Ok(RunLaunchArtifact {
        bytes: certificate_bytes.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(
                certificate_digest.algorithm(),
                *certificate_digest.digest(),
            ),
            digest: certificate_digest,
            byte_len: certificate_bytes.as_bytes().len() as u64,
            media_type: mfm_spec::v1::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)?,
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedSpecCertificate,
        },
    })
}

async fn start_proof_run(
    scheduler: &SerialTypedScheduler,
    store: &mut store::InMemoryTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    draft: &mfm_program::TypedProgramDraft,
    run_id: RunId,
) -> mfm_runtime::Result<()> {
    let launch = scheduler.prepare_run_launch(
        runtime_spec,
        run_id.clone(),
        run_start_evidence(runtime_spec, draft)?,
        store.expected_next_seq(&run_id),
    )?;
    scheduler.start_run(store, launch).await?;
    Ok(())
}

fn run_start_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
    draft: &mfm_program::TypedProgramDraft,
) -> mfm_runtime::Result<RunLaunchEvidence> {
    Ok(RunLaunchEvidence {
        spec_artifact: conformance_spec_launch_artifact(runtime_spec)?,
        certificate_artifact: conformance_certificate_launch_artifact(runtime_spec)?,
        config_artifacts: conformance_config_launch_artifacts(draft, runtime_spec.spec())?,
        framework_version: events::FrameworkVersion::new("mfm.proof.typed.v1")?,
        source_revision: events::SourceRevision::new("mfm-integration-tests-proof")?,
        adapter_executables: vec![executable(events::RunnerFactoryId::new(
            "deterministic-proof-adapter",
        )?)?],
        seed_cells: Vec::new(),
    })
}

fn executable(
    factory_id: events::RunnerFactoryId,
) -> mfm_runtime::Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-transports-proof-test-support")?,
        cargo_package_name: events::PackageName::new("mfm-integration-tests")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: digest_json(serde_json::json!({
            "crate": "mfm-integration-tests",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        binary_digest: digest_json(serde_json::json!({
            "crate": "mfm-transports-proof",
            "runner": "deterministic-proof",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

fn proof_fact() -> ProofFact {
    ProofFact { n: 1 }
}

fn proof_intent() -> ProofIntent {
    ProofIntent {
        fact_n: 1,
        action: "accept".to_owned(),
    }
}

fn proof_idempotency_input() -> ProofIdempotencyInput {
    ProofIdempotencyInput {
        action: "accept".to_owned(),
        fact_n: 1,
    }
}

fn proof_submission() -> mfm_runtime::Result<ProofSubmission> {
    let idempotency_digest = digest_value(&proof_idempotency_input())?
        .as_str()
        .to_owned();
    Ok(ProofSubmission {
        submission_id: "proof-submission-accept-1".to_owned(),
        idempotency_digest,
    })
}

fn proof_receipt() -> mfm_runtime::Result<ProofReceipt> {
    Ok(ProofReceipt {
        tx_hash: "0xproofaccept1".to_owned(),
        submission_id: proof_submission()?.submission_id,
    })
}

fn proof_confirmation() -> mfm_runtime::Result<ProofConfirmation> {
    Ok(ProofConfirmation {
        tx_hash: proof_receipt()?.tx_hash,
        confirmations: 1,
    })
}

fn replay_verifier_id() -> mfm_runtime::Result<events::ReplayVerifierId> {
    events::ReplayVerifierId::new("mfm.proof.replay.deterministic.v1")
        .map_err(|error| mfm_runtime::RuntimeError::InvalidSpec(error.to_string()))
}

fn digest_json(value: serde_json::Value) -> mfm_runtime::Result<ContentDigest> {
    canonical_value(&value).map(|canonical| canonical.content_digest())
}

fn digest_value<T: Serialize>(value: &T) -> mfm_runtime::Result<ContentDigest> {
    canonical_value(value).map(|canonical| canonical.content_digest())
}

fn canonical_value<T: Serialize>(value: &T) -> mfm_runtime::Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))
}

async fn verify_conformance_replay_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: Vec<store::KernelEventEnvelope>,
    artifacts: &InMemoryProofArtifacts,
) -> replay::Result<bool> {
    let run_id = stream
        .first()
        .map(|event| event.run_id().clone())
        .ok_or_else(|| {
            replay::ReplayError::new(
                replay::ReplayErrorKind::RunStartedMissing,
                "proof conformance stream is empty",
            )
        })?;
    let store = StaticRunStore {
        stream,
        projection: store::ProjectionSnapshot::default(),
    };
    verify_conformance_replay(runtime_spec, &run_id, &store, artifacts).await
}

struct StaticRunStore {
    stream: Vec<store::KernelEventEnvelope>,
    projection: store::ProjectionSnapshot,
}

impl store::TypedProjectionRead for StaticRunStore {
    fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        &self.projection
    }
}

impl store::TypedRunEventStore for StaticRunStore {
    fn append_prepared_typed_commit(
        &mut self,
        _commit: store::PreparedTypedCommit,
    ) -> store::Result<store::CommitOutcome> {
        Err(store::StoreError::Event(
            "static replay test store is read-only".to_owned(),
        ))
    }

    fn load_run_stream(&self, _run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        self.stream.clone()
    }

    fn expected_next_seq(&self, _run_id: &RunId) -> store::StreamSeq {
        store::StreamSeq::FIRST
    }
}

async fn conformance_stream() -> (
    CertifiedRuntimeSpec,
    Vec<store::KernelEventEnvelope>,
    InMemoryProofArtifacts,
) {
    let config = ProofWorkflowConfig::default();
    let draft = proof_program_draft(config.clone()).expect("proof draft");
    let certified = certified_proof_spec(config).expect("certified proof spec");
    let runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
    let artifacts = InMemoryProofArtifacts::default();

    let mut store = store::InMemoryTypedRunStore::new();
    let artifact_stager: Arc<dyn RuntimeArtifactStore> = Arc::new(artifacts.clone());
    let scheduler = SerialTypedScheduler::new(
        mfm_transports_proof::deterministic_proof_runner_registry().expect("runner registry"),
        artifact_stager,
    );
    let run_id = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x34; 32]),
    );
    start_proof_run(
        &scheduler,
        &mut store,
        &runtime_spec,
        &draft,
        run_id.clone(),
    )
    .await
    .expect("start run");

    for _ in 0..16 {
        match scheduler
            .drive_until_blocked(&mut store, &runtime_spec, &run_id)
            .await
            .expect("drive conformance run")
        {
            SchedulerStatus::PublicOutputProjected | SchedulerStatus::Blocked => break,
            SchedulerStatus::Advanced => continue,
        }
    }

    let stream = store.load_run_stream(&run_id);
    (runtime_spec, stream, artifacts)
}

fn standalone_retention_projection_history(
    valid_stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let run_id = valid_stream
        .first()
        .expect("valid stream is non-empty")
        .run_id()
        .clone();
    let retention_seq = valid_stream
        .iter()
        .find_map(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
            .then_some(event.seq())
        })
        .expect("retention projection event");
    let mut rewritten = Vec::new();
    let mut index = 0;
    while index < valid_stream.len() {
        let seq = valid_stream[index].seq();
        let commit_key = valid_stream[index].commit_key().clone();
        let mut end = index + 1;
        while end < valid_stream.len()
            && valid_stream[end].seq() == seq
            && valid_stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let payloads = valid_stream[index..end]
            .iter()
            .filter(|event| {
                if event.seq() != retention_seq {
                    return true;
                }
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                        | events::KernelEventPayload::RetentionRefsAppended(
                            events::RetentionRefsAppended {
                                reason: events::RetentionReason::ManifestProjection,
                                ..
                            }
                        )
                )
            })
            .map(|event| event.payload().clone())
            .collect::<Vec<_>>();
        if !payloads.is_empty() {
            let request = typed_commit_request(
                run_id.clone(),
                seq,
                commit_key,
                payloads,
                Vec::new(),
                store::CommitPreconditions::default(),
            );
            let batch =
                store::build_committed_batch(&request, seq).expect("rewritten commit batch");
            rewritten.extend(batch.events().iter().cloned());
        }
        index = end;
    }
    rewritten
}

fn remove_retention_projection_commit(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let mut rewritten = Vec::with_capacity(stream.len());
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let original_seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == original_seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        if commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        }) {
            index = end;
            continue;
        }
        let seq = rewritten
            .last()
            .map(|event: &store::KernelEventEnvelope| {
                store::StreamSeq::new(event.seq().as_u64() + 1).expect("next stream seq")
            })
            .unwrap_or(store::StreamSeq::FIRST);
        let request = typed_commit_request(
            first.run_id().clone(),
            seq,
            commit_key,
            commit
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>(),
            Vec::new(),
            store::CommitPreconditions::default(),
        );
        let batch = store::build_committed_batch(&request, seq).expect("rewritten commit batch");
        rewritten.extend(batch.events().iter().cloned());
        index = end;
    }
    rewritten
}

fn failed_completion_after_run_start(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let first = stream.first().expect("stream is non-empty");
    let start_seq = first.seq();
    let start_key = first.commit_key().clone();
    let mut rewritten = stream
        .iter()
        .take_while(|event| event.seq() == start_seq && event.commit_key() == &start_key)
        .cloned()
        .collect::<Vec<_>>();
    let run_started = rewritten
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .expect("run started");
    let seq = store::StreamSeq::new(start_seq.as_u64() + 1).expect("next stream seq");
    let request = typed_commit_request(
        run_started.run_id.clone(),
        seq,
        store::CommitKey::new("failed-completion-without-framework").expect("commit key"),
        vec![events::KernelEventPayload::RunCompleted(
            events::RunCompleted {
                run_id: run_started.run_id.clone(),
                spec_hash: run_started.spec_hash.clone(),
                outcome: events::RunCompletionOutcome::FailedWithoutAcdcClaim,
            },
        )],
        Vec::new(),
        store::CommitPreconditions::default(),
    );
    let batch = store::build_committed_batch(&request, seq).expect("failed completion batch");
    rewritten.extend(batch.events().iter().cloned());
    rewritten
}

fn append_post_completion_retention_refs(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let run_started = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .expect("run started");
    let retention = store::ProjectionSnapshot::rebuild_from_run_stream(stream)
        .expect("valid projection")
        .retention(&run_started.run_id)
        .and_then(|projection| projection.refs.values().next().cloned())
        .expect("retention ref");
    let seq = stream
        .last()
        .map(|event| store::StreamSeq::new(event.seq().as_u64() + 1).expect("next stream seq"))
        .unwrap_or(store::StreamSeq::FIRST);
    let request = typed_commit_request(
        run_started.run_id.clone(),
        seq,
        store::CommitKey::new("post-completion-retention-ref").expect("commit key"),
        vec![events::KernelEventPayload::RetentionRefsAppended(
            events::RetentionRefsAppended {
                run_id: run_started.run_id.clone(),
                spec_hash: run_started.spec_hash.clone(),
                refs: vec![retention],
                reason: events::RetentionReason::RuntimeEvidence,
            },
        )],
        Vec::new(),
        store::CommitPreconditions::default(),
    );
    let batch = store::build_committed_batch(&request, seq).expect("retention refs batch");
    let mut rewritten = stream.to_vec();
    rewritten.extend(batch.events().iter().cloned());
    rewritten
}

fn append_post_projection_retention_refs_before_completion(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let run_started = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .expect("run started");
    let completion = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("run completed");
    let retention = store::ProjectionSnapshot::rebuild_from_run_stream(stream)
        .expect("valid projection")
        .retention(&run_started.run_id)
        .and_then(|projection| projection.refs.values().next().cloned())
        .expect("retention ref");
    let mut rewritten = remove_completion_commit(stream);
    let retention_seq = next_seq(&rewritten);
    let retention_request = typed_commit_request(
        run_started.run_id.clone(),
        retention_seq,
        store::CommitKey::new("post-projection-retention-ref").expect("commit key"),
        vec![events::KernelEventPayload::RetentionRefsAppended(
            events::RetentionRefsAppended {
                run_id: run_started.run_id.clone(),
                spec_hash: run_started.spec_hash.clone(),
                refs: vec![retention],
                reason: events::RetentionReason::RuntimeEvidence,
            },
        )],
        Vec::new(),
        store::CommitPreconditions::default(),
    );
    let retention_batch = store::build_committed_batch(&retention_request, retention_seq)
        .expect("retention refs batch");
    rewritten.extend(retention_batch.events().iter().cloned());

    let completion_seq = next_seq(&rewritten);
    let completion_request = typed_commit_request(
        run_started.run_id.clone(),
        completion_seq,
        store::CommitKey::new("completion-after-post-projection-retention").expect("commit key"),
        vec![events::KernelEventPayload::RunCompleted(completion)],
        Vec::new(),
        store::CommitPreconditions::default(),
    );
    let completion_batch = store::build_committed_batch(&completion_request, completion_seq)
        .expect("completion batch");
    rewritten.extend(completion_batch.events().iter().cloned());
    rewritten
}

fn append_extra_retention_ref_to_projection_commit(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let mut rewritten = Vec::with_capacity(stream.len() + 1);
    let mut inserted = false;
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        if commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        }) {
            let runtime_evidence = commit
                .iter()
                .find_map(|event| match event.payload() {
                    events::KernelEventPayload::RetentionRefsAppended(payload)
                        if payload.reason == events::RetentionReason::RuntimeEvidence =>
                    {
                        Some(payload)
                    }
                    _ => None,
                })
                .expect("projection commit runtime evidence retention refs");
            let mut payloads = commit
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            payloads.push(events::KernelEventPayload::RetentionRefsAppended(
                events::RetentionRefsAppended {
                    run_id: runtime_evidence.run_id.clone(),
                    spec_hash: runtime_evidence.spec_hash.clone(),
                    refs: runtime_evidence.refs.clone(),
                    reason: events::RetentionReason::RuntimeEvidence,
                },
            ));
            let request = typed_commit_request(
                first.run_id().clone(),
                seq,
                commit_key,
                payloads,
                Vec::new(),
                store::CommitPreconditions::default(),
            );
            let batch = store::build_committed_batch(&request, seq).expect("rewritten commit");
            rewritten.extend(batch.events().iter().cloned());
            inserted = true;
        } else {
            rewritten.extend(commit.iter().cloned());
        }
        index = end;
    }
    assert!(inserted, "retention projection commit exists");
    rewritten
}

fn append_same_sequence_sidecar_to_retention_projection(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let mut rewritten = Vec::with_capacity(stream.len() + 1);
    let mut inserted = false;
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        rewritten.extend(commit.iter().cloned());
        if commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        }) {
            let runtime_evidence = commit
                .iter()
                .find_map(|event| match event.payload() {
                    events::KernelEventPayload::RetentionRefsAppended(payload)
                        if payload.reason == events::RetentionReason::RuntimeEvidence =>
                    {
                        Some(payload)
                    }
                    _ => None,
                })
                .expect("projection runtime evidence retention refs");
            let sidecar_key =
                store::CommitKey::new("forged-same-seq-retention-sidecar").expect("commit key");
            let request = typed_commit_request(
                first.run_id().clone(),
                seq,
                sidecar_key.clone(),
                vec![events::KernelEventPayload::RetentionRefsAppended(
                    events::RetentionRefsAppended {
                        run_id: runtime_evidence.run_id.clone(),
                        spec_hash: runtime_evidence.spec_hash.clone(),
                        refs: runtime_evidence.refs.clone(),
                        reason: events::RetentionReason::RuntimeEvidence,
                    },
                )],
                Vec::new(),
                store::CommitPreconditions::default(),
            );
            let batch = store::build_committed_batch(&request, seq).expect("sidecar commit batch");
            let next_ordinal = u32::try_from(commit.len()).expect("retention commit ordinal count");
            for (offset, event) in batch.events().iter().enumerate() {
                rewritten.push(rewrite_envelope(
                    event,
                    seq,
                    store::CommitOrdinal::new(
                        next_ordinal
                            .checked_add(u32::try_from(offset).expect("sidecar ordinal offset"))
                            .expect("sidecar ordinal"),
                    ),
                    sidecar_key.clone(),
                ));
            }
            inserted = true;
        }
        index = end;
    }
    assert!(inserted, "retention projection commit exists");
    rewritten
}

fn rewrite_envelope(
    event: &store::KernelEventEnvelope,
    seq: store::StreamSeq,
    ordinal: store::CommitOrdinal,
    commit_key: store::CommitKey,
) -> store::KernelEventEnvelope {
    store::KernelEventEnvelope::from_persisted_record(store::PersistedKernelEventRecord {
        event_id: event_id_for(event, seq, ordinal),
        event_schema_id: event.event_schema_id().clone(),
        run_id: event.run_id().clone(),
        seq,
        ordinal,
        spec_hash: event.spec_hash().clone(),
        commit_key,
        logical_key: event.logical_key().clone(),
        payload_hash: event.payload_hash().clone(),
        payload: event.payload().clone(),
        payload_canonical_byte_len: event.audit().payload_canonical_byte_len(),
    })
    .expect("rewritten envelope")
}

fn event_id_for(
    event: &store::KernelEventEnvelope,
    seq: store::StreamSeq,
    ordinal: store::CommitOrdinal,
) -> EventId {
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::json!({
            "event_schema_id": event.event_schema_id().as_str(),
            "ordinal": ordinal.as_u32(),
            "payload_hash": event.payload_hash().as_str(),
            "run_id": event.run_id().as_str(),
            "seq": seq.as_u64(),
        })
        .to_string(),
    )
    .expect("event id canonical");
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, canonical.digest_bytes())
}

fn remove_completion_commit(
    stream: &[store::KernelEventEnvelope],
) -> Vec<store::KernelEventEnvelope> {
    let mut rewritten = Vec::with_capacity(stream.len());
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let original_seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == original_seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        if commit
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
        {
            index = end;
            continue;
        }
        let seq = next_seq(&rewritten);
        let request = typed_commit_request(
            first.run_id().clone(),
            seq,
            commit_key,
            commit.iter().map(|event| event.payload().clone()).collect(),
            Vec::new(),
            store::CommitPreconditions::default(),
        );
        let batch = store::build_committed_batch(&request, seq).expect("rewritten commit");
        rewritten.extend(batch.events().iter().cloned());
        index = end;
    }
    rewritten
}

fn next_seq(stream: &[store::KernelEventEnvelope]) -> store::StreamSeq {
    stream
        .last()
        .map(|event| store::StreamSeq::new(event.seq().as_u64() + 1).expect("next stream seq"))
        .unwrap_or(store::StreamSeq::FIRST)
}
