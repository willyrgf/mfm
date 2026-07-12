use super::*;
use mfm_capabilities::CapabilitySpec;
use mfm_evm_capabilities::{
    EvmBlockReadResponse, EvmBlockSelector, EvmCallReadResponse, EvmCapabilityFuture,
    EvmChainIdentityResponse, EvmCodeReadResponse, EvmFeeReadResponse, EvmGasEstimateResponse,
    EvmLogsReadResponse, EvmNonceReadResponse, EvmReceiptReadResponse, EvmSourcePolicyId,
    EvmSourceRef, RedactedEvmSourceEvidence,
};
use mfm_evm_contract_model::{
    BlockSelector, BlockTag, ContractAddress, DeployProvenance, ImportFromMfmRun,
};
use mfm_evm_signing::{
    primitive_signature_from_bytes, recover_signing_address,
    EvmTransactionStyle as SigningTransactionStyle,
};
use mfm_ids::{
    ArtifactId, AttemptId, CellId, ContextRef, DescriptorId, DigestAlgorithm, DigestBytes, EventId,
    NodeId, RunId, SchemaId, ScopeId, SideEffectPairId, SpecHash, StoreScopeId,
};
use mfm_program::StateContext;
use mfm_replay::v1::SideEffectReplayVerifier;
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SigningError, SigningRequest, SigningResult,
};
use mfm_state_evm_contracts::ContextContractTransactionIntent;
use serde_json::json;
use std::sync::{Arc, Mutex};

const TEST_TRANSACTION_HASH: &str =
    "0x1111111111111111111111111111111111111111111111111111111111111111";
const MISMATCH_HASH: &str = TEST_TRANSACTION_HASH;

#[path = "replay_tests.rs"]
mod replay_tests;
use self::replay_tests::{RecoveryOccupancyMode, RecoveryReceiptMode};

#[path = "tests/behavior.rs"]
mod behavior;

#[path = "source_run_support.rs"]
mod source_run_support;
use self::source_run_support::*;
#[path = "transaction_support.rs"]
mod transaction_support;
use self::transaction_support::*;

fn test_signature_bytes() -> SignatureBytes {
    SignatureBytes::new(
        hex_to_bytes(
            "0x48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353\
             efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b",
        )
        .expect("signature hex"),
    )
    .expect("signature bytes")
}

fn test_signing_result(request: &SigningRequest) -> mfm_signing::Result<SigningResult> {
    let signature = test_signature_bytes();
    let primitive = primitive_signature_from_bytes(&signature)
        .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
    let recovered = recover_signing_address(B256::from(*request.digest().as_bytes()), primitive)
        .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
    let identity = PublicSigningIdentity::new(
        request.algorithm().clone(),
        None,
        Some(format!("{recovered:?}")),
    )?;
    SigningResult::for_request(request, identity, signature)
}

fn expected_test_signer_address(style: &str) -> String {
    expected_test_signer_address_for_chain(style, 1)
}

fn expected_test_signer_address_for_chain(style: &str, chain_id: u64) -> String {
    let signer_ref = SignerRef::new("deployer").expect("signer ref");
    let expected_from = Address::from([0_u8; 20]);
    let data = vec![0x60, 0x00];
    let request = match style {
        "legacy" => EvmSigningRequest::legacy(
            signer_ref,
            LegacyTxToSign {
                to: None,
                value_wei: 0,
                chain_id,
                nonce: 7,
                gas_price_wei: 7,
                gas_limit: 21_000,
                data,
            },
            expected_from,
        ),
        _ => EvmSigningRequest::eip1559(
            signer_ref,
            Eip1559TxToSign {
                to: None,
                value_wei: 0,
                chain_id,
                nonce: 7,
                max_fee_per_gas: 11,
                max_priority_fee_per_gas: 3,
                gas_limit: 21_000,
                data,
            },
            expected_from,
        ),
    }
    .expect("signing request");
    let primitive = primitive_signature_from_bytes(&test_signature_bytes()).expect("signature");
    let recovered =
        recover_signing_address(request.signing_hash(), primitive).expect("recovered address");
    format!("{recovered:?}")
}
