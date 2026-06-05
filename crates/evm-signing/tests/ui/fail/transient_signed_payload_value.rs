use mfm_evm_signing::TransientRawTransaction;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "bad_signed_payload_value",
    version = "1",
    schema = "mfm.trybuild.bad_signed_payload_value"
)]
struct BadSignedPayloadValue {
    signed_payload: TransientRawTransaction,
}

fn main() {}
