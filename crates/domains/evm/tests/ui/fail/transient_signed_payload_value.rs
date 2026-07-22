use mfm_evm::TransientSignedEip1559Envelope;
use mfm_values::MfmValue;
use serde::Serialize;

fn require_serialize<T: Serialize>() {}
fn require_mfm_value<T: MfmValue>() {}

fn main() {
    require_serialize::<TransientSignedEip1559Envelope>();
    require_mfm_value::<TransientSignedEip1559Envelope>();
}
