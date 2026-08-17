use mfm_evm::EvmPhysicalTarget;
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
