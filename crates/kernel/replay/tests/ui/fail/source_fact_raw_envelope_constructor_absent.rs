fn value<T>() -> T {
    panic!("compile-fail boundary test")
}

fn main() {
    let _ = mfm_replay::v1::RetainedSourceFactReplayEvent::new(value(), value());
}
