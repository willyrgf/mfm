use mfm_certify::{CertifiedSpecCertificate, CertifiedTypedSpec, ValidatedTypedExecutionSpec};

fn validated_spec() -> ValidatedTypedExecutionSpec {
    unimplemented!()
}

fn certificate() -> CertifiedSpecCertificate {
    unimplemented!()
}

fn main() {
    let _ = CertifiedTypedSpec {
        validated: validated_spec(),
        certificate: certificate(),
    };
}
