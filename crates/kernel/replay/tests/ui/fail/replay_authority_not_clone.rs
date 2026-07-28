fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<mfm_replay::v1::ReplayReadAuthority<'static>>();
    require_clone::<mfm_replay::v1::ReplayBroker<'static>>();
}
