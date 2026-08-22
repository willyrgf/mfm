fn require_display<T: std::fmt::Display>() {}

fn main() {
    require_display::<mfm_keystore::SecretSecp256k1Scalar>();
}
