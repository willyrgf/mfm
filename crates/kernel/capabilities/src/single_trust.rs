//! Capability-owned intent/evidence contracts for the final trust boundary.
//!
//! A capability owns one strict intent, one closed evidence value, and one sealed entry mode.
//! Provider bytes never become a public generic response or a retry authority.

use std::marker::PhantomData;
use std::num::NonZeroU16;

use mfm_facts::FactSelectionRequest;
use mfm_ids::StableId;
use mfm_values::MfmValue;

/// Redaction-safe capability contract error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    /// The capability's static contract is inconsistent.
    #[error("capability contract is invalid")]
    InvalidContract,
    /// Evidence did not bind to the exact intent.
    #[error("capability evidence does not bind to intent")]
    EvidenceBinding,
}

/// Result type for the final capability contract.
pub type Result<T> = std::result::Result<T, CapabilityError>;

mod private {
    pub trait AccessModeSealed {}
    pub trait EffectEntryModeSealed {}
    pub trait FactSelectionModeSealed {}
    pub trait EvidenceSealed {}

    impl AccessModeSealed for super::ReadMode {}
    impl<E: super::EffectEntryMode> AccessModeSealed for super::EffectMode<E> {}
    impl EffectEntryModeSealed for super::EntryOnce {}
    impl<const MAX: u16> EffectEntryModeSealed for super::EntryAbsorbing<MAX> {}
    impl FactSelectionModeSealed for super::NoPriorFacts {}
    impl FactSelectionModeSealed for super::PriorRunFacts {}
    impl<T: super::MfmValue> EvidenceSealed for T {}
}

/// Sealed Read/Effect mode.
pub trait AccessMode: private::AccessModeSealed + Send + Sync + 'static {
    /// Validates the declared total-attempt budget for this mode.
    fn validate_total_attempts(total: NonZeroU16) -> Result<()>;
}
impl AccessMode for ReadMode {
    fn validate_total_attempts(total: NonZeroU16) -> Result<()> {
        (total.get() <= 3)
            .then_some(())
            .ok_or(CapabilityError::InvalidContract)
    }
}
impl<E: EffectEntryMode> AccessMode for EffectMode<E> {
    fn validate_total_attempts(total: NonZeroU16) -> Result<()> {
        (total.get() == E::MAX_TOTAL_ENTRIES
            && total.get() <= 3
            && ((E::ABSORBING && total.get() > 1) || (!E::ABSORBING && total.get() == 1)))
            .then_some(())
            .ok_or(CapabilityError::InvalidContract)
    }
}

/// Non-mutating capability mode.
pub struct ReadMode;

/// Mutating capability mode parameterized by its entry discipline.
pub struct EffectMode<E: EffectEntryMode>(PhantomData<fn() -> E>);

/// Sealed Effect entry discipline.
pub trait EffectEntryMode: private::EffectEntryModeSealed + Send + Sync + 'static {
    /// Whether repeated attempts are allowed.
    const ABSORBING: bool;
    /// Maximum total entries including the initial attempt.
    const MAX_TOTAL_ENTRIES: u16;
}

/// An Effect that may enter at most once.
pub struct EntryOnce;
impl EffectEntryMode for EntryOnce {
    const ABSORBING: bool = false;
    const MAX_TOTAL_ENTRIES: u16 = 1;
}

/// An Effect with a proved stable key and post-state convergence bound.
pub struct EntryAbsorbing<const MAX_TOTAL_ENTRIES: u16>;
impl<const MAX_TOTAL_ENTRIES: u16> EffectEntryMode for EntryAbsorbing<MAX_TOTAL_ENTRIES> {
    const ABSORBING: bool = true;
    const MAX_TOTAL_ENTRIES: u16 = MAX_TOTAL_ENTRIES;
}

/// No prior-run fact selection is attached.
pub struct NoPriorFacts;

/// Prior-run fact selection is an interpretation-only product.
pub struct PriorRunFacts;

/// Sealed blanket marker for strict, bounded, secret-free evidence values.
pub trait AccessEvidenceValue: private::EvidenceSealed + MfmValue {}
impl<T: MfmValue> AccessEvidenceValue for T {}

/// Sealed fact-selection mode.
pub trait FactSelectionMode: private::FactSelectionModeSealed + Send + Sync + 'static {
    /// Whether preparation must carry a callback-free fact request.
    const REQUIRED: bool;
}
impl FactSelectionMode for NoPriorFacts {
    const REQUIRED: bool = false;
}
impl FactSelectionMode for PriorRunFacts {
    const REQUIRED: bool = true;
}

/// Capability-owned intent/evidence ABI.
pub trait AccessCapabilityContract: Send + Sync + 'static {
    /// One sealed Read or Effect mode.
    type Mode: AccessMode;
    /// Canonical, secret-free provider intent.
    type Intent: MfmValue;
    /// Closed, strict, secret-free provider evidence.
    type Evidence: AccessEvidenceValue;
    /// Prior-fact behavior.
    type Facts: FactSelectionMode;

    /// Returns the exact capability contract identity.
    fn contract_id() -> Result<StableId>;

    /// Returns the total attempt bound including the initial attempt.
    fn total_attempt_bound() -> NonZeroU16;

    /// Projects the fixed, interpretation-only prior-fact request from canonical intent.
    ///
    /// A capability that uses [`PriorRunFacts`] must override this method. The default keeps
    /// `NoPriorFacts` capabilities total while rejecting an incomplete prior-fact contract.
    fn prior_fact_selection(_intent: &Self::Intent) -> Result<Option<FactSelectionRequest>> {
        if Self::requires_prior_facts() {
            Err(CapabilityError::InvalidContract)
        } else {
            Ok(None)
        }
    }

    /// Validates the mode-specific total-entry discipline.
    fn validate() -> Result<()> {
        Self::Mode::validate_total_attempts(Self::total_attempt_bound())
    }

    /// Returns whether the preparation must fix a prior-fact request.
    fn requires_prior_facts() -> bool {
        Self::Facts::REQUIRED
    }

    /// Binds accepted evidence to the canonical intent and exact call.
    fn bind_evidence(intent: &Self::Intent, evidence: &Self::Evidence) -> Result<()>;
}

/// Typed state outcome; failure carries no ambient context.
#[derive(Debug, PartialEq, Eq)]
pub enum ProposedStateOutcome<O, F> {
    /// Complete successor context or declared root result.
    Success(O),
    /// Typed fail-fast failure.
    Failure(F),
}

/// Cold retained evidence qualified only against one recorded preparation.
pub struct QualifiedRecordedEvidence<C: AccessCapabilityContract> {
    preparation_ref: StableId,
    evidence: C::Evidence,
}

impl<C: AccessCapabilityContract> QualifiedRecordedEvidence<C> {
    /// Constructs cold evidence bound to one retained preparation identity.
    #[allow(dead_code)]
    pub(crate) fn new(preparation_ref: StableId, evidence: C::Evidence) -> Self {
        Self {
            preparation_ref,
            evidence,
        }
    }

    /// Returns the preparation binding.
    pub const fn preparation_ref(&self) -> &StableId {
        &self.preparation_ref
    }

    /// Returns retained evidence.
    pub const fn evidence(&self) -> &C::Evidence {
        &self.evidence
    }
}
