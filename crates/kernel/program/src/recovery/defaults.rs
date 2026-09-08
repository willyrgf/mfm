use super::{scope::ScopeId, *};

/// Framework classification default: automatic recovery is disabled.
pub struct NoRecovery;

impl<I: IncidentContract> Classifier<I> for NoRecovery {
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        StableId::new("mfm.recovery.no-recovery@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn classify(
        _: &NoParams,
        _: &I,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<Assessment, StateExecutionError> {
        Ok(Assessment::Nonrecoverable)
    }
}

/// Generic handler that stops automatic recovery without manufacturing a domain failure.
pub struct Stop;

impl Stop {
    pub(crate) fn id() -> Result<StableId> {
        StableId::new("mfm.recovery.stop@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl<I: IncidentContract> Handler<I> for Stop {
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        Self::id()
    }
    fn handle(
        _: &NoParams,
        _: &I,
        _: Assessment,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError> {
        Ok(RecoveryRequest::Stop)
    }
}

/// Independent authoring overrides for one occurrence. Missing settings inherit.
#[derive(Clone, Debug, Default)]
pub struct Occurrence {
    classifiers: Option<Classifiers>,
    handlers: Option<Handlers>,
    retries: Option<u32>,
    restarts: Option<u32>,
}

impl Occurrence {
    /// Inherits every nearest explicit Operation setting.
    pub fn new() -> Self {
        Self::default()
    }
    /// Replaces only this occurrence's classifier family.
    pub fn classifiers(mut self, family: Classifiers) -> Self {
        self.classifiers = Some(family);
        self
    }
    /// Replaces only this occurrence's handler family; installation checks checkpoint ownership.
    pub fn handlers(mut self, family: Handlers) -> Self {
        self.handlers = Some(family);
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
    pub(crate) classifiers: Option<Classifiers>,
    pub(crate) handlers: Option<Handlers>,
    pub(crate) allowances: RecoveryAllowances,
}

pub(crate) struct SelectedRecovery {
    pub(crate) classifier: ClassifierBinding,
    pub(crate) handler: HandlerBinding,
    pub(crate) allowances: RecoveryAllowances,
    pub(crate) checkpoints: Vec<super::scope::ScopedBoundary>,
}

impl RecoveryDefaults {
    pub(crate) fn resolve<D: MfmValue, E: MfmValue, X: MfmValue>(
        &self,
        scope: &ScopeId,
        occurrence: &Occurrence,
    ) -> Result<SelectedRecovery> {
        if let Some(handlers) = &occurrence.handlers {
            handlers.require_scope(scope)?;
        }
        let source = IncidentAbi::of::<Incident<D, E, X>>()?;
        let classifier = match occurrence
            .classifiers
            .as_ref()
            .or(self.classifiers.as_ref())
        {
            Some(family) => family.binding(&source)?.clone(),
            None => {
                let mut family = Classifiers::new();
                family.bind::<E, Identity<D>, Identity<X>, NoRecovery>(
                    NoParams, NoParams, NoParams,
                )?;
                family.binding(&source)?.clone()
            }
        };
        let (handler, checkpoints) = match occurrence.handlers.as_ref().or(self.handlers.as_ref()) {
            Some(family) => (
                family.binding(classifier.abi().mapped())?.clone(),
                family.checkpoints(classifier.abi().mapped())?.to_vec(),
            ),
            None => (
                HandlerBinding::stop(classifier.abi().mapped().clone())?,
                Vec::new(),
            ),
        };
        Ok(SelectedRecovery {
            classifier,
            handler,
            checkpoints,
            allowances: RecoveryAllowances::new(
                occurrence.retries.unwrap_or(self.allowances.retries()),
                occurrence.restarts.unwrap_or(self.allowances.restarts()),
            ),
        })
    }
}
