use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use alloy_primitives::{address, b256, hex, PrimitiveSignature};
use mfm_signing::{
    DeterministicSigningProvider, PublicSigningIdentity, SignatureBytes, SigningFuture,
    SigningProfileId, SigningProvider,
};

const EXPECTED_SENDER: Address = address!("dd6b8b3dc6b7ad97db52f08a275ff4483e024cea");

fn signer_ref() -> SignerRef {
    SignerRef::new("deployer").expect("signer ref")
}

fn alloy_vector_envelope() -> UnsignedEip1559Envelope {
    UnsignedEip1559Envelope::new(
        U256::from(1),
        U256::from(0x42),
        U256::from(0x3b9aca00_u64),
        U256::from(0x4a817c800_u64),
        U256::from(44_386),
        address!("6069a6c32cf691f5982febae4faf8a6f3ab2f0f6").into(),
        U256::ZERO,
        AccessList::default(),
        hex!("a22cb4650000000000000000000000005eee75727d804a2b13038928d36f8b188945a57a0000000000000000000000000000000000000000000000000000000000000000").into(),
    )
    .expect("envelope")
}

fn alloy_vector_signature() -> SignatureBytes {
    let signature = PrimitiveSignature::from_scalars_and_parity(
        b256!("840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca90058565"),
        b256!("25e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1"),
        false,
    );
    SignatureBytes::new(signature.as_bytes().to_vec()).expect("signature")
}

fn identity(account: Address, algorithm: SigningAlgorithmId) -> PublicSigningIdentity {
    PublicSigningIdentity::new(algorithm, None, Some(format!("{account:?}"))).expect("identity")
}

struct FixedProvider {
    calls: AtomicUsize,
    signature: SignatureBytes,
    account: Address,
}

impl FixedProvider {
    fn alloy_vector() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            signature: alloy_vector_signature(),
            account: EXPECTED_SENDER,
        }
    }
}

impl SigningProvider for FixedProvider {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = SigningResult::for_request(
            request,
            identity(self.account, request.algorithm().clone()),
            self.signature.clone(),
        );
        Box::pin(async move { result })
    }
}

impl DeterministicSigningProvider for FixedProvider {
    fn implementation_id(&self) -> &'static str {
        "mfm.test.fixed-signer"
    }

    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

struct WrongProfileProvider(FixedProvider);

impl SigningProvider for WrongProfileProvider {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        self.0.sign(request)
    }
}

impl DeterministicSigningProvider for WrongProfileProvider {
    fn implementation_id(&self) -> &'static str {
        "mfm.test.wrong-profile-signer"
    }

    fn deterministic_profile_id(&self) -> &'static str {
        "secp256k1.other.deterministic.v1"
    }
}

#[test]
fn admits_u256_inputs_only_when_alloy_can_represent_them_exactly() {
    let overflow_u64 = U256::from(u64::MAX) + U256::from(1);
    let overflow_u128 = U256::from(u128::MAX) + U256::from(1);

    for (chain_id, nonce, priority, max, gas, expected_field) in [
        (
            overflow_u64,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            Eip1559QuantityField::ChainId,
        ),
        (
            U256::from(1),
            overflow_u64,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            Eip1559QuantityField::Nonce,
        ),
        (
            U256::from(1),
            U256::ZERO,
            overflow_u128,
            overflow_u128,
            U256::ZERO,
            Eip1559QuantityField::MaxPriorityFeePerGas,
        ),
        (
            U256::from(1),
            U256::ZERO,
            U256::ZERO,
            overflow_u128,
            U256::ZERO,
            Eip1559QuantityField::MaxFeePerGas,
        ),
        (
            U256::from(1),
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            overflow_u64,
            Eip1559QuantityField::GasLimit,
        ),
    ] {
        let error = UnsignedEip1559Envelope::new(
            chain_id,
            nonce,
            priority,
            max,
            gas,
            TxKind::Create,
            U256::MAX,
            AccessList::default(),
            Bytes::new(),
        )
        .expect_err("overflow");
        assert_eq!(
            error,
            EvmSigningError::QuantityOutOfRange {
                field: expected_field
            }
        );
    }

    let envelope = UnsignedEip1559Envelope::new(
        U256::from(u64::MAX),
        U256::from(u64::MAX),
        U256::from(u128::MAX),
        U256::from(u128::MAX),
        U256::from(u64::MAX),
        TxKind::Create,
        U256::MAX,
        AccessList::default(),
        Bytes::new(),
    )
    .expect("exact maxima");
    assert_eq!(envelope.value(), U256::MAX);
}

#[test]
fn rejects_zero_chain_and_invalid_fee_relation() {
    let zero_chain = UnsignedEip1559Envelope::new(
        U256::ZERO,
        U256::ZERO,
        U256::ZERO,
        U256::ZERO,
        U256::ZERO,
        TxKind::Create,
        U256::ZERO,
        AccessList::default(),
        Bytes::new(),
    )
    .expect_err("zero chain");
    assert_eq!(zero_chain, EvmSigningError::ZeroChainId);

    let fees = UnsignedEip1559Envelope::new(
        U256::from(1),
        U256::ZERO,
        U256::from(3),
        U256::from(2),
        U256::from(21_000),
        TxKind::Create,
        U256::ZERO,
        AccessList::default(),
        Bytes::new(),
    )
    .expect_err("fee relation");
    assert_eq!(fees, EvmSigningError::PriorityFeeExceedsMaxFee);
}

#[test]
fn alloy_known_vector_uses_one_hash_and_encoding_path() {
    let envelope = alloy_vector_envelope();
    assert_eq!(
        envelope.signing_digest(),
        b256!("0d5688ac3897124635b6cf1bc0e29d6dfebceebdc10a54d74f2ef8b56535b682")
    );
    let request = envelope
        .signing_request(signer_ref(), EXPECTED_SENDER)
        .expect("request");
    assert_eq!(
        request.algorithm().as_str(),
        SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID
    );
    assert_eq!(
        request.profile().as_str(),
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    );
    let result = SigningResult::for_request(
        &request,
        identity(EXPECTED_SENDER, request.algorithm().clone()),
        alloy_vector_signature(),
    )
    .expect("result");
    let signed = envelope
        .finalize_signed(&request, EXPECTED_SENDER, &result)
        .expect("signed");

    assert_eq!(
        signed.transaction_hash(),
        b256!("0ec0b6a2df4d87424e5f6ad2a654e27aaeb7dac20ae9e8385cc09087ad532ee0")
    );
    assert_eq!(keccak256(signed.bytes()), signed.transaction_hash());
    assert!(signed.bytes().starts_with(&[0x02]));
    let rendered = format!("{signed:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains(&hex::encode(signed.bytes())));
}

#[tokio::test]
async fn canonical_service_calls_provider_exactly_once() {
    let provider = FixedProvider::alloy_vector();
    let signed = sign_eip1559(
        &alloy_vector_envelope(),
        signer_ref(),
        EXPECTED_SENDER,
        &provider,
    )
    .await
    .expect("signed");

    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(keccak256(signed.bytes()), signed.transaction_hash());
}

#[tokio::test]
async fn canonical_service_rejects_the_wrong_deterministic_provider_before_signing() {
    let provider = WrongProfileProvider(FixedProvider::alloy_vector());
    let error = sign_eip1559(
        &alloy_vector_envelope(),
        signer_ref(),
        EXPECTED_SENDER,
        &provider,
    )
    .await
    .expect_err("wrong deterministic provider profile");

    assert_eq!(error, EvmSigningError::DeterministicProfileMismatch);
    assert_eq!(provider.0.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn finalization_rejects_profile_mismatch_before_signature_use() {
    let envelope = alloy_vector_envelope();
    let request = envelope
        .signing_request(signer_ref(), EXPECTED_SENDER)
        .expect("request");
    let other_request = SigningRequest::from_digest(
        signer_ref(),
        request.algorithm().clone(),
        SigningProfileId::new("secp256k1.other.profile.v1").expect("profile"),
        request.domain().clone(),
        request.purpose().clone(),
        *request.digest(),
    )
    .require_public_identity(
        ExpectedSignerIdentity::account_id(format!("{EXPECTED_SENDER:?}")).expect("identity"),
    );
    let result = SigningResult::for_request(
        &other_request,
        identity(EXPECTED_SENDER, other_request.algorithm().clone()),
        SignatureBytes::new(vec![0xff]).expect("bounded signature"),
    )
    .expect("result");

    assert_eq!(
        envelope.finalize_signed(&request, EXPECTED_SENDER, &result),
        Err(EvmSigningError::SigningResultMismatch { field: "profile" })
    );
}

#[test]
fn finalization_rejects_noncanonical_parity_high_s_and_wrong_sender() {
    let envelope = alloy_vector_envelope();
    let request = envelope
        .signing_request(signer_ref(), EXPECTED_SENDER)
        .expect("request");

    let mut bad_parity = alloy_vector_signature().as_bytes().to_vec();
    bad_parity[64] = 0;
    let result = SigningResult::for_request(
        &request,
        identity(EXPECTED_SENDER, request.algorithm().clone()),
        SignatureBytes::new(bad_parity).expect("signature"),
    )
    .expect("result");
    assert_eq!(
        envelope.finalize_signed(&request, EXPECTED_SENDER, &result),
        Err(EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::InvalidParity
        })
    );

    let mut high_s = vec![0_u8; 65];
    high_s[31] = 1;
    high_s[32..64].copy_from_slice(
        &hex::decode("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364140")
            .expect("curve order minus one"),
    );
    high_s[64] = 27;
    let result = SigningResult::for_request(
        &request,
        identity(EXPECTED_SENDER, request.algorithm().clone()),
        SignatureBytes::new(high_s).expect("signature"),
    )
    .expect("result");
    assert_eq!(
        envelope.finalize_signed(&request, EXPECTED_SENDER, &result),
        Err(EvmSigningError::InvalidSignature {
            reason: EvmSignatureError::HighS
        })
    );

    let wrong_sender = address!("1111111111111111111111111111111111111111");
    let wrong_request = envelope
        .signing_request(signer_ref(), wrong_sender)
        .expect("request");
    let result = SigningResult::for_request(
        &wrong_request,
        identity(wrong_sender, wrong_request.algorithm().clone()),
        alloy_vector_signature(),
    )
    .expect("result");
    assert_eq!(
        envelope.finalize_signed(&wrong_request, wrong_sender, &result),
        Err(EvmSigningError::RecoveredAddressMismatch)
    );
}
