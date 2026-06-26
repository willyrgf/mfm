use super::*;

pub(super) fn require_admission_preconditions(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    payload: &KernelEventPayload,
    saga_admit_token: Option<&SagaAdmitToken>,
) -> Result<()> {
    match payload {
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            let token = require_saga_admit_token(
                projections,
                &payload.run_id,
                &payload.spec_hash,
                saga_admit_token,
            )?;
            projections.require_manual_resolution_admissible(
                &payload.run_id,
                token.saga_policy(),
                token.terminal_policies(),
            )
        }
        KernelEventPayload::RunCompleted(payload) if saga_admit_token.is_some() => {
            let token = require_saga_admit_token(
                projections,
                &payload.run_id,
                &payload.spec_hash,
                saga_admit_token,
            )?;
            let projected_outcome = projections.saga_terminal_completion_outcome(
                &payload.run_id,
                token.saga_policy(),
                token.terminal_policies(),
            )?;
            if projected_outcome != payload.outcome {
                return Err(StoreError::ProjectionConflict {
                    key: format!("run:{}:saga_terminal", payload.run_id),
                    message: "RunCompleted outcome does not match current saga projection"
                        .to_owned(),
                });
            }
            Ok(())
        }
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if !matches!(
                payload.ledger_purpose,
                events::SideEffectLedgerPurpose::Remediation { .. }
            ) {
                return Ok(());
            }
            let token = require_saga_admit_token(
                projections,
                run_id,
                &payload.spec_hash,
                saga_admit_token,
            )?;
            require_remediation_intent_admissible(
                projections,
                run_id,
                &payload.ledger_key,
                &payload.ledger_purpose,
                token.terminal_policies(),
            )
        }
        _ => Ok(()),
    }
}

fn require_saga_admit_token<'a>(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    spec_hash: &SpecHash,
    saga_admit_token: Option<&'a SagaAdmitToken>,
) -> Result<&'a SagaAdmitToken> {
    let token = saga_admit_token.ok_or_else(|| StoreError::ProjectionConflict {
        key: format!("run:{run_id}:saga_policy"),
        message: "saga admission token is required for saga admission".to_owned(),
    })?;
    if token.run_id() != run_id || token.spec_hash() != spec_hash {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "saga admission token run or spec hash does not match payload".to_owned(),
        });
    }
    let Some(projected_spec_hash) = projections.run_spec_hash(run_id) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "run-start spec hash is not projected".to_owned(),
        });
    };
    if projected_spec_hash != token.spec_hash() {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "saga admission token spec hash does not match run start".to_owned(),
        });
    }
    let Some(projected_digest) = projections.saga_policy_digest(run_id) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "run-start saga policy digest is not projected".to_owned(),
        });
    };
    if projected_digest != token.saga_policy_digest() {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:saga_policy"),
            message: "saga admission token digest does not match run start".to_owned(),
        });
    }
    Ok(token)
}
