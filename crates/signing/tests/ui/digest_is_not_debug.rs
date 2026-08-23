fn require_debug<T: std::fmt::Debug>() {}

fn main() {
    require_debug::<mfm_signing::SigningDigest>();
}
