use super::*;

const RESOURCE_LANE_ID_DOMAIN: &[u8] = b"mfm.resource_lane.id.v1";
const RESOURCE_LANE_WAITER_FINGERPRINT_DOMAIN: &[u8] = b"mfm.resource_lane.waiter.fingerprint.v1";
const RESOURCE_LANE_WAITER_ID_DOMAIN: &[u8] = b"mfm.resource_lane.waiter.id.v1";

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

/// Returns canonical JSON bytes for a resource key evidence value.
pub fn resource_key_canonical_json(evidence: &events::ResourceKeyEvidence) -> Result<Vec<u8>> {
    Ok(canonical_json(serde_json::json!({
        "key": evidence.key.as_str(),
    }))?
    .to_vec())
}

/// Derives the versioned exclusive lane id for resource key evidence.
pub fn resource_lane_id(evidence: &events::ResourceKeyEvidence) -> Result<[u8; 32]> {
    let key_canonical_json = resource_key_canonical_json(evidence)?;
    let canonical = canonical_json(serde_json::json!({
        "key_canonical_json": String::from_utf8(key_canonical_json)
            .map_err(|error| StoreError::Canonical(format!("resource key canonical JSON was not UTF-8: {error}")))?,
        "key_schema_id": evidence.key_schema_id.as_str(),
        "mode": "exclusive",
        "namespace": evidence.namespace.as_str(),
    }))?;
    let mut input = Vec::with_capacity(RESOURCE_LANE_ID_DOMAIN.len() + canonical.as_bytes().len());
    input.extend_from_slice(RESOURCE_LANE_ID_DOMAIN);
    input.extend_from_slice(canonical.as_bytes());
    let digest = sha256_digest_bytes(&input);
    let mut lane_id = [0_u8; 32];
    lane_id[0] = 1;
    lane_id[1..].copy_from_slice(&digest.as_bytes()[..31]);
    Ok(lane_id)
}

/// Derives the versioned exclusive lane id for an already-normalized lane key.
pub fn resource_lane_id_for_key(key: &ResourceLaneKey) -> Result<[u8; 32]> {
    resource_lane_id(&events::ResourceKeyEvidence {
        namespace: key.namespace.clone(),
        key_schema_id: key.key_schema_id.clone(),
        key: key.key.clone(),
    })
}

/// Derives the stable FIFO waiter fingerprint for a resource-lane claim intent.
pub fn resource_lane_claim_fingerprint(
    run_id: &RunId,
    lane_id: &[u8],
    intent: &events::ResourceLaneClaimIntent,
) -> Result<String> {
    let canonical = canonical_json(serde_json::json!({
        "attempt_id": intent.attempt_id.as_str(),
        "domain": String::from_utf8_lossy(RESOURCE_LANE_WAITER_FINGERPRINT_DOMAIN),
        "invocation_epoch": intent.invocation_epoch,
        "lane_id": bytes_hex(lane_id),
        "ledger_key": intent.ledger_key.as_str(),
        "node_id": intent.node_id.as_str(),
        "requirement_digest": intent.requirement_digest.as_str(),
        "resolved_by_capability_impl": intent.resolved_by_capability_impl.as_str(),
        "run_id": run_id.as_str(),
    }))?;
    let mut input = Vec::with_capacity(
        RESOURCE_LANE_WAITER_FINGERPRINT_DOMAIN.len() + canonical.as_bytes().len(),
    );
    input.extend_from_slice(RESOURCE_LANE_WAITER_FINGERPRINT_DOMAIN);
    input.extend_from_slice(canonical.as_bytes());
    Ok(bytes_hex(sha256_digest_bytes(&input).as_bytes()))
}

/// Derives the stable FIFO waiter id from a claim fingerprint.
pub fn resource_lane_waiter_id(claim_fingerprint: &str) -> String {
    let mut input =
        Vec::with_capacity(RESOURCE_LANE_WAITER_ID_DOMAIN.len() + claim_fingerprint.len());
    input.extend_from_slice(RESOURCE_LANE_WAITER_ID_DOMAIN);
    input.extend_from_slice(claim_fingerprint.as_bytes());
    format!(
        "resource_lane_waiter:{}",
        bytes_hex(sha256_digest_bytes(&input).as_bytes())
    )
}

fn bytes_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
