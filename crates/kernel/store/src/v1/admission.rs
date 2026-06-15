use super::*;

pub(super) fn require_admission_preconditions(
    projections: &ProjectionSnapshot,
    payload: &KernelEventPayload,
    saga_admit_token: Option<&SagaAdmitToken>,
) -> Result<()> {
    match payload {
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            let policy = require_saga_admit_token(
                projections,
                &payload.run_id,
                &payload.spec_hash,
                saga_admit_token,
            )?;
            projections.require_manual_resolution_admissible(&payload.run_id, policy)
        }
        KernelEventPayload::RunCompleted(payload)
            if matches!(
                &payload.outcome,
                events::RunCompletionOutcome::Compensated
                    | events::RunCompletionOutcome::ManuallyResolved
                    | events::RunCompletionOutcome::FailedWithoutAcdcClaim
            ) =>
        {
            let policy = require_saga_admit_token(
                projections,
                &payload.run_id,
                &payload.spec_hash,
                saga_admit_token,
            )?;
            projections.require_saga_terminal_outcome_admissible(
                &payload.run_id,
                policy,
                &payload.outcome,
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
) -> Result<&'a SagaPolicySpec> {
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
    Ok(token.saga_policy())
}
