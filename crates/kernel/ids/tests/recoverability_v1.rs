use mfm_ids::{
    AccessAttemptId, ArtifactId, ContentDigest, ContentRef, DigestAlgorithm, DigestBytes,
    FactQueryDigest, FailurePlanId, FragmentBoundaryId, OccurrenceId, RunId, SchemaId,
    SemanticCallId, SemanticDigest, StoreEpoch, StoreScopeId, TenantScopeId,
};

const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn digest() -> DigestBytes {
    DigestBytes::from_hex(DIGEST_HEX).expect("fixture digest")
}

#[test]
fn current_structured_semantic_identities_have_disjoint_checked_prefixes() {
    let semantic = SemanticDigest::from_digest(digest());
    let cases = [
        SemanticCallId::from_semantic_digest(semantic.clone()).to_string(),
        OccurrenceId::from_semantic_digest(semantic.clone()).to_string(),
        FragmentBoundaryId::from_semantic_digest(semantic.clone()).to_string(),
        FailurePlanId::from_semantic_digest(semantic.clone()).to_string(),
        AccessAttemptId::from_semantic_digest(semantic).to_string(),
    ];

    assert_eq!(
        cases[0],
        format!("semantic-call:sha256-jcs-v1:{DIGEST_HEX}")
    );
    assert_eq!(cases[1], format!("occurrence:sha256-jcs-v1:{DIGEST_HEX}"));
    assert_eq!(
        cases[2],
        format!("fragment-boundary:sha256-jcs-v1:{DIGEST_HEX}")
    );
    assert_eq!(cases[3], format!("failure-plan:sha256-jcs-v1:{DIGEST_HEX}"));
    assert_eq!(
        cases[4],
        format!("access-attempt:sha256-jcs-v1:{DIGEST_HEX}")
    );
    assert!(OccurrenceId::parse(&cases[0]).is_err());
    assert!(AccessAttemptId::parse(&cases[1]).is_err());
}

#[test]
fn current_digest_identities_preserve_their_algorithm_contracts() {
    let run = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest());
    let artifact = ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest());
    let query = FactQueryDigest::from_digest(digest());

    assert_eq!(run.as_str(), format!("run:sha256-jcs-v1:{DIGEST_HEX}"));
    assert_eq!(
        artifact.as_str(),
        format!("artifact:sha256-jcs-v1:{DIGEST_HEX}")
    );
    assert_eq!(query.as_str(), format!("sha256-jcs-v1:{DIGEST_HEX}"));
    assert!(RunId::parse(format!("run:sha256-v1:{DIGEST_HEX}")).is_err());
}

#[test]
fn content_refs_require_schema_interpretation_and_raw_content_digest() {
    let schema = SchemaId::new(
        "mfm.structured-value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest(),
    )
    .expect("schema id");
    let raw = ContentDigest::from_digest(DigestAlgorithm::Sha256V1, digest());
    let value = ContentRef::new(schema.clone(), raw).expect("content ref");

    assert_eq!(value.schema_id(), &schema);
    assert!(ContentRef::new(
        schema,
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest()),
    )
    .is_err());
}

#[test]
fn deployment_scopes_and_epoch_use_closed_wire_grammars() {
    let store = StoreScopeId::new("mfm.store_scope.v1:00000000000000000000000000000000")
        .expect("store scope");
    let tenant = TenantScopeId::new("mfm.tenant_scope.v1:11111111111111111111111111111111")
        .expect("tenant scope");
    let epoch = StoreEpoch::parse("42").expect("store epoch");

    assert_eq!(
        store.to_string(),
        "mfm.store_scope.v1:00000000000000000000000000000000"
    );
    assert_eq!(
        tenant.to_string(),
        "mfm.tenant_scope.v1:11111111111111111111111111111111"
    );
    assert_eq!(epoch.get(), 42);
    assert!(StoreEpoch::parse("042").is_err());
    assert!(TenantScopeId::new("mfm.tenant_scope.v1:UPPERCASE00000000000000000000000").is_err());
}

#[test]
fn content_ref_serde_rejects_unknown_fields() {
    let input = format!(
        "{{\"content_digest\":\"content:sha256-v1:{DIGEST_HEX}\",\"retired\":true,\"schema_id\":\"schema:mfm.value:1:sha256-jcs-v1:{DIGEST_HEX}\"}}"
    );
    assert!(serde_json::from_str::<ContentRef>(&input).is_err());
}
