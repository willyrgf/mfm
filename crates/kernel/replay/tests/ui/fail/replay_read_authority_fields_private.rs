fn value<T>() -> T {
    panic!("compile-fail boundary test")
}

fn main() {
    let _ = mfm_replay::v1::ReplayReadAuthority {
        view: value(),
        additional_artifacts: Vec::new(),
        source_fact_events: Vec::new(),
    };
}
