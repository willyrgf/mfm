use mfm_evm::{EvmEndpoint, EvmPhysicalTarget};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_values::{canonicalize_mfm_value, MfmValue};

#[test]
fn a_physical_target_has_one_interoperable_checked_identity() {
    let endpoint = ContentRef::new(
        SchemaId::new(
            "mfm.test.endpoint",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        )
        .expect("schema"),
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([2; 32])),
    )
    .expect("endpoint");
    let target = EvmPhysicalTarget::new(1, endpoint.clone()).expect("target");
    let (canonical, reference) = canonicalize_mfm_value(&target).expect("canonical target");
    let semantic = EvmPhysicalTarget::semantic_id().expect("semantic identity");
    assert_eq!(semantic.canonical_name(), Some("mfm.evm/physical-target"));
    assert_eq!(semantic.version(), Some("1"));
    assert_eq!(
        canonical.as_bytes(),
        br#"{"chain_id":1,"endpoint_ref":{"content_digest":"content:sha256-v1:0202020202020202020202020202020202020202020202020202020202020202","schema_id":"schema:mfm.test.endpoint:1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101"}}"#
    );
    assert_eq!(target.binding_ref().expect("binding"), reference);
    assert_eq!(
        serde_json::to_string(&reference).expect("reference JSON"),
        r#"{"content_digest":"content:sha256-v1:7701e82ec5bd36b79b7e361b85bfe358494b531ad687b2683a397eaf7f44037e","schema_id":"schema:mfm.evm-physical-target:1:sha256-jcs-v1:2ee09931b3289c2a2169788badc06101af145d6628751550bbd21d30d6ac346c"}"#
    );

    assert!(EvmPhysicalTarget::new(0, endpoint.clone()).is_err());
    assert!(
        serde_json::from_value::<EvmPhysicalTarget>(serde_json::json!({
            "chain_id": 0,
            "endpoint_ref": endpoint
        }))
        .is_err()
    );
}

#[test]
fn endpoint_names_are_the_only_stable_route_material() {
    let endpoint = EvmEndpoint::new("reth-dev").expect("endpoint");
    let (canonical, reference) = canonicalize_mfm_value(&endpoint).expect("canonical endpoint");
    assert_eq!(canonical.as_bytes(), br#"{"endpoint_id":"reth-dev"}"#);
    assert_eq!(endpoint.endpoint_ref().expect("endpoint ref"), reference);

    let target = EvmPhysicalTarget::new(1337, reference).expect("target");
    let same = EvmPhysicalTarget::new(
        1337,
        EvmEndpoint::new("reth-dev")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("same endpoint"),
    )
    .expect("same target");
    let other = EvmPhysicalTarget::new(
        1337,
        EvmEndpoint::new("reth-other")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("other endpoint"),
    )
    .expect("other target");
    assert_eq!(
        target.binding_ref().expect("binding"),
        same.binding_ref().expect("same binding")
    );
    assert_ne!(
        target.binding_ref().expect("binding"),
        other.binding_ref().expect("other binding")
    );

    for invalid in ["", "secret\u{7f}", &"a".repeat(257)] {
        assert!(EvmEndpoint::new(invalid).is_err());
    }
}
