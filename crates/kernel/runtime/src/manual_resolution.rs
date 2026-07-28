use std::ops::ControlFlow;

use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, SchemaId};
use mfm_manual_auth::{
    manual_authorization_proof_schema_id, ManualResolutionEvidenceRef,
    ManualResolutionPrefixAuthority, ManualResolutionProofAuthority,
    VerifiedManualResolutionForPrefix,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::verify_artifact_bytes;
use crate::commit::PreparedStagedArtifact;
use crate::spec_authority::{CurrentRuntimeSpecRef, CurrentSpecRead};
use crate::{Result, RuntimeError};

/// Manual resolution evidence artifact bytes supplied by app-level admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionEvidenceArtifact {
    /// Canonical evidence artifact bytes.
    pub bytes: Vec<u8>,
    /// Evidence artifact media type.
    pub media_type: spec::MediaType,
}

pub(crate) struct ManualResolutionCommitInput<'a> {
    pub(crate) runtime_spec: CurrentRuntimeSpecRef<'a>,
    pub(crate) lifecycle: store::current_lifecycle::CurrentLifecycleReader<'a>,
    pub(crate) verified: VerifiedManualResolutionForPrefix,
    pub(crate) evidence_artifact: ManualResolutionEvidenceArtifact,
    pub(crate) note: Option<events::ManualResolutionNote>,
}

pub(crate) fn verify_manual_resolution_for_prefix(
    prefix: ManualResolutionPrefixAuthority,
    outcome: events::ManualResolutionOutcome,
    evidence_artifact: &ManualResolutionEvidenceArtifact,
    authorization_proof_bytes: Vec<u8>,
) -> Result<VerifiedManualResolutionForPrefix> {
    let evidence = manual_resolution_content_ref(
        prefix.manual_policy().evidence_schema.clone(),
        &evidence_artifact.bytes,
    );
    let authorization = manual_authorization_proof_schema_id()
        .map(|schema_id| manual_resolution_content_ref(schema_id, &authorization_proof_bytes))
        .map_err(|error| {
            RuntimeError::InvalidRunStream(format!(
                "manual authorization proof schema failed: {error}"
            ))
        })?;
    ManualResolutionProofAuthority::new(
        prefix,
        outcome,
        evidence,
        authorization,
        authorization_proof_bytes,
    )
    .and_then(ManualResolutionProofAuthority::verify)
    .map_err(|error| {
        RuntimeError::InvalidRunStream(format!(
            "manual authorization proof failed verification: {error}"
        ))
    })
}

pub(crate) fn prepare_manual_resolution_commit(
    input: ManualResolutionCommitInput<'_>,
) -> Result<(
    store::PreparedCommit<store::ManualResolution>,
    Vec<PreparedStagedArtifact>,
)> {
    let ManualResolutionCommitInput {
        runtime_spec,
        lifecycle,
        verified,
        evidence_artifact,
        note,
    } = input;
    let claim = verified.claim();
    let prefix = verified.prefix();
    let expected_next_seq = lifecycle.next_sequence()?;
    let current_prefix = lifecycle
        .manual_resolution_prefix_authority()
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    if prefix != &current_prefix {
        return Err(RuntimeError::InvalidRunStream(
            "manual authorization claim does not match the current verified prefix".to_owned(),
        ));
    }

    let evidence_ref = manual_resolution_artifact_evidence_ref(
        &claim.evidence,
        evidence_artifact.bytes.len() as u64,
        evidence_artifact.media_type,
        events::ArtifactRole::ManualResolutionEvidence,
    );
    verify_artifact_bytes(&evidence_artifact.bytes, &evidence_ref)?;

    let authorization = verified.authorization();
    let authorization_ref = manual_resolution_artifact_evidence_ref(
        authorization,
        verified.proof_bytes().len() as u64,
        spec::MediaType::new("application/json")?,
        events::ArtifactRole::ManualResolutionAuthorization,
    );
    verify_artifact_bytes(verified.proof_bytes(), &authorization_ref)?;

    let mut payloads = manual_resolution_resource_lane_release_intents(&runtime_spec, &lifecycle)?;
    payloads.push(events::KernelEventPayload::ManualResolutionRecorded(
        events::ManualResolutionRecorded {
            run_id: claim.run_id.clone(),
            spec_hash: claim.spec_hash.clone(),
            outcome: claim.outcome,
            evidence_schema_id: claim.evidence.schema_id.clone(),
            evidence_hash: claim.evidence.content_hash.clone(),
            evidence_artifact_id: claim.evidence.artifact_id.clone(),
            evidence_artifact_evidence_hash: evidence_ref.evidence_hash()?,
            authorization_schema_id: authorization.schema_id.clone(),
            authorization_hash: authorization.content_hash.clone(),
            authorization_artifact_id: authorization.artifact_id.clone(),
            authorization_artifact_evidence_hash: authorization_ref.evidence_hash()?,
            note,
        },
    ));

    let request = store::CommitRequest::from_payloads(
        claim.run_id.clone(),
        expected_next_seq,
        store::CommitKey::new(format!(
            "manual-resolution:{}:{}",
            claim.outcome.as_str(),
            authorization.content_hash.as_str()
        ))?,
        payloads,
        vec![evidence_ref.clone(), authorization_ref.clone()],
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                claim.run_id.clone(),
                runtime_spec.spec(),
            )?),
            ..store::CommitPreconditions::default()
        },
    )?;
    let prepared = store::PreparedCommit::<store::ManualResolution>::new(
        request,
        store::CommitArtifactEvidenceSet::new(
            vec![evidence_ref.clone(), authorization_ref.clone()],
            vec![evidence_ref.clone(), authorization_ref.clone()],
        )?,
        &verified,
    )?;
    Ok((
        prepared,
        vec![
            PreparedStagedArtifact {
                bytes: evidence_artifact.bytes,
                evidence: evidence_ref,
            },
            PreparedStagedArtifact {
                bytes: verified.proof_bytes().to_vec(),
                evidence: authorization_ref,
            },
        ],
    ))
}

fn manual_resolution_resource_lane_release_intents<S>(
    runtime_spec: &S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<Vec<events::KernelEventPayload>>
where
    S: CurrentSpecRead + ?Sized,
{
    let mut payloads = Vec::new();
    let mut failure = None;
    let _ = lifecycle.visit_resource_lanes(|lane| {
        match events::ResourceLaneReleaseReason::new("manual_resolution") {
            Ok(release_reason) => {
                payloads.push(events::KernelEventPayload::ResourceLaneReleaseIntent(
                    events::ResourceLaneReleaseIntent {
                        spec_hash: runtime_spec.spec_hash().clone(),
                        ledger_key: lane.ledger_key().clone(),
                        ledger_purpose: lane.ledger_purpose().clone(),
                        pair_id: lane.holder().pair_id().clone(),
                        pair_role: events::SideEffectPairRole::Verify,
                        invocation_epoch: lane.invocation_epoch(),
                        claim_id: lane.claim_id().clone(),
                        release_authority: events::ResourceLaneReleaseAuthority::ManualResolution,
                        release_reason,
                    },
                ))
            }
            Err(error) => {
                failure = Some(RuntimeError::Identity(error.to_string()));
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    });
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(payloads)
}

fn manual_resolution_content_ref(schema_id: SchemaId, bytes: &[u8]) -> ManualResolutionEvidenceRef {
    let content_hash =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    ManualResolutionEvidenceRef {
        schema_id,
        artifact_id: ArtifactId::from_digest(content_hash.algorithm(), *content_hash.digest()),
        content_hash,
    }
}

fn manual_resolution_artifact_evidence_ref(
    evidence: &ManualResolutionEvidenceRef,
    byte_len: u64,
    media_type: spec::MediaType,
    artifact_role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        digest: evidence.content_hash.clone(),
        byte_len,
        media_type,
        schema_id: Some(evidence.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role,
    }
}
