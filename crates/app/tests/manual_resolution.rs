#![allow(clippy::disallowed_methods)]

use k256::ecdsa::SigningKey;
use mfm_app::{
    DriveMode, ManualResolutionDecision, ManualResolutionRecordRequest, RunLaunchConfigArtifact,
    TypedRunMode,
};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, RunId};
use mfm_manual_auth::{
    ManualAuthorizationSignatureBytes, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionEvidenceRef,
    ManualResolutionPrefixAuthority,
};
use mfm_runtime::{
    manual_resolution_block_reason, manual_resolution_stream_prefix_digest,
    unresolved_manual_obligations_digest,
};
use mfm_spec::v1 as spec;
use mfm_store::v1::{self as store, AsyncTypedRunEventStore};
use serde_json::Value;
use tempfile::TempDir;

const PROOF_SECRET_SENTINEL: &str = "manual-secret-proof-sentinel";

#[tokio::test]
async fn public_manual_resolution_scenario_records_resolution_and_hides_proof_bytes() {
    let temp = TempDir::new().expect("temp dir");
    let artifact_root = temp.path().join("typed-artifacts");
    std::fs::create_dir_all(&artifact_root).expect("artifact root");
    let artifacts = mfm_artifact_store_fs::FsTypedArtifactStore::new(&artifact_root);
    let runners = mfm_app::production_typed_runner_registry(artifacts.clone()).expect("runners");
    let registry = mfm_app::production_certification_registry().expect("cert registry");
    let services = mfm_app::make_async_typed_services_with_certification_registry(
        runners,
        store::AsyncInMemoryTypedRunStore::default(),
        artifacts,
        registry.clone(),
    );

    let proof_config = mfm_op_proof::ProofWorkflowConfig::default();
    let draft = mfm_op_proof::manual_resolution_proof_program_draft(proof_config.clone())
        .expect("manual proof draft");
    let certified = mfm_op_proof::certified_manual_resolution_proof_spec(proof_config)
        .expect("manual proof spec");
    let bundle = certified.bundle().expect("manual proof bundle");
    let run_id = mfm_app::new_run_id();
    let launch = mfm_app::prepare_verified_bundle_launch(
        mfm_app::UntrustedCertifiedBundleLaunchInput {
            spec_bytes: bundle.spec_bytes(),
            certificate_bytes: bundle.certificate_bytes(),
            registry: &registry,
            run_id: run_id.clone(),
            framework_version: "mfm.app.test.manual_resolution.v1",
            source_revision: "manual-resolution-test",
            launched_at_unix_ms: 1,
            drive: DriveMode::UntilBlocked,
        },
        config_artifacts_for_draft_and_spec(&draft, &certified.envelope().spec),
        Vec::new(),
    )
    .expect("launch request");

    let blocked = services.launch_run(launch).await.expect("launch");
    assert_eq!(blocked.run_mode, TypedRunMode::ManualBlocked);
    let blocked_replay = services
        .verify_replay_for_run(&run_id)
        .await
        .expect("manual-blocked proof replay");
    assert_eq!(blocked_replay.run_mode, TypedRunMode::ManualBlocked);
    assert_eq!(
        blocked.saga.manual_block_reason.as_deref(),
        Some("policy_manual_resolution")
    );
    let required = blocked
        .saga
        .required_manual_authorization
        .as_ref()
        .expect("required manual auth");
    assert_eq!(
        required.evidence_schema_id,
        mfm_op_proof::proof_manual_resolution_evidence_schema_id()
            .expect("manual schema id")
            .as_str()
    );
    assert_eq!(required.quorum_required_signatures, 1);

    let manual = manual_policy(&certified);
    let evidence_bytes = PlainCanonicalJsonBytes::from_json_str(r#"{"operator_note":"reviewed"}"#)
        .expect("canonical manual evidence")
        .to_vec();
    let proof_bytes = signed_manual_resolution_proof_bytes(
        &services,
        &run_id,
        &certified,
        manual.clone(),
        &evidence_bytes,
    )
    .await;

    let resolved = services
        .record_manual_resolution(ManualResolutionRecordRequest {
            run_id: run_id.clone(),
            outcome: ManualResolutionDecision::ConfirmRemediated,
            evidence_bytes: evidence_bytes.clone(),
            evidence_media_type: "application/json".to_owned(),
            authorization_proof_bytes: proof_bytes.clone(),
            note: Some("reviewed externally".to_owned()),
            drive: DriveMode::UntilBlocked,
        })
        .await
        .expect("manual resolution");

    assert_eq!(resolved.run_mode, TypedRunMode::ManuallyResolved);
    assert_eq!(resolved.saga.manual_block_reason, None);
    assert_eq!(resolved.saga.required_manual_authorization, None);
    let terminal = resolved
        .saga
        .terminal_resolution
        .as_ref()
        .expect("terminal resolution");
    assert_eq!(terminal.outcome, "manually_resolved");
    assert_eq!(terminal.claim, "manual_resolution");
    assert!(
        resolved.attempt_dispositions.iter().any(|attempt| {
            attempt.disposition == "completed"
                && resolve_saga_node_ids(&certified.envelope().spec)
                    .contains(&attempt.node_id.as_str())
        }),
        "missing completed ResolveSagaTerminal attempt"
    );

    let status = services.run_status(&run_id).await.expect("status");
    assert_eq!(status.run_mode, TypedRunMode::ManuallyResolved);

    let stream = services
        .store()
        .load_run_stream(&run_id)
        .await
        .expect("run stream");
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ManualResolutionRecorded(_)
        )),
        "stream must expose manual resolution event"
    );
    assert_resolve_saga_started_before_run_completed(&stream, &certified.envelope().spec);

    let public_rendered =
        serde_json::to_string(&(blocked, resolved, status)).expect("public responses serialize");
    let stream_response = services.run_stream(&run_id).await.expect("public stream");
    let stream_rendered = serde_json::to_string(&stream_response).expect("stream serializes");
    let proof_rendered = std::str::from_utf8(&proof_bytes).expect("proof utf8");
    let proof: Value = serde_json::from_slice(&proof_bytes).expect("proof json");
    let signature = proof["signatures"][0]["signature_hex"]
        .as_str()
        .expect("signature");
    for rendered in [&public_rendered, &stream_rendered] {
        assert!(!rendered.contains(proof_rendered));
        assert!(!rendered.contains(signature));
        assert!(!rendered.contains(PROOF_SECRET_SENTINEL));
    }
}

fn config_artifacts_for_draft_and_spec(
    draft: &mfm_program::TypedProgramDraft,
    certified_spec: &spec::TypedExecutionSpec,
) -> Vec<RunLaunchConfigArtifact> {
    let media_type = mfm_app::json_media_type().expect("json media type");
    let mut configs = draft
        .state_nodes()
        .iter()
        .map(|node| RunLaunchConfigArtifact {
            schema_id: node.config.schema_id.clone(),
            bytes: node.config.canonical_json.as_bytes().to_vec(),
            media_type: media_type.clone(),
        })
        .chain(
            draft
                .operation_lineage()
                .iter()
                .map(|frame| RunLaunchConfigArtifact {
                    schema_id: frame.config.schema_id.clone(),
                    bytes: frame.config.canonical_json.as_bytes().to_vec(),
                    media_type: media_type.clone(),
                }),
        )
        .collect::<Vec<_>>();
    configs.extend(certified_spec.nodes.iter().filter_map(|node| {
        node.framework.as_ref().map(|framework| {
            let bytes =
                spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
                    .expect("framework config");
            assert_eq!(
                bytes.content_digest(),
                node.config_ref.digest,
                "framework config helper must match certified config ref"
            );
            RunLaunchConfigArtifact {
                schema_id: node.config_ref.schema_id.clone(),
                bytes: bytes.as_bytes().to_vec(),
                media_type: media_type.clone(),
            }
        })
    }));
    configs
}

async fn signed_manual_resolution_proof_bytes(
    services: &mfm_app::RunServices<store::AsyncInMemoryTypedRunStore>,
    run_id: &RunId,
    certified: &mfm_certify::CertifiedTypedSpec,
    manual: spec::ManualResolutionEvidenceSpec,
    evidence_bytes: &[u8],
) -> Vec<u8> {
    let evidence_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(evidence_bytes),
    );
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        artifact_id: ArtifactId::from_digest(evidence_hash.algorithm(), *evidence_hash.digest()),
        content_hash: evidence_hash,
    };
    let stream = services
        .store()
        .load_run_stream(run_id)
        .await
        .expect("run stream");
    let projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("projection rebuild");
    let saga = projection.derive_saga_projection(run_id, &certified.envelope().spec.saga);
    let reason = saga.manual_block_reason.expect("manual block reason");
    let expected_next_seq = services
        .store()
        .expected_next_seq(run_id)
        .await
        .expect("expected next seq");
    let prefix = ManualResolutionPrefixAuthority::new(
        run_id.clone(),
        certified.spec_hash().clone(),
        expected_next_seq.as_u64(),
        manual_resolution_stream_prefix_digest(&stream).expect("prefix digest"),
        manual_resolution_block_reason(reason),
        unresolved_manual_obligations_digest(&saga).expect("obligation digest"),
        manual.clone(),
    )
    .expect("manual prefix");
    let claim = prefix
        .authorization_claim(events::ManualResolutionOutcome::ConfirmRemediated, evidence)
        .expect("manual claim");
    let operator = manual.authorization.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("claim digest");
    let proof = ManualResolutionAuthorizationProof {
        verifier_id: manual.authorization.verifier_id,
        signing_scheme: manual.authorization.signing_scheme,
        claim,
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("signature"),
        }],
    };
    proof
        .canonical_json()
        .expect("canonical manual proof")
        .to_vec()
}

fn manual_policy(
    certified: &mfm_certify::CertifiedTypedSpec,
) -> spec::ManualResolutionEvidenceSpec {
    match &certified.envelope().spec.saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual.clone(),
        _ => panic!("manual proof scenario must carry manual policy"),
    }
}

fn resolve_saga_node_ids(spec: &spec::TypedExecutionSpec) -> Vec<&str> {
    spec.nodes
        .iter()
        .filter(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .map(|node| node.node_id.as_str())
        .collect()
}

fn assert_resolve_saga_started_before_run_completed(
    stream: &[store::KernelEventEnvelope],
    spec: &spec::TypedExecutionSpec,
) {
    let resolve_nodes = resolve_saga_node_ids(spec);
    let start = stream
        .iter()
        .position(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                resolve_nodes.contains(&payload.node_id.as_str())
            }
            _ => false,
        })
        .expect("ResolveSagaTerminal attempt start");
    let completed = stream
        .iter()
        .position(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
        .expect("RunCompleted event");
    assert!(start < completed);
}

fn test_manual_signing_key() -> SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}
