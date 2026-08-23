fn require_display<T: std::fmt::Display>() {}

fn main() {
    require_display::<mfm_signing::SigningDigest>();
}
