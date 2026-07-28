use super::*;

const JOURNAL_PREFIX_DIGEST_ALGORITHM: &str = "mfm.runtime.manual_resolution.prefix.v1";
const UNRESOLVED_OBLIGATIONS_DIGEST_ALGORITHM: &str =
    "mfm.runtime.manual_resolution.unresolved_obligations.v1";

/// Builds manual-resolution authority from one verified journal prefix and its derived saga.
pub(in crate::v1) fn build_manual_prefix_authority(
    run_id: &RunId,
    spec_hash: &SpecHash,
    saga_policy: &SagaPolicySpec,
    expected_next_seq: StreamSeq,
    prefix_records: &[KernelEventEnvelope],
    pre_batch_saga: &SagaProjection,
) -> Result<mfm_manual_auth::ManualResolutionPrefixAuthority> {
    if &pre_batch_saga.run_id != run_id {
        return Err(manual_prefix_error(
            run_id,
            "manual-resolution saga belongs to a different run",
        ));
    }
    if pre_batch_saga.run_mode != RunMode::ManualBlocked {
        return Err(manual_prefix_error(
            run_id,
            format!(
                "manual resolution requires manual_blocked saga mode, found {}",
                pre_batch_saga.run_mode.as_str()
            ),
        ));
    }
    let reason = pre_batch_saga.manual_block_reason.ok_or_else(|| {
        manual_prefix_error(
            run_id,
            "manual-blocked saga has no manual-resolution reason",
        )
    })?;
    let manual_policy = manual_policy_for_block_reason(saga_policy, reason)
        .ok_or_else(|| {
            manual_prefix_error(
                run_id,
                "manual block reason is not permitted by the certified saga policy",
            )
        })?
        .clone();
    let observed_next_seq = prefix_records
        .last()
        .ok_or_else(|| manual_prefix_error(run_id, "manual-resolution prefix is empty"))?
        .seq()
        .checked_next()?;
    if observed_next_seq != expected_next_seq {
        return Err(manual_prefix_error(
            run_id,
            "manual-resolution expected sequence does not follow the exact prefix",
        ));
    }
    if prefix_records
        .iter()
        .any(|record| record.run_id() != run_id || record.spec_hash() != spec_hash)
    {
        return Err(manual_prefix_error(
            run_id,
            "manual-resolution prefix contains foreign run or spec authority",
        ));
    }

    mfm_manual_auth::ManualResolutionPrefixAuthority::new(
        run_id.clone(),
        spec_hash.clone(),
        expected_next_seq.as_u64(),
        journal_prefix_digest(prefix_records)?,
        manual_block_reason_for_auth(reason),
        unresolved_obligations_digest(pre_batch_saga)?,
        manual_policy,
    )
    .map_err(|error| manual_prefix_error(run_id, error.to_string()))
}

fn journal_prefix_digest(records: &[KernelEventEnvelope]) -> Result<ContentDigest> {
    let records = records
        .iter()
        .map(|record| {
            serde_json::json!({
                "commit_key": record.commit_key().as_str(),
                "event_id": record.event_id().as_str(),
                "event_schema_id": record.event_schema_id().as_str(),
                "logical_key": record.logical_key().as_str(),
                "ordinal": record.ordinal().as_u32(),
                "payload_hash": record.payload_hash().as_str(),
                "run_id": record.run_id().as_str(),
                "seq": record.seq().as_u64(),
                "spec_hash": record.spec_hash().as_str(),
            })
        })
        .collect::<Vec<_>>();
    Ok(canonical_json(serde_json::json!({
        "algorithm": JOURNAL_PREFIX_DIGEST_ALGORITHM,
        "events": records,
    }))?
    .content_digest())
}

fn unresolved_obligations_digest(saga: &SagaProjection) -> Result<ContentDigest> {
    let obligations = saga
        .obligations
        .values()
        .filter(|obligation| obligation_is_unresolved(obligation))
        .map(obligation_json)
        .collect::<Vec<_>>();
    Ok(canonical_json(serde_json::json!({
        "algorithm": UNRESOLVED_OBLIGATIONS_DIGEST_ALGORITHM,
        "obligations": obligations,
        "run_id": saga.run_id.as_str(),
    }))?
    .content_digest())
}

fn obligation_is_unresolved(obligation: &SagaObligationProjection) -> bool {
    matches!(
        obligation.classification,
        ForwardLedgerClassification::Owed | ForwardLedgerClassification::Unresolvable
    ) || obligation
        .remediation
        .as_ref()
        .and_then(|remediation| remediation.unresolved)
        .is_some()
}

fn obligation_json(obligation: &SagaObligationProjection) -> serde_json::Value {
    serde_json::json!({
        "classification": forward_classification_str(obligation.classification),
        "forward_ledger_key": obligation.forward_ledger_key.as_str(),
        "forward_phase": side_effect_phase_json(&obligation.forward_phase),
        "remediation": obligation.remediation.as_ref().map(remediation_json),
    })
}

fn remediation_json(remediation: &RemediationLedgerProjection) -> serde_json::Value {
    serde_json::json!({
        "closed": remediation.closed,
        "ledger_key": remediation.ledger_key.as_str(),
        "phase": side_effect_phase_json(&remediation.phase),
        "unresolved": remediation
            .unresolved
            .map(|reason| manual_block_reason_for_auth(reason).as_str()),
    })
}

fn side_effect_phase_json(phase: &SideEffectPhase) -> serde_json::Value {
    let mut value = serde_json::json!({
        "kind": phase.as_str(),
        "invocation_epoch": side_effect_phase_epoch(phase),
    });
    if let SideEffectPhase::Failed { failure_phase, .. } = phase {
        value["failure_phase"] = serde_json::json!(failure_phase_str(*failure_phase));
    }
    value
}

fn side_effect_phase_epoch(phase: &SideEffectPhase) -> u32 {
    match phase {
        SideEffectPhase::IntentPersisted { invocation_epoch }
        | SideEffectPhase::Claimed {
            invocation_epoch, ..
        }
        | SideEffectPhase::InvocationPrepared {
            invocation_epoch, ..
        }
        | SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        }
        | SideEffectPhase::SubmissionObserved { invocation_epoch }
        | SideEffectPhase::NotSubmittedProven { invocation_epoch }
        | SideEffectPhase::SubmissionUnknown { invocation_epoch }
        | SideEffectPhase::ReceiptObserved { invocation_epoch }
        | SideEffectPhase::ConfirmationObserved { invocation_epoch }
        | SideEffectPhase::Ambiguous { invocation_epoch }
        | SideEffectPhase::Failed {
            invocation_epoch, ..
        } => *invocation_epoch,
    }
}

fn forward_classification_str(classification: ForwardLedgerClassification) -> &'static str {
    match classification {
        ForwardLedgerClassification::Pending => "pending",
        ForwardLedgerClassification::NothingOwed => "nothing_owed",
        ForwardLedgerClassification::Owed => "owed",
        ForwardLedgerClassification::Unresolvable => "unresolvable",
    }
}

const fn manual_block_reason_for_auth(
    reason: ManualBlockReason,
) -> mfm_manual_auth::ManualResolutionBlockReason {
    match reason {
        ManualBlockReason::PolicyManualResolution => {
            mfm_manual_auth::ManualResolutionBlockReason::PolicyManualResolution
        }
        ManualBlockReason::ForwardAmbiguous => {
            mfm_manual_auth::ManualResolutionBlockReason::ForwardAmbiguous
        }
        ManualBlockReason::RemediationFailed => {
            mfm_manual_auth::ManualResolutionBlockReason::RemediationFailed
        }
        ManualBlockReason::RemediationAmbiguous => {
            mfm_manual_auth::ManualResolutionBlockReason::RemediationAmbiguous
        }
    }
}

fn manual_prefix_error(run_id: &RunId, message: impl Into<String>) -> StoreError {
    StoreError::ProjectionConflict {
        key: format!("run:{run_id}:manual_resolution_prefix"),
        message: message.into(),
    }
}
