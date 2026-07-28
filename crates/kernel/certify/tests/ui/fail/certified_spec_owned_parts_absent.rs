use mfm_certify::CertifiedTypedSpec;

fn certified_spec() -> CertifiedTypedSpec {
    unimplemented!()
}

fn main() {
    let authority = certified_spec();
    let _owned_parts = authority.into_parts();
}
