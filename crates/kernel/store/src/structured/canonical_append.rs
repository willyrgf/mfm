//! One store-owned bounded canonical append contract shared by every backend.
//!
//! Memory and PostgreSQL both validate object counts, frame sizes, canonical order, and
//! content-hash closure through this module before DML. Database checks remain defense in
//! depth, not an alternate acceptance contract.

use super::configuration::ConfigurationRevision;
use mfm_journal::structured::{canonical_json, CommittedBatch, HistoryObject};

use super::fold::StructuredStoreError;

/// Maximum canonical JSON frame size for one stored object or batch envelope.
pub const MAX_STORED_FRAME_BYTES: usize = 16_777_216;
/// Maximum number of objects retained with one atomic append.
pub const MAX_BATCH_OBJECTS: usize = 65_536;
/// Maximum number of records in one atomic append envelope.
pub const MAX_BATCH_RECORDS: usize = 65_536;

/// Store-owned canonical run append accepted by every persistence backend.
pub struct CanonicalRunAppend {
    committed: CommittedBatch,
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
        validate_append_objects(&committed.objects)?;
        let canonical =
            canonical_json(&committed).map_err(|_| StructuredStoreError::InvalidHistory)?;
        validate_envelope_frame(canonical.as_str())?;
        Ok(Self { committed })
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

    /// Consumes the canonical ingress value.
    pub fn into_committed(self) -> CommittedBatch {
        self.committed
    }
}

/// Store-owned canonical configured-value append accepted by every configuration backend.
pub struct CanonicalConfigurationAppend {
    revision: ConfigurationRevision,
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
        if revision.canonical_value().len() > MAX_STORED_FRAME_BYTES {
            return Err(StructuredStoreError::InvalidHistory);
        }
        Ok(Self { revision })
    }

    /// Returns the exact validated revision.
    pub const fn revision(&self) -> &ConfigurationRevision {
        &self.revision
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
        assert_eq!(MAX_STORED_FRAME_BYTES, 16_777_216);
        assert_eq!(MAX_BATCH_OBJECTS, 65_536);
    }
}
