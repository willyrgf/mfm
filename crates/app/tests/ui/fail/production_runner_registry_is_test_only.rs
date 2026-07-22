fn main() {
    let _ = mfm_app::production_runner_registry_for_test::<
        mfm_store::v1::AsyncInMemoryRunStore,
    >;
}
