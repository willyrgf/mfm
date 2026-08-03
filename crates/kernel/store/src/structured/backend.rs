use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_ids::{AppendRequestId, ContentDigest, RunId, TenantScopeId};
use mfm_journal::structured::{CommittedBatch, JournalHead, RecordRef, TenantFactFrontier};

use super::fact_scan::{verify_actionable_history, PriorRunFactSource};
use super::fold::{validate_resolved_batch, ProgramVerifier, StructuredStoreError, VerifiedStructuredRun};
use super::mutation::StructuredAppendAttempt;
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

/// Store-validated batch accepted by the narrow backend seam.
///
/// Fields are private so callers cannot bypass the shared fold. Concrete
/// backends may inspect the exact committed envelope only through the
/// purpose-limited accessor.
pub struct ValidatedBatch {
    committed: CommittedBatch,
}

impl std::fmt::Debug for ValidatedBatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedBatch")
            .field("run_id", &self.run_id())
            .field("head", &self.committed.head)
            .finish_non_exhaustive()
    }
}

impl ValidatedBatch {
    pub(super) const fn new(committed: CommittedBatch) -> Self {
        Self { committed }
    }

    /// Returns the target run.
    pub fn run_id(&self) -> &RunId {
        &self.committed.records[0].record_ref.run_id
    }

    /// Returns the exact expected predecessor.
    pub const fn predecessor(&self) -> Option<&JournalHead> {
        self.committed.predecessor.as_ref()
    }

    /// Returns the stable append acknowledgement identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.committed.append_request_id
    }

    /// Returns the complete candidate digest used for idempotency.
    pub const fn candidate_digest(&self) -> &ContentDigest {
        &self.committed.candidate_digest
    }

    /// Returns the exact store-validated assigned envelope.
    #[doc(hidden)]
    pub const fn committed(&self) -> &CommittedBatch {
        &self.committed
    }

    /// Consumes the validation proof into the exact assigned envelope.
    #[doc(hidden)]
    pub fn into_committed(self) -> CommittedBatch {
        self.committed
    }
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
        batch: ValidatedBatch,
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

    /// Loads and callback-free verifies one exact run for a Runtime action.
    pub async fn load_verified(&self, run_id: &RunId) -> super::Result<VerifiedStructuredRun> {
        let raw = self
            .backend
            .load(run_id)
            .await?
            .ok_or(StructuredStoreError::RunNotFound)?;
        verify_actionable_history(
            prior_run_fact_source(&self.backend),
            raw,
            Arc::clone(&self.program_verifier),
            Arc::clone(&self.physical_binding_verifier),
        )
        .await
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

    /// Resolves one unchanged attempt and accepts only its exact retained
    /// store-validated candidate.
    pub async fn resolve_attempt(
        &self,
        attempt: &mut StructuredAppendAttempt,
    ) -> super::Result<bool> {
        let candidate = attempt.candidate();
        let run_id = candidate
            .records
            .first()
            .ok_or(StructuredStoreError::InvalidHistory)?
            .record_ref
            .run_id
            .clone();
        let append_request_id = attempt.append_request_id().clone();
        let candidate_digest = attempt.candidate_digest().clone();
        let resolved = self
            .resolve_append(&run_id, &append_request_id, &candidate_digest)
            .await?;
        match resolved {
            Some(batch) if batch == *attempt.candidate() => {
                attempt.confirm_existing_same();
                Ok(true)
            }
            Some(_) => Err(StructuredStoreError::InvalidHistory),
            None => Ok(false),
        }
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
        let raw = self
            .backend
            .load(run_id)
            .await?
            .ok_or(StructuredStoreError::RunNotFound)?;
        verify_actionable_history(
            prior_run_fact_source(&self.backend),
            raw,
            Arc::clone(&self.program_verifier),
            Arc::clone(&self.physical_binding_verifier),
        )
        .await
    }
}
