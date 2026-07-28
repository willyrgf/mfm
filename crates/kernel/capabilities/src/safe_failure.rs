use mfm_ids::ContentRef;

/// Closed coarse classification shared by reviewed capability failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureClass {
    /// Authorization or routing authority was unavailable or rejected.
    Authorization,
    /// Certified configuration was unavailable or invalid.
    Configuration,
    /// The reviewed request was invalid.
    Request,
    /// Cancellation occurred at the capability boundary.
    Cancellation,
    /// Transport entry or observation failed.
    Transport,
    /// The destination returned a reviewed semantic rejection.
    Destination,
    /// A response could not be represented safely.
    UnrepresentableResponse,
    /// Integrity validation failed.
    Integrity,
    /// A typed external resource conflicted.
    ResourceConflict,
    /// No more specific safe class can be retained.
    Unclassified,
}

impl FailureClass {
    /// Returns the frozen recoverability-v1 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::Configuration => "configuration",
            Self::Request => "request",
            Self::Cancellation => "cancellation",
            Self::Transport => "transport",
            Self::Destination => "destination",
            Self::UnrepresentableResponse => "unrepresentable_response",
            Self::Integrity => "integrity",
            Self::ResourceConflict => "resource_conflict",
            Self::Unclassified => "unclassified",
        }
    }
}

/// Reviewed stage at which a capability failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BoundaryStage {
    /// Failure occurred before entering the independently meaningful operation.
    BeforeBoundaryEntry,
    /// Failure occurred while entering the independently meaningful operation.
    BoundaryEntry,
    /// Failure occurred while observing an operation whose identity was known.
    BoundaryObservation,
}

impl BoundaryStage {
    /// Returns the frozen recoverability-v1 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BeforeBoundaryEntry => "before_boundary_entry",
            Self::BoundaryEntry => "boundary_entry",
            Self::BoundaryObservation => "boundary_observation",
        }
    }
}

/// Reviewed coarse byte-size class for safe diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoarseSizeClass {
    /// No bytes were present.
    Zero,
    /// At most 16 KiB were present.
    UpTo16Kib,
    /// More than 16 KiB and at most 1 MiB were present.
    UpTo1Mib,
    /// More than 1 MiB were present.
    Over1Mib,
}

impl CoarseSizeClass {
    /// Returns the frozen recoverability-v1 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::UpTo16Kib => "up_to_16_kib",
            Self::UpTo1Mib => "up_to_1_mib",
            Self::Over1Mib => "over_1_mib",
        }
    }
}

/// Closed failure-code contract selected by one admitted capability binding.
pub trait SafeFailureCode: Clone + Eq {
    /// Returns the stable code persisted by the selected contract.
    fn as_str(&self) -> &'static str;

    /// Validates the exact class, stage, size, and diagnostic-presence tuple.
    fn accepts(
        &self,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size_class: Option<CoarseSizeClass>,
        has_diagnostic: bool,
    ) -> bool;
}

/// Error returned when a reviewed failure tuple violates its selected contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SafeFailureError {
    /// The stable code rejected the supplied closed tuple.
    #[error("safe failure tuple is not admitted by its contract")]
    InvalidTuple,
}

/// Generic redaction-safe failure envelope shared by live capability boundaries.
///
/// The selected contract reference fixes the closed code set, allowed tuple
/// combinations, and any diagnostic schema. This value contains no provider
/// message, body, URL, path, credential, or debug output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeFailure<Code, DiagnosticRef = ContentRef> {
    safe_failure_contract_ref: ContentRef,
    stable_code: Code,
    failure_class: FailureClass,
    boundary_stage: BoundaryStage,
    coarse_size_class: Option<CoarseSizeClass>,
    diagnostic_ref: Option<DiagnosticRef>,
}

impl<Code, DiagnosticRef> SafeFailure<Code, DiagnosticRef>
where
    Code: SafeFailureCode,
{
    /// Constructs an envelope after validating the selected closed tuple.
    pub fn new(
        safe_failure_contract_ref: ContentRef,
        stable_code: Code,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size_class: Option<CoarseSizeClass>,
        diagnostic_ref: Option<DiagnosticRef>,
    ) -> Result<Self, SafeFailureError> {
        if !stable_code.accepts(
            failure_class,
            boundary_stage,
            coarse_size_class,
            diagnostic_ref.is_some(),
        ) {
            return Err(SafeFailureError::InvalidTuple);
        }
        Ok(Self {
            safe_failure_contract_ref,
            stable_code,
            failure_class,
            boundary_stage,
            coarse_size_class,
            diagnostic_ref,
        })
    }

    /// Returns the exact selected safe-failure contract.
    pub const fn safe_failure_contract_ref(&self) -> &ContentRef {
        &self.safe_failure_contract_ref
    }

    /// Returns the stable closed code.
    pub const fn stable_code(&self) -> &Code {
        &self.stable_code
    }

    /// Returns the reviewed failure class.
    pub const fn failure_class(&self) -> FailureClass {
        self.failure_class
    }

    /// Returns the reviewed boundary stage.
    pub const fn boundary_stage(&self) -> BoundaryStage {
        self.boundary_stage
    }

    /// Returns the optional reviewed coarse size.
    pub const fn coarse_size_class(&self) -> Option<CoarseSizeClass> {
        self.coarse_size_class
    }

    /// Returns the optional reviewed diagnostic reference.
    pub const fn diagnostic_ref(&self) -> Option<&DiagnosticRef> {
        self.diagnostic_ref.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use mfm_canonical::{CanonicalValue, RecoverabilityContractV1};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TestCode {
        Unavailable,
    }

    impl SafeFailureCode for TestCode {
        fn as_str(&self) -> &'static str {
            "unavailable"
        }

        fn accepts(
            &self,
            failure_class: FailureClass,
            boundary_stage: BoundaryStage,
            coarse_size_class: Option<CoarseSizeClass>,
            has_diagnostic: bool,
        ) -> bool {
            failure_class == FailureClass::Transport
                && boundary_stage == BoundaryStage::BeforeBoundaryEntry
                && coarse_size_class.is_none()
                && !has_diagnostic
        }
    }

    fn reviewed_ref() -> ContentRef {
        let contract = RecoverabilityContractV1::embedded().expect("contract");
        let value = contract
            .encode(
                "mfm.primitive-stable_id.v1",
                &CanonicalValue::String("test.safe-failure".to_owned()),
            )
            .expect("reviewed value");
        contract.content_ref(&value).expect("content ref")
    }

    #[test]
    fn selected_code_controls_the_complete_safe_tuple() {
        assert!(SafeFailure::<TestCode>::new(
            reviewed_ref(),
            TestCode::Unavailable,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
            None,
            None,
        )
        .is_ok());
        assert_eq!(
            SafeFailure::<TestCode>::new(
                reviewed_ref(),
                TestCode::Unavailable,
                FailureClass::Transport,
                BoundaryStage::BoundaryEntry,
                None,
                None,
            )
            .expect_err("unreviewed stage"),
            SafeFailureError::InvalidTuple
        );
    }
}
