use mfm_manual_auth::VerifiedManualResolutionForPrefix;
use mfm_store::v1::{current_lifecycle::CurrentLifecycleReader, StreamSeq};

fn forge(reader: &CurrentLifecycleReader<'_>, proof: VerifiedManualResolutionForPrefix) {
    let _ = reader.saga_terminal_proof(StreamSeq::new(1).unwrap(), Some(proof));
}

fn main() {}
