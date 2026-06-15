use mfm_certify::ValidatedTypedExecutionSpec;
use mfm_spec::v1 as spec;

fn envelope() -> spec::HashedSpecEnvelope {
    unimplemented!()
}

fn main() {
    let _ = ValidatedTypedExecutionSpec {
        envelope: envelope(),
    };
}
