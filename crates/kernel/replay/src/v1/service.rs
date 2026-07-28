use std::collections::BTreeSet;

use mfm_ids::RunId;
use mfm_journal::v1::JournalHead;
use mfm_store::v1::{
    Export, FactSelectionStore, InspectAudit, InspectTrace, Replay, RunAccessAuthority,
    RunJournalStore, TransitionTracePageRequest, TransitionTraceSourceRequirements,
    VerifiedRunView,
};

use super::{
    store_error, AccessAuditEntry, AccessAuditPage, CanonicalTransitionTrace, ReplayError, Result,
    TransitionTracePage, VerifiedHistoryResult,
};

/// Verifies one complete recorded history without invoking runtime callbacks or live IO.
///
/// Every retained fact selection is rechecked by the authoritative store against its exact
/// immutable private attestation and dense writer prefix at the same verified physical head.
pub async fn verify_recorded_history<S: FactSelectionStore>(
    store: &S,
    authority: &RunAccessAuthority<Replay>,
) -> Result<VerifiedHistoryResult> {
    let view = load_verified_view(store, authority).await?;
    let completeness = store
        .verify_fact_selection_completeness(authority, &view)
        .await
        .map_err(|error| store_error(&error))?;
    if completeness.store_identity() != view.store_identity()
        || completeness.tenant_scope_id() != view.tenant_scope_id()
        || completeness.run_id() != view.run_id()
        || completeness.journal_head() != view.journal_head()
    {
        return Err(ReplayError::InvalidRecordedHistory);
    }
    let fact_selections = completeness.fact_selections().to_vec();
    Ok(VerifiedHistoryResult::new(view, fact_selections))
}

/// Inspects one independently authorized, head-fixed page of safe access-audit entries.
///
/// Omitting `complete_as_of_journal_head` atomically fixes the page to the
/// store's current physical head. The application owns opaque cursor
/// encoding and supplies the decoded head and zero-based start index here.
pub async fn inspect_access_audit<S: RunJournalStore>(
    store: &S,
    authority: &RunAccessAuthority<InspectAudit>,
    complete_as_of_journal_head: Option<&JournalHead>,
    start: u32,
    limit: u16,
) -> Result<AccessAuditPage> {
    let page = store
        .inspect_access_audit(authority, complete_as_of_journal_head, start, limit)
        .await
        .map_err(|error| store_error(&error))?;
    let projected = page
        .entries()
        .map(|entry| {
            AccessAuditEntry::new(
                entry.authorization_ref().clone(),
                entry.observation_ref().cloned(),
                entry.authorization_journal_head().clone(),
                entry.observation_journal_head().cloned(),
                entry.capability_binding_ref().clone(),
                entry.capability_operation_id().clone(),
                entry.request_ref().clone(),
                entry.status(),
                entry.result_ref().cloned(),
                entry.failure().cloned(),
                entry.effect_key().cloned(),
                entry.delivery_audit_ref().cloned(),
                entry.delivery_audit_terminal(),
            )
        })
        .collect();
    Ok(AccessAuditPage::new(
        page.run_id().clone(),
        page.complete_as_of_journal_head().clone(),
        projected,
        page.has_more(),
        page.next_index(),
    ))
}

/// Discovers direct source runs named by one exact head-fixed transition page.
///
/// The opaque requirements retain the verified root view and page coordinate. Its sorted source
/// identifiers grant no source access; the application must make a fresh separate `InspectTrace`
/// decision for each source before the second phase.
pub async fn discover_transition_trace_sources<S: RunJournalStore>(
    store: &S,
    authority: &RunAccessAuthority<InspectTrace>,
    request: TransitionTracePageRequest,
) -> Result<TransitionTraceSourceRequirements> {
    store
        .discover_transition_trace_sources(authority, request)
        .await
        .map_err(|error| store_error(&error))
}

/// Consumes one exact page token and renders only independently authorized source values.
///
/// Source authorities must be a canonical sorted, duplicate-free subset of the page requirements.
/// Missing authorities and authorized-but-absent source runs produce the same digest-only
/// redaction; corrupt authorized source history fails verification.
pub async fn inspect_transition_trace<S: RunJournalStore>(
    store: &S,
    authority: &RunAccessAuthority<InspectTrace>,
    requirements: TransitionTraceSourceRequirements,
    source_authorities: &[RunAccessAuthority<InspectTrace>],
) -> Result<TransitionTracePage> {
    let page = store
        .inspect_transition_trace(authority, requirements, source_authorities)
        .await
        .map_err(|error| store_error(&error))?;
    let transitions = page
        .transitions()
        .iter()
        .map(|trace| CanonicalTransitionTrace::encode(trace.canonical_value()))
        .collect::<Result<Vec<_>>>()?;
    Ok(TransitionTracePage::new(
        page.run_id().clone(),
        page.at_journal_head().clone(),
        transitions,
        page.has_more(),
        page.next_index(),
    ))
}

/// Discovers every direct source run fixed by one export-authorized admission.
///
/// This includes admitted sources that no transition ultimately consumes.
/// Discovery is deliberately non-recursive. The application authorizes each
/// returned run independently, repeats discovery for that newly authorized
/// source, and supplies the complete exact authority set to export.
pub async fn required_export_source_run_ids<S: RunJournalStore>(
    store: &S,
    authority: &RunAccessAuthority<Export>,
) -> Result<Vec<RunId>> {
    let view = load_verified_view(store, authority).await?;
    Ok(view
        .admission_source_requirements()
        .cross_run_sources()
        .iter()
        .map(|requirement| requirement.source_run_id().clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

pub(crate) async fn load_verified_view<S, G>(
    store: &S,
    authority: &RunAccessAuthority<G>,
) -> Result<VerifiedRunView>
where
    S: RunJournalStore,
    G: mfm_store::v1::CommittedJournalLoadGrant,
{
    let journal = store
        .load_committed_journal(authority)
        .await
        .map_err(|error| store_error(&error))?;
    journal
        .verify_recorded_history()
        .map_err(|error| store_error(&error))
}
