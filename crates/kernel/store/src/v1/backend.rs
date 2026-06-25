use super::*;

/// Admits artifact evidence into an authority map, rejecting conflicting evidence.
pub fn admit_artifact_evidence(
    artifacts: &mut ArtifactAuthorityMap,
    admitted_artifacts: &[ArtifactEvidenceRef],
) -> Result<()> {
    super::admit_artifact_evidence(artifacts, admitted_artifacts)
}

/// Looks up admitted evidence in a prepared bundle by artifact id and evidence hash.
pub fn admitted_artifact_evidence<'a>(
    bundle: &'a PreparedCommitBundle,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<&'a ArtifactEvidenceRef> {
    super::admitted_artifact_evidence(bundle, artifact_id, evidence_hash)
}

/// Returns whether a logical key is globally unique in a run stream.
pub fn is_unique_logical_key(key: &LogicalEventKey) -> bool {
    super::is_unique_logical_key(key)
}

/// Derives the stable PostgreSQL commit row id from store-owned commit authority.
pub fn derive_commit_id(
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
