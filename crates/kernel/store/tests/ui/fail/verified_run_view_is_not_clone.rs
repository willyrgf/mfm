use mfm_store::v1::VerifiedRunView;

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<VerifiedRunView>();
}
