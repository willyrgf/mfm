use mfm_canonical::CanonicalValue;

/// Conservative claim about whether an authorized external boundary was entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NonDomainEntryStatus {
    /// Affine authority custody proves that boundary entry did not occur.
    ProvenNotEntered,
    /// Boundary entry may have occurred.
    MayHaveEntered,
}

impl NonDomainEntryStatus {
    /// Returns the frozen recoverability-v3 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProvenNotEntered => "proven_not_entered",
            Self::MayHaveEntered => "may_have_entered",
        }
    }

    /// Parses the frozen recoverability-v3 spelling.
    pub fn parse(value: &str) -> Result<Self, NonDomainFailureError> {
        match value {
            "proven_not_entered" => Ok(Self::ProvenNotEntered),
            "may_have_entered" => Ok(Self::MayHaveEntered),
            _ => Err(NonDomainFailureError::InvalidValue),
        }
    }
}

/// Closed operational disposition of an audit-only failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NonDomainDisposition {
    /// A later drive may perform a separately authorized attempt.
    RetryableOperational,
    /// The verified run must remain blocked on an integrity condition.
    IntegrityBlocked,
}

impl NonDomainDisposition {
    /// Returns the frozen recoverability-v3 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RetryableOperational => "retryable_operational",
            Self::IntegrityBlocked => "integrity_blocked",
        }
    }

    /// Parses the frozen recoverability-v3 spelling.
    pub fn parse(value: &str) -> Result<Self, NonDomainFailureError> {
        match value {
            "retryable_operational" => Ok(Self::RetryableOperational),
            "integrity_blocked" => Ok(Self::IntegrityBlocked),
            _ => Err(NonDomainFailureError::InvalidValue),
        }
    }
}

/// Closed redaction-safe reason for a surviving non-domain failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NonDomainFailureCode {
    /// A registered adapter violated its reviewed boundary contract.
    AdapterContractViolation,
    /// A returned value could not be encoded under its frozen result contract.
    ResultEncodingFailure,
    /// The authoritative fact store was operationally unavailable.
    FactStoreUnavailable,
    /// Fact history or a store-authored fact proof was invalid.
    FactHistoryInvalid,
    /// The durable executor store was operationally unavailable.
    ExecutorStoreUnavailable,
    /// A definite executor concurrency conflict requires a new authorization.
    ExecutorContention,
    /// Durable executor history or proof material was invalid.
    ExecutorHistoryInvalid,
    /// The executor could not reserve the bounded evidence it would need.
    ExecutorCapacityExhausted,
}

impl NonDomainFailureCode {
    /// Returns the frozen recoverability-v3 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdapterContractViolation => "adapter_contract_violation",
            Self::ResultEncodingFailure => "result_encoding_failure",
            Self::FactStoreUnavailable => "fact_store_unavailable",
            Self::FactHistoryInvalid => "fact_history_invalid",
            Self::ExecutorStoreUnavailable => "executor_store_unavailable",
            Self::ExecutorContention => "executor_contention",
            Self::ExecutorHistoryInvalid => "executor_history_invalid",
            Self::ExecutorCapacityExhausted => "executor_capacity_exhausted",
        }
    }

    /// Parses the frozen recoverability-v3 spelling.
    pub fn parse(value: &str) -> Result<Self, NonDomainFailureError> {
        match value {
            "adapter_contract_violation" => Ok(Self::AdapterContractViolation),
            "result_encoding_failure" => Ok(Self::ResultEncodingFailure),
            "fact_store_unavailable" => Ok(Self::FactStoreUnavailable),
            "fact_history_invalid" => Ok(Self::FactHistoryInvalid),
            "executor_store_unavailable" => Ok(Self::ExecutorStoreUnavailable),
            "executor_contention" => Ok(Self::ExecutorContention),
            "executor_history_invalid" => Ok(Self::ExecutorHistoryInvalid),
            "executor_capacity_exhausted" => Ok(Self::ExecutorCapacityExhausted),
            _ => Err(NonDomainFailureError::InvalidValue),
        }
    }
}

/// Context in which a non-domain failure was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NonDomainFailureLayer {
    /// Registered immutable-read adapter.
    Read,
    /// Reserved fact-selection scan.
    Fact,
    /// Runtime-facing durable executor ensure.
    Ensure,
    /// Executor-internal target operation.
    ExecutorTarget,
}

/// Audit-only failure that state logic must never consume as domain evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonDomainFailure {
    entry_status: NonDomainEntryStatus,
    disposition: NonDomainDisposition,
    code: NonDomainFailureCode,
}

impl NonDomainFailure {
    /// Constructs one globally valid code/status/disposition relation.
    pub fn new(
        entry_status: NonDomainEntryStatus,
        disposition: NonDomainDisposition,
        code: NonDomainFailureCode,
    ) -> Result<Self, NonDomainFailureError> {
        let failure = Self {
            entry_status,
            disposition,
            code,
        };
        failure.validate_relation()?;
        Ok(failure)
    }

    /// Returns the conservative external-boundary entry status.
    pub const fn entry_status(self) -> NonDomainEntryStatus {
        self.entry_status
    }

    /// Returns the fixed retry-or-block disposition.
    pub const fn disposition(self) -> NonDomainDisposition {
        self.disposition
    }

    /// Returns the closed redaction-safe failure code.
    pub const fn code(self) -> NonDomainFailureCode {
        self.code
    }

    /// Returns all fields of the closed relation.
    pub const fn fields(self) -> NonDomainFailureFields {
        NonDomainFailureFields {
            entry_status: self.entry_status,
            disposition: self.disposition,
            code: self.code,
        }
    }

    /// Projects this closed relation into its generic canonical value.
    ///
    /// This conversion assigns no persisted schema. A persistence owner must
    /// embed the returned value in its own schema or validated wrapper.
    pub fn canonical_value(&self) -> mfm_canonical::Result<CanonicalValue> {
        CanonicalValue::object([
            (
                "entry_status",
                CanonicalValue::String(self.entry_status.as_str().to_owned()),
            ),
            (
                "disposition",
                CanonicalValue::String(self.disposition.as_str().to_owned()),
            ),
            (
                "code",
                CanonicalValue::String(self.code.as_str().to_owned()),
            ),
        ])
    }

    /// Reconstructs the closed relation from an exact generic canonical value.
    ///
    /// The object must contain exactly `entry_status`, `disposition`, and
    /// `code` string fields. This conversion validates no persisted schema.
    pub fn from_canonical_value(value: &CanonicalValue) -> Result<Self, NonDomainFailureError> {
        let CanonicalValue::Object(object) = value else {
            return Err(NonDomainFailureError::InvalidValue);
        };
        let mut entry_status = None;
        let mut disposition = None;
        let mut code = None;
        for (name, value) in object.entries() {
            let CanonicalValue::String(value) = value else {
                return Err(NonDomainFailureError::InvalidValue);
            };
            match name {
                "entry_status" => entry_status = Some(NonDomainEntryStatus::parse(value)?),
                "disposition" => disposition = Some(NonDomainDisposition::parse(value)?),
                "code" => code = Some(NonDomainFailureCode::parse(value)?),
                _ => return Err(NonDomainFailureError::InvalidValue),
            }
        }
        Self::new(
            entry_status.ok_or(NonDomainFailureError::InvalidValue)?,
            disposition.ok_or(NonDomainFailureError::InvalidValue)?,
            code.ok_or(NonDomainFailureError::InvalidValue)?,
        )
    }

    /// Validates that this failure code is legal at the owning access layer.
    pub fn validate_layer(self, layer: NonDomainFailureLayer) -> Result<(), NonDomainFailureError> {
        let valid = match self.code {
            NonDomainFailureCode::AdapterContractViolation
            | NonDomainFailureCode::ResultEncodingFailure => true,
            NonDomainFailureCode::FactStoreUnavailable
            | NonDomainFailureCode::FactHistoryInvalid => layer == NonDomainFailureLayer::Fact,
            NonDomainFailureCode::ExecutorStoreUnavailable
            | NonDomainFailureCode::ExecutorContention
            | NonDomainFailureCode::ExecutorHistoryInvalid
            | NonDomainFailureCode::ExecutorCapacityExhausted => {
                layer == NonDomainFailureLayer::Ensure
            }
        };
        if valid {
            Ok(())
        } else {
            Err(NonDomainFailureError::LayerMismatch)
        }
    }

    fn validate_relation(self) -> Result<(), NonDomainFailureError> {
        use NonDomainDisposition::{IntegrityBlocked, RetryableOperational};
        use NonDomainEntryStatus::{MayHaveEntered, ProvenNotEntered};
        use NonDomainFailureCode::{
            AdapterContractViolation, ExecutorCapacityExhausted, ExecutorContention,
            ExecutorHistoryInvalid, ExecutorStoreUnavailable, FactHistoryInvalid,
            FactStoreUnavailable, ResultEncodingFailure,
        };

        let valid = match self.code {
            AdapterContractViolation | ResultEncodingFailure => {
                self.disposition == IntegrityBlocked
            }
            FactStoreUnavailable => {
                self.entry_status == MayHaveEntered && self.disposition == RetryableOperational
            }
            FactHistoryInvalid => {
                self.entry_status == MayHaveEntered && self.disposition == IntegrityBlocked
            }
            ExecutorStoreUnavailable | ExecutorContention => {
                self.disposition == RetryableOperational
            }
            ExecutorHistoryInvalid => self.disposition == IntegrityBlocked,
            ExecutorCapacityExhausted => {
                self.entry_status == ProvenNotEntered && self.disposition == IntegrityBlocked
            }
        };
        if valid {
            Ok(())
        } else {
            Err(NonDomainFailureError::InvalidRelation)
        }
    }
}

/// Typed fields of one closed audit-only non-domain failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonDomainFailureFields {
    /// Conservative external-boundary entry status.
    pub entry_status: NonDomainEntryStatus,
    /// Fixed retry-or-block projection.
    pub disposition: NonDomainDisposition,
    /// Closed redaction-safe failure code.
    pub code: NonDomainFailureCode,
}

/// Failure to construct or project a closed non-domain failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NonDomainFailureError {
    /// A persisted spelling is outside the frozen taxonomy.
    #[error("non-domain failure contains an unknown closed value")]
    InvalidValue,
    /// Status, disposition, and code violate the frozen global relation.
    #[error("non-domain failure relation is invalid")]
    InvalidRelation,
    /// The failure code is not legal at the requested access layer.
    #[error("non-domain failure code is invalid for this access layer")]
    LayerMismatch,
}
