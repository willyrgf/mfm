use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, MfmValue as Value, Unsigned256};
use serde::{Deserialize, Serialize};

use super::{
    BalanceCollectionFailure, BalanceCollectionMetadata, BalanceRequest, BalanceSource,
    DecimalScale,
};
use crate::ObservationPoint;

/// Rejection of inconsistent retained collection facts, never an external observation failure.
#[derive(Debug, Serialize, thiserror::Error)]
pub enum BalanceContextError {
    /// Consolidation requires confirmation of every declared source.
    #[error("balance collection has {completed} confirmations for {expected} sources")]
    Incomplete {
        /// Number of confirmed sources.
        completed: usize,
        /// Number of declared sources.
        expected: usize,
    },
    /// The collection has no remaining declared source.
    #[error("balance collection has no active source")]
    Exhausted,
    /// A retained result does not match its declaration position.
    #[error("completed balance does not match request source {index}")]
    Prefix {
        /// Index of the inconsistent completed entry.
        index: usize,
    },
    /// Source and observation point refer to different ledgers.
    #[error("balance source and observation ledgers differ")]
    Ledger,
    /// Completed entries and preparation must share an exact point.
    #[error("balance collection observation points differ")]
    Point,
    /// A stored completed entry could not have passed amount admission.
    #[error("completed balance {index} has invalid amount semantics: {source}")]
    Amount {
        /// Index of the invalid completed entry.
        index: usize,
        /// Exact arithmetic rejection; here it describes malformed retained context.
        #[source]
        source: BalanceCollectionFailure,
    },
}

fn check_point(
    source: &BalanceSource,
    point: &ObservationPoint,
    common: Option<&ObservationPoint>,
) -> Result<(), BalanceContextError> {
    if source.target().ledger() != point.ledger() {
        return Err(BalanceContextError::Ledger);
    }
    if common.is_some_and(|common| common != point) {
        return Err(BalanceContextError::Point);
    }
    Ok(())
}

/// Request, typed caller and exact completed prefix; the next source is derived from prefix length.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct BalanceContext<K: Value> {
    request: BalanceRequest,
    caller: K,
    metadata: BalanceCollectionMetadata,
    completed: Vec<ConfirmedBalance>,
}
impl<K: Value> BalanceContext<K> {
    /// Starts a collection without inventing observed or completed facts.
    pub fn new(request: BalanceRequest, caller: K, metadata: BalanceCollectionMetadata) -> Self {
        Self {
            request,
            caller,
            metadata,
            completed: vec![],
        }
    }
    /// Complete declaration-ordered request.
    pub fn request(&self) -> &BalanceRequest {
        &self.request
    }
    /// Exactly typed caller continuation.
    pub fn caller(&self) -> &K {
        &self.caller
    }
    /// Retained occurrence correlation and route.
    pub fn metadata(&self) -> &BalanceCollectionMetadata {
        &self.metadata
    }
    /// Exact confirmed request prefix; candidates are never included.
    pub fn completed(&self) -> &[ConfirmedBalance] {
        &self.completed
    }
    /// Moves the retained request, caller, metadata and confirmed prefix to its consumer.
    pub fn into_parts(
        self,
    ) -> (
        BalanceRequest,
        K,
        BalanceCollectionMetadata,
        Vec<ConfirmedBalance>,
    ) {
        (self.request, self.caller, self.metadata, self.completed)
    }
    pub(super) fn require_complete(&self) -> Result<(), BalanceContextError> {
        self.request.validate_confirmed(&self.completed)
    }
    /// Selects the next source without a second mutable index.
    pub fn active_source(&self) -> Result<&BalanceSource, BalanceContextError> {
        self.request
            .sources()
            .get(self.completed.len())
            .ok_or(BalanceContextError::Exhausted)
    }
    fn validate(&self) -> Result<(), BalanceContextError> {
        self.request.validate_confirmed_prefix(&self.completed)
    }
}
impl BalanceRequest {
    /// Checks complete confirmed-source coverage, common observation point and amount admission.
    pub fn validate_confirmed(
        &self,
        completed: &[ConfirmedBalance],
    ) -> Result<(), BalanceContextError> {
        self.validate_confirmed_prefix(completed)?;
        if completed.len() != self.sources().len() {
            return Err(BalanceContextError::Incomplete {
                completed: completed.len(),
                expected: self.sources().len(),
            });
        }
        Ok(())
    }
    fn validate_confirmed_prefix(
        &self,
        completed: &[ConfirmedBalance],
    ) -> Result<(), BalanceContextError> {
        let common = completed.first().map(ConfirmedBalance::observed_at);
        for (index, entry) in completed.iter().enumerate() {
            if self.sources().get(index) != Some(&entry.source) {
                return Err(BalanceContextError::Prefix { index });
            }
            check_point(&entry.source, &entry.observed_at, common)?;
            self.scale_units(&entry.raw_units, entry.source_decimals)
                .map_err(|source| BalanceContextError::Amount { index, source })?;
        }
        Ok(())
    }
}
impl<'de, K: Value> Deserialize<'de> for BalanceContext<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K: Value> {
            request: BalanceRequest,
            caller: K,
            metadata: BalanceCollectionMetadata,
            completed: Vec<ConfirmedBalance>,
        }
        let wire = Wire::<K>::deserialize(deserializer)?;
        let value = Self {
            request: wire.request,
            caller: wire.caller,
            metadata: wire.metadata,
            completed: wire.completed,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

/// Active source prepared at an exact point with a checked source scale.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PreparedBalance<K: Value> {
    context: BalanceContext<K>,
    observed_at: ObservationPoint,
    source_decimals: DecimalScale,
}
impl<K: Value> PreparedBalance<K> {
    /// Checks source existence and common ledger/point; native stages qualify asset and scale.
    pub fn new(
        context: BalanceContext<K>,
        observed_at: ObservationPoint,
        source_decimals: DecimalScale,
    ) -> Result<Self, BalanceContextError> {
        let value = Self {
            context,
            observed_at,
            source_decimals,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<&BalanceSource, BalanceContextError> {
        let source = self.context.active_source()?;
        check_point(
            source,
            &self.observed_at,
            self.context
                .completed
                .first()
                .map(ConfirmedBalance::observed_at),
        )?;
        Ok(source)
    }
    /// Complete context before observing this source.
    pub fn context(&self) -> &BalanceContext<K> {
        &self.context
    }
    /// Exact point requested by the designated Read.
    pub fn observed_at(&self) -> &ObservationPoint {
        &self.observed_at
    }
    /// Checked scale established by native preparation.
    pub fn source_decimals(&self) -> DecimalScale {
        self.source_decimals
    }
}
impl<'de, K: Value> Deserialize<'de> for PreparedBalance<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K: Value> {
            context: BalanceContext<K>,
            observed_at: ObservationPoint,
            source_decimals: DecimalScale,
        }
        let wire = Wire::<K>::deserialize(deserializer)?;
        Self::new(wire.context, wire.observed_at, wire.source_decimals)
            .map_err(serde::de::Error::custom)
    }
}

/// Observed raw amount awaiting native anchor confirmation; it is not a completed result.
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "K: serde::de::DeserializeOwned")
)]
pub struct CandidateBalance<K: Value> {
    prepared: PreparedBalance<K>,
    raw_units: Unsigned256,
}
impl<K: Value> CandidateBalance<K> {
    /// Retains the observation without scaling; anchor confirmation must win over arithmetic failure.
    pub fn new(prepared: PreparedBalance<K>, raw_units: Unsigned256) -> Self {
        Self {
            prepared,
            raw_units,
        }
    }
    /// Complete retained preparation needed by native confirmation.
    pub fn prepared(&self) -> &PreparedBalance<K> {
        &self.prepared
    }
    /// Exact raw observation, before amount admission.
    pub fn raw_units(&self) -> &Unsigned256 {
        &self.raw_units
    }
    /// Appends after the native caller has authenticated matching confirmation evidence.
    /// This method checks semantic admission only; it cannot authenticate a native observation.
    /// Outer errors are local invocation failures; inner errors are legitimate arithmetic rejection.
    pub fn append_confirmed(
        self,
    ) -> Result<Result<BalanceContext<K>, BalanceCollectionFailure>, InvocationDiagnostic> {
        let source = self
            .prepared
            .validate()
            .map_err(|source| {
                InvocationDiagnostic::from_fields(
                    "state_internal",
                    "append_confirmed",
                    &source,
                    None,
                )
            })?
            .clone();
        if let Err(failure) = self
            .prepared
            .context
            .request
            .scale_units(&self.raw_units, self.prepared.source_decimals)
        {
            return Ok(Err(failure));
        }
        let PreparedBalance {
            mut context,
            observed_at,
            source_decimals,
        } = self.prepared;
        context.completed.push(ConfirmedBalance {
            source,
            observed_at,
            source_decimals,
            raw_units: self.raw_units,
        });
        Ok(Ok(context))
    }
}

/// One source admitted after native confirmation, without copying its Read evidence history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedBalance {
    source: BalanceSource,
    observed_at: ObservationPoint,
    source_decimals: DecimalScale,
    raw_units: Unsigned256,
}
impl ConfirmedBalance {
    /// Exact source declaration.
    pub fn source(&self) -> &BalanceSource {
        &self.source
    }
    /// Confirmed common observation point.
    pub fn observed_at(&self) -> &ObservationPoint {
        &self.observed_at
    }
    /// Native-qualified source scale.
    pub fn source_decimals(&self) -> DecimalScale {
        self.source_decimals
    }
    /// Original observed amount before scaling.
    pub fn raw_units(&self) -> &Unsigned256 {
        &self.raw_units
    }
}
impl<'de> Deserialize<'de> for ConfirmedBalance {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            source: BalanceSource,
            observed_at: ObservationPoint,
            source_decimals: DecimalScale,
            raw_units: Unsigned256,
        }
        let wire = Wire::deserialize(deserializer)?;
        check_point(&wire.source, &wire.observed_at, None).map_err(serde::de::Error::custom)?;
        Ok(Self {
            source: wire.source,
            observed_at: wire.observed_at,
            source_decimals: wire.source_decimals,
            raw_units: wire.raw_units,
        })
    }
}
