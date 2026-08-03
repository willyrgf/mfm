//! Callback-free replay and purpose-limited projections over the sole structured-history fold.

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{DigestAlgorithm, RunId, SchemaId};
use mfm_journal::structured::{
    AssignedRecord, ExternalAccessObserved, JournalHead, LexicalValueRef, ObservationOutcome,
    RunRecord, SemanticHead,
};
use mfm_spec::structured::OperationOutcome;
use mfm_store::structured::{
    StructuredHistoryBackend, StructuredRunHistoryReader, StructuredStoreError,
    VerifiedStructuredRun,
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

/// One canonical replay, trace, audit, or public-view projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredCanonicalProjection {
    schema_id: SchemaId,
    canonical: PlainCanonicalJsonBytes,
}

impl StructuredCanonicalProjection {
    fn encode(schema_name: &str, value: &impl Serialize) -> Result<Self> {
        let canonical = mfm_journal::structured::canonical_json(value)
            .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
        let schema_id = projection_schema_id(schema_name)?;
        Ok(Self {
            schema_id,
            canonical,
        })
    }

    /// Strictly decodes canonical float-free projection bytes under one schema name.
    pub fn strict_decode(schema_name: &str, bytes: &[u8]) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
        let _: serde_json::Value = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
        Ok(Self {
            schema_id: projection_schema_id(schema_name)?,
            canonical,
        })
    }

    /// Returns exact canonical projection bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }

    /// Returns the projection schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }
}

/// Canonical callback-free replay result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredReplayResult(StructuredCanonicalProjection);

impl StructuredReplayResult {
    /// Strictly decodes exact canonical structured replay-result bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let projection =
            StructuredCanonicalProjection::strict_decode("mfm.structured-replay-result", bytes)?;
        let wire: StructuredReplayResultWire = serde_json::from_slice(projection.as_bytes())
            .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
        wire.validate()?;
        Ok(Self(projection))
    }

    /// Returns exact canonical projection bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Returns the structured replay-result schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.0.schema_id()
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StructuredReplayResultWire {
    Verified {
        run_id: RunId,
        journal_head: JournalHead,
        semantic_head: Box<SemanticHead>,
        record_count: u64,
        status: StructuredReplayStatus,
    },
    ReproductionUnavailable {
        run_id: RunId,
        result: StructuredUnavailableResult,
    },
    ComparisonUnavailable {
        run_id: RunId,
        result: StructuredUnavailableResult,
    },
}

impl StructuredReplayResultWire {
    fn validate(self) -> Result<()> {
        match self {
            Self::Verified {
                run_id,
                journal_head,
                semantic_head,
                record_count,
                status,
            } => {
                let semantic_ref = match semantic_head.as_ref() {
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
                    | StructuredReplayStatus::WaitingReads
                    | StructuredReplayStatus::PossibleEntry
                    | StructuredReplayStatus::BlockedIntegrity
                    | StructuredReplayStatus::Closed => Ok(()),
                }
            }
            Self::ReproductionUnavailable { run_id, result }
            | Self::ComparisonUnavailable { run_id, result } => {
                if run_id.as_str().is_empty() {
                    return Err(StructuredReplayError::InvalidRecordedHistory);
                }
                match result {
                    StructuredUnavailableResult::Unavailable => Ok(()),
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StructuredReplayStatus {
    Actionable,
    WaitingReads,
    PossibleEntry,
    BlockedIntegrity,
    Closed,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StructuredUnavailableResult {
    Unavailable,
}
/// Canonical one-transition trace entry.
pub type StructuredTransitionTrace = StructuredCanonicalProjection;
/// Canonical one-access audit entry.
pub type StructuredAccessAuditEntry = StructuredCanonicalProjection;

/// One bounded head-fixed projection page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredProjectionPage<T> {
    run_id: RunId,
    at_journal_head: JournalHead,
    entries: Vec<T>,
    next_index: Option<u32>,
}

impl<T> StructuredProjectionPage<T> {
    /// Returns the inspected run.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact physical head fixed by the page.
    pub const fn at_journal_head(&self) -> &JournalHead {
        &self.at_journal_head
    }

    /// Returns declaration/commit-ordered projected entries.
    pub fn entries(&self) -> &[T] {
        &self.entries
    }

    /// Returns the next zero-based entry index, when more entries exist.
    pub const fn next_index(&self) -> Option<u32> {
        self.next_index
    }

    /// Consumes the page into transport-owned parts.
    pub fn into_parts(self) -> (RunId, JournalHead, Vec<T>, Option<u32>) {
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
/// authority. The returned view is derived only from committed records and
/// content-addressed objects.
pub async fn verify_recorded_history<B: StructuredHistoryBackend>(
    reader: &StructuredRunHistoryReader<B>,
    run_id: &RunId,
) -> Result<VerifiedStructuredRun> {
    reader
        .load_verified(run_id)
        .await
        .map_err(classify_store_error)
}

/// Projects the canonical callback-free replay summary of one verified prefix.
pub fn project_replay_result(run: &VerifiedStructuredRun) -> Result<StructuredReplayResult> {
    StructuredCanonicalProjection::encode(
        "mfm.structured-replay-result",
        &serde_json::json!({
            "kind": "verified",
            "run_id": run.run_id(),
            "journal_head": run.journal_head(),
            "semantic_head": run.semantic_head(),
            "record_count": run.records().len(),
            "status": frontier_status(run),
        }),
    )
    .map(StructuredReplayResult)
}

/// Projects the frozen result for exact reproduction when the admitted executable is unavailable.
pub fn project_unavailable_reproduction(run_id: &RunId) -> Result<StructuredReplayResult> {
    project_unavailable("reproduction_unavailable", run_id)
}

/// Projects the frozen result for current-candidate comparison when no candidate is available.
pub fn project_unavailable_comparison(run_id: &RunId) -> Result<StructuredReplayResult> {
    project_unavailable("comparison_unavailable", run_id)
}

fn project_unavailable(kind: &'static str, run_id: &RunId) -> Result<StructuredReplayResult> {
    StructuredCanonicalProjection::encode(
        "mfm.structured-replay-result",
        &serde_json::json!({
            "kind": kind,
            "run_id": run_id,
            "result": "unavailable",
        }),
    )
    .map(StructuredReplayResult)
}

/// Projects one bounded, head-fixed transition trace page.
pub fn project_transition_trace(
    run: &VerifiedStructuredRun,
    at_head: Option<&JournalHead>,
    start: u32,
    limit: u16,
) -> Result<StructuredProjectionPage<StructuredTransitionTrace>> {
    let at_journal_head = fixed_head(run, at_head)?;
    let transitions = records_at(run, &at_journal_head)
        .filter_map(|assigned| match &assigned.record {
            RunRecord::StateTransitionCommitted(transition) => Some((assigned, transition)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let (range, next_index) = page_range(transitions.len(), start, limit)?;
    let entries = transitions[range]
        .iter()
        .map(|(assigned, transition)| {
            StructuredCanonicalProjection::encode(
                "mfm.structured-transition-trace",
                &serde_json::json!({
                    "record_ref": assigned.record_ref,
                    "occurrence_id": transition.occurrence_id,
                    "occurrence_path_ref": transition.occurrence_path_ref,
                    "semantic_call_id": transition.semantic_call_id,
                    "input": transition.input,
                    "consumed_observation_ref": transition.consumed_observation_ref,
                    "outcome_ref": transition.outcome_ref,
                    "outcome": transition.outcome,
                    "facts": transition.facts,
                    "before_semantic_state_digest": transition.before_semantic_state_digest,
                    "after_semantic_state_digest": transition.after_semantic_state_digest,
                }),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(StructuredProjectionPage {
        run_id: run.run_id().clone(),
        at_journal_head,
        entries,
        next_index,
    })
}

/// Projects one bounded, head-fixed external-access audit page.
pub fn project_access_audit(
    run: &VerifiedStructuredRun,
    at_head: Option<&JournalHead>,
    start: u32,
    limit: u16,
) -> Result<StructuredProjectionPage<StructuredAccessAuditEntry>> {
    let at_journal_head = fixed_head(run, at_head)?;
    let records = records_at(run, &at_journal_head).collect::<Vec<_>>();
    let authorizations = records
        .iter()
        .filter_map(|assigned| match &assigned.record {
            RunRecord::ExternalAccessAuthorized(authorization) => Some((*assigned, authorization)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let (range, next_index) = page_range(authorizations.len(), start, limit)?;
    let entries = authorizations[range]
        .iter()
        .map(|(assigned, authorization)| {
            let observation = records
                .iter()
                .find_map(|candidate| match &candidate.record {
                    RunRecord::ExternalAccessObserved(observation)
                        if observation.access_attempt_id == authorization.access_attempt_id =>
                    {
                        Some((*candidate, observation))
                    }
                    _ => None,
                });
            access_projection(assigned, authorization, observation)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(StructuredProjectionPage {
        run_id: run.run_id().clone(),
        at_journal_head,
        entries,
        next_index,
    })
}

/// Resolves and validates the terminal selected typed value of a closed run.
pub fn project_operation_outcome(
    run: &VerifiedStructuredRun,
) -> Result<Option<StructuredOperationOutcomeView>> {
    let Some(outcome_ref) = run.closed_outcome_ref() else {
        return Ok(None);
    };
    let outcome = run
        .object(outcome_ref)
        .ok_or(StructuredReplayError::InvalidRecordedHistory)?;
    let outcome: OperationOutcome<LexicalValueRef, LexicalValueRef> = outcome
        .decode()
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
    let (kind, lexical) = match outcome {
        OperationOutcome::Success(value) => ("success", value),
        OperationOutcome::Failure(value) => ("failure", value),
    };
    let selected = run
        .object(&lexical.value.value_ref)
        .ok_or(StructuredReplayError::InvalidRecordedHistory)?;
    selected
        .validate()
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
    let canonical_value =
        PlainCanonicalJsonBytes::from_canonical_json_slice(selected.canonical_json.as_bytes())
            .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
    Ok(Some(StructuredOperationOutcomeView {
        kind,
        value: lexical,
        canonical_value,
    }))
}

fn frontier_status(run: &VerifiedStructuredRun) -> &'static str {
    use mfm_store::structured::StructuredFrontier;
    match run.frontier() {
        StructuredFrontier::Actions(_) => "actionable",
        StructuredFrontier::WaitingReads => "waiting_reads",
        StructuredFrontier::PossibleEntry => "possible_entry",
        StructuredFrontier::BlockedIntegrity => "blocked_integrity",
        StructuredFrontier::Complete => "closed",
    }
}

fn fixed_head(run: &VerifiedStructuredRun, requested: Option<&JournalHead>) -> Result<JournalHead> {
    let selected = requested.unwrap_or_else(|| run.journal_head());
    let index = usize::try_from(selected.run_sequence)
        .ok()
        .and_then(|sequence| sequence.checked_sub(1))
        .ok_or(StructuredReplayError::InvalidRecordedHistory)?;
    if run.journal_heads().get(index) != Some(selected) {
        return Err(StructuredReplayError::InvalidRecordedHistory);
    }
    Ok(selected.clone())
}

fn records_at<'a>(
    run: &'a VerifiedStructuredRun,
    head: &'a JournalHead,
) -> impl Iterator<Item = &'a AssignedRecord> {
    run.records()
        .iter()
        .filter(move |record| record.record_ref.run_sequence <= head.run_sequence)
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

fn access_projection(
    assigned: &AssignedRecord,
    authorization: &mfm_journal::structured::ExternalAccessAuthorized,
    observation: Option<(&AssignedRecord, &ExternalAccessObserved)>,
) -> Result<StructuredAccessAuditEntry> {
    let (observation_ref, status, outcome) = match observation {
        None => (None, "authorized", serde_json::Value::Null),
        Some((assigned, observed)) => (
            Some(&assigned.record_ref),
            observation_status(&observed.outcome),
            serde_json::to_value(&observed.outcome)
                .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?,
        ),
    };
    StructuredCanonicalProjection::encode(
        "mfm.structured-access-audit",
        &serde_json::json!({
            "authorization_ref": assigned.record_ref,
            "observation_ref": observation_ref,
            "access_attempt_id": authorization.access_attempt_id,
            "attempt_ordinal": authorization.attempt_ordinal,
            "occurrence_id": authorization.occurrence_id,
            "occurrence_path_ref": authorization.occurrence_path_ref,
            "semantic_call_id": authorization.semantic_call_id,
            "access_kind": authorization.access_kind,
            "capability_contract_ref": authorization.capability_contract_ref,
            "capability_implementation_ref": authorization.capability_implementation_ref,
            "adapter_contract_ref": authorization.adapter_contract_ref,
            "adapter_implementation_ref": authorization.adapter_implementation_ref,
            "request": authorization.request,
            "request_digest": authorization.request_digest,
            "physical_binding_ref": authorization.physical_binding_ref,
            "stable_resource_lineage_contract_ref":
                authorization.stable_resource_lineage_contract_ref,
            "status": status,
            "outcome": outcome,
        }),
    )
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

fn projection_schema_id(name: &str) -> Result<SchemaId> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.structured-projection-schema.v1:{name}:1").as_bytes()),
    )
    .map_err(|_| StructuredReplayError::InvalidRecordedHistory)
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
