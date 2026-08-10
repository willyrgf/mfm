use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mfm_facts::{
    prior_run_fact_selector_contract_ref, FactCandidate, FactSelectionReadFailure,
    FactSelectionReadFailureCode, FactSelectionReadResponse, FactSelectionRequest, FactSubject,
    FactTopK,
};
use mfm_ids::{RunId, StableId};
use mfm_journal::structured::{
    canonical_json, derive_fact_content_identity, derive_fact_logical_identity, CommittedBatch,
    JournalHead, ObservationOutcome, PriorRunFactCompletenessMode, PriorRunFactQueryResult,
    PriorRunFactScanAttestation, PriorRunFactSelectionResponse, PriorRunFactSourceManifest,
    RecordRef, RunAdmitted, RunRecord, SelectedPriorRunFact, TenantFactCoordinate,
    TenantFactFrontier,
};
use mfm_values::CanonicalJsonPersistedSchema;

use super::backend::{
    RawHistoryLoadLimit, RawRunHistory, StructuredBackendFuture, TenantFactPublication,
    MAX_RUN_HISTORY_OBJECTS,
};
use super::compiler::FactScanPermitSpec;
use super::qualification::{
    PhysicalObligationChecker, ProgramVerificationRegistry, StructuredStoreError,
};
#[cfg(feature = "test-support")]
use super::test_support::{
    count_fact_scan_fold_batches, count_fact_scan_invocation, count_fact_scan_producer_prefix_load,
    count_fact_scan_publication_page, record_fact_scan_maxima,
};
use super::VerifiedStructuredRun;

const SCAN_PAGE_ITEMS: u32 = 1_024;

/// Store-internal purpose-only authority available to the retained fact scanner.
///
/// This port deliberately excludes append, append resolution, generic run loads,
/// current-frontier reads, and every non-fact query surface.
pub(super) trait PriorRunFactSource: Send + Sync {
    /// Reads one bounded dense page of tenant fact-publication routes.
    fn scan_publications<'a>(
        &'a self,
        tenant_scope_id: &'a mfm_ids::TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>>;

    /// Reads one producer prefix ending at the exact routed journal head.
    fn load_producer_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through: &'a JournalHead,
        limit: RawHistoryLoadLimit,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>>;
}

/// Closed completion produced only by the store-owned prior-run fact scanner.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorRunFactScanCompletion {
    /// Complete typed response through the authorization frontier.
    Returned(FactSelectionReadResponse),
    /// Reviewed definite bounded-read failure.
    SafeFailure(FactSelectionReadFailure),
    /// Stable redaction-safe integrity fault.
    IntegrityFault(StableId),
}

trait PriorRunFactScanPort: Send {
    fn invoke(
        self: Box<Self>,
        request: FactSelectionRequest,
    ) -> Pin<Box<dyn Future<Output = PriorRunFactScanCompletion> + Send + 'static>>;
}

pub(super) struct FactScanPermit {
    port: Box<dyn PriorRunFactScanPort>,
}

impl std::fmt::Debug for FactScanPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FactScanPermit")
            .finish_non_exhaustive()
    }
}

impl FactScanPermit {
    pub(super) fn invoke(
        self,
        request: FactSelectionRequest,
    ) -> Pin<Box<dyn Future<Output = PriorRunFactScanCompletion> + Send + 'static>> {
        self.port.invoke(request)
    }
}

pub(super) fn build_committed_fact_read_capability(
    source: Arc<dyn PriorRunFactSource>,
    program_verifier: Arc<ProgramVerificationRegistry>,
    physical_binding_verifier: Arc<dyn PhysicalObligationChecker>,
    spec: Option<FactScanPermitSpec>,
    committed: &CommittedBatch,
    successor: &VerifiedStructuredRun,
) -> super::Result<Option<FactScanPermit>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let TenantFactCoordinate::FactSelectionBarrier { frontier } = &committed.tenant_fact_coordinate
    else {
        return Ok(None);
    };
    let [assigned] = committed.records.as_slice() else {
        return Err(invalid());
    };
    let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
        return Err(invalid());
    };
    if spec.authorization_ref != assigned.record_ref || spec.request_ref != authorization.request {
        return Err(invalid());
    }
    let request_object = successor
        .object(&authorization.request.value_ref)
        .ok_or_else(invalid)?;
    let expected_request =
        FactSelectionRequest::from_canonical_json(request_object.canonical_json.as_bytes())
            .map_err(|_| invalid())?;
    let source_object = successor
        .object(
            &successor
                .admission()
                .admission_material_refs
                .prior_run_source_manifest_ref,
        )
        .ok_or_else(invalid)?;
    let source_manifest = source_object
        .decode_persisted::<PriorRunFactSourceManifest>()
        .map_err(|_| invalid())?;
    let integrity_fault_code =
        StableId::new("prior-run-fact-scan-integrity-fault").map_err(|_| invalid())?;
    Ok(Some(FactScanPermit {
        port: Box::new(BackendFactScanPort {
            source,
            program_verifier,
            physical_binding_verifier,
            prefix_memo: Arc::new(PrefixVerificationMemo::default()),
            consumer_run_id: successor.run_id().clone(),
            consumer_admission: successor.admission().clone(),
            source_manifest,
            expected_request,
            authorization_ref: assigned.record_ref.clone(),
            physical_binding_ref: authorization.physical_binding_ref.clone(),
            frontier: frontier.clone(),
            integrity_fault_code,
        }),
    }))
}

const fn invalid() -> StructuredStoreError {
    StructuredStoreError::InvalidHistory
}

struct BackendFactScanPort {
    source: Arc<dyn PriorRunFactSource>,
    program_verifier: Arc<ProgramVerificationRegistry>,
    physical_binding_verifier: Arc<dyn PhysicalObligationChecker>,
    prefix_memo: Arc<PrefixVerificationMemo>,
    consumer_run_id: RunId,
    consumer_admission: RunAdmitted,
    source_manifest: PriorRunFactSourceManifest,
    expected_request: FactSelectionRequest,
    authorization_ref: RecordRef,
    physical_binding_ref: mfm_ids::ContentRef,
    frontier: TenantFactFrontier,
    integrity_fault_code: StableId,
}

/// One bounded recursive producer-verification session.  A session owns the
/// only producer cache used by a fact read, so shared producers are reduced at
/// most once and an active producer cannot be re-entered through a cycle.
struct FactScanSession {
    #[cfg(feature = "test-support")]
    counter_scope: String,
    prefixes: Vec<PrefixMemo>,
    shared_prefixes: Arc<PrefixVerificationMemo>,
    producer_history_bytes: u64,
    fold_work: u64,
    maximum_distinct_producers: u64,
    maximum_retained_source_bytes: u64,
    maximum_producer_fold_batches: u64,
    #[cfg(feature = "test-support")]
    publication_pages: u64,
    #[cfg(feature = "test-support")]
    producer_prefix_loads: u64,
    #[cfg(feature = "test-support")]
    fold_batches: u64,
}

enum PrefixMemoState {
    Visiting,
    Verified(Arc<VerifiedStructuredRun>),
}

#[derive(Default)]
pub(super) struct PrefixVerificationMemo {
    entries: Mutex<Vec<SharedPrefixMemo>>,
}

struct SharedPrefixMemo {
    run_id: RunId,
    head: JournalHead,
    state: SharedPrefixMemoState,
}

enum SharedPrefixMemoState {
    Visiting,
    Verified {
        run: Arc<VerifiedStructuredRun>,
        retained_bytes: u64,
        fold_batches: u64,
    },
}

enum SharedPrefixLookup {
    Missing,
    Visiting,
    Verified {
        run: Arc<VerifiedStructuredRun>,
        retained_bytes: u64,
        fold_batches: u64,
    },
}

impl PrefixVerificationMemo {
    fn lookup(&self, run_id: &RunId, head: &JournalHead) -> ScanResult<SharedPrefixLookup> {
        let entries = self.entries.lock().map_err(|_| ScanError::Integrity)?;
        Ok(entries
            .iter()
            .find(|entry| &entry.run_id == run_id && &entry.head == head)
            .map_or(SharedPrefixLookup::Missing, |entry| match &entry.state {
                SharedPrefixMemoState::Visiting => SharedPrefixLookup::Visiting,
                SharedPrefixMemoState::Verified {
                    run,
                    retained_bytes,
                    fold_batches,
                } => SharedPrefixLookup::Verified {
                    run: Arc::clone(run),
                    retained_bytes: *retained_bytes,
                    fold_batches: *fold_batches,
                },
            }))
    }

    fn begin(&self, run_id: RunId, head: JournalHead) -> ScanResult<()> {
        let mut entries = self.entries.lock().map_err(|_| ScanError::Integrity)?;
        if entries
            .iter()
            .any(|entry| entry.run_id == run_id && entry.head == head)
        {
            return Err(ScanError::Integrity);
        }
        entries.push(SharedPrefixMemo {
            run_id,
            head,
            state: SharedPrefixMemoState::Visiting,
        });
        Ok(())
    }

    fn finish(
        &self,
        run_id: &RunId,
        head: &JournalHead,
        run: Arc<VerifiedStructuredRun>,
        retained_bytes: u64,
        fold_batches: u64,
    ) -> ScanResult<()> {
        let mut entries = self.entries.lock().map_err(|_| ScanError::Integrity)?;
        let entry = entries
            .iter_mut()
            .find(|entry| &entry.run_id == run_id && &entry.head == head)
            .ok_or(ScanError::Integrity)?;
        if !matches!(entry.state, SharedPrefixMemoState::Visiting) {
            return Err(ScanError::Integrity);
        }
        entry.state = SharedPrefixMemoState::Verified {
            run,
            retained_bytes,
            fold_batches,
        };
        Ok(())
    }

    fn abandon(&self, run_id: &RunId, head: &JournalHead) -> ScanResult<()> {
        let mut entries = self.entries.lock().map_err(|_| ScanError::Integrity)?;
        let position = entries
            .iter()
            .position(|entry| &entry.run_id == run_id && &entry.head == head)
            .ok_or(ScanError::Integrity)?;
        entries.remove(position);
        Ok(())
    }

    pub(super) fn verified_prefixes(&self) -> super::Result<Vec<Arc<VerifiedStructuredRun>>> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| StructuredStoreError::InvalidHistory)?;
        if entries
            .iter()
            .any(|entry| matches!(entry.state, SharedPrefixMemoState::Visiting))
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
        entries
            .iter()
            .map(|entry| match &entry.state {
                SharedPrefixMemoState::Verified { run, .. }
                    if run.run_id() == &entry.run_id && run.journal_head() == &entry.head =>
                {
                    Ok(Arc::clone(run))
                }
                SharedPrefixMemoState::Visiting | SharedPrefixMemoState::Verified { .. } => {
                    Err(StructuredStoreError::InvalidHistory)
                }
            })
            .collect()
    }
}

struct PrefixMemo {
    run_id: RunId,
    head: JournalHead,
    state: PrefixMemoState,
}

impl FactScanSession {
    fn new(
        bounds: &mfm_facts::FactSelectionScanBounds,
        shared_prefixes: Arc<PrefixVerificationMemo>,
        #[cfg(feature = "test-support")] counter_scope: String,
    ) -> Self {
        Self {
            #[cfg(feature = "test-support")]
            counter_scope,
            prefixes: Vec::new(),
            shared_prefixes,
            producer_history_bytes: 0,
            fold_work: 0,
            maximum_distinct_producers: bounds.maximum_distinct_producers(),
            maximum_retained_source_bytes: bounds.maximum_retained_source_bytes(),
            maximum_producer_fold_batches: bounds.maximum_producer_fold_batches(),
            #[cfg(feature = "test-support")]
            publication_pages: 0,
            #[cfg(feature = "test-support")]
            producer_prefix_loads: 0,
            #[cfg(feature = "test-support")]
            fold_batches: 0,
        }
    }
}

impl PriorRunFactScanPort for BackendFactScanPort {
    fn invoke(
        self: Box<Self>,
        request: FactSelectionRequest,
    ) -> Pin<Box<dyn Future<Output = PriorRunFactScanCompletion> + Send + 'static>> {
        Box::pin(async move {
            match self.scan(request).await {
                Ok(response) => PriorRunFactScanCompletion::Returned(response),
                Err(ScanError::Safe(code)) => {
                    PriorRunFactScanCompletion::SafeFailure(FactSelectionReadFailure::new(code))
                }
                Err(ScanError::Unavailable) => PriorRunFactScanCompletion::SafeFailure(
                    FactSelectionReadFailure::new(FactSelectionReadFailureCode::StoreUnavailable),
                ),
                Err(ScanError::Integrity) => {
                    PriorRunFactScanCompletion::IntegrityFault(self.integrity_fault_code.clone())
                }
            }
        })
    }
}

enum ScanError {
    Safe(FactSelectionReadFailureCode),
    Unavailable,
    Integrity,
}

fn classify_backend_scan_error(error: StructuredStoreError) -> ScanError {
    match error {
        StructuredStoreError::BackendUnavailable => ScanError::Unavailable,
        _ => ScanError::Integrity,
    }
}

type ScanResult<T> = std::result::Result<T, ScanError>;

fn charge_prefix_work(
    session: &mut FactScanSession,
    retained_bytes: u64,
    fold_batches: u64,
) -> ScanResult<()> {
    session.producer_history_bytes = session
        .producer_history_bytes
        .checked_add(retained_bytes)
        .ok_or(ScanError::Integrity)?;
    if session.producer_history_bytes > session.maximum_retained_source_bytes {
        return Err(ScanError::Safe(
            FactSelectionReadFailureCode::RetainedSourceBoundExceeded,
        ));
    }
    session.fold_work = session
        .fold_work
        .checked_add(fold_batches)
        .ok_or(ScanError::Integrity)?;
    if session.fold_work > session.maximum_producer_fold_batches {
        return Err(ScanError::Safe(
            FactSelectionReadFailureCode::PublicationBoundExceeded,
        ));
    }
    Ok(())
}

impl BackendFactScanPort {
    async fn scan(&self, request: FactSelectionRequest) -> ScanResult<FactSelectionReadResponse> {
        let bounds = request.scan_bounds();
        #[cfg(feature = "test-support")]
        let counter_scope = self.consumer_admission.store_scope_id.as_str().to_owned();
        let mut session = FactScanSession::new(
            bounds,
            Arc::clone(&self.prefix_memo),
            #[cfg(feature = "test-support")]
            counter_scope.clone(),
        );
        #[cfg(feature = "test-support")]
        count_fact_scan_invocation(&counter_scope);
        let result = self.scan_with_session(request, &mut session).await;
        #[cfg(feature = "test-support")]
        record_fact_scan_maxima(
            &counter_scope,
            session.publication_pages,
            session.producer_prefix_loads,
            session.fold_batches,
        );
        result
    }

    fn scan_with_session<'a>(
        &'a self,
        request: FactSelectionRequest,
        session: &'a mut FactScanSession,
    ) -> Pin<Box<dyn Future<Output = ScanResult<FactSelectionReadResponse>> + Send + 'a>> {
        Box::pin(async move {
            if request != self.expected_request
                || request.admitted_source_manifest_ref()
                    != &self
                        .consumer_admission
                        .admission_material_refs
                        .prior_run_source_manifest_ref
                || request.selector_contract_ref()
                    != &prior_run_fact_selector_contract_ref().map_err(|_| ScanError::Integrity)?
            {
                return Err(ScanError::Integrity);
            }
            let bounds = request.scan_bounds();
            if self.frontier.fact_order > bounds.maximum_publications() {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::PublicationBoundExceeded,
                ));
            }
            let queries = request.queries();
            let mut accumulators = queries
                .iter()
                .map(|query| {
                    let query_ref = query.content_ref().map_err(|_| ScanError::Integrity)?;
                    Ok((query_ref, FactTopK::new(query.clone())))
                })
                .collect::<ScanResult<Vec<_>>>()?;
            let mut fact_count = 0_u64;
            let mut retained_source_bytes = 0_u64;
            let mut next_order = 1_u64;
            let mut page_count = 0_u64;
            let mut publications_by_order = Vec::new();
            while next_order <= self.frontier.fact_order {
                page_count = page_count.checked_add(1).ok_or(ScanError::Integrity)?;
                if page_count > bounds.maximum_pages() {
                    return Err(ScanError::Safe(
                        FactSelectionReadFailureCode::PublicationBoundExceeded,
                    ));
                }
                let publications = self
                    .source
                    .scan_publications(
                        &self.consumer_admission.tenant_scope_id,
                        next_order,
                        self.frontier.fact_order,
                        SCAN_PAGE_ITEMS,
                    )
                    .await
                    .map_err(classify_backend_scan_error)?;
                #[cfg(feature = "test-support")]
                {
                    session.publication_pages = session
                        .publication_pages
                        .checked_add(1)
                        .ok_or(ScanError::Integrity)?;
                    count_fact_scan_publication_page(&session.counter_scope);
                }
                if publications.is_empty()
                    || publications.len()
                        > usize::try_from(SCAN_PAGE_ITEMS).map_err(|_| ScanError::Integrity)?
                {
                    return Err(ScanError::Integrity);
                }
                for publication in publications {
                    if publication.frontier.store_scope_id != self.frontier.store_scope_id
                        || publication.frontier.store_epoch != self.frontier.store_epoch
                        || publication.frontier.tenant_scope_id != self.frontier.tenant_scope_id
                        || publication.frontier.fact_order != next_order
                        || publication.frontier.fact_order > self.frontier.fact_order
                    {
                        return Err(ScanError::Integrity);
                    }
                    publications_by_order.push(publication);
                    next_order = next_order.checked_add(1).ok_or(ScanError::Integrity)?;
                }
            }

            if u64::try_from(publications_by_order.len()).map_err(|_| ScanError::Integrity)?
                > bounds.maximum_publications()
            {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::PublicationBoundExceeded,
                ));
            }
            // Determine one maximum required prefix per producer before any producer load. This
            // keeps recursive/shared routes linear in distinct producers instead of repeatedly
            // reducing a growing prefix for every publication.
            let mut producer_heads = BTreeMap::<RunId, JournalHead>::new();
            for publication in &publications_by_order {
                if publication.producer_head.run_sequence != publication.transition_ref.run_sequence
                {
                    return Err(ScanError::Integrity);
                }
                let producer = publication.transition_ref.run_id.clone();
                match producer_heads.get_mut(&producer) {
                    Some(head) if head.run_sequence < publication.producer_head.run_sequence => {
                        *head = publication.producer_head.clone();
                    }
                    Some(head)
                        if head.run_sequence == publication.producer_head.run_sequence
                            && head != &publication.producer_head =>
                    {
                        return Err(ScanError::Integrity);
                    }
                    Some(_) => {}
                    None => {
                        producer_heads.insert(producer, publication.producer_head.clone());
                    }
                }
            }
            if u64::try_from(producer_heads.len()).map_err(|_| ScanError::Integrity)?
                > bounds.maximum_distinct_producers()
            {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::PublicationBoundExceeded,
                ));
            }
            let mut producers = BTreeMap::new();
            for (producer, through) in producer_heads {
                let verified = self.load_producer(&producer, &through, session).await?;
                producers.insert(producer, verified);
            }
            for publication in &publications_by_order {
                let producer = producers
                    .get(&publication.transition_ref.run_id)
                    .ok_or(ScanError::Integrity)?;
                self.scan_publication(
                    publication,
                    producer,
                    &mut accumulators,
                    &mut fact_count,
                    &mut retained_source_bytes,
                    bounds.maximum_facts(),
                    bounds.maximum_retained_source_bytes(),
                )
                .await?;
            }

            let mut selected_count = 0_u64;
            let mut response_source_bytes = 0_u64;
            let mut query_results = Vec::with_capacity(accumulators.len());
            for (query_ordinal, (query_ref, accumulator)) in accumulators.into_iter().enumerate() {
                let selected = accumulator
                    .finish()
                    .into_iter()
                    .map(FactCandidate::into_value)
                    .collect::<Vec<_>>();
                selected_count = selected_count
                    .checked_add(u64::try_from(selected.len()).map_err(|_| ScanError::Integrity)?)
                    .ok_or(ScanError::Integrity)?;
                if selected_count > bounds.maximum_selected_results() {
                    return Err(ScanError::Safe(
                        FactSelectionReadFailureCode::SelectedResultBoundExceeded,
                    ));
                }
                for fact in &selected {
                    let subject_bytes = u64::try_from(fact.subject_canonical_json.len())
                        .map_err(|_| ScanError::Integrity)?;
                    let response_bytes = u64::try_from(fact.response_canonical_json.len())
                        .map_err(|_| ScanError::Integrity)?;
                    let claim_bytes = u64::try_from(fact.claim_canonical_json.len())
                        .map_err(|_| ScanError::Integrity)?;
                    response_source_bytes = response_source_bytes
                        .checked_add(subject_bytes)
                        .and_then(|total| total.checked_add(response_bytes))
                        .and_then(|total| total.checked_add(claim_bytes))
                        .ok_or(ScanError::Integrity)?;
                }
                if response_source_bytes > bounds.maximum_response_bytes() {
                    return Err(ScanError::Safe(
                        FactSelectionReadFailureCode::ResponseBoundExceeded,
                    ));
                }
                query_results.push(PriorRunFactQueryResult {
                    query_ordinal: u32::try_from(query_ordinal)
                        .map_err(|_| ScanError::Integrity)?,
                    query_ref,
                    selected: selected
                        .into_iter()
                        .map(|fact| fact.as_ref().clone())
                        .collect(),
                });
            }
            let response = PriorRunFactSelectionResponse {
                version: PriorRunFactSelectionResponse::VERSION.to_owned(),
                request_digest: request.request_digest().map_err(|_| ScanError::Integrity)?,
                admitted_source_manifest_ref: self
                    .consumer_admission
                    .admission_material_refs
                    .prior_run_source_manifest_ref
                    .clone(),
                selector_contract_ref: prior_run_fact_selector_contract_ref()
                    .map_err(|_| ScanError::Integrity)?,
                attestation: PriorRunFactScanAttestation {
                    frontier: self.frontier.clone(),
                    tenant_scope_id: self.consumer_admission.tenant_scope_id.clone(),
                    authorization_ref: self.authorization_ref.clone(),
                    physical_binding_ref: self.physical_binding_ref.clone(),
                    completeness_mode:
                        PriorRunFactCompletenessMode::CompleteThroughAuthorizationFrontier,
                },
                query_results,
            };
            let canonical = canonical_json(&response).map_err(|_| ScanError::Integrity)?;
            if u64::try_from(canonical.as_bytes().len()).map_err(|_| ScanError::Integrity)?
                > bounds.maximum_response_bytes()
            {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::ResponseBoundExceeded,
                ));
            }
            let returned =
                FactSelectionReadResponse::from_canonical_json(canonical.as_str().to_owned())
                    .map_err(|_| ScanError::Integrity)?;
            let persisted = canonical_json(&returned).map_err(|_| ScanError::Integrity)?;
            if u64::try_from(persisted.as_bytes().len()).map_err(|_| ScanError::Integrity)?
                > bounds.maximum_response_bytes()
            {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::ResponseBoundExceeded,
                ));
            }
            Ok(returned)
        })
    }

    fn load_producer<'a>(
        &'a self,
        producer: &'a RunId,
        through: &'a JournalHead,
        session: &'a mut FactScanSession,
    ) -> Pin<Box<dyn Future<Output = ScanResult<Arc<VerifiedStructuredRun>>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(prefix) = session
                .prefixes
                .iter()
                .find(|prefix| &prefix.run_id == producer && &prefix.head == through)
            {
                return match &prefix.state {
                    PrefixMemoState::Visiting => Err(ScanError::Integrity),
                    PrefixMemoState::Verified(verified) => Ok(Arc::clone(verified)),
                };
            }
            let distinct_producers = session
                .prefixes
                .iter()
                .map(|prefix| &prefix.run_id)
                .collect::<BTreeSet<_>>();
            if !distinct_producers.contains(producer)
                && distinct_producers.len() as u64 >= session.maximum_distinct_producers
            {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::PublicationBoundExceeded,
                ));
            }
            match session.shared_prefixes.lookup(producer, through)? {
                SharedPrefixLookup::Visiting => return Err(ScanError::Integrity),
                SharedPrefixLookup::Verified {
                    run,
                    retained_bytes,
                    fold_batches,
                } => {
                    charge_prefix_work(session, retained_bytes, fold_batches)?;
                    session.prefixes.push(PrefixMemo {
                        run_id: producer.clone(),
                        head: through.clone(),
                        state: PrefixMemoState::Verified(Arc::clone(&run)),
                    });
                    return Ok(run);
                }
                SharedPrefixLookup::Missing => {}
            }
            session
                .shared_prefixes
                .begin(producer.clone(), through.clone())?;
            session.prefixes.push(PrefixMemo {
                run_id: producer.clone(),
                head: through.clone(),
                state: PrefixMemoState::Visiting,
            });
            let mut retained_bytes = 0_u64;
            let mut fold_batches = 0_u64;
            let result = async {
                #[cfg(feature = "test-support")]
                {
                    session.producer_prefix_loads = session
                        .producer_prefix_loads
                        .checked_add(1)
                        .ok_or(ScanError::Integrity)?;
                    count_fact_scan_producer_prefix_load(&session.counter_scope);
                }
                let remaining_bytes = session
                    .maximum_retained_source_bytes
                    .checked_sub(session.producer_history_bytes)
                    .ok_or(ScanError::Integrity)?;
                let remaining_batches = session
                    .maximum_producer_fold_batches
                    .checked_sub(session.fold_work)
                    .ok_or(ScanError::Integrity)?;
                let limit = RawHistoryLoadLimit::new(
                    usize::try_from(remaining_batches).map_err(|_| ScanError::Integrity)?,
                    MAX_RUN_HISTORY_OBJECTS,
                    usize::try_from(remaining_bytes).map_err(|_| ScanError::Integrity)?,
                );
                let raw = self
                    .source
                    .load_producer_prefix(producer, through, limit)
                    .await
                    .map_err(|error| match error {
                        StructuredStoreError::CapacityExceeded => ScanError::Safe(
                            FactSelectionReadFailureCode::RetainedSourceBoundExceeded,
                        ),
                        error => classify_backend_scan_error(error),
                    })?
                    .ok_or(ScanError::Integrity)?;
                if raw.batches.last().map(|batch| &batch.head) != Some(through) {
                    return Err(ScanError::Integrity);
                }
                retained_bytes = raw.batches.iter().try_fold(0_u64, |total, batch| {
                    let batch_bytes = u64::try_from(
                        canonical_json(batch)
                            .map_err(|_| ScanError::Integrity)?
                            .as_bytes()
                            .len(),
                    )
                    .map_err(|_| ScanError::Integrity)?;
                    total.checked_add(batch_bytes).ok_or(ScanError::Integrity)
                })?;
                fold_batches =
                    u64::try_from(raw.batches.len()).map_err(|_| ScanError::Integrity)?;
                charge_prefix_work(session, retained_bytes, fold_batches)?;
                #[cfg(feature = "test-support")]
                {
                    session.fold_batches = session
                        .fold_batches
                        .checked_add(fold_batches)
                        .ok_or(ScanError::Integrity)?;
                    count_fact_scan_fold_batches(&session.counter_scope, raw.batches.len());
                }
                let verified = Arc::new(
                    super::semantic_open::verify_qualified(
                        raw,
                        self.program_verifier.as_ref(),
                        self.physical_binding_verifier.as_ref(),
                    )
                    .map_err(|_| ScanError::Integrity)?,
                );
                if verified.journal_head() != through {
                    return Err(ScanError::Integrity);
                }
                self.verify_nested_barriers(&verified, session).await?;
                Ok(verified)
            }
            .await;
            let Some(position) = session
                .prefixes
                .iter()
                .position(|prefix| &prefix.run_id == producer && &prefix.head == through)
            else {
                return Err(ScanError::Integrity);
            };
            match result {
                Ok(verified) => {
                    session.shared_prefixes.finish(
                        producer,
                        through,
                        Arc::clone(&verified),
                        retained_bytes,
                        fold_batches,
                    )?;
                    session.prefixes[position].state =
                        PrefixMemoState::Verified(Arc::clone(&verified));
                    Ok(verified)
                }
                Err(error) => {
                    session.prefixes.remove(position);
                    session.shared_prefixes.abandon(producer, through)?;
                    Err(error)
                }
            }
        })
    }

    fn verify_nested_barriers<'a>(
        &'a self,
        verified: &'a VerifiedStructuredRun,
        session: &'a mut FactScanSession,
    ) -> Pin<Box<dyn Future<Output = ScanResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let source_object = verified
                .object(
                    &verified
                        .admission()
                        .admission_material_refs
                        .prior_run_source_manifest_ref,
                )
                .ok_or(ScanError::Integrity)?;
            let source_manifest = source_object
                .decode_persisted::<PriorRunFactSourceManifest>()
                .map_err(|_| ScanError::Integrity)?;
            for batch in verified.batches() {
                let TenantFactCoordinate::FactSelectionBarrier { frontier } =
                    &batch.tenant_fact_coordinate
                else {
                    continue;
                };
                let [assigned] = batch.records.as_slice() else {
                    return Err(ScanError::Integrity);
                };
                let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
                    return Err(ScanError::Integrity);
                };
                let Some((_, observation)) = verified.observation(&authorization.access_attempt_id)
                else {
                    continue;
                };
                let ObservationOutcome::Returned { value } = &observation.outcome else {
                    continue;
                };
                let request_object = verified
                    .object(&authorization.request.value_ref)
                    .ok_or(ScanError::Integrity)?;
                let request = FactSelectionRequest::from_canonical_json(
                    request_object.canonical_json.as_bytes(),
                )
                .map_err(|_| ScanError::Integrity)?;
                let returned_object = verified
                    .object(&value.value_ref)
                    .ok_or(ScanError::Integrity)?;
                let recorded: FactSelectionReadResponse =
                    serde_json::from_str(&returned_object.canonical_json)
                        .map_err(|_| ScanError::Integrity)?;
                let nested = BackendFactScanPort {
                    source: Arc::clone(&self.source),
                    program_verifier: Arc::clone(&self.program_verifier),
                    physical_binding_verifier: Arc::clone(&self.physical_binding_verifier),
                    prefix_memo: Arc::clone(&self.prefix_memo),
                    consumer_run_id: verified.run_id().clone(),
                    consumer_admission: verified.admission().clone(),
                    source_manifest: source_manifest.clone(),
                    expected_request: request.clone(),
                    authorization_ref: assigned.record_ref.clone(),
                    physical_binding_ref: authorization.physical_binding_ref.clone(),
                    frontier: frontier.clone(),
                    integrity_fault_code: self.integrity_fault_code.clone(),
                };
                let recomputed = nested.scan_with_session(request, session).await?;
                if recomputed != recorded {
                    return Err(ScanError::Integrity);
                }
            }
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn scan_publication(
        &self,
        publication: &TenantFactPublication,
        producer: &VerifiedStructuredRun,
        accumulators: &mut [(mfm_ids::ContentRef, FactTopK<Arc<SelectedPriorRunFact>>)],
        fact_count: &mut u64,
        retained_source_bytes: &mut u64,
        maximum_facts: u64,
        maximum_retained_source_bytes: u64,
    ) -> ScanResult<()> {
        let batch = producer
            .batches()
            .find(|batch| batch.head == publication.producer_head)
            .ok_or(ScanError::Integrity)?;
        if batch.tenant_fact_coordinate
            != (TenantFactCoordinate::FactPublication {
                frontier: publication.frontier.clone(),
            })
        {
            return Err(ScanError::Integrity);
        }
        let assigned = batch
            .records
            .get(
                usize::try_from(publication.transition_ref.ordinal)
                    .map_err(|_| ScanError::Integrity)?,
            )
            .filter(|assigned| assigned.record_ref == publication.transition_ref)
            .ok_or(ScanError::Integrity)?;
        let RunRecord::StateTransitionCommitted(transition) = &assigned.record else {
            return Err(ScanError::Integrity);
        };
        if transition.facts.is_empty() {
            return Err(ScanError::Integrity);
        }
        let transition = transition.clone();
        if producer.admission().store_scope_id != self.frontier.store_scope_id
            || producer.admission().store_epoch != self.frontier.store_epoch
            || producer.admission().tenant_scope_id != self.frontier.tenant_scope_id
        {
            return Err(ScanError::Integrity);
        }
        if producer.run_id() == &self.consumer_run_id {
            // A consumer cannot satisfy its own prior-run completeness witness.
            // Treating this route as an empty source would let a self-listed
            // publication bypass the independent producer closure.
            return Err(ScanError::Integrity);
        }
        for fact in &transition.facts {
            *fact_count = fact_count.checked_add(1).ok_or(ScanError::Integrity)?;
            if *fact_count > maximum_facts {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::FactBoundExceeded,
                ));
            }
            if !self
                .source_manifest
                .permits(producer.admission(), &fact.descriptor_ref)
            {
                continue;
            }
            let subject_object = producer
                .object(&fact.subject.value_ref)
                .ok_or(ScanError::Integrity)?;
            let response_object = producer
                .object(&fact.response.value_ref)
                .ok_or(ScanError::Integrity)?;
            let claim_object = producer
                .object(&fact.claim_ref)
                .ok_or(ScanError::Integrity)?;
            let claim_bytes = u64::try_from(claim_object.canonical_json.len())
                .map_err(|_| ScanError::Integrity)?;
            let source_bytes = u64::try_from(subject_object.canonical_json.len())
                .map_err(|_| ScanError::Integrity)?
                .checked_add(
                    u64::try_from(response_object.canonical_json.len())
                        .map_err(|_| ScanError::Integrity)?,
                )
                .and_then(|total| total.checked_add(claim_bytes))
                .ok_or(ScanError::Integrity)?;
            *retained_source_bytes = retained_source_bytes
                .checked_add(source_bytes)
                .ok_or(ScanError::Integrity)?;
            if *retained_source_bytes > maximum_retained_source_bytes {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::RetainedSourceBoundExceeded,
                ));
            }
            let subject =
                FactSubject::from_canonical_json(subject_object.canonical_json.as_bytes())
                    .map_err(|_| ScanError::Integrity)?;
            let content_identity =
                derive_fact_content_identity(fact).map_err(|_| ScanError::Integrity)?;
            let fact_identity = derive_fact_logical_identity(&publication.transition_ref, fact)
                .map_err(|_| ScanError::Integrity)?;
            let candidate_key = FactCandidate::new(
                fact.descriptor_ref.clone(),
                subject.clone(),
                content_identity.clone(),
                fact_identity.clone(),
                publication.frontier.fact_order,
                (),
            );
            if accumulators
                .iter()
                .all(|(_, accumulator)| !accumulator.matches(&candidate_key))
            {
                continue;
            }
            let selected = Arc::new(SelectedPriorRunFact {
                publication_frontier: publication.frontier.clone(),
                producer_transition_ref: publication.transition_ref.clone(),
                producer_certified_program_ref: producer.admission().certified_program_ref.clone(),
                producer_entry_point_operation_id: producer
                    .admission()
                    .entry_point_operation_id
                    .clone(),
                emission_ordinal: fact.emission_ordinal,
                descriptor_ref: fact.descriptor_ref.clone(),
                subject: fact.subject.clone(),
                response: fact.response.clone(),
                claim_ref: fact.claim_ref.clone(),
                content_identity: content_identity.clone(),
                fact_identity: fact_identity.clone(),
                subject_canonical_json: subject_object.canonical_json.clone(),
                response_canonical_json: response_object.canonical_json.clone(),
                claim_canonical_json: claim_object.canonical_json.clone(),
            });
            for (_, accumulator) in accumulators.iter_mut() {
                if !accumulator.matches(&candidate_key) {
                    continue;
                }
                accumulator.consider(FactCandidate::new(
                    fact.descriptor_ref.clone(),
                    subject.clone(),
                    content_identity.clone(),
                    fact_identity.clone(),
                    publication.frontier.fact_order,
                    Arc::clone(&selected),
                ));
            }
        }
        Ok(())
    }
}

pub(super) async fn qualify_and_reduce_for_scan(
    source: Arc<dyn PriorRunFactSource>,
    raw: RawRunHistory,
    program_verifier: Arc<ProgramVerificationRegistry>,
    physical_binding_verifier: Arc<dyn PhysicalObligationChecker>,
) -> super::Result<VerifiedStructuredRun> {
    qualify_and_reduce_for_scan_with_publications(
        source,
        raw,
        program_verifier,
        physical_binding_verifier,
    )
    .await
    .map(|(verified, _)| verified)
}

pub(super) async fn qualify_and_reduce_for_scan_with_publications(
    source: Arc<dyn PriorRunFactSource>,
    raw: RawRunHistory,
    program_verifier: Arc<ProgramVerificationRegistry>,
    physical_binding_verifier: Arc<dyn PhysicalObligationChecker>,
) -> super::Result<(VerifiedStructuredRun, Vec<TenantFactPublication>)> {
    qualify_and_reduce_for_scan_with_publications_and_memo(
        source,
        raw,
        program_verifier,
        physical_binding_verifier,
        Arc::new(PrefixVerificationMemo::default()),
    )
    .await
}

pub(super) async fn qualify_and_reduce_for_scan_with_publications_and_memo(
    source: Arc<dyn PriorRunFactSource>,
    raw: RawRunHistory,
    program_verifier: Arc<ProgramVerificationRegistry>,
    physical_binding_verifier: Arc<dyn PhysicalObligationChecker>,
    prefix_memo: Arc<PrefixVerificationMemo>,
) -> super::Result<(VerifiedStructuredRun, Vec<TenantFactPublication>)> {
    let fact_barriers = raw
        .batches
        .iter()
        .filter_map(|batch| match &batch.tenant_fact_coordinate {
            TenantFactCoordinate::FactSelectionBarrier { frontier } => {
                Some((frontier.clone(), batch.records.clone()))
            }
            TenantFactCoordinate::None | TenantFactCoordinate::FactPublication { .. } => None,
        })
        .collect::<Vec<_>>();
    let (verified, publications) = super::semantic_open::verify_qualified_with_fact_publications(
        raw,
        program_verifier.as_ref(),
        physical_binding_verifier.as_ref(),
    )?;
    let source_object = verified
        .object(
            &verified
                .admission()
                .admission_material_refs
                .prior_run_source_manifest_ref,
        )
        .ok_or_else(invalid)?;
    let source_manifest = source_object
        .decode_persisted::<PriorRunFactSourceManifest>()
        .map_err(|_| invalid())?;
    let integrity_fault_code =
        StableId::new("prior-run-fact-scan-integrity-fault").map_err(|_| invalid())?;
    for (frontier, records) in fact_barriers {
        let [assigned] = records.as_slice() else {
            return Err(invalid());
        };
        let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
            return Err(invalid());
        };
        let request_object = verified
            .object(&authorization.request.value_ref)
            .ok_or_else(invalid)?;
        let request =
            FactSelectionRequest::from_canonical_json(request_object.canonical_json.as_bytes())
                .map_err(|_| invalid())?;
        let port = BackendFactScanPort {
            source: Arc::clone(&source),
            program_verifier: Arc::clone(&program_verifier),
            physical_binding_verifier: Arc::clone(&physical_binding_verifier),
            prefix_memo: Arc::clone(&prefix_memo),
            consumer_run_id: verified.run_id().clone(),
            consumer_admission: verified.admission().clone(),
            source_manifest: source_manifest.clone(),
            expected_request: request.clone(),
            authorization_ref: assigned.record_ref.clone(),
            physical_binding_ref: authorization.physical_binding_ref.clone(),
            frontier,
            integrity_fault_code: integrity_fault_code.clone(),
        };
        let recomputed = port.scan(request).await;
        match &recomputed {
            Err(ScanError::Unavailable) => {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            Err(ScanError::Integrity) => return Err(invalid()),
            Ok(_) | Err(ScanError::Safe(_)) => {}
        }
        if let Some((_, observation)) = verified.observation(&authorization.access_attempt_id) {
            if let ObservationOutcome::Returned { value } = &observation.outcome {
                let returned_object = verified.object(&value.value_ref).ok_or_else(invalid)?;
                let recorded: FactSelectionReadResponse =
                    serde_json::from_str(&returned_object.canonical_json).map_err(|_| invalid())?;
                match recomputed {
                    Ok(recomputed) if recomputed == recorded => {}
                    Ok(_) | Err(ScanError::Safe(_)) => return Err(invalid()),
                    Err(ScanError::Unavailable) => {
                        return Err(StructuredStoreError::BackendUnavailable);
                    }
                    Err(ScanError::Integrity) => return Err(invalid()),
                }
            }
        }
    }
    Ok((verified, publications))
}
