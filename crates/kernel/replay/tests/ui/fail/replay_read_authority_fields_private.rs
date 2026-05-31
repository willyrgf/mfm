fn value<T>() -> T {
    panic!("compile-fail boundary test")
}

fn main() {
    let _ = mfm_replay::v1::ReplayReadAuthority {
        certified_spec: value(),
        stream: Vec::new(),
        canonicalizer_identity: value(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        artifact_evidence: Vec::new(),
    };
}
