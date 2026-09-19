use super::*;

/// Generic handler that stops automatic recovery without manufacturing a domain failure.
pub struct Stop;

impl Handler for Stop {
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        Ok(StableId::new("mfm.recovery.stop@1")?)
    }
    fn handle(
        _: &NoParams,
        _: Classification,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, mfm_values::InvocationDiagnostic> {
        Ok(RecoveryRequest::Stop)
    }
}

/// Explicitly selected generic recovery policy; the framework default remains Stop.
pub struct StandardRecovery;

impl Handler for StandardRecovery {
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        Ok(StableId::new("mfm.recovery.standard@1")?)
    }
    fn handle(
        _: &NoParams,
        classification: Classification,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, mfm_values::InvocationDiagnostic> {
        Ok(match (classification, context.phase()) {
            (Classification::Retryable, ExecutionPhase::Read | ExecutionPhase::EffectPending) => {
                RecoveryRequest::RetryState
            }
            (Classification::InputInvalidated, ExecutionPhase::Pure | ExecutionPhase::Read) => {
                context
                    .single_restart_target()
                    .map(RecoveryRequest::Restart)
                    .unwrap_or(RecoveryRequest::Stop)
            }
            _ => RecoveryRequest::Stop,
        })
    }
}
