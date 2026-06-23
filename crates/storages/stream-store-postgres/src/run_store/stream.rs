use super::*;

pub(super) fn run_commit_sort_key(
    run_id: &RunId,
    seq: StreamSeq,
    commit_key: &CommitKey,
    commit_id: &str,
    commit_batch_hash: &str,
) -> Result<Vec<u8>> {
    let digest = canonical_json(serde_json::json!({
        "commit_batch_hash": commit_batch_hash,
        "commit_key": commit_key.as_str(),
        "commit_id": commit_id,
        "domain": "mfm.run_commit_log.sort_key.v1",
        "run_id": run_id.as_str(),
        "seq": seq.as_u64(),
    }))?
    .digest_bytes();
    let mut key = Vec::with_capacity(32);
    key.push(1);
    key.extend_from_slice(&digest.as_bytes()[..31]);
    Ok(key)
}

pub(super) async fn read_head(pool: &PgPool, run_id: &RunId) -> Result<u64> {
    let row =
        sqlx::query("SELECT COALESCE(MAX(seq), 0) AS head_seq FROM commits WHERE run_id = $1")
            .bind(run_id.as_str())
            .fetch_one(pool)
            .await
            .map_err(|error| database_error("failed to query run head", error))?;
    let head_seq: i64 = row
        .try_get("head_seq")
        .map_err(|error| database_error("failed to decode run head", error))?;
    i64_to_nonnegative_u64(head_seq, "commits.seq")
}

pub(super) async fn read_head_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<u64> {
    let row =
        sqlx::query("SELECT COALESCE(MAX(seq), 0) AS head_seq FROM commits WHERE run_id = $1")
            .bind(run_id.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| database_error("failed to query run head", error))?;
    let head_seq: i64 = row
        .try_get("head_seq")
        .map_err(|error| database_error("failed to decode run head", error))?;
    i64_to_nonnegative_u64(head_seq, "commits.seq")
}

pub(super) async fn count_run_events_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM run_events WHERE run_id = $1")
        .bind(run_id.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| database_error("failed to count run events", error))
}

pub(super) async fn lock_run_tx(tx: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<()> {
    const RUN_LOCK_CLASS_ID: i32 = 0x4d46_5201;
    let object_id = advisory_object_id(run_id.as_str().as_bytes());
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(RUN_LOCK_CLASS_ID)
        .bind(object_id)
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to lock run append authority", error))?;
    Ok(())
}

pub(super) fn advisory_object_id(bytes: &[u8]) -> i32 {
    let digest = sha256_digest_bytes(bytes);
    i32::from_be_bytes([
        digest.as_bytes()[0],
        digest.as_bytes()[1],
        digest.as_bytes()[2],
        digest.as_bytes()[3],
    ])
}
