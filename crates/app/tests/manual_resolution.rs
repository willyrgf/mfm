#![allow(clippy::disallowed_methods)]

use k256::ecdsa::SigningKey;
use mfm_app::{
    DriveMode, EntryPointOpId, EntryPointOpPlan, EntryPointOpRegistry, EntryPointRunLaunchInput,
    LaunchableOp, ManualResolutionDecision, ManualResolutionRecordRequest, OpLaunchError,
    OpVersion, PublicOpName, TypedRunMode,
};
use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};
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

    let mut entry_points = EntryPointOpRegistry::new();
    entry_points
        .register(ManualResolutionProofEntryPointOp)
        .expect("register manual proof entry point");
    let run_id = mfm_app::new_run_id();
    let authored_config =
        AuthoredConfig::new(AuthoredConfigFormat::Json, b"{}".to_vec()).expect("authored config");
    let launch = mfm_app::prepare_entry_point_run_launch(EntryPointRunLaunchInput {
        entry_point_registry: &entry_points,
        public_op_name: PublicOpName::new("manual_resolution_proof").expect("public op name"),
        op_version: Some(OpVersion::new(1).expect("op version")),
        authored_config,
        certification_registry: &registry,
        run_id: run_id.clone(),
        drive: DriveMode::UntilBlocked,
    })
    .expect("launch request");

    let certified = launch.request.certified_spec.clone();
    let blocked = services.launch_run(launch.request).await.expect("launch");
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

static CONFIG_FORMATS: &[AuthoredConfigFormat] = &[AuthoredConfigFormat::Json];

struct ManualResolutionProofEntryPointOp;

impl LaunchableOp for ManualResolutionProofEntryPointOp {
    fn op_id(&self) -> EntryPointOpId {
        EntryPointOpId::new("mfm.test.manual", "manual_resolution_proof", self.version())
            .expect("test op id")
    }

    fn public_name(&self) -> PublicOpName {
        PublicOpName::new("manual_resolution_proof").expect("public op name")
    }

    fn version(&self) -> OpVersion {
        OpVersion::new(1).expect("op version")
    }

    fn accepted_config_formats(&self) -> &'static [AuthoredConfigFormat] {
        CONFIG_FORMATS
    }

    fn plan(&self, authored_config: AuthoredConfig) -> Result<EntryPointOpPlan, OpLaunchError> {
        let _normalized = authored_config.normalize::<Value>()?;
        let draft = mfm_op_proof::manual_resolution_proof_program_draft(
            mfm_op_proof::ProofWorkflowConfig::default(),
        )
        .map_err(|error| {
            OpLaunchError::new("ManualResolutionProofPlanFailed", error.to_string())
        })?;
        mfm_program::TypedProgramLaunchPlan::from_draft(draft)
            .map(EntryPointOpPlan::from)
            .map_err(|error| {
                OpLaunchError::new("ManualResolutionProofPlanFailed", error.to_string())
            })
    }
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
