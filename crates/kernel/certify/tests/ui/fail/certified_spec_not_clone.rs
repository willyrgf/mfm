use mfm_certify::CertifiedTypedSpec;

fn certified_spec() -> CertifiedTypedSpec {
    unimplemented!()
}

fn main() {
    let authority = certified_spec();
    let _duplicate = authority.clone();
}
