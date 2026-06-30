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
        StateKind::parse(format!(
            "state:mfm.evm:read-balance:sha256-jcs-v1:{DIGEST_HEX}"
        ))
        .expect("state kind"),
        "state:mfm.evm:read-balance:sha256-jcs-v1:",
        Some("mfm.evm/read-balance"),
        None,
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
    assert_identity!(
        AdapterKind::parse(format!(
            "adapter:mfm.evm:json-rpc:sha256-jcs-v1:{DIGEST_HEX}"
        ))
        .expect("adapter kind"),
        "adapter:mfm.evm:json-rpc:sha256-jcs-v1:",
        Some("mfm.evm/json-rpc"),
        None,
    );
    assert_identity!(
        OperationKind::parse(format!(
            "operation:mfm.portfolio:track:sha256-jcs-v1:{DIGEST_HEX}"
        ))
        .expect("operation kind"),
        "operation:mfm.portfolio:track:sha256-jcs-v1:",
        Some("mfm.portfolio/track"),
        None,
    );
}

#[test]
fn accepts_digest_only_identity_golden_strings() {
    let cases = [
        DescriptorId::parse(format!("descriptor:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("descriptor id")
            .to_string(),
        SpecHash::parse(format!("spec:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("spec hash")
            .to_string(),
        SideEffectPairId::parse(format!("side_effect_pair:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("side-effect pair id")
            .to_string(),
        OperationInstanceId::parse(format!("op:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("operation instance id")
            .to_string(),
        NodeId::parse(format!("node:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("node id")
            .to_string(),
        CellId::parse(format!("cell:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("cell id")
            .to_string(),
        ScopeId::parse(format!("scope:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("scope id")
            .to_string(),
        SeedId::parse(format!("seed:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("seed id")
            .to_string(),
        AttemptId::parse(format!("attempt:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("attempt id")
            .to_string(),
        RunId::parse(format!("run:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("run id")
            .to_string(),
        EventId::parse(format!("event:sha256-jcs-v1:{DIGEST_HEX}"))
            .expect("event id")
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
            format!("descriptor:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("spec:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("side_effect_pair:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("op:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("node:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("cell:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("scope:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("seed:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("attempt:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("run:sha256-jcs-v1:{DIGEST_HEX}"),
            format!("event:sha256-jcs-v1:{DIGEST_HEX}"),
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
        StateKind::new(
            "mfm.portfolio",
            "load",
            DigestAlgorithm::Sha256JcsV1,
            digest()
        )
        .expect("state constructor")
        .as_str(),
        format!("state:mfm.portfolio:load:sha256-jcs-v1:{DIGEST_HEX}")
    );

    assert_eq!(
        EffectKind::new("mfm.kernel", "pure", DigestAlgorithm::Sha256JcsV1, digest())
            .expect("effect constructor")
            .as_str(),
        format!("effect:mfm.kernel:pure:sha256-jcs-v1:{DIGEST_HEX}")
    );

    assert_eq!(
        DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest()).as_str(),
        format!("descriptor:sha256-jcs-v1:{DIGEST_HEX}")
    );
}

#[test]
fn trust_scope_id_accepts_only_stable_lowercase_hex_shape() {
    let trust_scope = TrustScopeId::new("mfm.trust_scope.v1:0123456789abcdef0123456789abcdef")
        .expect("trust scope");

    assert_eq!(
        trust_scope.as_str(),
        "mfm.trust_scope.v1:0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        trust_scope.to_string(),
        "mfm.trust_scope.v1:0123456789abcdef0123456789abcdef"
    );
    assert!(TrustScopeId::new("mfm.trust_scope.v2:0123456789abcdef0123456789abcdef").is_err());
    assert!(TrustScopeId::new("mfm.trust_scope.v1:0123456789ABCDEF0123456789abcdef").is_err());
    assert!(TrustScopeId::new("mfm.trust_scope.v1:0123456789abcdef").is_err());
}

#[test]
fn checked_versions_reject_stringly_mixups() {
    let state_version = StateVersion::new("mfm.state.v1").expect("state version");
    let effect_version = EffectVersion::new("mfm.effect.v1").expect("effect version");
    let operation_version = OperationVersion::new("mfm.operation.v1").expect("operation version");

    assert_eq!(state_version.as_str(), "mfm.state.v1");
    assert_eq!(effect_version.as_str(), "mfm.effect.v1");
    assert_eq!(operation_version.as_str(), "mfm.operation.v1");
    assert!(StateVersion::new("_private").is_err());
    assert!(SpecVersion::new("MFM.typed.v1").is_err());
}

#[test]
fn rejects_invalid_identity_strings() {
    type RejectCase = (&'static str, fn(&str) -> bool);

    let cases: &[RejectCase] = &[
        ("schema:mfm.name:1:sha256-jcs-v1:0123", |value| {
            SchemaId::parse(value).is_err()
        }),
        (
            "state:mfm:reader:1:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| StateKind::parse(value).is_err(),
        ),
        (
            "state:Mfm:reader:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| StateKind::parse(value).is_err(),
        ),
        (
            "state:mfm:_reader:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| StateKind::parse(value).is_err(),
        ),
        (
            "state:mfm:reader:sha256-jcs-v2:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| StateKind::parse(value).is_err(),
        ),
        (
            "state:mfm:reader:sha256-jcs-v1:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| StateKind::parse(value).is_err(),
        ),
        (
            "schema:mfm.name:1:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| StateKind::parse(value).is_err(),
        ),
        (
            "descriptor:mfm.name:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            |value| DescriptorId::parse(value).is_err(),
        ),
    ];

    for (value, rejects) in cases {
        assert!(rejects(value), "expected rejection for {value}");
    }
}
