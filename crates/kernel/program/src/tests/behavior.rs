use super::*;

#[path = "behavior/authoring.rs"]
mod authoring_tests;

#[path = "behavior/lineage.rs"]
mod lineage_tests;

#[path = "behavior/inputs.rs"]
mod input_tests;

#[path = "behavior/seeds.rs"]
mod seed_tests;

fn cell_path(node: &InputBindingNode) -> &str {
    let InputBindingNodeKind::Cell(cell) = &node.kind else {
        panic!("expected cell node");
    };
    cell.field_path.as_str()
}

fn artifact_ref_fixture(byte: u8) -> Result<mfm_values::ArtifactRef<LaunchValue>> {
    let artifact_id = mfm_ids::ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    );
    let content_digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte.wrapping_add(1); 32]),
    );
    mfm_values::ArtifactRef::<LaunchValue>::new(artifact_id, content_digest)
        .map_err(|error| PlanError::Value(error.to_string()))
}
