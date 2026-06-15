use mfm_certify::{certify_typed_spec, CertificationRegistry};
use mfm_spec::v1 as spec;

fn raw_spec() -> spec::TypedExecutionSpec {
    unimplemented!()
}

fn main() {
    let registry = CertificationRegistry::new();
    let _ = certify_typed_spec(raw_spec(), &registry);
}
