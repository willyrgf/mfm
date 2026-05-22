#[path = "../support/types.rs"]
mod types;

use mfm_ids::{CellId, DigestAlgorithm, DigestBytes, SchemaId, ScopeId, SemanticTypeId};

fn main() {
    let digest = DigestBytes::from_array([0x11; 32]);
    let _handle = mfm_program::Handle::<types::TryValue> {
        cell_id: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest.clone()),
        scope_id: ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest.clone()),
        schema_id: SchemaId::new(
            "mfm.program.trybuild.try_value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest.clone(),
        )
        .unwrap(),
        semantic_type_id: SemanticTypeId::new(
            "mfm.program.trybuild",
            "try_value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest,
        )
        .unwrap(),
        _program: std::marker::PhantomData,
        _scope: std::marker::PhantomData,
        _value: std::marker::PhantomData,
    };
}
