use mfm_runtime::CertifiedRuntimeSpec;
use mfm_spec::v1 as spec;

fn uncertified_spec() -> spec::TypedExecutionSpec {
    unimplemented!()
}

fn main() {
    let _ = CertifiedRuntimeSpec::new(uncertified_spec());
}
