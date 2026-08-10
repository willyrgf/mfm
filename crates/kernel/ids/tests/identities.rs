use mfm_ids::*;

const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn digest() -> DigestBytes {
    DigestBytes::from_hex(DIGEST_HEX).expect("fixture digest is valid")
}

#[test]
fn accepts_category_identity_golden_strings() {
    macro_rules! assert_identity {
        ($identity:expr, $prefix:expr, $canonical_name:expr, $version:expr $(,)?) => {{
            let identity = $identity;
            assert!(identity.as_str().starts_with($prefix));
            assert_eq!(identity.canonical_name(), $canonical_name);
            assert_eq!(identity.version(), $version);
            assert_eq!(identity.algorithm(), DigestAlgorithm::Sha256JcsV1);
            assert_eq!(identity.digest(), &digest());
            assert_eq!(identity.to_string(), identity.as_str());
        }};
    }

    assert_identity!(
        SemanticTypeId::parse(format!(
            "semantic:mfm.kernel:portfolio/position:1.2.3:sha256-jcs-v1:{DIGEST_HEX}"
        ))
        .expect("semantic type id"),
        "semantic:mfm.kernel:portfolio/position:1.2.3:sha256-jcs-v1:",
        Some("mfm.kernel/portfolio/position"),
        Some("1.2.3"),
    );
    assert_identity!(
        SchemaId::parse(format!(
            "schema:mfm.kernel.position:1:sha256-jcs-v1:{DIGEST_HEX}"
        ))
        .expect("schema id"),
        "schema:mfm.kernel.position:1:sha256-jcs-v1:",
        Some("mfm.kernel.position"),
        Some("1"),
    );
    assert_identity!(
        EffectKind::parse(format!(
            "effect:mfm.kernel:read-external:sha256-jcs-v1:{DIGEST_HEX}"
        ))
        .expect("effect kind"),
        "effect:mfm.kernel:read-external:sha256-jcs-v1:",
        Some("mfm.kernel/read-external"),
        None,
    );
    assert_identity!(
        CapabilityKind::parse(format!(
            "capability:mfm.evm:rpc-read:sha256-jcs-v1:{DIGEST_HEX}"
        ))
        .expect("capability kind"),
        "capability:mfm.evm:rpc-read:sha256-jcs-v1:",
        Some("mfm.evm/rpc-read"),
        None,
    );
}

#[test]
fn accepts_digest_only_identity_golden_strings() {
    let cases = [
        RunId::parse(format!("run:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("run id")
            .to_string(),
        ArtifactId::parse(format!("artifact:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("artifact id")
            .to_string(),
        ContentDigest::parse(format!("content:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("content digest")
            .to_string(),
    ];

    assert_eq!(
        cases,
        [
            format!("run:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("artifact:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("content:sha256-jcs-v1:{DIGEST_HEX}"),
        ]
    );
}

#[test]
fn checked_constructors_produce_canonical_strings() {
    assert_eq!(
        SemanticTypeId::new(
            "mfm.kernel",
            "balance",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest()
        )
        .expect("semantic constructor")
        .as_str(),
        format!("semantic:mfm.kernel:balance:1:sha256-jcs-v1:{DIGEST_HEX}")
    );

    assert_eq!(
        SchemaId::new(
            "mfm.kernel.balance",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest()
        )
        .expect("schema constructor")
        .as_str(),
        format!("schema:mfm.kernel.balance:1:sha256-jcs-v1:{DIGEST_HEX}")
    );

    assert_eq!(
        EffectKind::new("mfm.kernel", "pure", DigestAlgorithm::Sha256JcsV1, digest())
            .expect("effect constructor")
            .as_str(),
        format!("effect:mfm.kernel:pure:sha256-jcs-v1:{DIGEST_HEX}")
    );
}

#[test]
fn store_scope_id_accepts_only_stable_lowercase_hex_shape() {
    let store_scope = StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
        .expect("store scope");

    assert_eq!(
        store_scope.as_str(),
        "mfm.store_scope.v1:0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        store_scope.to_string(),
        "mfm.store_scope.v1:0123456789abcdef0123456789abcdef"
    );
    assert!(
        StoreScopeId::new("mfm.store_scope.unsupported:0123456789abcdef0123456789abcdef").is_err()
    );
    assert!(StoreScopeId::new("mfm.store_scope.v1:0123456789ABCDEF0123456789abcdef").is_err());
    assert!(StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef").is_err());
}

#[test]
fn checked_versions_reject_invalid_tokens() {
    let schema_version = SchemaVersion::new("1").expect("schema version");
    let effect_version = EffectVersion::new("mfm.effect.v1").expect("effect version");
    let capability_version =
        CapabilityVersion::new("mfm.capability.v1").expect("capability version");

    assert_eq!(schema_version.as_str(), "1");
    assert_eq!(effect_version.as_str(), "mfm.effect.v1");
    assert_eq!(capability_version.as_str(), "mfm.capability.v1");
    assert!(SchemaVersion::new("_private").is_err());
}

#[test]
fn rejects_invalid_identity_strings() {
    type RejectCase = (&'static str, fn(&str) -> bool);

    let cases: &[RejectCase] = &[
        ("schema:mfm.name:1:sha256-jcs-v1:0123", |value| {
            SchemaId::parse(value).is_err()
        }),
        (
            "effect:mfm:reader:1:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| EffectKind::parse(value).is_err(),
        ),
        (
            "effect:Mfm:reader:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| EffectKind::parse(value).is_err(),
        ),
        (
            "effect:mfm:_reader:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| EffectKind::parse(value).is_err(),
        ),
        (
            "effect:mfm:reader:sha256-jcs-unsupported:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| EffectKind::parse(value).is_err(),
        ),
        (
            "effect:mfm:reader:sha256-jcs-v1:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| EffectKind::parse(value).is_err(),
        ),
        (
            "schema:mfm.name:1:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| EffectKind::parse(value).is_err(),
        ),
    ];

    for (value, rejects) in cases {
        assert!(rejects(value), "expected rejection for {value}");
    }
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
