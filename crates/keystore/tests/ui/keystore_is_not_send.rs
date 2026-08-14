fn require_send<T: Send>() {}

fn main() {
    require_send::<mfm_keystore::Keystore>();
}
