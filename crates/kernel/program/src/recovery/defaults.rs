use super::{scope::ScopeId, *};

/// Generic handler that stops automatic recovery without manufacturing a domain failure.
pub struct Stop;

impl Handler for Stop {
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        StableId::new("mfm.recovery.stop@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        _: &IncidentSummary,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(RecoveryRequest::Stop)
    }
}

/// Explicitly selected generic recovery policy; the framework default remains Stop.
pub struct StandardRecovery;

impl Handler for StandardRecovery {
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        StableId::new("mfm.recovery.standard@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        incident: &IncidentSummary,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(match (incident.classification, context.phase()) {
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

/// Independent authoring overrides for one occurrence. Missing settings inherit.
#[derive(Clone, Debug, Default)]
pub struct Occurrence {
    handler: Option<HandlerBinding>,
    retries: Option<u32>,
    restarts: Option<u32>,
}

impl Occurrence {
    /// Inherits every nearest explicit Operation setting.
    pub fn new() -> Self {
        Self::default()
    }
    /// Replaces the handler, parameters and targets together.
    pub fn handler(mut self, binding: HandlerBinding) -> Self {
        self.handler = Some(binding);
        self
    }
    /// Replaces the retry allowance, including explicit zero.
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = Some(retries);
        self
    }
    /// Replaces the restart allowance, including explicit zero.
    pub fn restarts(mut self, restarts: u32) -> Self {
        self.restarts = Some(restarts);
        self
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecoveryDefaults {
    pub(crate) handler: Option<HandlerBinding>,
    pub(crate) allowances: RecoveryAllowances,
}

pub(crate) struct SelectedRecovery {
    pub(crate) handler: HandlerBinding,
    pub(crate) allowances: RecoveryAllowances,
    pub(crate) checkpoints: Vec<super::scope::ScopedBoundary>,
}

impl RecoveryDefaults {
    pub(crate) fn resolve(
        &self,
        scope: &ScopeId,
        occurrence: &Occurrence,
    ) -> Result<SelectedRecovery> {
        if let Some(handler) = &occurrence.handler {
            handler.require_scope(scope)?;
        }
        let mut handler = match occurrence.handler.as_ref().or(self.handler.as_ref()) {
            Some(binding) => binding.clone(),
            None => HandlerBinding::new::<Stop>(NoParams)?,
        };
        let checkpoints = std::mem::take(&mut handler.checkpoints);
        Ok(SelectedRecovery {
            handler,
            checkpoints,
            allowances: RecoveryAllowances::new(
                occurrence.retries.unwrap_or(self.allowances.retries()),
                occurrence.restarts.unwrap_or(self.allowances.restarts()),
            ),
        })
    }
}
