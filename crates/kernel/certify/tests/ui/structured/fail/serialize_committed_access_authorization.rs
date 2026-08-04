use mfm_certify::structured::CertifiedAccessAuthorization;
use serde::Serialize;

fn require_serialize<T: Serialize>() {}

fn main() {
    require_serialize::<CertifiedAccessAuthorization>();
}
