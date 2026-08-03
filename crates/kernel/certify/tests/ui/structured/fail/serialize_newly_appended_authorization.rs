use mfm_store::structured::NewlyAppendedAuthorization;
use serde::Serialize;

fn require_serialize<T: Serialize>() {}

fn main() {
    require_serialize::<NewlyAppendedAuthorization>();
}
