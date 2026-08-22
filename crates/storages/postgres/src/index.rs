use mfm_ids::{ContentDigest, DigestAlgorithm, RunId};
use mfm_journal::{MAX_RUN_BYTES, MAX_RUN_FRAMES};
use mfm_store::{RunIndexError, RunPage, RunPageLimit, RunSummary};
use sqlx::{PgPool, Row};

pub(super) async fn list_runs(
    pool: &PgPool,
    after: Option<&RunId>,
    limit: RunPageLimit,
) -> Result<RunPage, RunIndexError> {
    let after = after.map_or("", RunId::as_str);
    let query_limit = i64::try_from(limit.get() + 1).map_err(|_| RunIndexError::Corrupt)?;
    let rows = sqlx::query(
        "SELECT h.run_id, h.head_sequence, f.head_digest, h.total_bytes \
         FROM public.mfm_run_heads h \
         JOIN public.mfm_run_frames f \
           ON f.run_id = h.run_id AND f.run_sequence = h.head_sequence \
         WHERE h.run_id > $1 ORDER BY h.run_id COLLATE \"C\" LIMIT $2",
    )
    .bind(after)
    .bind(query_limit)
    .fetch_all(pool)
    .await
    .map_err(|_| RunIndexError::Unavailable)?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let run_id: &str = row.try_get("run_id").map_err(|_| RunIndexError::Corrupt)?;
        let sequence: i64 = row
            .try_get("head_sequence")
            .map_err(|_| RunIndexError::Corrupt)?;
        let digest: &str = row
            .try_get("head_digest")
            .map_err(|_| RunIndexError::Corrupt)?;
        let total_bytes: i64 = row
            .try_get("total_bytes")
            .map_err(|_| RunIndexError::Corrupt)?;
        let run_id = RunId::parse(run_id).map_err(|_| RunIndexError::Corrupt)?;
        let sequence = u64::try_from(sequence).map_err(|_| RunIndexError::Corrupt)?;
        let digest = ContentDigest::parse(digest).map_err(|_| RunIndexError::Corrupt)?;
        let total_bytes = u64::try_from(total_bytes).map_err(|_| RunIndexError::Corrupt)?;
        if sequence == 0
            || sequence > MAX_RUN_FRAMES
            || digest.algorithm() != DigestAlgorithm::Sha256V1
            || total_bytes == 0
            || total_bytes > MAX_RUN_BYTES
        {
            return Err(RunIndexError::Corrupt);
        }
        items.push(
            RunSummary::new(run_id, sequence, digest, total_bytes)
                .map_err(|_| RunIndexError::Corrupt)?,
        );
    }
    let has_more = items.len() > limit.get();
    if has_more {
        items.pop();
    }
    let next_after = if has_more {
        let last = items.last().ok_or(RunIndexError::Corrupt)?;
        Some(last.run_id().clone())
    } else {
        None
    };
    RunPage::new(items, next_after)
}
