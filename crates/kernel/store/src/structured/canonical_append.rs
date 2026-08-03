//! One store-owned bounded canonical append contract shared by every backend.
//!
//! Memory and PostgreSQL both validate object counts, frame sizes, canonical order, and
//! content-hash closure through this module before DML. Database checks remain defense in
//! depth, not an alternate acceptance contract.

use mfm_journal::structured::HistoryObject;

use super::fold::StructuredStoreError;

/// Maximum canonical JSON frame size for one stored object or batch envelope.
pub const MAX_STORED_FRAME_BYTES: usize = 16_777_216;
/// Maximum number of objects retained with one atomic append.
pub const MAX_BATCH_OBJECTS: usize = 65_536;

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
