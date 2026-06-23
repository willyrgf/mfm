use super::*;

pub(super) fn payload_canonical_bytes(
    payload: &events::KernelEventPayload,
) -> Result<PlainCanonicalJsonBytes> {
    Ok(mfm_store::v1::payload_canonical_json(payload)?)
}

pub(super) fn payload_json_value(payload: &events::KernelEventPayload) -> Result<Value> {
    let canonical = payload_canonical_bytes(payload)?;
    serde_json::from_slice(canonical.as_bytes()).map_err(|error| {
        PostgresStoreError::Corruption(format!("canonical payload was not JSON: {error}"))
    })
}

pub(super) fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(&value.to_string()).map_err(|error| {
        PostgresStoreError::Corruption(format!(
            "failed to canonicalize Postgres authority JSON: {error}"
        ))
    })
}

pub(super) fn derive_commit_id(
    run_id: &RunId,
    seq: StreamSeq,
    commit_key: &CommitKey,
    commit_purpose: &str,
    fingerprint: &CommitFingerprint,
) -> Result<String> {
    let digest = canonical_json(serde_json::json!({
        "commit_key": commit_key.as_str(),
        "commit_purpose": commit_purpose,
        "domain": "mfm.commit.id.v1",
        "prepared_commit_plan_fingerprint": fingerprint.as_digest().as_str(),
        "run_id": run_id.as_str(),
        "seq": seq.as_u64(),
    }))?
    .content_digest();
    Ok(format!("mfm.commit.id.v1:{}", digest.as_str()))
}

pub(super) fn is_unique_logical_key(key: &LogicalEventKey) -> bool {
    !key.as_str().starts_with("attempt:")
}

pub(super) fn next_seq_from_head(head: u64) -> Result<StreamSeq> {
    let next = head.checked_add(1).ok_or(StoreError::SequenceOverflow)?;
    Ok(StreamSeq::new(next)?)
}

pub(super) fn u64_to_i64(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        PostgresStoreError::Corruption(format!("{field} exceeded PostgreSQL bigint range"))
    })
}

pub(super) fn i64_to_nonnegative_u64(value: i64, field: &'static str) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} contained a negative bigint")))
}

pub(super) fn i64_to_positive_u64(value: i64, field: &'static str) -> Result<u64> {
    let value = i64_to_nonnegative_u64(value, field)?;
    if value == 0 {
        return Err(PostgresStoreError::Corruption(format!(
            "{field} contained a non-positive sequence"
        )));
    }
    Ok(value)
}

pub(super) fn parse_optional_identity<T>(value: Option<String>) -> Result<Option<T>>
where
    T: FromStr<Err = IdentityError>,
{
    Ok(value.as_deref().map(parse_identity).transpose()?)
}
