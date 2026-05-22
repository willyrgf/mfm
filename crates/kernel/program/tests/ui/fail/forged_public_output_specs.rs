#[path = "../support/types.rs"]
mod types;

use mfm_ids::{CellId, DigestAlgorithm, DigestBytes, SchemaId, ScopeId, SemanticTypeId};

fn main() {
    let digest = DigestBytes::from_array([0x22; 32]);
    let cell_id = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest.clone());
    let scope_id = ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest.clone());
    let schema_id = SchemaId::new(
        "mfm.program.trybuild.try_value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest.clone(),
    )
    .unwrap();
    let semantic_type_id = SemanticTypeId::new(
        "mfm.program.trybuild",
        "try_value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest,
    )
    .unwrap();

    let handle_ref = mfm_program::TypedHandleRef {
        cell_id,
        scope_id,
        schema_id: schema_id.clone(),
        semantic_type_id,
    };
    let cell = mfm_program::PublicOutputCellSpec {
        public_field_path: mfm_program::PublicFieldPath::new("result").unwrap(),
        cell: handle_ref,
    };
    let _spec = mfm_program::PublicOutputSpec {
        key: mfm_program::PublicOutputKey::new("terminal").unwrap(),
        public_schema_id: schema_id,
        outputs: vec![cell],
    };
}
