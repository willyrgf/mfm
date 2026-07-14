use super::*;

pub(crate) fn payload_spec_hash(payload: &KernelEventPayload) -> SpecHash {
    payload.spec_hash().clone()
}

pub(crate) fn derive_event_id(
    run_id: &RunId,
    seq: StreamSeq,
    ordinal: CommitOrdinal,
    event_schema_id: &SchemaId,
    payload_hash: &ContentDigest,
) -> Result<EventId> {
    let canonical = canonical_json(serde_json::json!({
        "event_schema_id": event_schema_id.as_str(),
        "ordinal": ordinal.as_u32(),
        "payload_hash": payload_hash.as_str(),
        "run_id": run_id.as_str(),
        "seq": seq.as_u64(),
    }))?;
    Ok(EventId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical.digest_bytes(),
    ))
}

pub(crate) fn derive_logical_key(
    run_id: &RunId,
    seq: StreamSeq,
    ordinal: CommitOrdinal,
    payload: &KernelEventPayload,
    payload_hash: &ContentDigest,
) -> Result<LogicalEventKey> {
    let key = match payload {
        KernelEventPayload::RunAdmitted(_) => "run:admission".to_owned(),
        KernelEventPayload::ManualResolutionRecorded(_) => "run:manual_resolution".to_owned(),
        KernelEventPayload::RunCompleted(_) => "run:complete".to_owned(),
        KernelEventPayload::StateAttemptStarted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptCompleted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::FactRecorded(_) => {
            let claim_id =
                mfm_facts::derive_fact_claim_id(run_id.clone(), seq.as_u64(), ordinal.as_u32())
                    .map_err(|error| StoreError::Event(error.to_string()))?;
            fact_claim_projection_key("fact", &claim_id)
        }
        KernelEventPayload::ArtifactReferenced(payload) => {
            format!("artifact:{}:ref", payload.artifact_ref.artifact_id)
        }
        KernelEventPayload::CellProduced(payload) => {
            format!("cell:{}:terminal", payload.cell_id)
        }
        KernelEventPayload::CellSkipped(payload) => {
            format!("cell:{}:terminal", payload.cell_id)
        }
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            format!(
                "sidefx:{}:{}:intent",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.pair_id
            )
        }
        KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx:{}:{}:claim:{}:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx:{}:{}:claim:{}:{}:taken_over",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::ResourceLaneClaimed(payload) => format!(
            "resource_lane:{}:{}:claim:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.claim_id
        ),
        KernelEventPayload::ResourceLaneClaimIntent(_)
        | KernelEventPayload::ResourceLaneReleaseIntent(_) => {
            return Err(StoreError::Event(
                "resource-lane intent payload reached persisted logical-key derivation".to_owned(),
            ));
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx:{}:{}:invocation:{}:prepared:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx:{}:{}:invocation:{}:started",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:receipt",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:confirmation",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!(
                "sidefx:{}:{}:ambiguous",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.pair_id
            )
        }
        KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx:{}:{}:invocation:{}:failure",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::ResourceLaneReleased(payload) => format!(
            "resource_lane:{}:{}:release:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.release_id
        ),
        KernelEventPayload::PublicOutputProduced(payload) => {
            format!("public_output:{}", payload.public_schema_id)
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            format!(
                "public_output_failed:{}:{}:{}",
                payload.public_schema_id, payload.node_id, payload.attempt_id
            )
        }
        KernelEventPayload::RetentionRefsAppended(payload) => {
            format!("retention:{}:refs:{}", payload.run_id, payload_hash)
        }
        KernelEventPayload::RetentionManifestProjected(payload) => {
            format!(
                "retention:{}:manifest:{}",
                payload.run_id, payload.manifest_seq
            )
        }
    };
    LogicalEventKey::new(key)
}

pub(crate) fn is_unique_logical_key(key: &LogicalEventKey) -> bool {
    let key = key.as_str();
    !key.starts_with("attempt:")
}

pub(crate) fn unique_logical_key_rewrite_allowed(
    base: &CommitBase,
    run_id: &RunId,
    logical_key: &LogicalEventKey,
    payload: &KernelEventPayload,
    projections: &ProjectionSnapshot,
) -> Result<bool> {
    if !base
        .logical_keys
        .contains(&(run_id.clone(), logical_key.clone()))
    {
        return Ok(false);
    }
    let Some((pair_id, invocation_epoch)) = recoverable_submission_result_payload(payload) else {
        return Ok(false);
    };
    Ok(matches!(
        projections.side_effect_state_for_pair(run_id, pair_id)?,
        Some(state)
            if matches!(
                state.phase(),
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::Unknown,
                } if claim.invocation_epoch == invocation_epoch
            )
    ))
}

fn recoverable_submission_result_payload(
    payload: &KernelEventPayload,
) -> Option<(&SideEffectPairId, u32)> {
    match payload {
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        _ => None,
    }
}

fn side_effect_ledger_purpose_key(purpose: &events::SideEffectLedgerPurpose) -> String {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => "forward".to_owned(),
        events::SideEffectLedgerPurpose::Remediation {
            forward_pair_id, ..
        } => {
            format!("remediation:{forward_pair_id}")
        }
    }
}
