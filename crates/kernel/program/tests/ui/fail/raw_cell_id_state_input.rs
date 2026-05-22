use mfm_ids::{CellId, DigestAlgorithm, DigestBytes};

fn main() {
    let cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x33; 32]),
    );
    let _ = <CellId as mfm_program::IntoStateInput<'static, 'static, CellId>>::into_binding(cell);
}
