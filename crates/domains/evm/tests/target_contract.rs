use mfm_evm::{EvmEndpoint, EvmPhysicalTarget, EvmReadIntent, EvmReadSubject};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_values::canonicalize_mfm_value;
use mfm_values::MfmValue;

fn endpoint(byte: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.endpoint",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        )
        .expect("schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            DigestBytes::from_array([byte; 32]),
        ),
    )
    .expect("endpoint")
}

#[test]
fn physical_target_has_one_exact_checked_identity() {
    let target = EvmPhysicalTarget::new(1, endpoint(2)).expect("target");
    let (canonical, reference) = canonicalize_mfm_value(&target).expect("canonical target");
    let semantic = EvmPhysicalTarget::semantic_id().expect("semantic identity");
    assert_eq!(
        format!(
            "{}@{}",
            semantic.canonical_name().expect("semantic name"),
            semantic.version().expect("semantic version")
        ),
        "mfm.evm/physical-target@1"
    );
    let schema = EvmPhysicalTarget::schema_descriptor()
        .expect("schema descriptor")
        .schema_id()
        .expect("schema identity");
    assert_eq!(schema.canonical_name(), Some("mfm.evm-physical-target"));
    assert_eq!(schema.version(), Some("1"));
    assert_eq!(
        std::str::from_utf8(canonical.as_bytes()).expect("utf8"),
        "{\"chain_id\":1,\"endpoint_ref\":{\"content_digest\":\"content:sha256-v1:0202020202020202020202020202020202020202020202020202020202020202\",\"schema_id\":\"schema:mfm.test.endpoint:1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101\"}}"
    );
    assert_eq!(target.binding_ref().expect("binding"), reference);
    assert_eq!(
        serde_json::to_string(&reference).expect("reference json"),
        "{\"content_digest\":\"content:sha256-v1:7701e82ec5bd36b79b7e361b85bfe358494b531ad687b2683a397eaf7f44037e\",\"schema_id\":\"schema:mfm.evm-physical-target:1:sha256-jcs-v1:4f21dfcf2cbe47e513963fff2d6da4e73c7f603dcaf5fb558b670f27f82cab32\"}"
    );

    assert!(EvmPhysicalTarget::new(0, endpoint(2)).is_err());
    assert!(
        serde_json::from_value::<EvmPhysicalTarget>(serde_json::json!({
            "chain_id": 0,
            "endpoint_ref": endpoint(2)
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<EvmPhysicalTarget>(serde_json::json!({
            "chain_id": 1,
            "endpoint_ref": endpoint(2),
            "unknown": true
        }))
        .is_err()
    );
}

#[test]
fn endpoint_identity_changes_binding_while_handle_identity_is_absent() {
    let first = EvmPhysicalTarget::new(1, endpoint(2)).expect("first");
    let second = EvmPhysicalTarget::new(1, endpoint(3)).expect("second");
    assert_ne!(
        first.binding_ref().expect("first ref"),
        second.binding_ref().expect("second ref")
    );
}

#[test]
fn named_endpoint_is_the_only_route_material_and_derives_one_stable_ref() {
    let endpoint = EvmEndpoint::new("reth-dev").expect("endpoint");
    let (canonical, reference) = canonicalize_mfm_value(&endpoint).expect("canonical endpoint");
    let semantic = EvmEndpoint::semantic_id().expect("semantic identity");
    assert_eq!(
        format!(
            "{}@{}",
            semantic.canonical_name().expect("semantic name"),
            semantic.version().expect("semantic version")
        ),
        "mfm.evm/endpoint@1"
    );
    let schema = EvmEndpoint::schema_descriptor()
        .expect("schema descriptor")
        .schema_id()
        .expect("schema identity");
    assert_eq!(schema.canonical_name(), Some("mfm.evm-endpoint"));
    assert_eq!(schema.version(), Some("1"));
    assert_eq!(
        std::str::from_utf8(canonical.as_bytes()).expect("utf8"),
        "{\"endpoint_id\":\"reth-dev\"}"
    );
    assert_eq!(endpoint.endpoint_ref().expect("endpoint ref"), reference);

    // The same name derives the same target across processes; a different name does not.
    let target =
        EvmPhysicalTarget::new(1337, endpoint.endpoint_ref().expect("ref")).expect("target");
    assert_eq!(
        target.binding_ref().expect("binding"),
        EvmPhysicalTarget::new(
            1337,
            EvmEndpoint::new("reth-dev")
                .expect("repeat endpoint")
                .endpoint_ref()
                .expect("repeat ref")
        )
        .expect("repeat target")
        .binding_ref()
        .expect("repeat binding")
    );
    assert_ne!(
        target.binding_ref().expect("binding"),
        EvmPhysicalTarget::new(
            1337,
            EvmEndpoint::new("reth-other")
                .expect("other endpoint")
                .endpoint_ref()
                .expect("other ref")
        )
        .expect("other target")
        .binding_ref()
        .expect("other binding")
    );

    assert!(EvmEndpoint::new("").is_err());
    assert!(EvmEndpoint::new("secret\u{7f}").is_err());
    assert!(EvmEndpoint::new("a".repeat(257)).is_err());
    assert!(serde_json::from_str::<EvmEndpoint>(r#"{"endpoint_id":""}"#).is_err());
    assert!(
        serde_json::from_str::<EvmEndpoint>(r#"{"endpoint_id":"reth-dev","url":"x"}"#).is_err()
    );
}

#[test]
fn live_providers_read_the_checked_intent_subject_without_a_wire_mirror() {
    let target = EvmPhysicalTarget::new(1337, endpoint(2)).expect("target");
    let intent: EvmReadIntent = serde_json::from_value(serde_json::json!({
        "operation": "mfm.evm.read-native-balance@1",
        "chain_id": 1337,
        "subject": {
            "kind": "native_balance",
            "value": {
                "source": {
                    "source_id": "wallet.native",
                    "chain_id": 1337,
                    "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
                    "token": null
                },
                "anchor": { "number": "12", "hash": "0xabc" }
            }
        },
        "route_ref": target.binding_ref().expect("binding")
    }))
    .expect("checked intent");

    assert_eq!(
        intent.operation_and_chain_id(),
        ("mfm.evm.read-native-balance@1", 1337)
    );
    let EvmReadSubject::NativeBalance { source, anchor } = intent.subject() else {
        panic!("native balance intent must expose its typed subject");
    };
    assert_eq!(source.address, "0x70997970c51812dc3a010c7d01b50e0d17dc79c8");
    assert_eq!(anchor.number(), "12");
    assert_eq!(anchor.hash(), "0xabc");

    // The checked deserializer, not the provider, rejects an unbound subject.
    assert!(serde_json::from_value::<EvmReadIntent>(serde_json::json!({
        "operation": "mfm.evm.read-native-balance@1",
        "chain_id": 1337,
        "subject": {
            "kind": "native_balance",
            "value": {
                "source": {
                    "source_id": "wallet.native",
                    "chain_id": 7,
                    "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
                    "token": null
                },
                "anchor": { "number": "12", "hash": "0xabc" }
            }
        },
        "route_ref": target.binding_ref().expect("binding")
    }))
    .is_err());
}

#[test]
fn surviving_balance_result_fixture_remains_exact_canonical_json() {
    let fixture =
        include_str!("../../../../docs/contracts/evm-portfolio/evm-balance-collection.json").trim();
    mfm_canonical::PlainCanonicalJsonBytes::from_canonical_json_slice(fixture.as_bytes())
        .expect("canonical public result fixture");
    assert_eq!(
        mfm_canonical::raw_content_digest(fixture.as_bytes()).as_str(),
        "content:sha256-v1:189aabd993e34900051abb97befabca3d846dc4d0adf24c50b1de38546c30ad6"
    );
}
