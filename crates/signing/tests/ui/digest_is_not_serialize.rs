fn require_serialize<T: serde::Serialize>() {}

fn main() {
    require_serialize::<mfm_signing::SigningDigest>();
}
