use super::*;

#[test]
fn retained_checked_ids_enforce_their_full_bounds() {
    assert!(StableId::new("a".repeat(512)).is_ok());
    assert!(StableId::new("a".repeat(513)).is_err());

    let entry = format!("mfm.{}/x@1", "a".repeat(504));
    assert_eq!(entry.len(), 512);
    assert!(EntryPointId::new(&entry).is_ok());
    assert!(EntryPointId::new(format!("{entry}a")).is_err());
}

#[test]
fn content_ref_requires_schema_and_exact_byte_algorithms() {
    let bytes = DigestBytes::from_array([0; 32]);
    let schema =
        SchemaId::new("mfm-test", "1", DigestAlgorithm::Sha256JcsV1, bytes).expect("schema");
    let content = ContentDigest::from_digest(DigestAlgorithm::Sha256V1, bytes);
    assert!(ContentRef::new(schema, content).is_ok());
}

#[test]
fn fixed_digest_ids_construct_parse_and_serialize_with_jcs_algorithm() {
    let bytes = DigestBytes::from_array([7; 32]);
    let run = RunId::from_digest(bytes);
    let artifact = ArtifactId::from_digest(bytes);

    assert_eq!(run.algorithm(), DigestAlgorithm::Sha256JcsV1);
    assert_eq!(artifact.algorithm(), DigestAlgorithm::Sha256JcsV1);
    assert_eq!(RunId::parse(run.as_str()), Ok(run.clone()));
    assert_eq!(ArtifactId::parse(artifact.as_str()), Ok(artifact.clone()));
    assert_eq!(
        serde_json::to_string(&run).expect("run json"),
        format!("\"run:sha256-jcs-v1:{bytes}\"")
    );
    assert_eq!(
        serde_json::to_string(&artifact).expect("artifact json"),
        format!("\"artifact:sha256-jcs-v1:{bytes}\"")
    );
}
