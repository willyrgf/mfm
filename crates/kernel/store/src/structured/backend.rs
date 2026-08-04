use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_ids::{AppendRequestId, ContentDigest, RunId, TenantScopeId};
use mfm_journal::structured::{CommittedBatch, JournalHead, RecordRef, TenantFactFrontier};

use super::canonical_append::CanonicalRunAppend;
use super::fact_scan::{verify_actionable_history, PriorRunFactSource};
use super::fold::{
    validate_resolved_batch, ProgramVerifier, StructuredStoreError, VerifiedStructuredRun,
};
use super::qualification::PublicPhysicalBindingVerifier;

pub use mfm_runtime::history::StructuredStoreIdentity;

/// Boxed asynchronous structured-history backend operation.
pub type StructuredBackendFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, StructuredStoreError>> + Send + 'a>>;

/// Complete raw durable prefix loaded from one backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRunHistory {
    /// Target run.
    pub run_id: RunId,
    /// Atomic append envelopes in exact sequence order.
    pub batches: Vec<CommittedBatch>,
}

/// One append-only dense route from a tenant publication order to its transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantFactPublication {
    /// Exact frontier advanced by this publication.
    pub frontier: TenantFactFrontier,
    /// Exact non-empty state transition that published the facts.
    pub transition_ref: RecordRef,
}

/// Result of one backend exact-head transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendAppendOutcome {
    /// This transaction atomically committed the batch.
    NewlyCommitted(CommittedBatch),
    /// The same append identity already committed byte-identical content.
    ExistingSame(CommittedBatch),
    /// The locked run head differs from the exact expected predecessor.
    StaleHead,
    /// Commit acknowledgement is unavailable and must be resolved unchanged.
    AcknowledgementUnknown,
}

/// One immutable backend snapshot containing the retained prefix and its indexed head.
///
/// PostgreSQL implementations return both projections from one `REPEATABLE READ` transaction;
/// callers must never combine a prefix from one snapshot with a head from another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredRunSnapshot {
    /// Complete or bounded retained prefix, when the run exists.
    pub history: Option<RawRunHistory>,
    /// Indexed head observed in the same snapshot.
    pub head: Option<JournalHead>,
}

/// Raw atomic persistence seam behind the one shared structured fold.
///
/// Implementations perform only exact-head locking/CAS, immutable object and
/// record persistence, logical append-id resolution, and atomic visibility.
/// They receive no callback, cursor policy, or domain hook.
pub trait StructuredHistoryBackend: Send + Sync + 'static {
    /// Returns the immutable qualified writer identity bound to this backend.
    fn identity(&self) -> &StructuredStoreIdentity;

    /// Loads the complete immutable prefix and objects for one run.
    fn load<'a>(&'a self, run_id: &'a RunId) -> StructuredBackendFuture<'a, Option<RawRunHistory>>;

    /// Loads retained history and its indexed head from one backend snapshot.
    fn load_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, StructuredRunSnapshot> {
        Box::pin(async move {
            let history = self.load(run_id).await?;
            let head = self.current_head(run_id).await?;
            Ok(StructuredRunSnapshot { history, head })
        })
    }

    /// Returns the exact current physical head without loading the retained prefix.
    ///
    /// Every backend supplies this projection directly. Production PostgreSQL reads its indexed
    /// head row; conformance backends read their equivalent exact-head state.
    fn current_head<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<JournalHead>>;

    /// Loads the immutable run prefix ending at one exact retained sequence.
    fn load_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through_sequence: u64,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            if through_sequence == 0 {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let Some(mut raw) = self.load(run_id).await? else {
                return Ok(None);
            };
            raw.batches
                .retain(|batch| batch.head.run_sequence <= through_sequence);
            Ok((!raw.batches.is_empty()).then_some(raw))
        })
    }

    /// Returns the current dense tenant fact frontier under this store identity.
    fn tenant_fact_frontier<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, TenantFactFrontier>;

    /// Reads one bounded dense page of append-only tenant publication routes.
    fn scan_fact_publications<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>>;

    /// Atomically compare-and-appends one shared-fold-validated batch.
    fn append<'a>(
        &'a self,
        batch: CanonicalRunAppend,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome>;

    /// Resolves one unchanged physical append identity after ambiguity.
    fn resolve_append<'a>(
        &'a self,
        run_id: &'a RunId,
        append_request_id: &'a AppendRequestId,
        candidate_digest: &'a ContentDigest,
    ) -> StructuredBackendFuture<'a, Option<CommittedBatch>>;
}

struct BackendPriorRunFactSource<B: StructuredHistoryBackend> {
    backend: Arc<B>,
}

impl<B: StructuredHistoryBackend> PriorRunFactSource for BackendPriorRunFactSource<B> {
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
        through_sequence: u64,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        self.backend.load_prefix(run_id, through_sequence)
    }
}

pub(super) fn prior_run_fact_source<B: StructuredHistoryBackend>(
    backend: &Arc<B>,
) -> Arc<dyn PriorRunFactSource> {
    Arc::new(BackendPriorRunFactSource {
        backend: Arc::clone(backend),
    })
}

/// Private one-shot qualified store assembly before writer/read authority separation.
pub(super) struct StructuredRunStore<B: StructuredHistoryBackend> {
    backend: Arc<B>,
    program_verifier: Arc<dyn ProgramVerifier>,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
}

impl<B: StructuredHistoryBackend> StructuredRunStore<B> {
    pub(super) fn new(
        backend: B,
        program_verifier: Arc<dyn ProgramVerifier>,
        physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
    ) -> Self {
        Self {
            backend: Arc::new(backend),
            program_verifier,
            physical_binding_verifier,
        }
    }

    pub(super) fn split(self) -> (StructuredRunHistoryWriter<B>, StructuredRunHistoryReader<B>) {
        (
            StructuredRunHistoryWriter {
                backend: Arc::clone(&self.backend),
                program_verifier: Arc::clone(&self.program_verifier),
                physical_binding_verifier: Arc::clone(&self.physical_binding_verifier),
            },
            StructuredRunHistoryReader {
                backend: self.backend,
                program_verifier: self.program_verifier,
                physical_binding_verifier: self.physical_binding_verifier,
            },
        )
    }
}

/// Private non-cloneable structured RunHistory mutation authority.
pub(super) struct StructuredRunHistoryWriter<B: StructuredHistoryBackend> {
    pub(super) backend: Arc<B>,
    pub(super) program_verifier: Arc<dyn ProgramVerifier>,
    pub(super) physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
}

impl<B: StructuredHistoryBackend> StructuredRunHistoryWriter<B> {
    /// Returns the immutable qualified store identity owned by this writer.
    pub fn store_identity(&self) -> &StructuredStoreIdentity {
        self.backend.identity()
    }

    /// Returns the exact current physical head without loading retained history.
    pub async fn current_head(&self, run_id: &RunId) -> super::Result<Option<JournalHead>> {
        self.backend.current_head(run_id).await
    }

    /// Loads and callback-free verifies one exact run for a Runtime action.
    pub async fn load_verified(&self, run_id: &RunId) -> super::Result<VerifiedStructuredRun> {
        let snapshot = self.backend.load_snapshot(run_id).await?;
        let raw = snapshot.history.ok_or(StructuredStoreError::RunNotFound)?;
        verify_actionable_history(
            prior_run_fact_source(&self.backend),
            raw,
            Arc::clone(&self.program_verifier),
            Arc::clone(&self.physical_binding_verifier),
        )
        .await
        .and_then(|verified| {
            if snapshot.head.as_ref() == Some(verified.journal_head()) {
                Ok(verified)
            } else {
                Err(StructuredStoreError::InvalidHistory)
            }
        })
    }

    /// Resolves an unchanged append identity after acknowledgement ambiguity.
    pub async fn resolve_append(
        &self,
        run_id: &RunId,
        append_request_id: &AppendRequestId,
        candidate_digest: &ContentDigest,
    ) -> super::Result<Option<CommittedBatch>> {
        let resolved = self
            .backend
            .resolve_append(run_id, append_request_id, candidate_digest)
            .await?;
        let Some(batch) = resolved else {
            return Ok(None);
        };
        validate_resolved_batch(
            run_id,
            append_request_id,
            candidate_digest,
            self.backend.identity(),
            &batch,
        )?;
        Ok(Some(batch))
    }
}

/// Private complete-history reader used only to build sealed purpose readers.
pub(super) struct StructuredRunHistoryReader<B: StructuredHistoryBackend> {
    backend: Arc<B>,
    program_verifier: Arc<dyn ProgramVerifier>,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
}

impl<B: StructuredHistoryBackend> Clone for StructuredRunHistoryReader<B> {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            program_verifier: Arc::clone(&self.program_verifier),
            physical_binding_verifier: Arc::clone(&self.physical_binding_verifier),
        }
    }
}

impl<B: StructuredHistoryBackend> StructuredRunHistoryReader<B> {
    /// Returns the immutable qualified store identity.
    pub fn store_identity(&self) -> &StructuredStoreIdentity {
        self.backend.identity()
    }

    /// Probes backend readability without requiring an existing application run.
    pub async fn check_ready(&self) -> super::Result<()> {
        let probe = RunId::from_digest(
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.structured-store.readiness-probe.v1"),
        );
        self.backend.load(&probe).await.map(|_| ())
    }

    /// Loads and callback-free verifies one recorded run without live IO.
    pub async fn load_verified(&self, run_id: &RunId) -> super::Result<VerifiedStructuredRun> {
        let snapshot = self.backend.load_snapshot(run_id).await?;
        let raw = snapshot.history.ok_or(StructuredStoreError::RunNotFound)?;
        verify_actionable_history(
            prior_run_fact_source(&self.backend),
            raw,
            Arc::clone(&self.program_verifier),
            Arc::clone(&self.physical_binding_verifier),
        )
        .await
        .and_then(|verified| {
            if snapshot.head.as_ref() == Some(verified.journal_head()) {
                Ok(verified)
            } else {
                Err(StructuredStoreError::InvalidHistory)
            }
        })
    }
}
