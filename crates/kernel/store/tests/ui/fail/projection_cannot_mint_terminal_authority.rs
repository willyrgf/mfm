use mfm_ids::SpecHash;
use mfm_spec::v1::SagaPolicySpec;
use mfm_store::v1::{SagaProjection, SagaTerminalProof, StreamSeq};

fn forge(policy: &SagaPolicySpec, saga: &SagaProjection, spec_hash: &SpecHash) {
    let _ = SagaTerminalProof::new(policy, saga, StreamSeq::new(1).unwrap(), spec_hash, None);
}

fn main() {}
