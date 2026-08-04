use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_canonical::RecoverabilityContract;
use mfm_facts::{
    prior_run_fact_selector_contract_ref, FactCandidate, FactSelectionReadFailure,
    FactSelectionReadFailureCode, FactSelectionReadResponse, FactSelectionRequest, FactSubject,
    FactTopK,
};
use mfm_ids::{FactContentIdentityDigest, FactLogicalIdentityDigest, RunId, StableId};
use mfm_journal::structured::{
    canonical_json, CommittedBatch, ObservationOutcome, PriorRunFactCompletenessMode,
    PriorRunFactQueryResult, PriorRunFactScanAttestation, PriorRunFactSelectionResponse,
    PriorRunFactSourceManifest, RecordRef, RunAdmitted, RunRecord, SelectedPriorRunFact,
    TenantFactCoordinate, TenantFactFrontier,
};
use serde::Serialize;

use super::backend::{RawRunHistory, StructuredBackendFuture, TenantFactPublication};
use super::fold::{
    verify_recorded_history, ProgramVerifier, StructuredStoreError, VerifiedStructuredRun,
};
use super::qualification::PublicPhysicalBindingVerifier;

const SCAN_PAGE_ITEMS: u32 = 1_024;
const FACT_CONTENT_IDENTITY_PREIMAGE_CONTRACT: &str = "mfm.fact-content-identity-preimage.v1";
const FACT_LOGICAL_IDENTITY_PREIMAGE_CONTRACT: &str = "mfm.fact-logical-identity-preimage.v1";

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

    /// Reads one producer prefix ending at the exact routed transition sequence.
    fn load_producer_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through_sequence: u64,
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

pub(super) fn fact_scan_permit(
    source: Arc<dyn PriorRunFactSource>,
    program_verifier: Arc<dyn ProgramVerifier>,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
    committed: &CommittedBatch,
    successor: &VerifiedStructuredRun,
) -> super::Result<Option<FactScanPermit>> {
    let TenantFactCoordinate::FactSelectionBarrier { frontier } = &committed.tenant_fact_coordinate
    else {
        return Ok(None);
    };
    let [assigned] = committed.records.as_slice() else {
        return Err(invalid("fact scan authorization batch has the wrong shape"));
    };
    let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
        return Err(invalid(
            "fact scan barrier does not belong to an authorization",
        ));
    };
    let request_object = successor
        .object(&authorization.request.value_ref)
        .ok_or_else(|| invalid("fact scan request object is absent"))?;
    let expected_request: FactSelectionRequest = request_object
        .decode()
        .map_err(|_| invalid("fact scan request object cannot be decoded"))?;
    let source_object = successor
        .object(
            &successor
                .admission()
                .admission_material_refs
                .prior_run_source_manifest_ref,
        )
        .ok_or_else(|| invalid("fact scan source manifest is absent"))?;
    let source_manifest = PriorRunFactSourceManifest::from_history_object(source_object)
        .map_err(|_| invalid("fact scan source manifest is invalid"))?;
    let integrity_fault_code = StableId::new("prior-run-fact-scan-integrity-fault")
        .map_err(|_| invalid("fact scan integrity code cannot be derived"))?;
    Ok(Some(FactScanPermit {
        port: Box::new(BackendFactScanPort {
            source,
            program_verifier,
            physical_binding_verifier,
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

fn invalid(_message: &'static str) -> StructuredStoreError {
    StructuredStoreError::InvalidHistory
}

struct BackendFactScanPort {
    source: Arc<dyn PriorRunFactSource>,
    program_verifier: Arc<dyn ProgramVerifier>,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
    consumer_run_id: RunId,
    consumer_admission: RunAdmitted,
    source_manifest: PriorRunFactSourceManifest,
    expected_request: FactSelectionRequest,
    authorization_ref: RecordRef,
    physical_binding_ref: mfm_ids::ContentRef,
    frontier: TenantFactFrontier,
    integrity_fault_code: StableId,
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

impl BackendFactScanPort {
    async fn scan(&self, request: FactSelectionRequest) -> ScanResult<FactSelectionReadResponse> {
        if request != self.expected_request
            || request
                .admitted_source_manifest_ref()
                .map_err(|_| ScanError::Integrity)?
                != self
                    .consumer_admission
                    .admission_material_refs
                    .prior_run_source_manifest_ref
            || request
                .selector_contract_ref()
                .map_err(|_| ScanError::Integrity)?
                != prior_run_fact_selector_contract_ref().map_err(|_| ScanError::Integrity)?
        {
            return Err(ScanError::Integrity);
        }
        let bounds = request.scan_bounds().map_err(|_| ScanError::Integrity)?;
        if self.frontier.fact_order > bounds.maximum_publications() {
            return Err(ScanError::Safe(
                FactSelectionReadFailureCode::PublicationBoundExceeded,
            ));
        }
        let queries = request.queries().map_err(|_| ScanError::Integrity)?;
        let mut accumulators = queries
            .into_iter()
            .map(|query| {
                let query_ref = query.content_ref().map_err(|_| ScanError::Integrity)?;
                Ok((query_ref, FactTopK::new(query)))
            })
            .collect::<ScanResult<Vec<_>>>()?;
        let mut fact_count = 0_u64;
        let mut retained_source_bytes = 0_u64;
        let mut next_order = 1_u64;
        let mut publications_by_order = Vec::new();
        while next_order <= self.frontier.fact_order {
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
        // folding a growing prefix for every publication.
        let mut producer_heads = BTreeMap::<RunId, u64>::new();
        for publication in &publications_by_order {
            producer_heads
                .entry(publication.transition_ref.run_id.clone())
                .and_modify(|sequence| {
                    *sequence = (*sequence).max(publication.transition_ref.run_sequence)
                })
                .or_insert(publication.transition_ref.run_sequence);
        }
        if u64::try_from(producer_heads.len()).map_err(|_| ScanError::Integrity)?
            > bounds.maximum_publications()
        {
            return Err(ScanError::Safe(
                FactSelectionReadFailureCode::PublicationBoundExceeded,
            ));
        }
        let mut producer_cache = BTreeMap::<RunId, VerifiedStructuredRun>::new();
        let mut producer_history_bytes = 0_u64;
        let mut fold_work = 0_u64;
        for (producer, through_sequence) in producer_heads {
            let raw = self
                .source
                .load_producer_prefix(&producer, through_sequence)
                .await
                .map_err(classify_backend_scan_error)?
                .ok_or(ScanError::Integrity)?;
            producer_history_bytes = producer_history_bytes
                .checked_add(raw.batches.iter().try_fold(0_u64, |total, batch| {
                    let batch_bytes = u64::try_from(
                        canonical_json(batch)
                            .map_err(|_| ScanError::Integrity)?
                            .as_bytes()
                            .len(),
                    )
                    .map_err(|_| ScanError::Integrity)?;
                    total.checked_add(batch_bytes).ok_or(ScanError::Integrity)
                })?)
                .ok_or(ScanError::Integrity)?;
            if producer_history_bytes > bounds.maximum_retained_source_bytes() {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::RetainedSourceBoundExceeded,
                ));
            }
            fold_work = fold_work
                .checked_add(u64::try_from(raw.batches.len()).map_err(|_| ScanError::Integrity)?)
                .ok_or(ScanError::Integrity)?;
            if fold_work > bounds.maximum_publications() {
                return Err(ScanError::Safe(
                    FactSelectionReadFailureCode::PublicationBoundExceeded,
                ));
            }
            let verified = verify_recorded_history(
                raw,
                self.program_verifier.as_ref(),
                self.physical_binding_verifier.as_ref(),
            )
            .map_err(|_| ScanError::Integrity)?;
            producer_cache.insert(producer, verified);
        }
        for publication in &publications_by_order {
            let producer = producer_cache
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
                query_ordinal: u32::try_from(query_ordinal).map_err(|_| ScanError::Integrity)?,
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
            .iter()
            .find(|batch| batch.head.run_sequence == publication.transition_ref.run_sequence)
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
            return Ok(());
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
            let content_identity = fact_content_identity(fact)?;
            let fact_identity = fact_logical_identity(
                &publication.transition_ref,
                fact.emission_ordinal,
                &content_identity,
            )?;
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

pub(super) async fn verify_actionable_history(
    source: Arc<dyn PriorRunFactSource>,
    raw: RawRunHistory,
    program_verifier: Arc<dyn ProgramVerifier>,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> super::Result<VerifiedStructuredRun> {
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
    let verified = verify_recorded_history(
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
        .ok_or_else(|| invalid("verified fact source manifest is absent"))?;
    let source_manifest = PriorRunFactSourceManifest::from_history_object(source_object)
        .map_err(|_| invalid("verified fact source manifest is invalid"))?;
    let integrity_fault_code = StableId::new("prior-run-fact-scan-integrity-fault")
        .map_err(|_| invalid("fact scan integrity code cannot be derived"))?;
    for (frontier, records) in fact_barriers {
        let [assigned] = records.as_slice() else {
            return Err(invalid(
                "verified fact authorization batch has the wrong shape",
            ));
        };
        let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
            return Err(invalid(
                "verified fact barrier does not belong to authorization",
            ));
        };
        let Some((_, observation)) = verified.observation(&authorization.access_attempt_id) else {
            continue;
        };
        let ObservationOutcome::Returned { value } = &observation.outcome else {
            continue;
        };
        let request_object = verified
            .object(&authorization.request.value_ref)
            .ok_or_else(|| invalid("verified fact request object is absent"))?;
        let request: FactSelectionRequest = request_object
            .decode()
            .map_err(|_| invalid("verified fact request object cannot be decoded"))?;
        let returned_object = verified
            .object(&value.value_ref)
            .ok_or_else(|| invalid("verified fact response object is absent"))?;
        let recorded: FactSelectionReadResponse = returned_object
            .decode()
            .map_err(|_| invalid("verified fact response object cannot be decoded"))?;
        let port = BackendFactScanPort {
            source: Arc::clone(&source),
            program_verifier: Arc::clone(&program_verifier),
            physical_binding_verifier: Arc::clone(&physical_binding_verifier),
            consumer_run_id: verified.run_id().clone(),
            consumer_admission: verified.admission().clone(),
            source_manifest: source_manifest.clone(),
            expected_request: request.clone(),
            authorization_ref: assigned.record_ref.clone(),
            physical_binding_ref: authorization.physical_binding_ref.clone(),
            frontier,
            integrity_fault_code: integrity_fault_code.clone(),
        };
        match port.scan(request).await {
            Ok(recomputed) if recomputed == recorded => {}
            Err(ScanError::Unavailable) => return Err(StructuredStoreError::BackendUnavailable),
            Ok(_) | Err(ScanError::Safe(_) | ScanError::Integrity) => {
                return Err(invalid(
                    "retained prior-run fact response differs from complete recomputation",
                ));
            }
        }
    }
    Ok(verified)
}

#[derive(Serialize)]
struct FactContentIdentityPreimage<'a> {
    fact_descriptor_ref: &'a mfm_ids::ContentRef,
    subject_ref: &'a mfm_journal::structured::TypedValueRef,
    response_ref: &'a mfm_journal::structured::TypedValueRef,
}

#[derive(Serialize)]
struct FactLogicalIdentityPreimage<'a> {
    transition_ref: &'a RecordRef,
    emission_ordinal: u32,
    fact_content_identity: &'a FactContentIdentityDigest,
}

fn fact_content_identity(
    fact: &mfm_journal::structured::CommittedFactRef,
) -> ScanResult<FactContentIdentityDigest> {
    let canonical = canonical_json(&FactContentIdentityPreimage {
        fact_descriptor_ref: &fact.descriptor_ref,
        subject_ref: &fact.subject,
        response_ref: &fact.response,
    })
    .map_err(|_| ScanError::Integrity)?;
    let recoverability = RecoverabilityContract::embedded().map_err(|_| ScanError::Integrity)?;
    let preimage = recoverability
        .strict_decode(
            FACT_CONTENT_IDENTITY_PREIMAGE_CONTRACT,
            canonical.as_bytes(),
        )
        .map_err(|_| ScanError::Integrity)?;
    recoverability
        .derive_fact_content_identity(&preimage)
        .map_err(|_| ScanError::Integrity)
}

fn fact_logical_identity(
    producer_transition_ref: &RecordRef,
    emission_ordinal: u32,
    fact_content_identity: &FactContentIdentityDigest,
) -> ScanResult<FactLogicalIdentityDigest> {
    let canonical = canonical_json(&FactLogicalIdentityPreimage {
        transition_ref: producer_transition_ref,
        emission_ordinal,
        fact_content_identity,
    })
    .map_err(|_| ScanError::Integrity)?;
    let recoverability = RecoverabilityContract::embedded().map_err(|_| ScanError::Integrity)?;
    let preimage = recoverability
        .strict_decode(
            FACT_LOGICAL_IDENTITY_PREIMAGE_CONTRACT,
            canonical.as_bytes(),
        )
        .map_err(|_| ScanError::Integrity)?;
    recoverability
        .derive_fact_logical_identity(&preimage)
        .map_err(|_| ScanError::Integrity)
}

#[cfg(test)]
mod tests {
    use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId};
    use mfm_journal::structured::{CommittedFactRef, TypedValueRef};

    use super::{fact_content_identity, fact_logical_identity};

    fn content_ref(label: &str) -> ContentRef {
        ContentRef::new(
            SchemaId::new(
                &format!("mfm.fact-scan.test/{label}"),
                "1",
                DigestAlgorithm::Sha256JcsV1,
                mfm_canonical::sha256_digest_bytes(format!("schema:{label}").as_bytes()),
            )
            .expect("test schema"),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                mfm_canonical::sha256_digest_bytes(format!("content:{label}").as_bytes()),
            ),
        )
        .expect("test content ref")
    }

    fn typed_value(label: &str) -> TypedValueRef {
        TypedValueRef {
            contract_ref: content_ref(&format!("{label}-contract")),
            value_ref: content_ref(&format!("{label}-value")),
        }
    }

    #[test]
    fn fact_identities_preserve_content_and_tenant_order_independence() {
        let first = CommittedFactRef {
            emission_ordinal: 0,
            fact_slot_ordinal: 0,
            descriptor_ref: content_ref("descriptor"),
            subject: typed_value("first-subject"),
            response: typed_value("shared-response"),
            claim_ref: content_ref("first-claim"),
        };
        let mut different_subject = first.clone();
        different_subject.subject = typed_value("second-subject");
        different_subject.claim_ref = content_ref("second-claim");
        let first_identity =
            fact_content_identity(&first).unwrap_or_else(|_| panic!("first content identity"));
        let second_identity = fact_content_identity(&different_subject)
            .unwrap_or_else(|_| panic!("second content identity"));
        assert_ne!(first_identity, second_identity);

        let transition = mfm_journal::structured::RecordRef {
            run_id: mfm_ids::RunId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                mfm_canonical::sha256_digest_bytes(b"fact producer"),
            ),
            run_sequence: 2,
            ordinal: 0,
            record_hash: mfm_ids::JournalRecordHash::from_digest(
                mfm_canonical::sha256_digest_bytes(b"fact transition"),
            ),
        };
        let logical = fact_logical_identity(&transition, 0, &first_identity)
            .unwrap_or_else(|_| panic!("logical identity"));
        assert_eq!(
            logical,
            fact_logical_identity(&transition, 0, &first_identity)
                .unwrap_or_else(|_| panic!("repeat logical identity"))
        );
        assert_ne!(
            logical,
            fact_logical_identity(&transition, 1, &first_identity)
                .unwrap_or_else(|_| panic!("different-ordinal logical identity"))
        );
        assert_eq!(
            (first_identity.as_str(), logical.as_str()),
            (
                "sha256-jcs-v1:750c25181393c210eb78395460a19d4ba6589f44e24263e898abcdd3c02531a4",
                "sha256-jcs-v1:ecb2957d79215dd8d7538e97546ce719b173da0629d1b3073eec947a541603d6",
            )
        );
    }
}
