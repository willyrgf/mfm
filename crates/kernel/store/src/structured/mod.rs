//! Qualified immutable history, pure reduction, and sealed mechanical persistence.

use std::sync::Arc;

mod adapter;
mod backend;
mod compiler;
mod configuration;
mod coordinator;
mod fact_scan;
#[cfg(any(test, feature = "test-support"))]
mod memory;
mod obligations;
mod purpose;
mod qualification;
mod reducer;
mod semantic_open;
#[cfg(feature = "test-support")]
pub mod test_support;
#[cfg(all(test, not(feature = "test-support")))]
mod test_support;
mod validated_append;

pub use backend::{
    AppendAttemptLookup, BackendAppendOutcome, EffectEntryAttentionRoute, PhysicalTargetIdentity,
    RawHistoryLoadLimit, RawRunHistory, StructuredBackendFuture, StructuredHistoryBackend,
    StructuredRunSnapshot, StructuredSemanticOpenAudit, StructuredStoreIdentity,
    TenantFactPublication, MAX_RUN_HISTORY_BATCHES, MAX_RUN_HISTORY_CANONICAL_BYTES,
    MAX_RUN_HISTORY_OBJECTS, SEMANTIC_OPEN_KEY_PAGE_ITEMS, SEMANTIC_OPEN_ROUTE_PAGE_ITEMS,
};
pub use configuration::{
    qualify_and_open_configuration_history, ConfigurationAppendRequest,
    ConfigurationBackendAppendOutcome, ConfigurationBackendFuture, ConfigurationHistoryBackend,
    ConfigurationHistoryHead, ConfigurationHistoryReader, ConfigurationHistoryStore,
    ConfigurationHistoryWriter, ConfigurationRevision, ConfigurationRevisionObject,
    ConfigurationStreamKey, MemoryConfigurationHistoryBackend, RawConfigurationHistory,
    VerifiedConfiguredValue, MAX_CONFIGURATION_REVISION_BYTES,
};
#[doc(hidden)]
pub use fact_scan::PriorRunFactScanCompletion;
#[cfg(any(test, feature = "test-support"))]
pub use memory::StructuredMemoryBackend;
pub use qualification::{ProgramVerificationRegistry, StructuredStoreError};
pub use semantic_open::{
    qualify_and_open_structured_store, verify_offline_run_closure, OfflineRunClosure,
    OpenedStructuredStore, StructuredRuntime,
};
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use test_support::{fact_scan_counters, reset_fact_scan_counters, FactScanCounters};
pub use validated_append::{
    RunCurrentProjection, RunProjectionPlan, TenantFactProjectionPlan,
    ValidatedConfigurationAppend, ValidatedRunAppend, MAX_STORED_FRAME_BYTES,
};
// Semantic command types are owned by mfm-runtime; re-export for store tests and reduction.
pub use mfm_runtime::history::CommittedAccessAuthorization;
pub use mfm_runtime::history::{
    AccessAuthorizationProposal, AccessObservationProposal, ProposedCanonicalValue,
    ProposedObservationOutcome, ProposedTransitionValue, StateTransitionProposal,
    StructuredAdmissionMaterial,
};
pub use mfm_runtime::history::{
    ActionableState, EffectEntryAttentionResolution, EffectEntrySubject, LaneCursor, ProgramCursor,
    StateLeaf, StructuredFrontier,
};
pub use purpose::{
    expand_export_source_closure, AuditAccessEntry, AuditObservation, AuditRunEvidence,
    AuditRunReader, EffectEntryAttentionEntry, EffectEntryAttentionPage,
    EffectEntryAttentionReader, ExportEncoderSource, ExportEncoderView, ExportFactRoute,
    ExportRunEvidence, ExportRunReader, ExportSourceClosureError, OfflineVerifiedRun,
    PublicRunEvidence, PublicRunReader, RecordedRunEvidence, ReplayRunReader, RunEvidenceStatus,
    TraceRunEvidence, TraceRunReader, TraceTransitionEntry, MAX_PORTABLE_FACT_ROUTES,
    MAX_PORTABLE_SOURCE_RUNS,
};
pub use qualification::{
    PhysicalBindingAuthorization, PhysicalBindingSupersession, PhysicalObligationChecker,
};

/// Exact-prefix semantic view: immutable qualified evidence plus compact reduction.
pub(crate) struct VerifiedStructuredRun {
    history: Arc<qualification::QualifiedHistory>,
    reduced: Box<reducer::ReducedRunState>,
}

impl std::fmt::Debug for VerifiedStructuredRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedStructuredRun")
            .field("run_id", self.run_id())
            .field("journal_head", self.journal_head())
            .finish_non_exhaustive()
    }
}

impl VerifiedStructuredRun {
    fn from_finalized(
        history: Arc<qualification::QualifiedHistory>,
        finalized: obligations::FinalizedReduction,
    ) -> Self {
        let (reduced, _compiled) = finalized.into_parts();
        Self { history, reduced }
    }

    fn from_reduced(
        history: Arc<qualification::QualifiedHistory>,
        reduced: Box<reducer::ReducedRunState>,
        _seal: coordinator::AppendSeal,
    ) -> Self {
        Self { history, reduced }
    }

    /// Returns the exact run identity.
    pub fn run_id(&self) -> &mfm_ids::RunId {
        self.reduced.run_id()
    }

    /// Returns the exact admission record.
    pub fn admission(&self) -> &mfm_journal::structured::RunAdmitted {
        self.history
            .records()
            .find_map(|assigned| match &assigned.record {
                mfm_journal::structured::RunRecord::RunAdmitted(admission) => Some(admission),
                _ => None,
            })
            .expect("qualified history always contains admission")
    }

    /// Returns the current program cursor.
    pub fn cursor(&self) -> &ProgramCursor {
        self.reduced
            .cursor()
            .expect("verified state always contains a cursor")
    }

    /// Returns the closed current action frontier.
    pub fn frontier(&self) -> &StructuredFrontier {
        self.reduced
            .frontier()
            .expect("verified state always contains a frontier")
    }

    /// Returns the exact immutable journal head.
    pub fn journal_head(&self) -> &mfm_journal::structured::JournalHead {
        self.reduced
            .journal_head()
            .expect("verified state always contains a journal head")
    }

    /// Returns the exact semantic head.
    pub fn semantic_head(&self) -> &mfm_journal::structured::SemanticHead {
        self.reduced
            .semantic_head()
            .expect("verified state always contains a semantic head")
    }

    /// Returns the reducer-owned current attention item.
    pub fn effect_entry_attention(&self) -> Option<&reducer::EffectEntryAttention> {
        self.reduced.effect_entry_attention()
    }

    pub(super) fn current_projection(&self) -> RunCurrentProjection {
        RunCurrentProjection {
            run_id: self.run_id().clone(),
            tenant_scope_id: self.admission().tenant_scope_id.clone(),
            journal_head: self.journal_head().clone(),
            has_effect_entry_attention: self.effect_entry_attention().is_some(),
        }
    }

    /// Resolves one qualified retained object.
    pub fn object(
        &self,
        reference: &mfm_ids::ContentRef,
    ) -> Option<&mfm_journal::structured::HistoryObject> {
        self.history.objects.object(reference)
    }

    /// Resolves an authorization and its assigned identity.
    pub fn authorization(
        &self,
        attempt: &mfm_ids::AccessAttemptId,
    ) -> Option<(
        &mfm_journal::structured::RecordRef,
        &mfm_journal::structured::ExternalAccessAuthorized,
    )> {
        let reference = self.reduced.authorization_ref(attempt)?;
        self.history.records().find_map(|assigned| {
            if &assigned.record_ref != reference {
                return None;
            }
            match &assigned.record {
                mfm_journal::structured::RunRecord::ExternalAccessAuthorized(authorization) => {
                    Some((&assigned.record_ref, authorization))
                }
                _ => None,
            }
        })
    }

    /// Resolves an observation and its assigned identity.
    pub fn observation(
        &self,
        attempt: &mfm_ids::AccessAttemptId,
    ) -> Option<(
        &mfm_journal::structured::RecordRef,
        &mfm_journal::structured::ExternalAccessObserved,
    )> {
        let reference = self.reduced.observation_ref(attempt)?;
        self.history.records().find_map(|assigned| {
            if &assigned.record_ref != reference {
                return None;
            }
            match &assigned.record {
                mfm_journal::structured::RunRecord::ExternalAccessObserved(observation) => {
                    Some((&assigned.record_ref, observation))
                }
                _ => None,
            }
        })
    }

    /// Returns every assigned record in physical order.
    pub fn records(&self) -> impl Iterator<Item = &mfm_journal::structured::AssignedRecord> {
        self.history.records()
    }

    /// Returns every immutable append batch.
    pub fn batches(
        &self,
    ) -> impl ExactSizeIterator<Item = &mfm_journal::structured::CommittedBatch> {
        self.history.batches.iter().map(|batch| &batch.committed)
    }

    /// Returns every physical append head.
    pub fn journal_heads(
        &self,
    ) -> impl ExactSizeIterator<Item = &mfm_journal::structured::JournalHead> {
        self.history
            .batches
            .iter()
            .map(|batch| &batch.committed.head)
    }

    /// Returns the terminal nominal outcome, when closed.
    pub fn closed_outcome_ref(&self) -> Option<&mfm_ids::ContentRef> {
        match self.cursor() {
            ProgramCursor::Closed { outcome_ref } => Some(outcome_ref),
            _ => None,
        }
    }
}

/// Result type for structured RunHistory operations.
pub type Result<T> = std::result::Result<T, StructuredStoreError>;
