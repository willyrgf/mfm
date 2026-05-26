use mfm_runtime::CertifiedRuntimeSpec;
use mfm_spec::v1 as spec;

fn hash_only_envelope() -> spec::HashedSpecEnvelope {
    unimplemented!()
}

fn main() {
    let _ = CertifiedRuntimeSpec::new(hash_only_envelope());
}
