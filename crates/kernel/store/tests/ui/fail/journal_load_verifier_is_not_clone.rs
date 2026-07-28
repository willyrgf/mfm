use mfm_store::v1::JournalLoadVerifier;

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<JournalLoadVerifier>();
}
