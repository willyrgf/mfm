use mfm_certify::LoweredTypedSpec;

fn lowered_spec() -> LoweredTypedSpec {
    unimplemented!()
}

fn main() {
    let lowered = lowered_spec();
    lowered.spec().nodes.clear();
}
