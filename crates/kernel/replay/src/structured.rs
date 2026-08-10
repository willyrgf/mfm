//! Callback-free replay and purpose-limited projections over qualified reduction.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{
    AccessAttemptId, ContentRef, OccurrenceId, RequestDigest, RunId, RunSemanticStateDigest,
    SemanticCallId,
};
use mfm_journal::structured::{
    derive_record_hash, AccessKind, CommittedFactRef, ExternalAccessObserved, JournalHead,
    LexicalValueRef, ObservationOutcome, RecordHashPreimage, RecordRef, RunRecord, SemanticHead,
    StateOutcomeRef, StateTransitionCommitted, TypedValueRef,
};
use mfm_store::structured::{
    AuditAccessEntry, AuditRunEvidence, PublicRunEvidence, RecordedRunEvidence, ReplayRunReader,
    StructuredHistoryBackend, StructuredStoreError, TraceRunEvidence, TraceTransitionEntry,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

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

fn encode_exact_projection<T: Serialize>(wire: &T) -> Result<PlainCanonicalJsonBytes> {
    mfm_journal::structured::canonical_json(wire)
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)
}

fn decode_exact_projection<T>(bytes: &[u8]) -> Result<(PlainCanonicalJsonBytes, T)>
where
    T: DeserializeOwned + Serialize,
{
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
    let wire: T = serde_json::from_slice(canonical.as_bytes())
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
    if encode_exact_projection(&wire)?.as_bytes() != canonical.as_bytes() {
        return Err(StructuredReplayError::InvalidRecordedHistory);
    }
    Ok((canonical, wire))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    fn validate(&self) -> Result<()> {
        match self {
            Self::Verified {
                version,
                run_id,
                journal_head,
                semantic_head,
                record_count,
                status,
            } => {
                let semantic_ref = match &semantic_head {
                    SemanticHead::Genesis { admission_ref, .. } => admission_ref,
                    SemanticHead::Transition { transition_ref, .. } => transition_ref,
                };
                let maximum_record_count = journal_head.run_sequence.checked_mul(2);
                if version != "mfm.structured-replay-result.v1"
                    || record_count == &0
                    || journal_head.run_sequence == 0
                    || semantic_ref.run_id != *run_id
                    || semantic_ref.run_sequence == 0
                    || semantic_ref.run_sequence > journal_head.run_sequence
                    || *record_count < journal_head.run_sequence
                    || maximum_record_count.is_none_or(|maximum| *record_count > maximum)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StructuredReplayStatus {
    Actionable,
    PossibleEntry,
    BlockedIntegrity,
    Closed,
}

impl StructuredReplayStatus {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "actionable" => Ok(Self::Actionable),
            "possible_entry" => Ok(Self::PossibleEntry),
            "blocked_integrity" => Ok(Self::BlockedIntegrity),
            "closed" => Ok(Self::Closed),
            _ => Err(StructuredReplayError::InvalidRecordedHistory),
        }
    }
}

/// Canonical callback-free replay verification summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredReplayResult {
    canonical: PlainCanonicalJsonBytes,
}

impl StructuredReplayResult {
    fn from_wire(wire: &StructuredReplayResultWire) -> Result<Self> {
        wire.validate()?;
        let canonical = encode_exact_projection(wire)?;
        Self::strict_decode(canonical.as_bytes())
    }

    /// Strictly decodes the one current replay-result language.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let (canonical, wire) = decode_exact_projection::<StructuredReplayResultWire>(bytes)?;
        wire.validate()?;
        Ok(Self { canonical })
    }

    /// Returns exact canonical projection bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredTransitionTraceWire {
    version: String,
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

impl StructuredTransitionTraceWire {
    fn record(&self) -> RunRecord {
        RunRecord::StateTransitionCommitted(StateTransitionCommitted {
            occurrence_id: self.occurrence_id.clone(),
            occurrence_path_ref: self.occurrence_path_ref.clone(),
            semantic_call_id: self.semantic_call_id.clone(),
            input: self.input.clone(),
            consumed_observation_ref: self.consumed_observation_ref.clone(),
            outcome_ref: self.outcome_ref.clone(),
            outcome: self.outcome.clone(),
            facts: self.facts.clone(),
            before_semantic_state_digest: self.before_semantic_state_digest.clone(),
            after_semantic_state_digest: self.after_semantic_state_digest.clone(),
        })
    }

    fn validate(&self) -> Result<()> {
        if self.version != "mfm.structured-transition-trace.v1"
            || self.record_ref.run_sequence == 0
            || self.record_ref.ordinal != 0
            || self
                .consumed_observation_ref
                .as_ref()
                .is_some_and(|record| {
                    record.run_id != self.record_ref.run_id
                        || record.run_sequence == 0
                        || record.run_sequence >= self.record_ref.run_sequence
                        || record.ordinal != 0
                })
        {
            return Err(StructuredReplayError::InvalidRecordedHistory);
        }
        let record = self.record();
        let expected_hash = derive_record_hash(&RecordHashPreimage {
            run_id: &self.record_ref.run_id,
            run_sequence: self.record_ref.run_sequence,
            ordinal: self.record_ref.ordinal,
            record: &record,
        })
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
        if self.record_ref.record_hash != expected_hash {
            return Err(StructuredReplayError::InvalidRecordedHistory);
        }
        Ok(())
    }
}

/// One canonical state-transition trace entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredTransitionTrace {
    canonical: PlainCanonicalJsonBytes,
}

impl StructuredTransitionTrace {
    fn from_wire(wire: &StructuredTransitionTraceWire) -> Result<Self> {
        wire.validate()?;
        let canonical = encode_exact_projection(wire)?;
        Self::strict_decode(canonical.as_bytes())
    }

    /// Strictly decodes the one current transition-trace language.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let (canonical, wire) = decode_exact_projection::<StructuredTransitionTraceWire>(bytes)?;
        wire.validate()?;
        Ok(Self { canonical })
    }

    /// Returns exact canonical projection bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StructuredAccessStatusWire {
    Authorized,
    Returned,
    SafeFailure,
    SupersededBeforeEntry,
    EntryUnknown,
    IntegrityFault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredAccessAuditWire {
    version: String,
    authorization_ref: RecordRef,
    observation_ref: Option<RecordRef>,
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
    status: StructuredAccessStatusWire,
    outcome: Option<ObservationOutcome>,
}

impl StructuredAccessAuditWire {
    fn validate(&self) -> Result<()> {
        if self.version != "mfm.structured-access-audit.v1"
            || self.authorization_ref.run_sequence == 0
            || self.authorization_ref.ordinal != 0
        {
            return Err(StructuredReplayError::InvalidRecordedHistory);
        }
        match (
            self.observation_ref.as_ref(),
            self.outcome.as_ref(),
            self.status,
        ) {
            (None, None, StructuredAccessStatusWire::Authorized) => return Ok(()),
            (Some(observation_ref), Some(outcome), status)
                if observation_status_wire(outcome) == status =>
            {
                if observation_ref.run_id != self.authorization_ref.run_id
                    || observation_ref.run_sequence <= self.authorization_ref.run_sequence
                    || observation_ref.ordinal != 0
                    || (matches!(
                        outcome,
                        ObservationOutcome::SupersededBeforeEntry { .. }
                            | ObservationOutcome::EntryUnknown { .. }
                    ) && self.access_kind != AccessKind::Effect)
                {
                    return Err(StructuredReplayError::InvalidRecordedHistory);
                }
                let record = RunRecord::ExternalAccessObserved(ExternalAccessObserved {
                    authorization_ref: self.authorization_ref.clone(),
                    access_attempt_id: self.access_attempt_id.clone(),
                    outcome: outcome.clone(),
                });
                let expected_hash = derive_record_hash(&RecordHashPreimage {
                    run_id: &observation_ref.run_id,
                    run_sequence: observation_ref.run_sequence,
                    ordinal: observation_ref.ordinal,
                    record: &record,
                })
                .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
                if observation_ref.record_hash != expected_hash {
                    return Err(StructuredReplayError::InvalidRecordedHistory);
                }
                return Ok(());
            }
            _ => {}
        }
        Err(StructuredReplayError::InvalidRecordedHistory)
    }
}

/// One canonical external-access audit entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredAccessAuditEntry {
    canonical: PlainCanonicalJsonBytes,
}

impl StructuredAccessAuditEntry {
    fn from_wire(wire: &StructuredAccessAuditWire) -> Result<Self> {
        wire.validate()?;
        let canonical = encode_exact_projection(wire)?;
        Self::strict_decode(canonical.as_bytes())
    }

    /// Strictly decodes the one current access-audit language.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let (canonical, wire) = decode_exact_projection::<StructuredAccessAuditWire>(bytes)?;
        wire.validate()?;
        Ok(Self { canonical })
    }

    /// Returns exact canonical projection bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
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

/// Replays one complete structured run through the same callback-free reducer.
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
    let record_count = u64::try_from(run.record_count())
        .map_err(|_| StructuredReplayError::InvalidRecordedHistory)?;
    StructuredReplayResult::from_wire(&StructuredReplayResultWire::Verified {
        version: "mfm.structured-replay-result.v1".to_owned(),
        run_id: run.run_id().clone(),
        journal_head: run.journal_head().clone(),
        semantic_head: run.semantic_head().clone(),
        record_count,
        status: StructuredReplayStatus::parse(run.status().as_str())?,
    })
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
            StructuredTransitionTrace::from_wire(&StructuredTransitionTraceWire {
                version: "mfm.structured-transition-trace.v1".to_owned(),
                record_ref: transition.record_ref().clone(),
                occurrence_id: transition.occurrence_id().clone(),
                occurrence_path_ref: transition.occurrence_path_ref().clone(),
                semantic_call_id: transition.semantic_call_id().clone(),
                input: transition.input().clone(),
                consumed_observation_ref: transition.consumed_observation_ref().cloned(),
                outcome_ref: transition.outcome_ref().clone(),
                outcome: transition.outcome().clone(),
                facts: transition.facts().to_vec(),
                before_semantic_state_digest: transition.before_semantic_state_digest().clone(),
                after_semantic_state_digest: transition.after_semantic_state_digest().clone(),
            })
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
        .map(|authorization| access_projection(authorization, &at_journal_head))
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

fn access_projection(
    authorization: &AuditAccessEntry,
    at_journal_head: &JournalHead,
) -> Result<StructuredAccessAuditEntry> {
    let observation = authorization.observation().filter(|observation| {
        observation.record_ref().run_sequence <= at_journal_head.run_sequence
    });
    let (observation_ref, status, outcome) = match observation {
        None => (None, StructuredAccessStatusWire::Authorized, None),
        Some(observation) => (
            Some(observation.record_ref().clone()),
            observation_status_wire(observation.outcome()),
            Some(observation.outcome().clone()),
        ),
    };
    StructuredAccessAuditEntry::from_wire(&StructuredAccessAuditWire {
        version: "mfm.structured-access-audit.v1".to_owned(),
        authorization_ref: authorization.authorization_ref().clone(),
        observation_ref,
        access_attempt_id: authorization.access_attempt_id().clone(),
        attempt_ordinal: authorization.attempt_ordinal(),
        occurrence_id: authorization.occurrence_id().clone(),
        occurrence_path_ref: authorization.occurrence_path_ref().clone(),
        semantic_call_id: authorization.semantic_call_id().clone(),
        access_kind: authorization.access_kind(),
        capability_contract_ref: authorization.capability_contract_ref().clone(),
        capability_implementation_ref: authorization.capability_implementation_ref().clone(),
        adapter_contract_ref: authorization.adapter_contract_ref().clone(),
        adapter_implementation_ref: authorization.adapter_implementation_ref().clone(),
        request: authorization.request().clone(),
        request_digest: authorization.request_digest().clone(),
        physical_binding_ref: authorization.physical_binding_ref().clone(),
        stable_resource_lineage_contract_ref: authorization
            .stable_resource_lineage_contract_ref()
            .cloned(),
        status,
        outcome,
    })
}

fn observation_status_wire(outcome: &ObservationOutcome) -> StructuredAccessStatusWire {
    match outcome {
        ObservationOutcome::Returned { .. } => StructuredAccessStatusWire::Returned,
        ObservationOutcome::SafeFailure { .. } => StructuredAccessStatusWire::SafeFailure,
        ObservationOutcome::SupersededBeforeEntry { .. } => {
            StructuredAccessStatusWire::SupersededBeforeEntry
        }
        ObservationOutcome::EntryUnknown { .. } => StructuredAccessStatusWire::EntryUnknown,
        ObservationOutcome::IntegrityFault { .. } => StructuredAccessStatusWire::IntegrityFault,
    }
}

fn classify_store_error(error: StructuredStoreError) -> StructuredReplayError {
    match error {
        StructuredStoreError::RunNotFound => StructuredReplayError::RunNotFound,
        StructuredStoreError::BackendUnavailable | StructuredStoreError::AcknowledgementUnknown => {
            StructuredReplayError::StoreUnavailable
        }
        StructuredStoreError::InvalidHistory
        | StructuredStoreError::CapacityExceeded
        | StructuredStoreError::CandidateRejected
        | StructuredStoreError::Certification
        | StructuredStoreError::StaleHead
        | StructuredStoreError::AppendConflict => StructuredReplayError::InvalidRecordedHistory,
    }
}

#[cfg(test)]
mod tests {
    use mfm_canonical::sha256_digest_bytes;
    use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, JournalRecordHash, SchemaId};

    use super::*;

    #[test]
    fn transition_trace_owner_rederives_its_record_hash_and_rejects_cross_owner_bytes() {
        let mut wire = transition_wire();
        let record = wire.record();
        wire.record_ref.record_hash = derive_record_hash(&RecordHashPreimage {
            run_id: &wire.record_ref.run_id,
            run_sequence: wire.record_ref.run_sequence,
            ordinal: wire.record_ref.ordinal,
            record: &record,
        })
        .expect("transition record hash");

        let projection = StructuredTransitionTrace::from_wire(&wire).expect("transition trace");
        assert_eq!(
            StructuredTransitionTrace::strict_decode(projection.as_bytes())
                .expect("strict transition trace")
                .as_bytes(),
            projection.as_bytes(),
        );
        assert!(StructuredReplayResult::strict_decode(projection.as_bytes()).is_err());
        assert!(StructuredAccessAuditEntry::strict_decode(projection.as_bytes()).is_err());

        wire.record_ref.record_hash = JournalRecordHash::from_digest(digest(99));
        let foreign_hash = encode_exact_projection(&wire).expect("foreign hash trace");
        assert!(StructuredTransitionTrace::strict_decode(foreign_hash.as_bytes()).is_err());

        let mut unknown: serde_json::Value =
            serde_json::from_slice(projection.as_bytes()).expect("transition trace JSON");
        unknown["extra"] = serde_json::json!(true);
        let unknown = mfm_journal::structured::canonical_json(&unknown).expect("unknown trace");
        assert!(StructuredTransitionTrace::strict_decode(unknown.as_bytes()).is_err());
    }

    #[tokio::test]
    async fn fixed_head_audit_treats_a_later_observation_as_absent() {
        let fixture = mfm_store::structured::test_support::observed_read_export(218)
            .await
            .expect("observed-read fixture");
        let authorization = fixture.audit.records().first().expect("read authorization");
        let observation = authorization.observation().expect("read observation");
        assert!(
            observation.record_ref().run_sequence > authorization.authorization_ref().run_sequence
        );
        let authorization_index = usize::try_from(authorization.authorization_ref().run_sequence)
            .expect("authorization sequence")
            .checked_sub(1)
            .expect("one-based authorization sequence");
        let authorization_head = fixture
            .audit
            .journal_heads()
            .get(authorization_index)
            .expect("authorization head");

        let fixed = project_access_audit(&fixture.audit, Some(authorization_head), 0, 500)
            .expect("fixed-head audit");
        let fixed_entry = fixed.entries().first().expect("fixed-head audit entry");
        let fixed_json: serde_json::Value =
            serde_json::from_slice(fixed_entry.as_bytes()).expect("fixed audit JSON");
        assert_eq!(fixed_json["status"], "authorized");
        assert!(fixed_json["observation_ref"].is_null());
        assert!(fixed_json["outcome"].is_null());

        let complete =
            project_access_audit(&fixture.audit, None, 0, 500).expect("complete audit projection");
        let complete_entry = complete.entries().first().expect("complete audit entry");
        let complete_json: serde_json::Value =
            serde_json::from_slice(complete_entry.as_bytes()).expect("complete audit JSON");
        assert_eq!(complete_json["status"], "returned");
        assert!(!complete_json["observation_ref"].is_null());
        assert!(!complete_json["outcome"].is_null());
        assert!(StructuredReplayResult::strict_decode(complete_entry.as_bytes()).is_err());
        assert!(StructuredTransitionTrace::strict_decode(complete_entry.as_bytes()).is_err());

        let mut mismatched = complete_json;
        mismatched["status"] = serde_json::json!("authorized");
        let mismatched =
            mfm_journal::structured::canonical_json(&mismatched).expect("mismatched audit");
        assert!(StructuredAccessAuditEntry::strict_decode(mismatched.as_bytes()).is_err());
    }

    fn transition_wire() -> StructuredTransitionTraceWire {
        let run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(1));
        let input = LexicalValueRef::new(
            content_ref(2),
            TypedValueRef {
                contract_ref: content_ref(3),
                value_ref: content_ref(4),
            },
        );
        StructuredTransitionTraceWire {
            version: "mfm.structured-transition-trace.v1".to_owned(),
            record_ref: RecordRef::new(run_id, 2, 0, JournalRecordHash::from_digest(digest(5))),
            occurrence_id: OccurrenceId::from_digest(digest(6)),
            occurrence_path_ref: content_ref(7),
            semantic_call_id: SemanticCallId::from_digest(digest(8)),
            input: input.clone(),
            consumed_observation_ref: None,
            outcome_ref: content_ref(9),
            outcome: StateOutcomeRef::Success(input),
            facts: Vec::new(),
            before_semantic_state_digest: RunSemanticStateDigest::from_digest(digest(10)),
            after_semantic_state_digest: RunSemanticStateDigest::from_digest(digest(11)),
        }
    }

    fn content_ref(discriminator: u8) -> ContentRef {
        ContentRef::new(
            SchemaId::new(
                "mfm.replay.test",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest(discriminator),
            )
            .expect("schema id"),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(&[discriminator]),
            ),
        )
        .expect("content ref")
    }

    fn digest(discriminator: u8) -> DigestBytes {
        DigestBytes::from_array([discriminator; 32])
    }
}
