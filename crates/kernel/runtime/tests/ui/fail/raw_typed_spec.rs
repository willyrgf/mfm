use mfm_runtime::CertifiedRuntimeSpec;
use mfm_spec::v1 as spec;

fn raw_spec() -> spec::TypedExecutionSpec {
    unimplemented!()
}

fn main() {
    let _ = CertifiedRuntimeSpec::new(raw_spec());
}
