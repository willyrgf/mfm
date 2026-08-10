//! Sealed data-only commands accepted by mechanical persistence backends.

use mfm_ids::{AppendRequestId, ContentDigest, RunId, TenantScopeId};
use mfm_journal::structured::{CommittedBatch, JournalHead, TenantFactFrontier};

use super::configuration::{
    ConfigurationHistoryHead, ConfigurationRevisionObject, MAX_CONFIGURATION_REVISION_BYTES,
};
use super::obligations::FinalizedReduction;
use super::qualification::StructuredStoreError;

/// Maximum bytes in one stored canonical frame.
pub const MAX_STORED_FRAME_BYTES: usize = 33_554_432;

/// One run's complete disposable current projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCurrentProjection {
    /// Exact run identity.
    pub run_id: RunId,
    /// Admitted tenant.
    pub tenant_scope_id: TenantScopeId,
    /// Exact immutable journal head.
    pub journal_head: JournalHead,
    /// Reducer-derived Effect attention membership.
    pub has_effect_entry_attention: bool,
}

/// Exact compare-and-replace plan for a current run projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunProjectionPlan {
    expected: Option<RunCurrentProjection>,
    successor: RunCurrentProjection,
}

impl RunProjectionPlan {
    pub(super) fn new(
        expected: Option<RunCurrentProjection>,
        successor: RunCurrentProjection,
    ) -> Self {
        Self {
            expected,
            successor,
        }
    }

    /// Returns the expected row, or absence at admission.
    pub const fn expected(&self) -> Option<&RunCurrentProjection> {
        self.expected.as_ref()
    }

    /// Returns the exact successor row.
    pub const fn successor(&self) -> &RunCurrentProjection {
        &self.successor
    }
}

/// Supplied tenant-fact projection action.
#[derive(Debug, Clone, PartialEq, Eq)]
// Projection commands retain their supplied publication without another allocation.
#[allow(clippy::large_enum_variant)]
pub enum TenantFactProjectionPlan {
    /// No tenant lock or mutation.
    None,
    /// Require an exact frontier without mutation.
    Barrier {
        /// Required current frontier.
        expected_frontier: TenantFactFrontier,
    },
    /// Install one exact next publication.
    Publish {
        /// Required current frontier.
        expected_predecessor: TenantFactFrontier,
        /// Immutable route to install.
        publication: super::backend::TenantFactPublication,
    },
}

/// Store-sealed atomic run append with inseparable projection plans.
pub struct ValidatedRunAppend {
    committed: CommittedBatch,
    run_projection: RunProjectionPlan,
    tenant_fact_plan: TenantFactProjectionPlan,
}

impl std::fmt::Debug for ValidatedRunAppend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedRunAppend")
            .field("head", &self.committed.head)
            .finish_non_exhaustive()
    }
}

impl ValidatedRunAppend {
    pub(super) fn from_finalized(finalized: &FinalizedReduction) -> Self {
        Self {
            committed: finalized.committed.clone(),
            run_projection: finalized.run_projection.clone(),
            tenant_fact_plan: finalized.tenant_fact_plan.clone(),
        }
    }

    /// Returns the target run.
    pub fn run_id(&self) -> &RunId {
        &self.run_projection.successor.run_id
    }

    /// Returns the stable append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.committed.append_request_id
    }

    /// Returns the candidate identity.
    pub const fn candidate_digest(&self) -> &ContentDigest {
        &self.committed.candidate_digest
    }

    /// Consumes the complete inseparable backend command.
    pub fn into_parts<C: mfm_authority_seal::ValidatedAppendConsumerSeal>(
        self,
        _consumer: &C,
    ) -> (CommittedBatch, RunProjectionPlan, TenantFactProjectionPlan) {
        (self.committed, self.run_projection, self.tenant_fact_plan)
    }
}

/// Store-sealed append of one exact typed configuration object.
pub struct ValidatedConfigurationAppend {
    object: ConfigurationRevisionObject,
    expected_head: Option<ConfigurationHistoryHead>,
    successor_head: ConfigurationHistoryHead,
}

impl ValidatedConfigurationAppend {
    pub(super) fn from_object(
        object: ConfigurationRevisionObject,
    ) -> Result<Self, StructuredStoreError> {
        object.revision().validate_for_ingress()?;
        if object.object().canonical_json.len() > MAX_CONFIGURATION_REVISION_BYTES {
            return Err(StructuredStoreError::InvalidHistory);
        }
        let sequence = object.revision().sequence();
        let expected_head = match object.revision().predecessor_ref().cloned() {
            Some(object_ref) => Some(ConfigurationHistoryHead::new(
                sequence
                    .checked_sub(1)
                    .ok_or(StructuredStoreError::InvalidHistory)?,
                object_ref,
            )),
            None => None,
        };
        let successor_head = ConfigurationHistoryHead::new(sequence, object.content_ref().clone());
        Ok(Self {
            object,
            expected_head,
            successor_head,
        })
    }

    /// Consumes the complete inseparable backend command.
    pub fn into_parts<C: mfm_authority_seal::ValidatedAppendConsumerSeal>(
        self,
        _consumer: &C,
    ) -> (
        ConfigurationRevisionObject,
        Option<ConfigurationHistoryHead>,
        ConfigurationHistoryHead,
    ) {
        (self.object, self.expected_head, self.successor_head)
    }
}
