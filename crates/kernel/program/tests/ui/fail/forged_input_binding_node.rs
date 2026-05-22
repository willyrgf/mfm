use mfm_ids::{
    CellId, ContentDigest, DigestAlgorithm, DigestBytes, SchemaId, SemanticTypeId,
};

fn main() {
    let digest = DigestBytes::from_array([0x44; 32]);
    let _node = mfm_program::InputBindingNode::Cell {
        field_path: mfm_program::InputFieldPath::new("value").unwrap(),
        cell_id: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest.clone()),
        semantic_type_id: SemanticTypeId::new(
            "mfm.program.trybuild",
            "try_value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest.clone(),
        )
        .unwrap(),
        schema_id: SchemaId::new(
            "mfm.program.trybuild.try_value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest.clone(),
        )
        .unwrap(),
        value_lineage: mfm_program::ValueLineageRef::new(ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest,
        )),
        required_terminal: mfm_program::RequiredTerminal::ProducedOnly,
    };
}
