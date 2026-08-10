//! Mechanical structured-history persistence contract.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_ids::{AppendRequestId, RunId, TenantScopeId};
use mfm_journal::structured::{CommittedBatch, JournalHead, RecordRef, TenantFactFrontier};

use super::fact_scan::PriorRunFactSource;
use super::qualification::{
    PhysicalObligationChecker, ProgramVerificationRegistry, StructuredStoreError,
};
use super::validated_append::{RunCurrentProjection, ValidatedRunAppend};
use super::VerifiedStructuredRun;

pub use mfm_runtime::history::{PhysicalTargetIdentity, StructuredStoreIdentity};

/// Boxed asynchronous backend operation.
pub type StructuredBackendFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, StructuredStoreError>> + Send + 'a>>;

/// Complete raw durable run prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRunHistory {
    /// Target run.
    pub run_id: RunId,
    /// Atomic append envelopes in sequence order.
    pub batches: Vec<CommittedBatch>,
}

/// One dense tenant publication route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantFactPublication {
    /// Exact frontier advanced by this publication.
    pub frontier: TenantFactFrontier,
    /// Exact publishing transition.
    pub transition_ref: RecordRef,
    /// Exact immutable producer prefix containing the transition.
    pub producer_head: JournalHead,
}

/// Mechanical append transaction outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendAppendOutcome {
    /// The supplied command committed atomically.
    NewlyCommitted(CommittedBatch),
    /// The stable key already names byte-identical immutable content.
    ExistingSame(CommittedBatch),
    /// The supplied current projection was stale.
    StaleHead,
    /// Commit acknowledgement is unavailable.
    AcknowledgementUnknown,
}

/// One immutable snapshot plus its disposable current projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredRunSnapshot {
    /// Complete retained history, when the run exists.
    pub history: Option<RawRunHistory>,
    /// Current projection observed in the same snapshot.
    pub current_projection: Option<RunCurrentProjection>,
}

/// One run key and both state-bearing surfaces observed in semantic snapshot `S0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredStoreRunSnapshot {
    /// Union key from immutable history and the disposable projection.
    pub run_id: RunId,
    /// Complete immutable history, when present.
    pub history: Option<RawRunHistory>,
    /// Complete current projection, when present.
    pub current_projection: Option<RunCurrentProjection>,
}

/// One tenant's complete disposable fact route and head surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantFactProjectionSnapshot {
    /// Union key from immutable routes and the disposable head.
    pub tenant_scope_id: TenantScopeId,
    /// Positive current head; zero is represented by absence.
    pub current_frontier: Option<TenantFactFrontier>,
    /// Complete immutable publication route in fact order.
    pub publications: Vec<TenantFactPublication>,
}

/// Complete raw physical state captured by one semantic-open snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredStoreSnapshot {
    /// Union of immutable and projection run keys.
    pub runs: Vec<StructuredStoreRunSnapshot>,
    /// Union of immutable fact-route and fact-head tenant keys.
    pub tenant_facts: Vec<TenantFactProjectionSnapshot>,
}

/// Stable append lookup result, ending at the exact retained attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendAttemptLookup {
    /// Immutable prefix through the attempt.
    pub history: RawRunHistory,
}

/// Raw backend. Implementations decode, compare, insert, and CAS only.
pub trait StructuredHistoryBackend: Send + Sync + 'static {
    /// Returns the fixed store identity.
    fn identity(&self) -> &StructuredStoreIdentity;

    /// Loads immutable history and current projection from one snapshot.
    fn load_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, StructuredRunSnapshot>;

    /// Loads one exact immutable prefix without consulting current projection.
    fn load_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through: &'a JournalHead,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>>;

    /// Reads the disposable current run projection.
    fn current_run_projection<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<RunCurrentProjection>>;

    /// Performs stable pre-authoring lookup.
    fn lookup_append_attempt<'a>(
        &'a self,
        run_id: &'a RunId,
        append_request_id: &'a AppendRequestId,
    ) -> StructuredBackendFuture<'a, Option<AppendAttemptLookup>>;

    /// Enumerates the union of immutable and projection run keys in order.
    fn scan_run_ids<'a>(
        &'a self,
        after_run_id: Option<&'a RunId>,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<RunId>>;

    /// Captures every semantic-open decision surface in one physical snapshot.
    fn load_store_snapshot(&self) -> StructuredBackendFuture<'_, StructuredStoreSnapshot>;

    /// Freshly revalidates the target, fence, release, and store identity after `S0`.
    fn validate_authority(&self) -> StructuredBackendFuture<'_, ()>;

    /// Returns the current dense tenant fact frontier.
    fn tenant_fact_frontier<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, TenantFactFrontier>;

    /// Reads one bounded dense publication page.
    fn scan_fact_publications<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>>;

    /// Applies one sealed command atomically.
    fn append<'a>(
        &'a self,
        command: ValidatedRunAppend,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome>;

    /// Scans the current partial Effect-attention route.
    fn scan_effect_entry_attention_routes<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
        after_run_id: Option<&'a RunId>,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<EffectEntryAttentionRoute>>;
}

/// One current attention route. It is a location, never semantic evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntryAttentionRoute {
    /// Routed run.
    pub run_id: RunId,
    /// Exact routed head.
    pub journal_head: JournalHead,
}

struct BackendFactSource<B: StructuredHistoryBackend> {
    backend: Arc<B>,
}

impl<B: StructuredHistoryBackend> PriorRunFactSource for BackendFactSource<B> {
    fn scan_publications<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        self.backend.scan_fact_publications(
            tenant_scope_id,
            first_order,
            through_order,
            maximum_items,
        )
    }

    fn load_producer_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through: &'a JournalHead,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        self.backend.load_prefix(run_id, through)
    }
}

pub(super) fn prior_run_fact_source<B: StructuredHistoryBackend>(
    backend: &Arc<B>,
) -> Arc<dyn PriorRunFactSource> {
    Arc::new(BackendFactSource {
        backend: Arc::clone(backend),
    })
}

/// Non-cloneable mutation authority released only by semantic open.
pub(super) struct StructuredRunHistoryWriter<B: StructuredHistoryBackend> {
    pub(super) backend: Arc<B>,
    pub(super) programs: Arc<ProgramVerificationRegistry>,
    pub(super) physical: Arc<dyn PhysicalObligationChecker>,
}

impl<B: StructuredHistoryBackend> StructuredRunHistoryWriter<B> {
    pub fn store_identity(&self) -> &StructuredStoreIdentity {
        self.backend.identity()
    }

    pub async fn load_verified(&self, run_id: &RunId) -> super::Result<VerifiedStructuredRun> {
        super::semantic_open::load_and_compare(
            &self.backend,
            run_id,
            &self.programs,
            &self.physical,
        )
        .await
    }

    pub(super) async fn commit_event(
        &self,
        previous: Option<VerifiedStructuredRun>,
        intent: mfm_runtime::history::QualifiedRuntimeIntent,
    ) -> super::Result<(RunId, mfm_runtime::history::StructuredAppendAttempt)> {
        super::coordinator::commit_event(self, previous, intent).await
    }
}

/// Cloneable purpose-reader authority released only by semantic open.
pub(super) struct StructuredRunHistoryReader<B: StructuredHistoryBackend> {
    pub(super) backend: Arc<B>,
    pub(super) programs: Arc<ProgramVerificationRegistry>,
    pub(super) physical: Arc<dyn PhysicalObligationChecker>,
}

impl<B: StructuredHistoryBackend> Clone for StructuredRunHistoryReader<B> {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            programs: Arc::clone(&self.programs),
            physical: Arc::clone(&self.physical),
        }
    }
}

impl<B: StructuredHistoryBackend> StructuredRunHistoryReader<B> {
    pub fn store_identity(&self) -> &StructuredStoreIdentity {
        self.backend.identity()
    }

    pub async fn check_ready(&self) -> super::Result<()> {
        let probe = RunId::from_digest(
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.structured-store.readiness-probe.v1"),
        );
        self.backend.load_snapshot(&probe).await.map(|_| ())
    }

    pub(super) async fn scan_effect_entry_attention_routes(
        &self,
        tenant_scope_id: &TenantScopeId,
        after_run_id: Option<&RunId>,
        maximum_items: u32,
    ) -> super::Result<Vec<EffectEntryAttentionRoute>> {
        self.backend
            .scan_effect_entry_attention_routes(tenant_scope_id, after_run_id, maximum_items)
            .await
    }

    pub(super) async fn scan_fact_publications(
        &self,
        tenant_scope_id: &TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> super::Result<Vec<TenantFactPublication>> {
        self.backend
            .scan_fact_publications(tenant_scope_id, first_order, through_order, maximum_items)
            .await
    }

    pub async fn load_verified(&self, run_id: &RunId) -> super::Result<VerifiedStructuredRun> {
        super::semantic_open::load_and_compare(
            &self.backend,
            run_id,
            &self.programs,
            &self.physical,
        )
        .await
    }
}
