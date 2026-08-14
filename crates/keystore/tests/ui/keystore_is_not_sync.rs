fn require_sync<T: Sync>() {}

fn main() {
    require_sync::<mfm_keystore::Keystore>();
}
