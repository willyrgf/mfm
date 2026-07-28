use mfm_runtime::CertifiedRuntimeSpec;

fn runtime_spec() -> CertifiedRuntimeSpec {
    unimplemented!()
}

fn main() {
    let authority = runtime_spec();
    let _duplicate = authority.clone();
}
