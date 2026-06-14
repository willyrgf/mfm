use super::*;

pub(super) fn require_admission_preconditions(
    projections: &ProjectionSnapshot,
    payload: &KernelEventPayload,
    saga_policy: Option<&SagaPolicySpec>,
) -> Result<()> {
    match payload {
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            let policy = require_saga_policy_precondition(&payload.run_id, saga_policy)?;
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
            let policy = require_saga_policy_precondition(&payload.run_id, saga_policy)?;
            projections.require_saga_terminal_outcome_admissible(
                &payload.run_id,
                policy,
                &payload.outcome,
            )
        }
        _ => Ok(()),
    }
}

fn require_saga_policy_precondition<'a>(
    run_id: &RunId,
    saga_policy: Option<&'a SagaPolicySpec>,
) -> Result<&'a SagaPolicySpec> {
    saga_policy.ok_or_else(|| StoreError::ProjectionConflict {
        key: format!("run:{run_id}:saga_policy"),
        message: "certified saga policy precondition is required for saga admission".to_owned(),
    })
}
