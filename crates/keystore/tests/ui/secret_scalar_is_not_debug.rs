fn require_debug<T: std::fmt::Debug>() {}

fn main() {
    require_debug::<mfm_keystore::SecretSecp256k1Scalar>();
}
