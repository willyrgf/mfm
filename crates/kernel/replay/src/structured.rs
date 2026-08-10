//! Callback-free replay and purpose-limited projections over the sole structured-history fold.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::RunId;
use mfm_journal::structured::{JournalHead, LexicalValueRef, ObservationOutcome, SemanticHead};
use mfm_store::structured::{
    AuditAccessEntry, AuditRunEvidence, PublicRunEvidence, RecordedRunEvidence, ReplayRunReader,
    StructuredHistoryBackend, StructuredStoreError, TraceRunEvidence, TraceTransitionEntry,
};
use serde::{Deserialize, Serialize};

/// Result of callback-free structured-history replay verification.
pub type Result<T> = std::result::Result<T, StructuredReplayError>;

/// Stable redaction-safe failure from structured replay verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StructuredReplayError {
    /// No durable prefix exists for the selected run.
    #[error("structured replay run was not found")]
    RunNotFound,
    /// The durable history backend could not be read.
    #[error("structured replay history is unavailable")]
    StoreUnavailable,
    /// Persisted history or certification failed callback-free verification.
    #[error("structured replay history is invalid")]
    InvalidRecordedHistory,
}

/// One canonical projection retained behind a concrete public DTO.
///
/// A projection is codec-only: it carries no schema identity, because nothing
/// retains it under a `ContentRef`. Its exact bytes and its typed wire shape are
/// the whole contract.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedProjection {
    canonical: PlainCanonicalJsonBytes,
}

impl GeneratedProjection {
    fn encode(value: &impl Serialize) -> Result<Self> {
        let canonical = mfm_journal::structured::canonical_json(value)
            .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
        Ok(Self { canonical })
    }

    fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
        Ok(Self { canonical })
    }

    /// Returns exact canonical projection bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}

macro_rules! generated_projection {
    ($name:ident, $contract:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(GeneratedProjection);

        impl $name {
            fn encode(value: &impl Serialize) -> Result<Self> {
                GeneratedProjection::encode(value).map(Self)
            }

            /// Strictly decodes exact canonical projection bytes.
            pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
                let projection = GeneratedProjection::strict_decode(bytes)?;
                validate_projection($contract, projection.as_bytes())?;
                Ok(Self(projection))
            }

            /// Returns exact canonical projection bytes.
            pub fn as_bytes(&self) -> &[u8] {
                self.0.as_bytes()
            }
        }
    };
}

fn validate_projection(contract: &str, bytes: &[u8]) -> Result<()> {
    if contract == "mfm.structured-replay-result.v1" {
        let wire: StructuredReplayResultWire = serde_json::from_slice(bytes)
            .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
        wire.validate()?;
    }
    Ok(())
}

generated_projection!(
    StructuredReplayResult,
    "mfm.structured-replay-result.v1",
    "Canonical callback-free replay verification summary."
);
generated_projection!(
    StructuredTransitionTrace,
    "mfm.structured-transition-trace.v1",
    "One canonical state-transition trace entry."
);
generated_projection!(
    StructuredAccessAuditEntry,
    "mfm.structured-access-audit.v1",
    "One canonical external-access audit entry."
);

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StructuredReplayResultWire {
    Verified {
        version: String,
        run_id: RunId,
        journal_head: JournalHead,
        semantic_head: SemanticHead,
        record_count: u64,
        status: StructuredReplayStatus,
    },
}

impl StructuredReplayResultWire {
    fn validate(self) -> Result<()> {
        match self {
            Self::Verified {
                version,
                run_id,
                journal_head,
                semantic_head,
                record_count,
                status,
            } => {
                if version != "mfm.structured-replay-result.v1" {
                    return Err(StructuredReplayError::InvalidRecordedHistory);
                }
                let semantic_ref = match &semantic_head {
                    SemanticHead::Genesis { admission_ref, .. } => admission_ref,
                    SemanticHead::Transition { transition_ref, .. } => transition_ref,
                };
                if record_count == 0
                    || journal_head.run_sequence == 0
                    || semantic_ref.run_id != run_id
                    || semantic_ref.run_sequence > journal_head.run_sequence
                {
                    return Err(StructuredReplayError::InvalidRecordedHistory);
                }
                match status {
                    StructuredReplayStatus::Actionable
                    | StructuredReplayStatus::PossibleEntry
                    | StructuredReplayStatus::BlockedIntegrity
                    | StructuredReplayStatus::Closed => Ok(()),
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StructuredReplayStatus {
    Actionable,
    PossibleEntry,
    BlockedIntegrity,
    Closed,
}

/// One bounded head-fixed transition-trace projection page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredTransitionTracePage {
    run_id: RunId,
    at_journal_head: JournalHead,
    entries: Vec<StructuredTransitionTrace>,
    next_index: Option<u32>,
}

impl StructuredTransitionTracePage {
    /// Returns the inspected run.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact physical head fixed by the page.
    pub const fn at_journal_head(&self) -> &JournalHead {
        &self.at_journal_head
    }

    /// Returns declaration/commit-ordered projected entries.
    pub fn entries(&self) -> &[StructuredTransitionTrace] {
        &self.entries
    }

    /// Returns the next zero-based entry index, when more entries exist.
    pub const fn next_index(&self) -> Option<u32> {
        self.next_index
    }

    /// Consumes the page into transport-owned parts.
    pub fn into_parts(
        self,
    ) -> (
        RunId,
        JournalHead,
        Vec<StructuredTransitionTrace>,
        Option<u32>,
    ) {
        (
            self.run_id,
            self.at_journal_head,
            self.entries,
            self.next_index,
        )
    }
}

/// One bounded head-fixed external-access audit projection page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredAccessAuditPage {
    run_id: RunId,
    at_journal_head: JournalHead,
    entries: Vec<StructuredAccessAuditEntry>,
    next_index: Option<u32>,
}

impl StructuredAccessAuditPage {
    /// Returns the inspected run.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact physical head fixed by the page.
    pub const fn at_journal_head(&self) -> &JournalHead {
        &self.at_journal_head
    }

    /// Returns declaration/commit-ordered projected entries.
    pub fn entries(&self) -> &[StructuredAccessAuditEntry] {
        &self.entries
    }

    /// Returns the next zero-based entry index, when more entries exist.
    pub const fn next_index(&self) -> Option<u32> {
        self.next_index
    }

    /// Consumes the page into transport-owned parts.
    pub fn into_parts(
        self,
    ) -> (
        RunId,
        JournalHead,
        Vec<StructuredAccessAuditEntry>,
        Option<u32>,
    ) {
        (
            self.run_id,
            self.at_journal_head,
            self.entries,
            self.next_index,
        )
    }
}

/// Exact terminal operation outcome and its selected typed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOperationOutcomeView {
    kind: &'static str,
    value: LexicalValueRef,
    canonical_value: PlainCanonicalJsonBytes,
}

impl StructuredOperationOutcomeView {
    /// Returns `success` or `failure`.
    pub const fn kind(&self) -> &'static str {
        self.kind
    }

    /// Returns the exact producer-bound terminal value reference.
    pub const fn value(&self) -> &LexicalValueRef {
        &self.value
    }

    /// Returns the selected typed value bytes.
    pub const fn canonical_value(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical_value
    }
}

/// Replays one complete structured run through the same callback-free store fold.
///
/// The reader exposes no writer, process callback, adapter, signer, or provider
/// authority. The returned sealed evidence is derived only from committed records
/// and content-addressed objects and cannot be used as export or public evidence.
pub async fn verify_recorded_history<B: StructuredHistoryBackend>(
    reader: &ReplayRunReader<B>,
    run_id: &RunId,
) -> Result<RecordedRunEvidence> {
    reader
        .load_for_recorded_verify(run_id)
        .await
        .map_err(classify_store_error)
}

/// Projects the canonical callback-free replay summary of one verified prefix.
pub fn project_replay_result(run: &RecordedRunEvidence) -> Result<StructuredReplayResult> {
    let result = StructuredReplayResult::encode(&serde_json::json!({
        "version": "mfm.structured-replay-result.v1",
        "kind": "verified",
        "run_id": run.run_id(),
        "journal_head": run.journal_head(),
        "semantic_head": run.semantic_head(),
        "record_count": run.record_count(),
        "status": run.status().as_str(),
    }))?;
    validate_projection("mfm.structured-replay-result.v1", result.as_bytes())?;
    Ok(result)
}

/// Projects one bounded, head-fixed transition trace page.
pub fn project_transition_trace(
    run: &TraceRunEvidence,
    at_head: Option<&JournalHead>,
    start: u32,
    limit: u16,
) -> Result<StructuredTransitionTracePage> {
    let at_journal_head = fixed_head(run.journal_head(), run.journal_heads(), at_head)?;
    let transitions = trace_records_at(run.records(), &at_journal_head).collect::<Vec<_>>();
    let (range, next_index) = page_range(transitions.len(), start, limit)?;
    let entries = transitions[range]
        .iter()
        .map(|transition| {
            StructuredTransitionTrace::encode(&serde_json::json!({
                "version": "mfm.structured-transition-trace.v1",
                "record_ref": transition.record_ref(),
                "occurrence_id": transition.occurrence_id(),
                "occurrence_path_ref": transition.occurrence_path_ref(),
                "semantic_call_id": transition.semantic_call_id(),
                "input": transition.input(),
                "consumed_observation_ref": transition.consumed_observation_ref(),
                "outcome_ref": transition.outcome_ref(),
                "outcome": transition.outcome(),
                "facts": transition.facts(),
                "before_semantic_state_digest": transition.before_semantic_state_digest(),
                "after_semantic_state_digest": transition.after_semantic_state_digest(),
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(StructuredTransitionTracePage {
        run_id: run.run_id().clone(),
        at_journal_head,
        entries,
        next_index,
    })
}

/// Projects one bounded, head-fixed external-access audit page.
pub fn project_access_audit(
    run: &AuditRunEvidence,
    at_head: Option<&JournalHead>,
    start: u32,
    limit: u16,
) -> Result<StructuredAccessAuditPage> {
    let at_journal_head = fixed_head(run.journal_head(), run.journal_heads(), at_head)?;
    let authorizations = audit_records_at(run.records(), &at_journal_head).collect::<Vec<_>>();
    let (range, next_index) = page_range(authorizations.len(), start, limit)?;
    let entries = authorizations[range]
        .iter()
        .map(|authorization| access_projection(authorization))
        .collect::<Result<Vec<_>>>()?;
    Ok(StructuredAccessAuditPage {
        run_id: run.run_id().clone(),
        at_journal_head,
        entries,
        next_index,
    })
}

/// Resolves and validates the terminal selected typed value of a closed run.
pub fn project_operation_outcome(
    run: &PublicRunEvidence,
) -> Result<Option<StructuredOperationOutcomeView>> {
    let Some(outcome) = run.terminal_outcome() else {
        return Ok(None);
    };
    Ok(Some(StructuredOperationOutcomeView {
        kind: outcome.kind(),
        value: outcome.value().clone(),
        canonical_value: PlainCanonicalJsonBytes::from_canonical_json_slice(
            outcome.canonical_value().as_bytes(),
        )
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?,
    }))
}

fn fixed_head(
    journal_head: &JournalHead,
    journal_heads: &[JournalHead],
    requested: Option<&JournalHead>,
) -> Result<JournalHead> {
    let selected = requested.unwrap_or(journal_head);
    let index = usize::try_from(selected.run_sequence)
        .ok()
        .and_then(|sequence| sequence.checked_sub(1))
        .ok_or(StructuredReplayError::InvalidRecordedHistory)?;
    if journal_heads.get(index) != Some(selected) {
        return Err(StructuredReplayError::InvalidRecordedHistory);
    }
    Ok(selected.clone())
}

fn trace_records_at<'a>(
    records: &'a [TraceTransitionEntry],
    head: &'a JournalHead,
) -> impl Iterator<Item = &'a TraceTransitionEntry> {
    records
        .iter()
        .filter(move |record| record.record_ref().run_sequence <= head.run_sequence)
}

fn audit_records_at<'a>(
    records: &'a [AuditAccessEntry],
    head: &'a JournalHead,
) -> impl Iterator<Item = &'a AuditAccessEntry> {
    records
        .iter()
        .filter(move |record| record.authorization_ref().run_sequence <= head.run_sequence)
}

fn page_range(
    count: usize,
    start: u32,
    limit: u16,
) -> Result<(std::ops::Range<usize>, Option<u32>)> {
    if limit == 0 {
        return Err(StructuredReplayError::InvalidRecordedHistory);
    }
    let start =
        usize::try_from(start).map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
    if start > count {
        return Err(StructuredReplayError::InvalidRecordedHistory);
    }
    let end = start.saturating_add(usize::from(limit)).min(count);
    let next_index = (end < count)
        .then(|| u32::try_from(end))
        .transpose()
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
    Ok((start..end, next_index))
}

fn access_projection(authorization: &AuditAccessEntry) -> Result<StructuredAccessAuditEntry> {
    let (observation_ref, status, outcome) = match authorization.observation() {
        None => (None, "authorized", serde_json::Value::Null),
        Some(observation) => (
            Some(observation.record_ref()),
            observation_status(observation.outcome()),
            serde_json::to_value(observation.outcome())
                .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?,
        ),
    };
    StructuredAccessAuditEntry::encode(&serde_json::json!({
        "version": "mfm.structured-access-audit.v1",
        "authorization_ref": authorization.authorization_ref(),
        "observation_ref": observation_ref,
        "access_attempt_id": authorization.access_attempt_id(),
        "attempt_ordinal": authorization.attempt_ordinal(),
        "occurrence_id": authorization.occurrence_id(),
        "occurrence_path_ref": authorization.occurrence_path_ref(),
        "semantic_call_id": authorization.semantic_call_id(),
        "access_kind": authorization.access_kind(),
        "capability_contract_ref": authorization.capability_contract_ref(),
        "capability_implementation_ref": authorization.capability_implementation_ref(),
        "adapter_contract_ref": authorization.adapter_contract_ref(),
        "adapter_implementation_ref": authorization.adapter_implementation_ref(),
        "request": authorization.request(),
        "request_digest": authorization.request_digest(),
        "physical_binding_ref": authorization.physical_binding_ref(),
        "stable_resource_lineage_contract_ref":
            authorization.stable_resource_lineage_contract_ref(),
        "status": status,
        "outcome": outcome,
    }))
}

fn observation_status(outcome: &ObservationOutcome) -> &'static str {
    match outcome {
        ObservationOutcome::Returned { .. } => "returned",
        ObservationOutcome::SafeFailure { .. } => "safe_failure",
        ObservationOutcome::SupersededBeforeEntry { .. } => "superseded_before_entry",
        ObservationOutcome::EntryUnknown { .. } => "entry_unknown",
        ObservationOutcome::IntegrityFault { .. } => "integrity_fault",
    }
}

fn classify_store_error(error: StructuredStoreError) -> StructuredReplayError {
    match error {
        StructuredStoreError::RunNotFound => StructuredReplayError::RunNotFound,
        StructuredStoreError::BackendUnavailable | StructuredStoreError::AcknowledgementUnknown => {
            StructuredReplayError::StoreUnavailable
        }
        StructuredStoreError::InvalidHistory
        | StructuredStoreError::CandidateRejected
        | StructuredStoreError::Certification
        | StructuredStoreError::StaleHead
        | StructuredStoreError::AppendConflict => StructuredReplayError::InvalidRecordedHistory,
    }
}
