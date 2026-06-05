use mfm_signing::{
    SignerRef, SigningAlgorithmId, SigningDomainId, SigningPurposeId, SigningRequest,
};

fn main() {
    let _request = SigningRequest::from_message(
        SignerRef::new("deployer").unwrap(),
        SigningAlgorithmId::new("mfm.signing.test").unwrap(),
        SigningDomainId::new("mfm.test").unwrap(),
        SigningPurposeId::new("test.sign").unwrap(),
        b"arbitrary message".to_vec(),
    );
}
