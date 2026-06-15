use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, RunId};
use mfm_manual_auth::{
    manual_authorization_proof_schema_id, ManualResolutionAuthorizationClaim,
    ManualResolutionBlockReason, ManualResolutionEvidenceRef, VerifiedManualResolution,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::verify_artifact_bytes;
use crate::commit::PreparedStagedArtifact;
use crate::{canonical_json, CertifiedRuntimeSpec, Result, RuntimeError};

const STREAM_PREFIX_DIGEST_ALGORITHM: &str = "mfm.runtime.manual_resolution.prefix.v1";
const UNRESOLVED_OBLIGATIONS_DIGEST_ALGORITHM: &str =
    "mfm.runtime.manual_resolution.unresolved_obligations.v1";

/// Manual resolution evidence artifact bytes supplied by app-level admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionEvidenceArtifact {
    /// Canonical evidence artifact bytes.
    pub bytes: Vec<u8>,
    /// Evidence artifact media type.
    pub media_type: spec::MediaType,
}

/// Builds the manual authorization claim for the current store prefix.
pub fn build_manual_resolution_claim<S: store::TypedRunEventStore + ?Sized>(
    store: &S,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    outcome: events::ManualResolutionOutcome,
    evidence: ManualResolutionEvidenceRef,
) -> Result<ManualResolutionAuthorizationClaim> {
    let stream = store.load_run_stream(run_id);
    let expected_next_seq = store.expected_next_seq(run_id);
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(run_id, &runtime_spec.spec().saga);
    let reason = saga.manual_block_reason.ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "manual resolution prefix is not manually blocked".to_owned(),
        )
    })?;
    if saga.run_mode != store::RunMode::ManualBlocked {
        return Err(RuntimeError::InvalidRunStream(format!(
            "manual resolution requires ManualBlocked prefix, found {}",
            saga.run_mode.as_str()
        )));
    }
    Ok(ManualResolutionAuthorizationClaim {
        run_id: run_id.clone(),
        spec_hash: runtime_spec.spec_hash().clone(),
        expected_next_seq: expected_next_seq.as_u64(),
        stream_prefix_digest: manual_resolution_stream_prefix_digest(&stream)?,
        manual_block_reason: manual_resolution_block_reason(reason),
        unresolved_obligations_digest: unresolved_manual_obligations_digest(&saga)?,
        outcome,
        evidence,
    })
}

pub(crate) fn prepare_manual_resolution_commit(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    saga: &store::SagaProjection,
    expected_next_seq: store::StreamSeq,
    verified: VerifiedManualResolution,
    evidence_artifact: ManualResolutionEvidenceArtifact,
    note: Option<events::ManualResolutionNote>,
) -> Result<(store::PreparedTypedCommit, Vec<PreparedStagedArtifact>)> {
    let claim = verified.claim();
    if saga.run_mode != store::RunMode::ManualBlocked {
        return Err(RuntimeError::InvalidRunStream(format!(
            "manual resolution requires ManualBlocked prefix, found {}",
            saga.run_mode.as_str()
        )));
    }
    let reason = saga.manual_block_reason.ok_or_else(|| {
        RuntimeError::InvalidRunStream("manual resolution prefix lacks block reason".to_owned())
    })?;
    if claim.run_id != saga.run_id {
        return Err(RuntimeError::InvalidRunStream(
            "manual authorization claim run id does not match prefix".to_owned(),
        ));
    }
    if claim.spec_hash != *runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "manual authorization claim spec hash does not match runtime spec".to_owned(),
        ));
    }
    if claim.expected_next_seq != expected_next_seq.as_u64() {
        return Err(RuntimeError::InvalidRunStream(
            "manual authorization claim expected sequence is stale".to_owned(),
        ));
    }
    if claim.stream_prefix_digest != manual_resolution_stream_prefix_digest(stream)? {
        return Err(RuntimeError::InvalidRunStream(
            "manual authorization claim prefix digest does not match stream".to_owned(),
        ));
    }
    if claim.manual_block_reason != manual_resolution_block_reason(reason) {
        return Err(RuntimeError::InvalidRunStream(
            "manual authorization claim block reason does not match prefix".to_owned(),
        ));
    }
    if claim.unresolved_obligations_digest != unresolved_manual_obligations_digest(saga)? {
        return Err(RuntimeError::InvalidRunStream(
            "manual authorization claim obligations digest does not match prefix".to_owned(),
        ));
    }

    let evidence_ref = store::ArtifactEvidenceRef {
        artifact_id: claim.evidence.artifact_id.clone(),
        digest: claim.evidence.content_hash.clone(),
        byte_len: evidence_artifact.bytes.len() as u64,
        media_type: evidence_artifact.media_type,
        schema_id: Some(claim.evidence.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::ManualResolutionEvidence,
    };
    verify_artifact_bytes(&evidence_artifact.bytes, &evidence_ref)?;

    let proof_bytes = verified.proof().canonical_json().map_err(|error| {
        RuntimeError::InvalidRunStream(format!("manual authorization proof is invalid: {error}"))
    })?;
    let authorization_hash = proof_bytes.content_digest();
    let authorization_artifact_id =
        ArtifactId::from_digest(authorization_hash.algorithm(), *authorization_hash.digest());
    let authorization_schema_id = manual_authorization_proof_schema_id().map_err(|error| {
        RuntimeError::InvalidRunStream(format!("manual authorization proof schema failed: {error}"))
    })?;
    let authorization_ref = store::ArtifactEvidenceRef {
        artifact_id: authorization_artifact_id.clone(),
        digest: authorization_hash.clone(),
        byte_len: proof_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(authorization_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::ManualResolutionAuthorization,
    };
    verify_artifact_bytes(proof_bytes.as_bytes(), &authorization_ref)?;

    let request = store::TypedCommitRequest {
        run_id: claim.run_id.clone(),
        expected_next_seq,
        commit_key: store::CommitKey::new(format!(
            "manual-resolution:{}:{}",
            claim.outcome.as_str(),
            authorization_hash.as_str()
        ))?,
        payloads: vec![events::KernelEventPayload::ManualResolutionRecorded(
            events::ManualResolutionRecorded {
                run_id: claim.run_id.clone(),
                spec_hash: claim.spec_hash.clone(),
                outcome: claim.outcome,
                evidence_schema_id: claim.evidence.schema_id.clone(),
                evidence_hash: claim.evidence.content_hash.clone(),
                evidence_artifact_id: claim.evidence.artifact_id.clone(),
                authorization_schema_id,
                authorization_hash,
                authorization_artifact_id,
                note,
            },
        )],
        required_artifacts: vec![evidence_ref.clone(), authorization_ref.clone()],
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            saga_policy: Some(runtime_spec.spec().saga.clone()),
            ..store::CommitPreconditions::default()
        },
    };
    let prepared = store::PreparedTypedCommit::new(
        request,
        vec![evidence_ref.clone(), authorization_ref.clone()],
    )?;
    Ok((
        prepared,
        vec![
            PreparedStagedArtifact {
                bytes: evidence_artifact.bytes,
                evidence: evidence_ref,
            },
            PreparedStagedArtifact {
                bytes: proof_bytes.to_vec(),
                evidence: authorization_ref,
            },
        ],
    ))
}

/// Computes the digest of a verified run stream prefix.
pub fn manual_resolution_stream_prefix_digest(
    stream: &[store::KernelEventEnvelope],
) -> Result<ContentDigest> {
    let canonical = canonical_json(serde_json::json!({
        "algorithm": STREAM_PREFIX_DIGEST_ALGORITHM,
        "events": stream
            .iter()
            .map(|event| {
                serde_json::json!({
                    "commit_key": event.commit_key().as_str(),
                    "event_id": event.event_id().as_str(),
                    "event_schema_id": event.event_schema_id().as_str(),
                    "logical_key": event.logical_key().as_str(),
                    "ordinal": event.ordinal().as_u32(),
                    "payload_hash": event.payload_hash().as_str(),
                    "run_id": event.run_id().as_str(),
                    "seq": event.seq().as_u64(),
                    "spec_hash": event.spec_hash().as_str(),
                })
            })
            .collect::<Vec<_>>(),
    }))?;
    Ok(canonical.content_digest())
}

/// Computes the digest of unresolved obligations in a manual-blocked saga projection.
pub fn unresolved_manual_obligations_digest(saga: &store::SagaProjection) -> Result<ContentDigest> {
    let canonical = canonical_json(serde_json::json!({
        "algorithm": UNRESOLVED_OBLIGATIONS_DIGEST_ALGORITHM,
        "obligations": saga
            .obligations
            .values()
            .filter(|obligation| obligation_is_unresolved(obligation))
            .map(obligation_json)
            .collect::<Vec<_>>(),
        "run_id": saga.run_id.as_str(),
    }))?;
    Ok(canonical.content_digest())
}

/// Converts store manual block reasons into manual-authorization claim reasons.
pub const fn manual_resolution_block_reason(
    reason: store::ManualBlockReason,
) -> ManualResolutionBlockReason {
    match reason {
        store::ManualBlockReason::PolicyManualResolution => {
            ManualResolutionBlockReason::PolicyManualResolution
        }
        store::ManualBlockReason::ForwardAmbiguous => ManualResolutionBlockReason::ForwardAmbiguous,
        store::ManualBlockReason::RemediationFailed => {
            ManualResolutionBlockReason::RemediationFailed
        }
        store::ManualBlockReason::RemediationAmbiguous => {
            ManualResolutionBlockReason::RemediationAmbiguous
        }
    }
}

fn obligation_is_unresolved(obligation: &store::SagaObligationProjection) -> bool {
    matches!(
        obligation.classification,
        store::ForwardLedgerClassification::Owed | store::ForwardLedgerClassification::Unresolvable
    ) || obligation
        .remediation
        .as_ref()
        .and_then(|remediation| remediation.unresolved)
        .is_some()
}

fn obligation_json(obligation: &store::SagaObligationProjection) -> serde_json::Value {
    serde_json::json!({
        "classification": forward_classification_str(obligation.classification),
        "forward_ledger_key": obligation.forward_ledger_key.as_str(),
        "forward_phase": side_effect_phase_json(&obligation.forward_phase),
        "remediation": obligation.remediation.as_ref().map(remediation_json),
    })
}

fn remediation_json(remediation: &store::RemediationLedgerProjection) -> serde_json::Value {
    serde_json::json!({
        "closed": remediation.closed,
        "ledger_key": remediation.ledger_key.as_str(),
        "phase": side_effect_phase_json(&remediation.phase),
        "unresolved": remediation.unresolved.map(|reason| manual_resolution_block_reason(reason).as_str()),
    })
}

fn side_effect_phase_json(phase: &store::SideEffectPhase) -> serde_json::Value {
    let mut value = serde_json::json!({
        "kind": phase.as_str(),
        "invocation_epoch": side_effect_phase_epoch(phase),
    });
    if let store::SideEffectPhase::Failed { failure_phase, .. } = phase {
        value["failure_phase"] = serde_json::json!(failure_phase_str(*failure_phase));
    }
    value
}

fn side_effect_phase_epoch(phase: &store::SideEffectPhase) -> u32 {
    match phase {
        store::SideEffectPhase::IntentPersisted { invocation_epoch }
        | store::SideEffectPhase::Claimed {
            invocation_epoch, ..
        }
        | store::SideEffectPhase::InvocationPrepared {
            invocation_epoch, ..
        }
        | store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        }
        | store::SideEffectPhase::SubmissionObserved { invocation_epoch }
        | store::SideEffectPhase::NotSubmittedProven { invocation_epoch }
        | store::SideEffectPhase::SubmissionUnknown { invocation_epoch }
        | store::SideEffectPhase::ReceiptObserved { invocation_epoch }
        | store::SideEffectPhase::ConfirmationObserved { invocation_epoch }
        | store::SideEffectPhase::Ambiguous { invocation_epoch }
        | store::SideEffectPhase::Failed {
            invocation_epoch, ..
        } => *invocation_epoch,
    }
}

fn forward_classification_str(classification: store::ForwardLedgerClassification) -> &'static str {
    match classification {
        store::ForwardLedgerClassification::Pending => "pending",
        store::ForwardLedgerClassification::NothingOwed => "nothing_owed",
        store::ForwardLedgerClassification::Owed => "owed",
        store::ForwardLedgerClassification::Unresolvable => "unresolvable",
    }
}

fn failure_phase_str(phase: events::side_effect::FailurePhase) -> &'static str {
    match phase {
        events::side_effect::FailurePhase::BeforeInvocationStarted => "before_invocation_started",
        events::side_effect::FailurePhase::AfterNotSubmittedProven => "after_not_submitted_proven",
    }
}
