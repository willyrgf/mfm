//! Typed recovery policy contracts and checked authoring descriptors.

use std::marker::PhantomData;

use mfm_ids::{StableId, StatePosition};
use mfm_values::MfmValue;

use crate::{Never, ProgramError, Result, StateExecutionError};

mod bindings;
pub(crate) mod scope;
pub use scope::Checkpoint;
pub(crate) mod bounds;
pub(crate) mod defaults;
pub use bindings::{
    ClassifierAbi, ClassifierBinding, Classifiers, HandlerAbi, HandlerBinding, Handlers,
    IncidentAbi, MapAbi, MapBinding, PolicyParams,
};
pub use bounds::{ConclusionBound, EffectBounds, HistoryBound};
pub use defaults::{NoRecovery, Occurrence, Stop};

/// Original domain failure or an operational adapter error with State-owned context.
pub enum Incident<D, E, X> {
    /// The deterministic State returned its declared failure.
    Domain(D),
    /// The adapter failed before returning accepted evidence.
    Adapter {
        /// Unmodified capability-owned operational cause.
        original: E,
        /// Meaning supplied by the selected deterministic State.
        context: X,
    },
}

mod private {
    pub trait Sealed {}
}

/// Closed structural incident ABI with independently typed components.
pub trait IncidentContract: private::Sealed + Send + Sync + 'static {
    /// State domain failure contract.
    type Domain: MfmValue;
    /// Capability operational error contract.
    type Error: MfmValue;
    /// State adapter context contract.
    type Context: MfmValue;
}

impl<D: MfmValue, E: MfmValue, X: MfmValue> private::Sealed for Incident<D, E, X> {}
impl<D: MfmValue, E: MfmValue, X: MfmValue> IncidentContract for Incident<D, E, X> {
    type Domain = D;
    type Error = E;
    type Context = X;
}

/// Deterministic assessment, separate from the handler's requested action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assessment {
    /// Automatic recovery may be useful.
    Recoverable,
    /// The incident should terminate automatic recovery.
    Nonrecoverable,
}

/// Authoritative execution phase supplied to policy callbacks by Runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionPhase {
    /// Deterministic execution without IO.
    Pure,
    /// Observational execution.
    Read,
    /// An acknowledged command still has settlement authority.
    EffectPending,
    /// Accepted evidence settled the Effect.
    EffectSettled,
}

/// One checked authoring target; it grants no history or append authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryTarget {
    pub(crate) position: StatePosition,
}

impl RecoveryTarget {
    /// Returns the permitted declaration boundary.
    pub const fn position(self) -> StatePosition {
        self.position
    }
}

/// Requested action, which Runtime must independently authorize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryRequest {
    /// Retry this State with its unchanged input.
    RetryState,
    /// Restore an eligible checkpoint's retained input.
    Restart(RecoveryTarget),
    /// Stop automatic recovery.
    Stop,
}

/// Exhausted committed-decision allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryLimit {
    /// This declaration exhausted its retry allowance.
    StateRetry,
    /// This declaration exhausted its restart allowance.
    StateRestart,
    /// The run exhausted its global allowance.
    Run,
}

/// Safety rule that disallowed a requested recovery action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDenial {
    /// Deterministic Pure execution cannot retry identical input.
    PureRetry,
    /// The requested checkpoint is not active and permitted.
    CheckpointUnavailable,
    /// A retained Effect command prevents restoring this checkpoint.
    EffectBarrier,
    /// Settled Effects cannot be retried or restarted.
    EffectSettled,
}

/// Reviewed reason automatic recovery stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The classifier marked the cause nonrecoverable.
    Nonrecoverable,
    /// The handler requested Stop.
    Requested,
    /// A committed-decision allowance was exhausted.
    Exhausted(RecoveryLimit),
    /// A Runtime safety rule denied recovery.
    Disallowed(RecoveryDenial),
}

/// Per-occurrence maxima for committed recovery decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAllowances {
    retries: u32,
    restarts: u32,
}

impl RecoveryAllowances {
    /// Constructs finite per-occurrence allowances. Admission checks their complete history cost.
    pub const fn new(retries: u32, restarts: u32) -> Self {
        Self { retries, restarts }
    }

    /// Returns the retry allowance.
    pub const fn retries(self) -> u32 {
        self.retries
    }

    /// Returns the restart allowance.
    pub const fn restarts(self) -> u32 {
        self.restarts
    }
}

/// Program-wide committed recovery-decision ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramLimits {
    max_recovery_decisions: u32,
}

impl ProgramLimits {
    /// Constructs a finite global allowance. Admission checks the expanded sequence cost.
    pub const fn new(max_recovery_decisions: u32) -> Self {
        Self {
            max_recovery_decisions,
        }
    }

    /// Returns the maximum committed decisions across the run.
    pub const fn max_recovery_decisions(self) -> u32 {
        self.max_recovery_decisions
    }
}

/// Counters reconstructed from committed history, never callback invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryUsage {
    /// Retry decisions committed for the selected declaration.
    pub state_retries: u32,
    /// Restart decisions committed for the selected declaration.
    pub state_restarts: u32,
    /// All recovery decisions committed in the run.
    pub run_decisions: u32,
}

/// Borrowed policy inspection data without execution authority.
pub struct RecoveryContext<'a> {
    phase: ExecutionPhase,
    remaining: RecoveryAllowances,
    remaining_run_decisions: u32,
    eligible: &'a [RecoveryTarget],
}

impl<'a> RecoveryContext<'a> {
    /// Constructs inspection data, including for downstream pure policy tests.
    pub const fn new(
        phase: ExecutionPhase,
        remaining: RecoveryAllowances,
        remaining_run_decisions: u32,
        eligible: &'a [RecoveryTarget],
    ) -> Self {
        Self {
            phase,
            remaining,
            remaining_run_decisions,
            eligible,
        }
    }

    /// Returns the actual execution phase.
    pub const fn phase(&self) -> ExecutionPhase {
        self.phase
    }

    /// Returns remaining per-occurrence decision allowances.
    pub const fn remaining(&self) -> RecoveryAllowances {
        self.remaining
    }

    /// Returns remaining global decisions.
    pub const fn remaining_run_decisions(&self) -> u32 {
        self.remaining_run_decisions
    }

    /// Returns currently active permitted targets in declaration order.
    pub const fn eligible_restart_targets(&self) -> &[RecoveryTarget] {
        self.eligible
    }
}

/// Deterministic typed incident classification.
pub trait Classifier<I: IncidentContract>: Send + Sync + 'static {
    /// Immutable checked configuration bound into Program identity.
    type Params: MfmValue;
    /// Returns this implementation's stable identity.
    fn implementation_id() -> Result<StableId>;
    /// Assesses an incident without performing IO or changing its original cause.
    fn classify(
        params: &Self::Params,
        incident: &I,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<Assessment, StateExecutionError>;
}

/// Deterministic typed recovery selection, independently configurable from classification.
pub trait Handler<I: IncidentContract>: Send + Sync + 'static {
    /// Immutable checked configuration bound into Program identity.
    type Params: MfmValue;
    /// Returns this implementation's stable identity.
    fn implementation_id() -> Result<StableId>;
    /// Proposes an action; Runtime validates every proposal against committed history.
    fn handle(
        params: &Self::Params,
        incident: &I,
        assessment: Assessment,
        context: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, StateExecutionError>;
}

/// Explicit typed consuming conversion for policy context or root domain failure.
pub trait ValueMap: Send + Sync + 'static {
    /// Exact input contract.
    type Input: MfmValue;
    /// Exact output contract.
    type Output: MfmValue;
    /// Immutable checked parameters.
    type Params: MfmValue;
    /// Returns the conversion implementation identity.
    fn implementation_id() -> Result<StableId>;
    /// Converts one value without IO. Original incident retention belongs to Runtime.
    fn apply(
        params: &Self::Params,
        value: Self::Input,
    ) -> std::result::Result<Self::Output, StateExecutionError>;
}

/// Identity conversion without requiring values to implement Clone.
pub struct Identity<T>(PhantomData<T>);

impl<T: MfmValue> ValueMap for Identity<T> {
    type Input = T;
    type Output = T;
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        StableId::new("mfm.recovery.identity@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn apply(_: &NoParams, value: T) -> std::result::Result<T, StateExecutionError> {
        Ok(value)
    }
}

/// Unreachable root conversion for States with no domain failure.
pub struct FromNever<T>(PhantomData<T>);

impl<T: MfmValue> ValueMap for FromNever<T> {
    type Input = Never;
    type Output = T;
    type Params = NoParams;
    fn implementation_id() -> Result<StableId> {
        StableId::new("mfm.recovery.from-never@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn apply(_: &NoParams, value: Never) -> std::result::Result<T, StateExecutionError> {
        match value {}
    }
}

macro_rules! framework_unit {
    ($name:ident, $label:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        pub struct $name;

        impl MfmValue for $name {
            fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
                mfm_ids::SemanticTypeId::new(
                    "mfm.kernel",
                    $label,
                    "1",
                    mfm_ids::DigestAlgorithm::Sha256JcsV1,
                    mfm_canonical::raw_content_digest(
                        concat!("mfm.recovery.", $label, ".v1").as_bytes(),
                    )
                    .digest()
                    .clone(),
                )
                .map_err(|_| mfm_values::ValueError::InvalidSchemaIdentity)
            }

            fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
                mfm_values::framework_value_descriptor(
                    "mfm-program",
                    Self::semantic_id()?,
                    concat!("mfm.kernel.", $label),
                    mfm_values::SchemaShape::Unit,
                    concat!("mfm_program::", stringify!($name)),
                )
            }
        }
    };
}

framework_unit!(
    NoParams,
    "no-params",
    "Checked unit configuration for parameterless policies."
);
framework_unit!(
    NoContext,
    "no-context",
    "Checked unit context for Pure incidents."
);
