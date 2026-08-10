//! Sealed purpose-specific run history readers and evidence.
//!
//! Each purpose reader loads through the same qualification and reducer, then wraps the
//! verified prefix in a purpose-sealed evidence newtype. Evidence types have no
//! public field, no `Deref`, and expose only the accessors that purpose needs.
//! Cross-purpose substitution is a type error: `PublicRunEvidence` cannot be
//! passed where `ExportRunEvidence` is required.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_ids::{
    AccessAttemptId, ContentRef, InvocationIdentity, OccurrenceId, RequestDigest, RunId,
    RunSemanticStateDigest, SemanticCallId, TenantScopeId,
};
#[cfg(any(test, feature = "test-support"))]
use mfm_journal::structured::HistoryObject;
use mfm_journal::structured::{
    canonical_json, AccessKind, AssignedRecord, CommittedBatch, CommittedFactRef,
    ExternalAccessAuthorized, JournalHead, LexicalValueRef, ObservationOutcome, RecordRef,
    RunAdmitted, RunRecord, SemanticHead, StateOutcomeRef, TenantFactCoordinate,
    TenantFactFrontier, TypedValueRef,
};
use mfm_spec::structured::OperationOutcome;

use super::backend::{StructuredHistoryBackend, StructuredRunHistoryReader, TenantFactPublication};
use super::{PhysicalTargetIdentity, Result, StructuredFrontier, VerifiedStructuredRun};

/// Maximum recursively authorized source runs in one export closure.
pub const MAX_PORTABLE_SOURCE_RUNS: usize = 4096;

/// Maximum fact routes carried by one export closure.
pub const MAX_PORTABLE_FACT_ROUTES: usize = 1048576;

const EXPORT_FACT_ROUTE_PAGE_ITEMS: u32 = 1_024;

/// Reducer-derived status exposed by public and recorded-replay evidence.
///
/// Purpose projections deliberately retain only this status and never expose
/// the actionable cursor, capability references, or other `StructuredFrontier`
/// details owned by the internal reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEvidenceStatus {
    /// At least one executable action is ready for the next drive.
    Actionable,
    /// An effect entry may have happened and blocks later work.
    PossibleEntry,
    /// Committed integrity evidence blocks semantic progress.
    BlockedIntegrity,
    /// The root operation has a terminal outcome.
    Closed,
}

impl RunEvidenceStatus {
    fn from_frontier(frontier: &StructuredFrontier) -> Self {
        match frontier {
            StructuredFrontier::Actions(_) => Self::Actionable,
            StructuredFrontier::PossibleEntry(_) => Self::PossibleEntry,
            StructuredFrontier::BlockedIntegrity => Self::BlockedIntegrity,
            StructuredFrontier::Complete => Self::Closed,
        }
    }

    /// Returns the redaction-safe transport status tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Actionable => "actionable",
            Self::PossibleEntry => "possible_entry",
            Self::BlockedIntegrity => "blocked_integrity",
            Self::Closed => "closed",
        }
    }
}

/// Minimum identity header retained by every purpose projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEvidenceHeader {
    run_id: RunId,
    store_scope_id: mfm_ids::StoreScopeId,
    store_epoch: mfm_ids::StoreEpoch,
    tenant_scope_id: mfm_ids::TenantScopeId,
    invocation_identity: InvocationIdentity,
    entry_point_operation_id: mfm_ids::StableId,
}

impl RunEvidenceHeader {
    fn from_admission(admission: &RunAdmitted) -> Self {
        Self {
            run_id: admission.run_id.clone(),
            store_scope_id: admission.store_scope_id.clone(),
            store_epoch: admission.store_epoch,
            tenant_scope_id: admission.tenant_scope_id.clone(),
            invocation_identity: admission.invocation_identity.clone(),
            entry_point_operation_id: admission.entry_point_operation_id.clone(),
        }
    }

    /// Exact run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }
    /// Immutable store lineage.
    pub const fn store_scope_id(&self) -> &mfm_ids::StoreScopeId {
        &self.store_scope_id
    }
    /// Authoritative writer epoch.
    pub const fn store_epoch(&self) -> mfm_ids::StoreEpoch {
        self.store_epoch
    }
    /// Authorized tenant scope.
    pub const fn tenant_scope_id(&self) -> &mfm_ids::TenantScopeId {
        &self.tenant_scope_id
    }
    /// Caller invocation identity.
    pub const fn invocation_identity(&self) -> &InvocationIdentity {
        &self.invocation_identity
    }
    /// Qualified entry-point operation.
    pub const fn entry_point_operation_id(&self) -> &mfm_ids::StableId {
        &self.entry_point_operation_id
    }
}

macro_rules! purpose_reader_shell {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        pub struct $name<B: StructuredHistoryBackend> {
            reader: StructuredRunHistoryReader<B>,
        }

        impl<B: StructuredHistoryBackend> $name<B> {
            pub(super) fn new(reader: StructuredRunHistoryReader<B>) -> Self {
                Self { reader }
            }

            /// Returns the immutable qualified store identity.
            pub fn store_identity(&self) -> &super::StructuredStoreIdentity {
                self.reader.store_identity()
            }

            /// Probes backend readability without requiring an existing application run.
            pub async fn check_ready(&self) -> Result<()> {
                self.reader.check_ready().await
            }
        }
    };
}

purpose_reader_shell!(
    /// Target-bound public-read projection authority.
    PublicRunReader
);
purpose_reader_shell!(
    /// Target-bound transition-trace projection authority.
    TraceRunReader
);
purpose_reader_shell!(
    /// Target-bound access-audit projection authority.
    AuditRunReader
);
purpose_reader_shell!(
    /// Target-bound recorded-replay projection authority.
    ReplayRunReader
);
purpose_reader_shell!(
    /// Target-bound export projection authority.
    ExportRunReader
);
purpose_reader_shell!(
    /// Target-bound current-Effect-entry-attention inventory authority.
    EffectEntryAttentionReader
);

impl<B: StructuredHistoryBackend> PublicRunReader<B> {
    /// Loads one verified run as public-read evidence only.
    pub async fn load_public(&self, run_id: &RunId) -> Result<PublicRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .and_then(PublicRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> TraceRunReader<B> {
    /// Loads one verified run as transition-trace evidence only.
    pub async fn load_transition_trace(&self, run_id: &RunId) -> Result<TraceRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(TraceRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> AuditRunReader<B> {
    /// Loads one verified run as access-audit evidence only.
    pub async fn load_access_audit(&self, run_id: &RunId) -> Result<AuditRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(AuditRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> ReplayRunReader<B> {
    /// Loads one verified run as recorded-replay evidence only.
    pub async fn load_for_recorded_verify(&self, run_id: &RunId) -> Result<RecordedRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(RecordedRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> EffectEntryAttentionReader<B> {
    /// Lists the runs in one tenant that currently require manual Effect-entry
    /// attention, ordered strictly by run identity.
    ///
    /// The backend scan is a route, not an answer. For each route this method
    /// loads history and the current projection from one snapshot, skips a route
    /// whose head moved before the load as a live-sweep race, and otherwise
    /// qualifies and reduces the complete prefix. A run whose head still matches
    /// but whose reduction disagrees is invalid history, not an empty result.
    pub async fn list_effect_entry_attention(
        &self,
        tenant_scope_id: &mfm_ids::TenantScopeId,
        after_run_id: Option<&RunId>,
        limit: u32,
    ) -> Result<EffectEntryAttentionPage> {
        if limit == 0 {
            return Err(super::qualification::StructuredStoreError::InvalidHistory);
        }
        // One extra route is a sentinel proving more work exists; it is never
        // processed and never becomes the continuation key.
        let sentinel_limit = limit.saturating_add(1);
        let routes = self
            .reader
            .scan_effect_entry_attention_routes(tenant_scope_id, after_run_id, sentinel_limit)
            .await?;
        let has_more = routes.len() as u32 > limit;
        let mut entries = Vec::new();
        let mut last_scanned = None;
        for route in routes.into_iter().take(limit as usize) {
            last_scanned = Some(route.run_id.clone());
            let Some(evidence) = self.resolve_route(tenant_scope_id, &route).await? else {
                continue;
            };
            entries.push(evidence);
        }
        Ok(EffectEntryAttentionPage {
            entries,
            // Advance by the last scanned route, never the last emitted result,
            // so a race yields a sparse page rather than a skipped run.
            next_after_run_id: has_more.then_some(last_scanned).flatten(),
        })
    }

    /// Re-derives one route's attention, or `None` when the route lost a race.
    async fn resolve_route(
        &self,
        tenant_scope_id: &mfm_ids::TenantScopeId,
        route: &super::backend::EffectEntryAttentionRoute,
    ) -> Result<Option<EffectEntryAttentionEntry>> {
        let verified = match self.reader.load_verified(&route.run_id).await {
            Ok(verified) => verified,
            // The run was scanned and then vanished from this snapshot: a live
            // sweep race, not a contradiction.
            Err(super::qualification::StructuredStoreError::RunNotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        if verified.journal_head() != &route.journal_head {
            return Ok(None);
        }
        // Same head: tenant and attention membership must agree exactly, and the
        // reducer must return a complete subject.
        let attention = verified
            .effect_entry_attention()
            .ok_or(super::qualification::StructuredStoreError::InvalidHistory)?;
        if &verified.admission().tenant_scope_id != tenant_scope_id {
            return Err(super::qualification::StructuredStoreError::InvalidHistory);
        }
        Ok(Some(EffectEntryAttentionEntry {
            header: RunEvidenceHeader::from_admission(verified.admission()),
            journal_head: verified.journal_head().clone(),
            subject: attention.subject().clone(),
            resolution: attention.resolution(),
        }))
    }
}

impl<B: StructuredHistoryBackend> ExportRunReader<B> {
    /// Loads one verified run as portable-export evidence only.
    pub async fn load_for_export(&self, run_id: &RunId) -> Result<ExportRunEvidence> {
        let physical_target = self
            .reader
            .store_identity()
            .physical_target
            .clone()
            .ok_or(super::qualification::StructuredStoreError::InvalidHistory)?;
        let verified = self.reader.load_verified(run_id).await?;
        let fact_routes = load_export_fact_routes(&self.reader, &verified).await?;
        ExportRunEvidence::from_verified(verified, physical_target, fact_routes)
    }
}

async fn load_export_fact_routes<B: StructuredHistoryBackend>(
    reader: &StructuredRunHistoryReader<B>,
    verified: &VerifiedStructuredRun,
) -> Result<Vec<ExportFactRoute>> {
    let Some(frontier) = maximum_fact_barrier(verified.batches(), None)? else {
        return Ok(Vec::new());
    };
    let mut next_order = 1_u64;
    let mut routes = Vec::new();
    while next_order <= frontier.fact_order {
        let publications = reader
            .scan_fact_publications(
                &frontier.tenant_scope_id,
                next_order,
                frontier.fact_order,
                EXPORT_FACT_ROUTE_PAGE_ITEMS,
            )
            .await?;
        if publications.is_empty() || publications.len() > EXPORT_FACT_ROUTE_PAGE_ITEMS as usize {
            return Err(super::qualification::StructuredStoreError::InvalidHistory);
        }
        for publication in publications {
            if publication.frontier.store_scope_id != frontier.store_scope_id
                || publication.frontier.store_epoch != frontier.store_epoch
                || publication.frontier.tenant_scope_id != frontier.tenant_scope_id
                || publication.frontier.fact_order != next_order
                || publication.frontier.fact_order > frontier.fact_order
            {
                return Err(super::qualification::StructuredStoreError::InvalidHistory);
            }
            ensure_fact_route_capacity(routes.len())?;
            routes.push(ExportFactRoute::from_publication(publication));
            next_order = next_order
                .checked_add(1)
                .ok_or(super::qualification::StructuredStoreError::InvalidHistory)?;
        }
    }
    Ok(routes)
}

/// Sealed public-read evidence. Cannot be used as export, trace, audit, or replay evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicTerminalOutcome {
    kind: &'static str,
    value: LexicalValueRef,
    canonical_value: String,
}

impl PublicTerminalOutcome {
    /// Returns the public success/failure tag.
    pub const fn kind(&self) -> &'static str {
        self.kind
    }

    /// Returns the selected terminal value reference.
    pub const fn value(&self) -> &LexicalValueRef {
        &self.value
    }

    /// Returns the exact canonical terminal value bytes.
    pub fn canonical_value(&self) -> &str {
        &self.canonical_value
    }
}

/// Minimum public run projection produced after callback-free reduction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRunEvidence {
    run_id: RunId,
    header: RunEvidenceHeader,
    journal_head: JournalHead,
    semantic_head: SemanticHead,
    status: RunEvidenceStatus,
    closed_outcome_ref: Option<ContentRef>,
    terminal_outcome: Option<PublicTerminalOutcome>,
}

impl PublicRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Result<Self> {
        let terminal_outcome = terminal_public_outcome(&verified)?;
        Ok(Self {
            run_id: verified.run_id().clone(),
            header: RunEvidenceHeader::from_admission(verified.admission()),
            journal_head: verified.journal_head().clone(),
            semantic_head: verified.semantic_head().clone(),
            status: RunEvidenceStatus::from_frontier(verified.frontier()),
            closed_outcome_ref: verified.closed_outcome_ref().cloned(),
            terminal_outcome,
        })
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the minimum purpose header.
    pub const fn header(&self) -> &RunEvidenceHeader {
        &self.header
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        &self.semantic_head
    }

    /// Returns the reducer-derived status without exposing internal action details.
    pub const fn status(&self) -> RunEvidenceStatus {
        self.status
    }

    /// Returns the terminal nominal operation-outcome reference, when closed.
    pub const fn closed_outcome_ref(&self) -> Option<&ContentRef> {
        self.closed_outcome_ref.as_ref()
    }

    /// Returns the reducer-derived terminal public outcome, when present.
    pub const fn terminal_outcome(&self) -> Option<&PublicTerminalOutcome> {
        self.terminal_outcome.as_ref()
    }
}

/// One data-only transition retained by the trace projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceTransitionEntry {
    record_ref: RecordRef,
    occurrence_id: OccurrenceId,
    occurrence_path_ref: ContentRef,
    semantic_call_id: SemanticCallId,
    input: LexicalValueRef,
    consumed_observation_ref: Option<RecordRef>,
    outcome_ref: ContentRef,
    outcome: StateOutcomeRef,
    facts: Vec<CommittedFactRef>,
    before_semantic_state_digest: RunSemanticStateDigest,
    after_semantic_state_digest: RunSemanticStateDigest,
}

impl TraceTransitionEntry {
    fn from_assigned(assigned: &AssignedRecord) -> Option<Self> {
        let RunRecord::StateTransitionCommitted(transition) = &assigned.record else {
            return None;
        };
        Some(Self {
            record_ref: assigned.record_ref.clone(),
            occurrence_id: transition.occurrence_id.clone(),
            occurrence_path_ref: transition.occurrence_path_ref.clone(),
            semantic_call_id: transition.semantic_call_id.clone(),
            input: transition.input.clone(),
            consumed_observation_ref: transition.consumed_observation_ref.clone(),
            outcome_ref: transition.outcome_ref.clone(),
            outcome: transition.outcome.clone(),
            facts: transition.facts.clone(),
            before_semantic_state_digest: transition.before_semantic_state_digest.clone(),
            after_semantic_state_digest: transition.after_semantic_state_digest.clone(),
        })
    }

    /// Exact assigned record reference.
    pub const fn record_ref(&self) -> &RecordRef {
        &self.record_ref
    }
    /// Exact executable occurrence.
    pub const fn occurrence_id(&self) -> &OccurrenceId {
        &self.occurrence_id
    }
    /// Canonical normalized occurrence path reference.
    pub const fn occurrence_path_ref(&self) -> &ContentRef {
        &self.occurrence_path_ref
    }
    /// Stable semantic call identity.
    pub const fn semantic_call_id(&self) -> &SemanticCallId {
        &self.semantic_call_id
    }
    /// Exact reducer-derived state input.
    pub const fn input(&self) -> &LexicalValueRef {
        &self.input
    }
    /// Exact normal observation consumed by the transition, when present.
    pub const fn consumed_observation_ref(&self) -> Option<&RecordRef> {
        self.consumed_observation_ref.as_ref()
    }
    /// Nominal state-outcome object reference.
    pub const fn outcome_ref(&self) -> &ContentRef {
        &self.outcome_ref
    }
    /// Selected nominal state outcome.
    pub const fn outcome(&self) -> &StateOutcomeRef {
        &self.outcome
    }
    /// Declaration-ordered produced facts.
    pub fn facts(&self) -> &[CommittedFactRef] {
        &self.facts
    }
    /// Semantic state before the transition.
    pub const fn before_semantic_state_digest(&self) -> &RunSemanticStateDigest {
        &self.before_semantic_state_digest
    }
    /// Semantic state after the transition.
    pub const fn after_semantic_state_digest(&self) -> &RunSemanticStateDigest {
        &self.after_semantic_state_digest
    }
}

/// Sealed transition-trace evidence. Cannot be used as public, export, audit, or replay evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRunEvidence {
    run_id: RunId,
    header: RunEvidenceHeader,
    journal_head: JournalHead,
    journal_heads: Vec<JournalHead>,
    records: Vec<TraceTransitionEntry>,
}

impl TraceRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        let records = verified
            .records()
            .filter_map(TraceTransitionEntry::from_assigned)
            .collect();
        Self {
            run_id: verified.run_id().clone(),
            header: RunEvidenceHeader::from_admission(verified.admission()),
            journal_head: verified.journal_head().clone(),
            journal_heads: verified.journal_heads().cloned().collect(),
            records,
        }
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the minimum purpose header.
    pub const fn header(&self) -> &RunEvidenceHeader {
        &self.header
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        &self.journal_heads
    }

    /// Returns every data-only transition in physical append order.
    pub fn records(&self) -> &[TraceTransitionEntry] {
        &self.records
    }
}

/// One closed observation retained by the access-audit projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditObservation {
    record_ref: RecordRef,
    outcome: ObservationOutcome,
}

impl AuditObservation {
    /// Exact assigned observation record reference.
    pub const fn record_ref(&self) -> &RecordRef {
        &self.record_ref
    }
    /// Exact closed observation outcome.
    pub const fn outcome(&self) -> &ObservationOutcome {
        &self.outcome
    }
}

/// One data-only authorization and its optional closed observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditAccessEntry {
    authorization_ref: RecordRef,
    access_attempt_id: AccessAttemptId,
    attempt_ordinal: u64,
    occurrence_id: OccurrenceId,
    occurrence_path_ref: ContentRef,
    semantic_call_id: SemanticCallId,
    access_kind: AccessKind,
    capability_contract_ref: ContentRef,
    capability_implementation_ref: ContentRef,
    adapter_contract_ref: ContentRef,
    adapter_implementation_ref: ContentRef,
    request: TypedValueRef,
    request_digest: RequestDigest,
    physical_binding_ref: ContentRef,
    stable_resource_lineage_contract_ref: Option<ContentRef>,
    observation: Option<AuditObservation>,
}

impl AuditAccessEntry {
    fn from_records(
        assigned: &AssignedRecord,
        authorization: &ExternalAccessAuthorized,
        observation: Option<AuditObservation>,
    ) -> Self {
        Self {
            authorization_ref: assigned.record_ref.clone(),
            access_attempt_id: authorization.access_attempt_id.clone(),
            attempt_ordinal: authorization.attempt_ordinal,
            occurrence_id: authorization.occurrence_id.clone(),
            occurrence_path_ref: authorization.occurrence_path_ref.clone(),
            semantic_call_id: authorization.semantic_call_id.clone(),
            access_kind: authorization.access_kind,
            capability_contract_ref: authorization.capability_contract_ref.clone(),
            capability_implementation_ref: authorization.capability_implementation_ref.clone(),
            adapter_contract_ref: authorization.adapter_contract_ref.clone(),
            adapter_implementation_ref: authorization.adapter_implementation_ref.clone(),
            request: authorization.request.clone(),
            request_digest: authorization.request_digest.clone(),
            physical_binding_ref: authorization.physical_binding_ref.clone(),
            stable_resource_lineage_contract_ref: authorization
                .stable_resource_lineage_contract_ref
                .clone(),
            observation,
        }
    }

    /// Exact authorization record reference.
    pub const fn authorization_ref(&self) -> &RecordRef {
        &self.authorization_ref
    }
    /// Exact access attempt identity.
    pub const fn access_attempt_id(&self) -> &AccessAttemptId {
        &self.access_attempt_id
    }
    /// Reducer-derived access ordinal.
    pub const fn attempt_ordinal(&self) -> u64 {
        self.attempt_ordinal
    }
    /// Exact executable occurrence.
    pub const fn occurrence_id(&self) -> &OccurrenceId {
        &self.occurrence_id
    }
    /// Canonical normalized occurrence path reference.
    pub const fn occurrence_path_ref(&self) -> &ContentRef {
        &self.occurrence_path_ref
    }
    /// Stable semantic call identity.
    pub const fn semantic_call_id(&self) -> &SemanticCallId {
        &self.semantic_call_id
    }
    /// Read or effect protocol kind.
    pub const fn access_kind(&self) -> AccessKind {
        self.access_kind
    }
    /// Semantic capability contract reference.
    pub const fn capability_contract_ref(&self) -> &ContentRef {
        &self.capability_contract_ref
    }
    /// Secret-free capability implementation reference.
    pub const fn capability_implementation_ref(&self) -> &ContentRef {
        &self.capability_implementation_ref
    }
    /// Semantic adapter contract reference.
    pub const fn adapter_contract_ref(&self) -> &ContentRef {
        &self.adapter_contract_ref
    }
    /// Secret-free adapter implementation reference.
    pub const fn adapter_implementation_ref(&self) -> &ContentRef {
        &self.adapter_implementation_ref
    }
    /// Immutable typed request.
    pub const fn request(&self) -> &TypedValueRef {
        &self.request
    }
    /// Canonical request digest.
    pub const fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }
    /// Secret-free physical binding reference.
    pub const fn physical_binding_ref(&self) -> &ContentRef {
        &self.physical_binding_ref
    }
    /// Stable resource-lineage contract for refreshable effects.
    pub const fn stable_resource_lineage_contract_ref(&self) -> Option<&ContentRef> {
        self.stable_resource_lineage_contract_ref.as_ref()
    }
    /// Closed observation, when one has been committed.
    pub const fn observation(&self) -> Option<&AuditObservation> {
        self.observation.as_ref()
    }
}

/// Sealed access-audit evidence. Cannot be used as public, export, trace, or replay evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRunEvidence {
    run_id: RunId,
    header: RunEvidenceHeader,
    journal_head: JournalHead,
    journal_heads: Vec<JournalHead>,
    records: Vec<AuditAccessEntry>,
}

impl AuditRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        let all_records = verified.records().collect::<Vec<_>>();
        let records = all_records
            .iter()
            .filter_map(|assigned| {
                let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
                    return None;
                };
                let observation = all_records.iter().find_map(|candidate| {
                    let RunRecord::ExternalAccessObserved(observed) = &candidate.record else {
                        return None;
                    };
                    (observed.access_attempt_id == authorization.access_attempt_id).then(|| {
                        AuditObservation {
                            record_ref: candidate.record_ref.clone(),
                            outcome: observed.outcome.clone(),
                        }
                    })
                });
                Some(AuditAccessEntry::from_records(
                    assigned,
                    authorization,
                    observation,
                ))
            })
            .collect();
        Self {
            run_id: verified.run_id().clone(),
            header: RunEvidenceHeader::from_admission(verified.admission()),
            journal_head: verified.journal_head().clone(),
            journal_heads: verified.journal_heads().cloned().collect(),
            records,
        }
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the minimum purpose header.
    pub const fn header(&self) -> &RunEvidenceHeader {
        &self.header
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        &self.journal_heads
    }

    /// Returns every data-only authorization in physical append order.
    pub fn records(&self) -> &[AuditAccessEntry] {
        &self.records
    }
}

/// Opaque result of one explicit offline replay reduction.
///
/// The store consumes the complete verified cursor and object graph before
/// constructing this value. Replay receives only the recorded status and the
/// identifier-level metadata required to validate a portable export; callers
/// cannot recover the internal frontier, cursor, records, or objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineVerifiedRun {
    recorded: RecordedRunEvidence,
    run_id: RunId,
    header: RunEvidenceHeader,
    journal_head: JournalHead,
    semantic_head: SemanticHead,
    source_prefixes: Vec<OfflineVerifiedPrefix>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OfflineVerifiedPrefix {
    run_id: RunId,
    tenant_scope_id: TenantScopeId,
    journal_head: JournalHead,
    semantic_head: SemanticHead,
}

impl OfflineVerifiedRun {
    pub(crate) fn from_verified_closure(
        verified: VerifiedStructuredRun,
        sources: Vec<Arc<VerifiedStructuredRun>>,
    ) -> Result<Self> {
        let run_id = verified.run_id().clone();
        if sources.len() > MAX_PORTABLE_SOURCE_RUNS
            || sources.iter().any(|source| source.run_id() == &run_id)
            || sources
                .iter()
                .map(|source| source.run_id())
                .collect::<BTreeSet<_>>()
                .len()
                != sources.len()
        {
            return Err(super::StructuredStoreError::InvalidHistory);
        }
        let source_prefixes = sources
            .iter()
            .map(|source| OfflineVerifiedPrefix {
                run_id: source.run_id().clone(),
                tenant_scope_id: source.admission().tenant_scope_id.clone(),
                journal_head: source.journal_head().clone(),
                semantic_head: source.semantic_head().clone(),
            })
            .collect();
        let header = RunEvidenceHeader::from_admission(verified.admission());
        let journal_head = verified.journal_head().clone();
        let semantic_head = verified.semantic_head().clone();
        let recorded = RecordedRunEvidence::from_verified(verified);
        Ok(Self {
            recorded,
            run_id,
            header,
            journal_head,
            semantic_head,
            source_prefixes,
        })
    }

    /// Returns the exact reduced run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the authenticated tenant scope fixed by the reduced admission.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        self.header.tenant_scope_id()
    }

    /// Returns the exact reduced physical head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns the exact reduced semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        &self.semantic_head
    }

    /// Confirms one exact source prefix reduced as part of this complete closure.
    pub fn matches_verified_prefix(
        &self,
        run_id: &RunId,
        tenant_scope_id: &TenantScopeId,
        journal_head: &JournalHead,
        semantic_head: &SemanticHead,
    ) -> bool {
        self.source_prefixes.iter().any(|source| {
            &source.run_id == run_id
                && &source.tenant_scope_id == tenant_scope_id
                && &source.journal_head == journal_head
                && &source.semantic_head == semantic_head
        })
    }

    /// Consumes the opaque reduced result into recorded-replay evidence.
    pub fn into_recorded(self) -> RecordedRunEvidence {
        self.recorded
    }
}

/// Sealed recorded-replay evidence. Cannot be used as public, export, trace, or audit evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRunEvidence {
    run_id: RunId,
    header: RunEvidenceHeader,
    journal_head: JournalHead,
    semantic_head: SemanticHead,
    status: RunEvidenceStatus,
    record_count: usize,
}

impl RecordedRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self {
            run_id: verified.run_id().clone(),
            header: RunEvidenceHeader::from_admission(verified.admission()),
            journal_head: verified.journal_head().clone(),
            semantic_head: verified.semantic_head().clone(),
            status: RunEvidenceStatus::from_frontier(verified.frontier()),
            record_count: verified.records().count(),
        }
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the minimum purpose header.
    pub const fn header(&self) -> &RunEvidenceHeader {
        &self.header
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        &self.semantic_head
    }

    /// Returns the reducer-derived status without exposing internal action details.
    pub const fn status(&self) -> RunEvidenceStatus {
        self.status
    }

    /// Returns the number of verified records in the recorded prefix.
    pub const fn record_count(&self) -> usize {
        self.record_count
    }
}

/// Data-only fragment retained for the portable encoder after reduction.
///
/// This deliberately contains no verified-run authority or callback. Reduction is
/// complete before this product is constructed; only the bounded append envelopes
/// and identifier-level dependency metadata survive.
struct ExportFragment {
    run_id: RunId,
    header: RunEvidenceHeader,
    physical_target: PhysicalTargetIdentity,
    journal_head: JournalHead,
    semantic_head: SemanticHead,
    #[cfg(any(test, feature = "test-support"))]
    journal_heads: Vec<JournalHead>,
    batches: Vec<CommittedBatch>,
    batch_frames: Vec<Vec<u8>>,
    records: Vec<AssignedRecord>,
    #[cfg(any(test, feature = "test-support"))]
    objects: Vec<(u64, HistoryObject)>,
    #[cfg(any(test, feature = "test-support"))]
    cursor: super::ProgramCursor,
    #[cfg(any(test, feature = "test-support"))]
    closed_outcome_ref: Option<ContentRef>,
    direct_source_run_ids: BTreeSet<RunId>,
    fact_frontiers: Vec<TenantFactFrontier>,
    fact_routes: Vec<ExportFactRoute>,
}

/// One exact dense fact publication retained for recursive export planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFactRoute {
    producer_transition: RecordRef,
    producer_head: JournalHead,
    publication_frontier: TenantFactFrontier,
}

impl ExportFactRoute {
    fn from_publication(publication: TenantFactPublication) -> Self {
        Self {
            producer_transition: publication.transition_ref,
            producer_head: publication.producer_head,
            publication_frontier: publication.frontier,
        }
    }

    /// Producer transition routed by the dense publication.
    pub const fn producer_transition(&self) -> &RecordRef {
        &self.producer_transition
    }
    /// Exact immutable producer prefix containing the transition.
    pub const fn producer_head(&self) -> &JournalHead {
        &self.producer_head
    }
    /// Dense tenant frontier containing the publication.
    pub const fn publication_frontier(&self) -> &TenantFactFrontier {
        &self.publication_frontier
    }
}

/// Sealed portable-export evidence. Cannot be used as public, trace, audit, or replay evidence.
pub struct ExportRunEvidence {
    fragment: ExportFragment,
    header: RunEvidenceHeader,
    authorized_sources: Vec<ExportFragment>,
}

impl std::fmt::Debug for ExportRunEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExportRunEvidence")
            .field("run_id", &self.fragment.run_id)
            .field("source_run_count", &self.authorized_sources.len())
            .field("batch_count", &self.fragment.batches.len())
            .field("record_count", &self.fragment.records.len())
            .finish()
    }
}

/// Encoder-only view of opaque export fragments. The view is borrowed for the
/// duration of one callback and cannot be stored or converted into another
/// purpose projection.
pub struct ExportEncoderView<'a> {
    fragment: &'a ExportFragment,
    authorized_sources: &'a [ExportFragment],
}

impl<'a> ExportEncoderView<'a> {
    /// Exact root run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.fragment.run_id
    }
    /// Minimum export header.
    pub fn header(&self) -> RunEvidenceHeader {
        self.fragment.header.clone()
    }
    /// Exact physical target fixation retained by deployment qualification.
    pub const fn physical_target(&self) -> &PhysicalTargetIdentity {
        &self.fragment.physical_target
    }
    /// Exact physical head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.fragment.journal_head
    }
    /// Exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        &self.fragment.semantic_head
    }
    /// Canonical committed-batch frames in append order.
    pub fn batch_frames(&self) -> impl Iterator<Item = &[u8]> {
        self.fragment.batch_frames.iter().map(Vec::as_slice)
    }
    /// Verified source prefixes in deterministic order.
    pub fn authorized_source_prefixes(&self) -> impl Iterator<Item = ExportEncoderSource<'_>> {
        self.authorized_sources.iter().map(ExportEncoderSource::new)
    }
    /// Exact identifier-level direct source dependencies.
    pub fn direct_source_run_ids(&self) -> &BTreeSet<RunId> {
        &self.fragment.direct_source_run_ids
    }
    /// Exact fact frontiers captured by this reduced fragment.
    pub fn fact_frontiers(&self) -> &[TenantFactFrontier] {
        &self.fragment.fact_frontiers
    }
    /// Verified selected-fact routes in reduced record order.
    pub fn fact_routes(&self) -> &[ExportFactRoute] {
        &self.fragment.fact_routes
    }
    /// Complete dense fact routes required through one run cutoff.
    pub fn fact_routes_through(&self, cutoff: Option<u64>) -> Vec<ExportFactRoute> {
        self.fragment.fact_routes_through(cutoff)
    }
}

/// One encoder-only source fragment. It exposes data needed to encode a frame
/// stream but never exposes reducer internals, callbacks, or verified-run type.
pub struct ExportEncoderSource<'a> {
    fragment: &'a ExportFragment,
}

impl<'a> ExportEncoderSource<'a> {
    fn new(fragment: &'a ExportFragment) -> Self {
        Self { fragment }
    }
    /// Exact source run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.fragment.run_id
    }
    /// Minimum source header.
    pub const fn header(&self) -> &RunEvidenceHeader {
        &self.fragment.header
    }
    /// Exact source physical target fixation.
    pub const fn physical_target(&self) -> &PhysicalTargetIdentity {
        &self.fragment.physical_target
    }
    /// Exact source physical head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.fragment.journal_head
    }
    /// Exact source semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        &self.fragment.semantic_head
    }
    /// Canonical source batch frames in deterministic order.
    pub fn batch_frames(&self) -> impl Iterator<Item = &[u8]> {
        self.fragment.batch_frames.iter().map(Vec::as_slice)
    }
    /// Fact frontiers captured in source append coordinates.
    pub fn fact_frontiers(&self) -> &[TenantFactFrontier] {
        &self.fragment.fact_frontiers
    }
    /// Identifier-level source dependencies.
    pub fn direct_source_run_ids(&self) -> &BTreeSet<RunId> {
        &self.fragment.direct_source_run_ids
    }
    /// Verified selected-fact routes in this source prefix.
    pub fn fact_routes(&self) -> &[ExportFactRoute] {
        &self.fragment.fact_routes
    }
    /// Complete dense fact routes required through one source cutoff.
    pub fn fact_routes_through(&self, cutoff: Option<u64>) -> Vec<ExportFactRoute> {
        self.fragment.fact_routes_through(cutoff)
    }
}

impl ExportRunEvidence {
    fn from_verified(
        verified: VerifiedStructuredRun,
        physical_target: PhysicalTargetIdentity,
        fact_routes: Vec<ExportFactRoute>,
    ) -> Result<Self> {
        let fragment = ExportFragment::from_verified(verified, physical_target, fact_routes)?;
        Ok(Self {
            header: fragment.header.clone(),
            fragment,
            authorized_sources: Vec::new(),
        })
    }

    /// Seals the already-authorized recursive source prefixes into this export
    /// evidence. The supplied values are the complete flattened closure, with
    /// one exact required-head cutoff per source. Graph validation below
    /// requires every value to be reachable from an immediate source and
    /// rejects omissions, substitutions, cycles, and unrelated values. The
    /// source values remain inaccessible outside the encoder accessors below
    /// and are never interchangeable with other purpose data.
    pub fn with_authorized_sources(
        mut self,
        root_cutoff: Option<u64>,
        sources: Vec<(ExportRunEvidence, Option<u64>)>,
    ) -> Result<Self> {
        if !self.authorized_sources.is_empty() {
            return Err(super::qualification::StructuredStoreError::InvalidHistory);
        }
        let supplied_count = sources.iter().try_fold(0usize, |count, (source, _)| {
            count
                .checked_add(1)
                .and_then(|count| count.checked_add(source.authorized_sources.len()))
                .ok_or(super::qualification::StructuredStoreError::InvalidHistory)
        })?;
        if self.authorized_sources.len().saturating_add(supplied_count) > MAX_PORTABLE_SOURCE_RUNS {
            return Err(super::qualification::StructuredStoreError::InvalidHistory);
        }
        let expected_direct = self.direct_source_run_ids_through(root_cutoff)?;
        let mut seen = BTreeSet::new();
        let mut direct = BTreeSet::new();
        let mut source_cutoffs = BTreeMap::<RunId, Option<u64>>::new();
        for (source, cutoff) in sources {
            if !source.authorized_sources.is_empty() {
                return Err(super::qualification::StructuredStoreError::InvalidHistory);
            }
            if source.run_id() == self.run_id()
                || source.header().tenant_scope_id() != self.header.tenant_scope_id()
                || source.header().store_scope_id() != self.header.store_scope_id()
                || source.header().store_epoch() != self.header.store_epoch()
                || source.fragment.physical_target != self.fragment.physical_target
                || !seen.insert(source.run_id().clone())
            {
                return Err(super::qualification::StructuredStoreError::InvalidHistory);
            }
            direct.insert(source.run_id().clone());
            if source.direct_source_run_ids_through(cutoff)?.len() > MAX_PORTABLE_SOURCE_RUNS {
                return Err(super::qualification::StructuredStoreError::InvalidHistory);
            }
            source_cutoffs.insert(source.run_id().clone(), cutoff);
            self.authorized_sources.push(source.fragment);
            self.authorized_sources.extend(source.authorized_sources);
        }
        if !expected_direct.is_subset(&direct) {
            return Err(super::qualification::StructuredStoreError::InvalidHistory);
        }
        self.authorized_sources
            .sort_by(|left, right| left.run_id.cmp(&right.run_id));
        if self.authorized_sources.windows(2).any(|pair| {
            pair[0].run_id == pair[1].run_id
                || pair[0].header.tenant_scope_id != *self.header.tenant_scope_id()
                || pair[1].header.tenant_scope_id != *self.header.tenant_scope_id()
                || pair[0].header.store_scope_id != *self.header.store_scope_id()
                || pair[1].header.store_scope_id != *self.header.store_scope_id()
                || pair[0].header.store_epoch != self.header.store_epoch
                || pair[1].header.store_epoch != self.header.store_epoch
                || pair[0].physical_target != self.fragment.physical_target
                || pair[1].physical_target != self.fragment.physical_target
        }) {
            return Err(super::qualification::StructuredStoreError::InvalidHistory);
        }
        let fragments = self
            .authorized_sources
            .iter()
            .map(|fragment| (fragment.run_id.clone(), fragment))
            .collect::<BTreeMap<_, _>>();
        let mut graph = BTreeMap::<RunId, BTreeSet<RunId>>::new();
        graph.insert(self.fragment.run_id.clone(), expected_direct.clone());
        for fragment in &self.authorized_sources {
            let cutoff = source_cutoffs
                .get(&fragment.run_id)
                .copied()
                .ok_or(super::qualification::StructuredStoreError::InvalidHistory)?;
            graph.insert(
                fragment.run_id.clone(),
                fragment.direct_source_run_ids_through(cutoff)?,
            );
        }
        // Validate the complete supplied graph, not only the root's immediate
        // closure. This prevents a caller from bypassing the pure expander by
        // preloading an A↔B cycle into nested evidence.
        let mut colors = BTreeMap::<RunId, u8>::new();
        for start in graph.keys().cloned().collect::<Vec<_>>() {
            if colors.get(&start).copied().unwrap_or_default() != 0 {
                continue;
            }
            let mut stack = vec![(start, false)];
            while let Some((run_id, exiting)) = stack.pop() {
                if exiting {
                    colors.insert(run_id, 2);
                    continue;
                }
                match colors.get(&run_id).copied().unwrap_or_default() {
                    1 => return Err(super::qualification::StructuredStoreError::InvalidHistory),
                    2 => continue,
                    _ => {}
                }
                colors.insert(run_id.clone(), 1);
                stack.push((run_id.clone(), true));
                let children = graph
                    .get(&run_id)
                    .ok_or(super::qualification::StructuredStoreError::InvalidHistory)?;
                for child in children.iter().rev() {
                    if child == &self.fragment.run_id || !graph.contains_key(child) {
                        return Err(super::qualification::StructuredStoreError::InvalidHistory);
                    }
                    stack.push((child.clone(), false));
                }
            }
        }
        let mut reachable = BTreeSet::new();
        let mut pending = expected_direct;
        while let Some(run_id) = pending.pop_first() {
            if !reachable.insert(run_id.clone()) {
                continue;
            }
            fragments
                .get(&run_id)
                .ok_or(super::qualification::StructuredStoreError::InvalidHistory)?;
            for nested in graph
                .get(&run_id)
                .ok_or(super::qualification::StructuredStoreError::InvalidHistory)?
            {
                if nested == &self.fragment.run_id {
                    return Err(super::qualification::StructuredStoreError::InvalidHistory);
                }
                pending.insert(nested.clone());
            }
        }
        if reachable != fragments.keys().cloned().collect() {
            return Err(super::qualification::StructuredStoreError::InvalidHistory);
        }
        Ok(self)
    }

    /// Gives the portable encoder one borrowed view of the sealed fragments.
    pub fn with_encoder_view<C, T>(
        &self,
        _consumer: C,
        f: impl FnOnce(ExportEncoderView<'_>) -> T,
    ) -> T
    where
        C: mfm_authority_seal::ExportEncoderConsumerSeal,
    {
        f(ExportEncoderView {
            fragment: &self.fragment,
            authorized_sources: &self.authorized_sources,
        })
    }

    /// Returns the exact run identity.
    pub(crate) const fn run_id(&self) -> &RunId {
        &self.fragment.run_id
    }

    /// Returns the minimum export header.
    pub(crate) const fn header(&self) -> &RunEvidenceHeader {
        &self.header
    }

    /// Returns the exact semantic cutoff retained by this export evidence.
    pub const fn semantic_head(&self) -> &SemanticHead {
        &self.fragment.semantic_head
    }

    /// Returns every verified physical append head in sequence order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn journal_heads(&self) -> &[JournalHead] {
        &self.fragment.journal_heads
    }

    /// Returns every exact committed-batch envelope in append order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn batches(&self) -> &[CommittedBatch] {
        &self.fragment.batches
    }

    /// Returns every verified assigned record in physical append order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn records(&self) -> &[AssignedRecord] {
        &self.fragment.records
    }

    /// Returns the verified record prefix through the exact semantic head record.
    #[cfg(any(test, feature = "test-support"))]
    pub fn semantic_records(&self) -> impl Iterator<Item = &AssignedRecord> {
        let cutoff = match &self.fragment.semantic_head {
            SemanticHead::Genesis { admission_ref, .. } => admission_ref,
            SemanticHead::Transition { transition_ref, .. } => transition_ref,
        };
        self.fragment.records.iter().take_while(move |record| {
            record.record_ref.run_sequence < cutoff.run_sequence
                || (record.record_ref.run_sequence == cutoff.run_sequence
                    && record.record_ref.ordinal <= cutoff.ordinal)
        })
    }

    /// Returns verified objects first admitted no later than one physical append sequence.
    #[cfg(any(test, feature = "test-support"))]
    pub fn objects_through(&self, run_sequence: u64) -> impl Iterator<Item = &HistoryObject> {
        self.fragment
            .objects
            .iter()
            .filter(move |(sequence, _)| *sequence <= run_sequence)
            .map(|(_, object)| object)
    }

    /// Resolves one exact verified content-addressed history object.
    #[cfg(any(test, feature = "test-support"))]
    pub fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.fragment
            .objects
            .iter()
            .find(|(_, object)| &object.content_ref == content_ref)
            .map(|(_, object)| object)
    }

    /// Returns the terminal nominal operation-outcome reference, when closed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn closed_outcome_ref(&self) -> Option<&ContentRef> {
        self.fragment.closed_outcome_ref.as_ref()
    }

    /// Returns the sole callback-free cursor.
    #[cfg(any(test, feature = "test-support"))]
    pub fn cursor(&self) -> &super::ProgramCursor {
        &self.fragment.cursor
    }

    /// Returns every distinct prior-run producer referenced by retained fact selections.
    ///
    /// Discovery uses only identifier-bearing fact-selection metadata. It does not
    /// load those source runs and does not expose their object content for
    /// serialization. Callers must authorize each returned identity before any
    /// export byte is emitted.
    pub fn direct_source_run_ids(&self) -> Result<BTreeSet<RunId>> {
        self.direct_source_run_ids_through(None)
    }

    /// Returns fact routes whose consumer record is no later than the supplied
    /// physical append sequence. `None` retains the complete reduced history.
    pub fn fact_routes_through(&self, cutoff: Option<u64>) -> Vec<ExportFactRoute> {
        self.fragment.fact_routes_through(cutoff)
    }

    /// Returns distinct prior-run producers required through one exact export
    /// cutoff. The result is derived from retained verified routes and remains
    /// bounded by the portable source budget.
    pub fn direct_source_run_ids_through(&self, cutoff: Option<u64>) -> Result<BTreeSet<RunId>> {
        self.fragment.direct_source_run_ids_through(cutoff)
    }

    /// Returns whether this opaque evidence belongs to the caller's authorized
    /// tenant without exposing its persisted header.
    pub fn is_for_tenant(&self, tenant_scope_id: &TenantScopeId) -> bool {
        self.fragment.header.tenant_scope_id() == tenant_scope_id
    }

    /// Returns whether this opaque evidence is the requested run.
    pub fn is_for_run(&self, run_id: &RunId) -> bool {
        &self.fragment.run_id == run_id
    }

    /// Returns source prefixes in deterministic run-id order for the portable encoder.
    #[cfg(any(test, feature = "test-support"))]
    pub fn authorized_source_prefixes(&self) -> impl Iterator<Item = ExportEncoderSource<'_>> {
        self.authorized_sources.iter().map(ExportEncoderSource::new)
    }
}

impl ExportFragment {
    fn fact_routes_through(&self, cutoff: Option<u64>) -> Vec<ExportFactRoute> {
        let frontier = maximum_fact_barrier(self.batches.iter(), cutoff)
            .ok()
            .flatten();
        self.fact_routes
            .iter()
            .filter(|route| {
                frontier.as_ref().is_some_and(|frontier| {
                    route.publication_frontier.store_scope_id == frontier.store_scope_id
                        && route.publication_frontier.store_epoch == frontier.store_epoch
                        && route.publication_frontier.tenant_scope_id == frontier.tenant_scope_id
                        && route.publication_frontier.fact_order <= frontier.fact_order
                })
            })
            .cloned()
            .collect()
    }

    fn direct_source_run_ids_through(&self, cutoff: Option<u64>) -> Result<BTreeSet<RunId>> {
        if cutoff.is_none() {
            return Ok(self.direct_source_run_ids.clone());
        }
        let mut sources = BTreeSet::new();
        for route in self.fact_routes_through(cutoff) {
            let producer = route.producer_transition.run_id.clone();
            if producer == self.run_id || sources.contains(&producer) {
                continue;
            }
            if sources.len() >= MAX_PORTABLE_SOURCE_RUNS {
                return Err(super::qualification::StructuredStoreError::InvalidHistory);
            }
            sources.insert(producer);
        }
        Ok(sources)
    }

    fn from_verified(
        verified: VerifiedStructuredRun,
        physical_target: PhysicalTargetIdentity,
        fact_routes: Vec<ExportFactRoute>,
    ) -> Result<Self> {
        let batches = verified.batches().cloned().collect::<Vec<_>>();
        let batch_frames = batches
            .iter()
            .map(|batch| {
                canonical_json(batch)
                    .map(|json| json.as_bytes().to_vec())
                    .map_err(|_| super::qualification::StructuredStoreError::InvalidHistory)
            })
            .collect::<Result<Vec<_>>>()?;
        #[cfg(any(test, feature = "test-support"))]
        let mut objects = Vec::new();
        #[cfg(any(test, feature = "test-support"))]
        for batch in &batches {
            for object in &batch.objects {
                objects.push((batch.head.run_sequence, object.clone()));
            }
        }
        let fact_frontiers = batches
            .iter()
            .filter_map(|batch| match &batch.tenant_fact_coordinate {
                TenantFactCoordinate::FactPublication { frontier }
                | TenantFactCoordinate::FactSelectionBarrier { frontier } => Some(frontier.clone()),
                TenantFactCoordinate::None => None,
            })
            .collect::<Vec<_>>();
        let direct_source_run_ids = fact_routes
            .iter()
            .map(|route| route.producer_transition.run_id.clone())
            .filter(|run_id| run_id != verified.run_id())
            .collect();
        Ok(Self {
            run_id: verified.run_id().clone(),
            header: RunEvidenceHeader::from_admission(verified.admission()),
            physical_target,
            journal_head: verified.journal_head().clone(),
            semantic_head: verified.semantic_head().clone(),
            #[cfg(any(test, feature = "test-support"))]
            journal_heads: verified.journal_heads().cloned().collect(),
            records: verified.records().cloned().collect(),
            #[cfg(any(test, feature = "test-support"))]
            cursor: verified.cursor().clone(),
            #[cfg(any(test, feature = "test-support"))]
            closed_outcome_ref: verified.closed_outcome_ref().cloned(),
            batches,
            batch_frames,
            #[cfg(any(test, feature = "test-support"))]
            objects,
            direct_source_run_ids,
            fact_frontiers,
            fact_routes,
        })
    }
}

fn ensure_fact_route_capacity(current: usize) -> Result<()> {
    if current >= MAX_PORTABLE_FACT_ROUTES {
        return Err(super::StructuredStoreError::InvalidHistory);
    }
    Ok(())
}

fn maximum_fact_barrier<'a>(
    batches: impl IntoIterator<Item = &'a CommittedBatch>,
    cutoff: Option<u64>,
) -> Result<Option<TenantFactFrontier>> {
    let mut maximum: Option<TenantFactFrontier> = None;
    for batch in batches {
        if cutoff.is_some_and(|sequence| batch.head.run_sequence > sequence) {
            continue;
        }
        let TenantFactCoordinate::FactSelectionBarrier { frontier } = &batch.tenant_fact_coordinate
        else {
            continue;
        };
        if let Some(current) = &maximum {
            if current.store_scope_id != frontier.store_scope_id
                || current.store_epoch != frontier.store_epoch
                || current.tenant_scope_id != frontier.tenant_scope_id
            {
                return Err(super::StructuredStoreError::InvalidHistory);
            }
        }
        if maximum
            .as_ref()
            .is_none_or(|current| current.fact_order < frontier.fact_order)
        {
            maximum = Some(frontier.clone());
        }
    }
    Ok(maximum)
}

fn terminal_public_outcome(
    verified: &VerifiedStructuredRun,
) -> Result<Option<PublicTerminalOutcome>> {
    let Some(outcome_ref) = verified.closed_outcome_ref() else {
        return Ok(None);
    };
    let outcome_object = verified
        .object(outcome_ref)
        .ok_or(super::StructuredStoreError::InvalidHistory)?;
    let outcome: OperationOutcome<LexicalValueRef, LexicalValueRef> =
        serde_json::from_str(&outcome_object.canonical_json)
            .map_err(|_| super::StructuredStoreError::InvalidHistory)?;
    let (kind, value) = match outcome {
        OperationOutcome::Success(value) => ("success", value),
        OperationOutcome::Failure(value) => ("failure", value),
    };
    let selected = verified
        .object(&value.value.value_ref)
        .ok_or(super::StructuredStoreError::InvalidHistory)?;
    selected
        .validate()
        .map_err(|_| super::StructuredStoreError::InvalidHistory)?;
    Ok(Some(PublicTerminalOutcome {
        kind,
        value,
        canonical_value: selected.canonical_json.clone(),
    }))
}

/// Expands the bounded recursive export source graph from one already-loaded root.
///
/// `load_sources` must return only identifier-level direct dependencies for one
/// already-authorized run. Shared DAGs are accepted once; cycles and over-budget
/// closures fail closed. The returned set excludes the root and is ordered by
/// `RunId` so encounter order cannot change authorization or serialization
/// requirements.
pub fn expand_export_source_closure<E, F>(
    root_run_id: &RunId,
    root_sources: BTreeSet<RunId>,
    mut load_sources: F,
) -> std::result::Result<BTreeSet<RunId>, E>
where
    F: FnMut(&RunId) -> std::result::Result<BTreeSet<RunId>, E>,
    E: From<ExportSourceClosureError>,
{
    if root_sources.len() > MAX_PORTABLE_SOURCE_RUNS {
        return Err(ExportSourceClosureError::OverBudget.into());
    }
    let mut authorized = BTreeSet::new();
    let mut pending = root_sources.clone();
    pending.remove(root_run_id);
    if pending.len() > MAX_PORTABLE_SOURCE_RUNS {
        return Err(ExportSourceClosureError::OverBudget.into());
    }
    let mut edges: std::collections::BTreeMap<RunId, BTreeSet<RunId>> =
        std::collections::BTreeMap::from([(root_run_id.clone(), root_sources)]);

    while let Some(run_id) = pending.pop_first() {
        if run_id == *root_run_id || authorized.contains(&run_id) {
            continue;
        }
        if authorized.len() >= MAX_PORTABLE_SOURCE_RUNS {
            return Err(ExportSourceClosureError::OverBudget.into());
        }
        authorized.insert(run_id.clone());
        let sources = load_sources(&run_id)?;
        if sources.len() > MAX_PORTABLE_SOURCE_RUNS {
            return Err(ExportSourceClosureError::OverBudget.into());
        }
        edges.insert(run_id.clone(), sources.clone());
        for source in sources {
            if source != *root_run_id && !authorized.contains(&source) {
                if !pending.contains(&source) {
                    let queued_new = pending
                        .iter()
                        .filter(|run_id| !authorized.contains(*run_id))
                        .count();
                    if authorized.len().saturating_add(queued_new) >= MAX_PORTABLE_SOURCE_RUNS {
                        return Err(ExportSourceClosureError::OverBudget.into());
                    }
                }
                pending.insert(source);
            }
        }
    }

    reject_export_source_cycles(&edges)?;
    Ok(authorized)
}

/// Stable failure while expanding an export source closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSourceClosureError {
    /// The graph exceeds the fixed source-run budget.
    OverBudget,
    /// The dependency graph contains a cycle.
    Cycle,
}

fn reject_export_source_cycles(
    graph: &std::collections::BTreeMap<RunId, BTreeSet<RunId>>,
) -> std::result::Result<(), ExportSourceClosureError> {
    let nodes = graph
        .keys()
        .cloned()
        .chain(graph.values().flat_map(|deps| deps.iter().cloned()))
        .collect::<BTreeSet<_>>();
    let mut incoming = nodes
        .iter()
        .cloned()
        .map(|run_id| (run_id, 0usize))
        .collect::<std::collections::BTreeMap<_, _>>();
    for dependencies in graph.values() {
        for dependency in dependencies {
            if let Some(count) = incoming.get_mut(dependency) {
                *count = count.saturating_add(1);
            }
        }
    }
    let mut ready = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(run_id, _)| run_id.clone())
        .collect::<BTreeSet<_>>();
    let mut seen = 0usize;
    while let Some(run_id) = ready.pop_first() {
        seen = seen.saturating_add(1);
        if let Some(dependencies) = graph.get(&run_id) {
            for dependency in dependencies {
                if let Some(count) = incoming.get_mut(dependency) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        ready.insert(dependency.clone());
                    }
                }
            }
        }
    }
    if seen != nodes.len() {
        return Err(ExportSourceClosureError::Cycle);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/export_source_closure_unit.rs"]
mod export_source_closure_tests;

// ---------------------------------------------------------------------------
// Current Effect-entry-attention inventory
// ---------------------------------------------------------------------------

/// One run currently requiring manual Effect-entry attention.
///
/// Every field is re-derived from qualified, reduced history. The index route
/// only says *where to look*; it is never the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntryAttentionEntry {
    header: RunEvidenceHeader,
    journal_head: JournalHead,
    subject: mfm_runtime::history::EffectEntrySubject,
    resolution: mfm_runtime::history::EffectEntryAttentionResolution,
}

impl EffectEntryAttentionEntry {
    /// Minimum redaction-safe run header.
    pub const fn header(&self) -> &RunEvidenceHeader {
        &self.header
    }

    /// Exact head at which attention was re-derived.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Exact occurrence, attempt, and capability awaiting resolution.
    pub const fn subject(&self) -> &mfm_runtime::history::EffectEntrySubject {
        &self.subject
    }

    /// Exact reducer-derived resolution required for this entry.
    pub const fn resolution(&self) -> mfm_runtime::history::EffectEntryAttentionResolution {
        self.resolution
    }
}

/// One bounded page of current attention, ordered strictly by run identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntryAttentionPage {
    entries: Vec<EffectEntryAttentionEntry>,
    next_after_run_id: Option<RunId>,
}

impl EffectEntryAttentionPage {
    /// Runs on this page, in strict run order.
    pub fn entries(&self) -> &[EffectEntryAttentionEntry] {
        &self.entries
    }

    /// Continuation key, or `None` when the sweep reached the end.
    ///
    /// This advances by the last *scanned* route, not the last emitted result,
    /// so a live-sweep race yields a sparse page rather than a skipped run.
    pub const fn next_after_run_id(&self) -> Option<&RunId> {
        self.next_after_run_id.as_ref()
    }
}
