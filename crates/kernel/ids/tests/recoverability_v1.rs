//! Recoverability-v1 identity conformance.

use mfm_ids::{
    AppendRequestId, AttemptId, ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, EffectKey,
    InvocationIdentity, RecordId, RequestDigest, SchemaId, SemanticDigest, StableId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};

#[path = "../../../../tests/support/recoverability_v1.rs"]
mod recoverability_v1_support;

const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn digest() -> DigestBytes {
    DigestBytes::from_hex(DIGEST_HEX).expect("valid fixture digest")
}

fn schema_id() -> SchemaId {
    SchemaId::new(
        "mfm.content-ref",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest(),
    )
    .expect("valid schema id")
}

#[test]
fn ids_execute_every_frozen_recoverability_vector() {
    recoverability_v1_support::run_consumer("mfm-ids", |vector| {
        recoverability_v1_support::assert_lower_layer_owner_vector(vector);
    });
}

#[test]
fn tenant_scope_is_a_checked_scalar_and_round_trips() {
    let value = "mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef";
    let tenant = TenantScopeId::new(value).expect("valid tenant");

    assert_eq!(tenant.as_str(), value);
    assert_eq!(
        value.parse::<TenantScopeId>().expect("parse tenant"),
        tenant
    );
    assert_eq!(tenant.to_string(), value);

    for invalid in [
        "mfm.store_scope.v1:0123456789abcdef0123456789abcdef",
        "mfm.tenant_scope.v1:0123456789ABCDEF0123456789abcdef",
        "mfm.tenant_scope.v1:0123456789abcdef",
        "tenant-a",
    ] {
        assert!(
            TenantScopeId::new(invalid).is_err(),
            "expected rejection for {invalid}"
        );
    }
}

#[test]
fn journal_and_store_identifiers_use_only_the_frozen_scalar_grammars() {
    let append = AppendRequestId::new("append/run-1").expect("append request id");
    let stable = StableId::new("append/run-1").expect("stable id");
    assert_eq!(append.as_str(), stable.as_str());
    for invalid in [".append", "Append", "append:one", "append one"] {
        assert!(AppendRequestId::new(invalid).is_err(), "{invalid}");
        assert!(StableId::new(invalid).is_err(), "{invalid}");
    }
    assert!(StableId::new("a".repeat(513)).is_err());

    let invocation =
        InvocationIdentity::new("01234567-89ab-4cde-8fab-0123456789ab").expect("UUIDv4");
    assert_eq!(
        serde_json::from_value::<InvocationIdentity>(
            serde_json::to_value(&invocation).expect("serialize invocation")
        )
        .expect("deserialize invocation"),
        invocation
    );
    for invalid in [
        "01234567-89ab-3cde-8fab-0123456789ab",
        "01234567-89ab-4cde-7fab-0123456789ab",
        "01234567-89AB-4cde-8fab-0123456789ab",
        "sample",
    ] {
        assert!(InvocationIdentity::new(invalid).is_err(), "{invalid}");
    }

    let epoch = StoreEpoch::new(u64::MAX);
    assert_eq!(
        serde_json::to_string(&epoch).expect("serialize epoch"),
        format!("\"{}\"", u64::MAX)
    );
    assert_eq!(
        serde_json::from_str::<StoreEpoch>("\"0\"").expect("zero epoch"),
        StoreEpoch::new(0)
    );
    for invalid in ["\"00\"", "\"+1\"", "1", "\"18446744073709551616\""] {
        assert!(
            serde_json::from_str::<StoreEpoch>(invalid).is_err(),
            "{invalid}"
        );
    }

    let store_scope = StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
        .expect("store scope");
    assert_eq!(
        serde_json::from_value::<StoreScopeId>(
            serde_json::to_value(&store_scope).expect("serialize store scope")
        )
        .expect("deserialize store scope"),
        store_scope
    );

    let record = RecordId::from_digest(digest());
    assert_eq!(
        record.as_str(),
        format!("record:sha256-jcs-v1:{DIGEST_HEX}")
    );
    assert!(RecordId::parse(format!("effect:sha256-jcs-v1:{DIGEST_HEX}")).is_err());
    assert!(RecordId::parse(format!("record:sha256-v1:{DIGEST_HEX}")).is_err());
}

#[test]
fn content_ref_accepts_only_semantic_schema_and_raw_content_algorithms() {
    let raw = ContentDigest::from_digest(DigestAlgorithm::Sha256V1, digest());
    let content_ref = ContentRef::new(schema_id(), raw.clone()).expect("valid content ref");

    assert_eq!(content_ref.schema_id(), &schema_id());
    assert_eq!(content_ref.content_digest(), &raw);

    let legacy = ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest());
    assert!(ContentRef::new(schema_id(), legacy).is_err());

    assert!(SchemaId::new("mfm.content-ref", "1", DigestAlgorithm::Sha256V1, digest()).is_err());
}

#[test]
fn generic_identities_serde_as_strict_category_checked_scalars() {
    let schema = schema_id();
    assert_eq!(
        serde_json::from_value::<SchemaId>(
            serde_json::to_value(&schema).expect("serialize schema id")
        )
        .expect("deserialize schema id"),
        schema
    );
    assert!(
        serde_json::from_value::<SchemaId>(serde_json::Value::String(format!(
            "schema:mfm.content-ref:1:sha256-v1:{DIGEST_HEX}"
        )))
        .is_err()
    );
    assert!(
        serde_json::from_value::<SchemaId>(serde_json::Value::String(format!(
            "run:sha256-jcs-v1:{DIGEST_HEX}"
        )))
        .is_err()
    );
    for malformed in [
        format!("schema:mfm.content-ref:1:sha256-jcs-v1:{}", "A".repeat(64)),
        format!("schema:mfm.content-ref:01:sha256-jcs-v1:{DIGEST_HEX}"),
        format!("schema:mfm.content-ref:1:sha256-jcs-v1:{DIGEST_HEX}00"),
    ] {
        assert!(serde_json::from_value::<SchemaId>(serde_json::Value::String(malformed)).is_err());
    }
    assert!(serde_json::from_value::<SchemaId>(serde_json::json!({
        "schema_id": schema.as_str()
    }))
    .is_err());
}

#[test]
fn digest_algorithm_parser_distinguishes_raw_and_semantic_sha256() {
    assert_eq!(
        "sha256-v1"
            .parse::<DigestAlgorithm>()
            .expect("raw algorithm"),
        DigestAlgorithm::Sha256V1
    );
    assert_eq!(
        "sha256-jcs-v1"
            .parse::<DigestAlgorithm>()
            .expect("semantic algorithm"),
        DigestAlgorithm::Sha256JcsV1
    );
    assert!("sha256".parse::<DigestAlgorithm>().is_err());

    let secret = "bearer-secret-material";
    let error = secret
        .parse::<DigestAlgorithm>()
        .expect_err("unregistered algorithm must reject");
    assert!(
        !error.to_string().contains(secret),
        "identity errors must not echo rejected input"
    );
}

#[test]
fn semantic_identities_cannot_be_substituted_with_content_digests() {
    let semantic = SemanticDigest::from_digest(digest());
    let request = RequestDigest::from_semantic_digest(semantic.clone());
    let effect_key = EffectKey::from_semantic_digest(semantic.clone());
    let attempt_id = AttemptId::from_semantic_digest(semantic);

    assert_eq!(request.as_str(), format!("sha256-jcs-v1:{DIGEST_HEX}"));
    assert_eq!(
        effect_key.as_str(),
        format!("effect:sha256-jcs-v1:{DIGEST_HEX}")
    );
    assert_eq!(
        attempt_id.as_str(),
        format!("attempt:sha256-jcs-v1:{DIGEST_HEX}")
    );
    assert_eq!(
        effect_key
            .to_string()
            .parse::<EffectKey>()
            .expect("effect-key round trip"),
        effect_key
    );
    assert_eq!(
        attempt_id
            .to_string()
            .parse::<AttemptId>()
            .expect("attempt-id round trip"),
        attempt_id
    );

    for invalid in [
        format!("content:sha256-v1:{DIGEST_HEX}"),
        format!("sha256-v1:{DIGEST_HEX}"),
        format!("sha256-jcs-v1:{DIGEST_HEX}00"),
    ] {
        assert!(SemanticDigest::parse(invalid).is_err());
    }
    assert!(EffectKey::parse(format!("attempt:sha256-jcs-v1:{DIGEST_HEX}")).is_err());
    assert!(AttemptId::parse(format!("effect:sha256-jcs-v1:{DIGEST_HEX}")).is_err());
}
