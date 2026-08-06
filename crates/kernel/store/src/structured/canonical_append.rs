//! One store-owned bounded canonical append contract shared by every backend.
//!
//! Memory and PostgreSQL both validate object counts, frame sizes, canonical order, and
//! content-hash closure through this module before DML. Database checks remain defense in
//! depth, not an alternate acceptance contract.

use std::collections::BTreeSet;

use super::configuration::ConfigurationRevision;
use mfm_canonical::limits::{
    MAX_ARRAY_ITEMS, MAX_CONFIGURATION_REVISION_BYTES, MAX_OBJECT_ENTRIES,
};
pub use mfm_canonical::limits::{MAX_BATCH_OBJECTS, MAX_BATCH_RECORDS, MAX_STORED_FRAME_BYTES};
use mfm_canonical::MAX_CANONICAL_JSON_DEPTH;
use mfm_ids::ContentRef;
use mfm_journal::structured::{
    canonical_json, CommittedBatch, HistoryObject, ObservationOutcome, RunRecord, StateOutcomeRef,
};

use super::fold::StructuredStoreError;

/// Store-owned canonical run append accepted by every persistence backend.
pub struct CanonicalRunAppend {
    committed: CommittedBatch,
    store_verified: bool,
}

impl std::fmt::Debug for CanonicalRunAppend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalRunAppend")
            .field("head", &self.committed.head)
            .finish_non_exhaustive()
    }
}

impl CanonicalRunAppend {
    /// Validates the complete envelope before backend dispatch.
    pub fn new(committed: CommittedBatch) -> Result<Self, StructuredStoreError> {
        if committed.records.is_empty() || committed.records.len() > MAX_BATCH_RECORDS {
            return Err(StructuredStoreError::InvalidHistory);
        }
        super::fold::verify_batch_envelope(
            &committed
                .records
                .first()
                .ok_or(StructuredStoreError::InvalidHistory)?
                .record_ref
                .run_id,
            &committed,
            committed.predecessor.as_ref(),
            Some(&(committed.store_scope_id.clone(), committed.store_epoch)),
        )?;
        validate_append_objects(&committed.objects)?;
        let canonical =
            canonical_json(&committed).map_err(|_| StructuredStoreError::InvalidHistory)?;
        validate_envelope_frame(canonical.as_str())?;
        Ok(Self {
            committed,
            store_verified: false,
        })
    }

    /// Constructs the backend ingress value after the sole fold has verified
    /// the complete record/object closure. This is crate-private so a backend
    /// cannot be handed an unverified candidate by an ordinary caller.
    pub(crate) fn from_store_verified(
        committed: CommittedBatch,
    ) -> Result<Self, StructuredStoreError> {
        let mut append = Self::new(committed)?;
        // The fold's semantic closure proof runs immediately before this
        // constructor. It includes certified component-closure objects whose
        // only durable relationship is through the certified program, rather
        // than a direct record reference. Reapplying the record-only helper
        // here would reject those valid genesis batches.
        append.store_verified = true;
        Ok(append)
    }

    /// Returns the exact run identity.
    pub fn run_id(&self) -> &mfm_ids::RunId {
        &self.committed.records[0].record_ref.run_id
    }

    /// Returns the exact expected predecessor.
    pub const fn predecessor(&self) -> Option<&mfm_journal::structured::JournalHead> {
        self.committed.predecessor.as_ref()
    }

    /// Returns the stable append acknowledgement identity.
    pub const fn append_request_id(&self) -> &mfm_ids::AppendRequestId {
        &self.committed.append_request_id
    }

    /// Returns the complete candidate digest used for idempotency.
    pub const fn candidate_digest(&self) -> &mfm_ids::ContentDigest {
        &self.committed.candidate_digest
    }

    /// Returns the exact committed envelope for a backend implementation.
    pub const fn committed(&self) -> &CommittedBatch {
        &self.committed
    }

    /// Returns whether this ingress value passed the store fold's complete
    /// closure proof.
    pub const fn is_store_verified(&self) -> bool {
        self.store_verified
    }

    /// Consumes the canonical ingress value.
    pub fn into_committed(self) -> CommittedBatch {
        self.committed
    }
}

/// Store-owned canonical configured-value append accepted by every configuration backend.
pub struct CanonicalConfigurationAppend {
    revision: ConfigurationRevision,
    store_verified: bool,
}

impl std::fmt::Debug for CanonicalConfigurationAppend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalConfigurationAppend")
            .field("revision_ref", &self.revision.revision_ref())
            .finish_non_exhaustive()
    }
}

impl CanonicalConfigurationAppend {
    /// Validates the complete configuration revision before backend dispatch.
    pub fn new(revision: ConfigurationRevision) -> Result<Self, StructuredStoreError> {
        revision.validate_for_ingress()?;
        let canonical_revision =
            canonical_json(&revision).map_err(|_| StructuredStoreError::InvalidHistory)?;
        if canonical_revision.as_bytes().len() > MAX_CONFIGURATION_REVISION_BYTES {
            return Err(StructuredStoreError::InvalidHistory);
        }
        Ok(Self {
            revision,
            store_verified: false,
        })
    }

    /// Constructs the backend ingress value after the store has verified the
    /// complete append-only configuration prefix and predecessor.
    pub(crate) fn from_store_verified(
        revision: ConfigurationRevision,
    ) -> Result<Self, StructuredStoreError> {
        let mut append = Self::new(revision)?;
        append.store_verified = true;
        Ok(append)
    }

    /// Returns the exact validated revision.
    pub const fn revision(&self) -> &ConfigurationRevision {
        &self.revision
    }

    /// Returns whether the store's complete configuration proof ran first.
    pub const fn is_store_verified(&self) -> bool {
        self.store_verified
    }

    /// Consumes the canonical ingress value.
    pub fn into_revision(self) -> ConfigurationRevision {
        self.revision
    }
}

/// Validates the object closure of one already store-validated batch.
pub fn validate_append_objects(objects: &[HistoryObject]) -> Result<(), StructuredStoreError> {
    if objects.len() > MAX_BATCH_OBJECTS {
        return Err(StructuredStoreError::InvalidHistory);
    }
    if objects
        .windows(2)
        .any(|pair| pair[0].content_ref >= pair[1].content_ref)
    {
        return Err(StructuredStoreError::InvalidHistory);
    }
    for object in objects {
        if object.canonical_json.is_empty()
            || object.canonical_json.len() > MAX_STORED_FRAME_BYTES
            || object.validate().is_err()
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
    }
    Ok(())
}

/// Verifies that every content reference directly named by an append record is
/// already retained or present in this append.
pub fn validate_record_object_closure(
    batch: &CommittedBatch,
    prior_object_refs: &BTreeSet<ContentRef>,
) -> Result<(), StructuredStoreError> {
    let actual = batch
        .objects
        .iter()
        .map(|object| object.content_ref.clone())
        .collect::<BTreeSet<_>>();
    let mut required = BTreeSet::new();
    for assigned in &batch.records {
        match &assigned.record {
            RunRecord::RunAdmitted(admission) => {
                required.extend([
                    admission.certified_program_root_ref.clone(),
                    admission.admission_material_refs.configuration_ref.clone(),
                    admission
                        .admission_material_refs
                        .context_manifest_ref
                        .clone(),
                    admission
                        .admission_material_refs
                        .prior_run_source_manifest_ref
                        .clone(),
                    admission.admission_material_refs.routing_policy_ref.clone(),
                ]);
                required.extend(
                    admission
                        .admission_material_refs
                        .stable_resource_lineage_contract_refs
                        .iter()
                        .cloned(),
                );
                required.extend(
                    admission
                        .initial_bindings
                        .iter()
                        .map(|binding| binding.value.value_ref.clone()),
                );
            }
            RunRecord::StateTransitionCommitted(transition) => {
                required.extend([
                    transition.input.value.value_ref.clone(),
                    transition.outcome_ref.clone(),
                ]);
                match &transition.outcome {
                    StateOutcomeRef::Success(value) | StateOutcomeRef::Failure(value) => {
                        required.insert(value.value.value_ref.clone());
                    }
                }
                for fact in &transition.facts {
                    required.extend([
                        fact.descriptor_ref.clone(),
                        fact.subject.value_ref.clone(),
                        fact.response.value_ref.clone(),
                        fact.claim_ref.clone(),
                    ]);
                }
            }
            RunRecord::ExternalAccessAuthorized(authorization) => {
                required.extend([
                    authorization.state_input_ref.value.value_ref.clone(),
                    authorization.request.value_ref.clone(),
                    authorization.physical_binding_ref.clone(),
                ]);
                if let Some(lineage) = authorization.stable_resource_lineage_contract_ref.as_ref() {
                    required.insert(lineage.clone());
                }
            }
            RunRecord::ExternalAccessObserved(observation) => match &observation.outcome {
                ObservationOutcome::Returned { value }
                | ObservationOutcome::SafeFailure { value } => {
                    required.insert(value.value_ref.clone());
                }
                ObservationOutcome::SupersededBeforeEntry {
                    public_lineage_head_ref,
                    evidence_ref,
                } => {
                    required.extend([public_lineage_head_ref.clone(), evidence_ref.clone()]);
                }
                ObservationOutcome::EntryUnknown { .. }
                | ObservationOutcome::IntegrityFault { .. } => {}
            },
            RunRecord::RunClosed(closed) => {
                required.insert(closed.outcome_ref.clone());
            }
        }
    }
    // Object payloads are themselves canonical structured values. Scan every
    // nested `{schema_id, content_digest}` pair and require each pair that is
    // used as a content reference to resolve either in this append or in the
    // retained prefix. The record-derived closure remains authoritative: not
    // every schema-shaped value inside a typed payload is a HistoryObject.
    // The scan is bounded by the same canonical array/object/depth budgets as
    // the decoder; it never allocates from an unbounded recursive input.
    let mut nested_refs = BTreeSet::new();
    for object in &batch.objects {
        let value: serde_json::Value = serde_json::from_str(&object.canonical_json)
            .map_err(|_| StructuredStoreError::InvalidHistory)?;
        collect_nested_content_refs(&value, 0, &mut nested_refs)?;
    }
    required.extend(
        nested_refs
            .iter()
            .filter(|content_ref| actual.contains(*content_ref))
            .cloned(),
    );
    required.retain(|content_ref| !prior_object_refs.contains(content_ref));
    if required == actual {
        Ok(())
    } else {
        Err(StructuredStoreError::InvalidHistory)
    }
}

fn collect_nested_content_refs(
    value: &serde_json::Value,
    depth: usize,
    required: &mut BTreeSet<ContentRef>,
) -> Result<(), StructuredStoreError> {
    if depth > MAX_CANONICAL_JSON_DEPTH {
        return Err(StructuredStoreError::InvalidHistory);
    }
    match value {
        serde_json::Value::Array(items) => {
            if items.len() > MAX_ARRAY_ITEMS {
                return Err(StructuredStoreError::InvalidHistory);
            }
            for item in items {
                collect_nested_content_refs(item, depth + 1, required)?;
            }
        }
        serde_json::Value::Object(entries) => {
            if entries.len() > MAX_OBJECT_ENTRIES {
                return Err(StructuredStoreError::InvalidHistory);
            }
            if entries.contains_key("schema_id") && entries.contains_key("content_digest") {
                if let Ok(content_ref) = serde_json::from_value::<ContentRef>(value.clone()) {
                    required.insert(content_ref);
                }
            }
            for item in entries.values() {
                collect_nested_content_refs(item, depth + 1, required)?;
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
    Ok(())
}

/// Validates one durable envelope frame size.
pub fn validate_envelope_frame(canonical_envelope: &str) -> Result<(), StructuredStoreError> {
    if canonical_envelope.len() < 2 || canonical_envelope.len() > MAX_STORED_FRAME_BYTES {
        return Err(StructuredStoreError::InvalidHistory);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_BATCH_OBJECTS, MAX_STORED_FRAME_BYTES};

    #[test]
    fn shared_bounds_match_the_authoritative_postgres_limits() {
        assert_eq!(
            MAX_STORED_FRAME_BYTES,
            mfm_canonical::limits::MAX_CANONICAL_JSON_BYTES
        );
        assert_eq!(MAX_BATCH_OBJECTS, 65_536);
    }
}
