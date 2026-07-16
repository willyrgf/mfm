use super::*;
use mfm_evm_capabilities::{
    EvmBlockReadResponse, EvmCallReadResponse, EvmCapabilityFuture, EvmChainIdentityResponse,
    EvmCodeReadRequest, EvmCodeReadResponse, EvmFeeReadResponse, EvmGasEstimateResponse,
    EvmLogsReadResponse, EvmNonceReadResponse, EvmReceiptReadResponse, EvmSourcePolicyId,
    EvmSourceRef, RedactedEvmSourceEvidence,
};
use mfm_evm_contract_model::{
    BlockSelector, BlockTag, ConfiguredContractAnchor, ConfiguredContractInstance, ConfiguredFrom,
    ContractAddress, EvmBlockHash, EvmCodeHash, EvmContractContext, ValidationCodeIdentityEvidence,
    ValidationCodeIdentitySelector,
};
use mfm_evm_signing::{
    primitive_signature_from_bytes, recover_signing_address,
    EvmTransactionStyle as SigningTransactionStyle,
};
use mfm_ids::{ContextRef, DigestAlgorithm, DigestBytes};
use mfm_program::{SideEffectState, StateContext};
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SigningError, SigningRequest, SigningResult,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

const TEST_TRANSACTION_HASH: &str =
    "0x1111111111111111111111111111111111111111111111111111111111111111";
const MISMATCH_HASH: &str = TEST_TRANSACTION_HASH;

#[derive(Clone, Copy)]
pub(super) enum RecoveryReceiptMode {
    Landed,
    Pending,
    ProviderFailure,
}

#[derive(Clone, Copy)]
pub(super) enum RecoveryOccupancyMode {
    Unknown,
    Occupied { transaction_hash: B256 },
}

#[path = "transaction_support.rs"]
mod transaction_support;
use self::transaction_support::*;

fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn test_evm_provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(mfm_evm_capabilities::evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::TransportFailed,
    ))
}

fn evidence_for_source(
    network_id: &EvmNetworkId,
    expected_chain_id: u64,
) -> RedactedEvmSourceEvidence {
    RedactedEvmSourceEvidence {
        network_id: network_id.clone(),
        expected_chain_id,
        observed_chain_id: expected_chain_id,
        source_ref: EvmSourceRef::new("local").expect("source reference"),
        policy_id: EvmSourcePolicyId::new("test").expect("source policy"),
    }
}

fn test_evm_evidence() -> RedactedEvmSourceEvidence {
    evidence_for_source(&EvmNetworkId::new("ethereum-mainnet").expect("network"), 1)
}

fn artifact_json() -> serde_json::Value {
    json!({
        "abi": {
            "json_text": json!([
                {"type": "constructor", "inputs": []},
                {
                    "type": "function",
                    "name": "configure",
                    "inputs": [],
                    "outputs": [],
                    "stateMutability": "nonpayable"
                },
                {
                    "type": "function",
                    "name": "owner",
                    "inputs": [],
                    "outputs": [{"name": "", "type": "bool"}],
                    "stateMutability": "view"
                }
            ]).to_string()
        },
        "bytecode": {"json_text": json!({"object": "0x6000"}).to_string()}
    })
}

fn certified_contract_context(
    network_id: &str,
    expected_chain_id: u64,
) -> mfm_program::CertifiedContext<EvmContractContext> {
    let value: EvmContractContext = serde_json::from_value(json!({
        "lifecycle_key": "adapter-test-contract",
        "network": {
            "network_id": network_id,
            "expected_chain_id": expected_chain_id,
        },
        "contract_profile": {"profile_id": "adapter-test-profile"},
    }))
    .expect("contract context");
    let mfm_program::StateContextDescriptorSpec::Required(requirement) =
        <EvmContractContext as StateContext>::descriptor().expect("context descriptor")
    else {
        panic!("contract context must be certified");
    };
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&value).expect("context JSON"),
    )
    .expect("canonical context");
    let context_ref = mfm_program::CertifiedContextSpec::derive_context_ref(
        &requirement.context_descriptor_id,
        &requirement.schema_id,
        &requirement.semantic_type_id,
        &requirement.canonicalizer_identity,
        &canonical,
    )
    .expect("context ref");
    mfm_program::CertifiedContext::from_certified_spec(&mfm_program::CertifiedContextSpec {
        context_ref,
        context_descriptor_id: requirement.context_descriptor_id,
        schema_id: requirement.schema_id,
        semantic_type_id: requirement.semantic_type_id,
        canonicalizer_identity: requirement.canonicalizer_identity,
        canonical_context_digest: canonical.content_digest(),
        canonical_context_byte_len: canonical.as_bytes().len() as u64,
        canonical_context: canonical,
    })
    .expect("certified context")
}

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
                data: vec![0x60, 0x00],
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
                data: vec![0x60, 0x00],
            },
            expected_from,
        ),
    }
    .expect("signing request");
    let primitive = primitive_signature_from_bytes(&test_signature_bytes()).expect("signature");
    let recovered = recover_signing_address(request.signing_hash(), primitive).expect("recovered");
    format!("{recovered:?}")
}

fn prepared_invocation_fixture() -> PreparedContractInvocation {
    PreparedContractInvocation {
        prepared_version: 1,
        phase: ContractMutationPhase::Deploy,
        context_ref: mfm_values::ContextRefValue::from(ContextRef::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_with(0x44),
        )),
        evm_network_context_ref:
            "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
        resource_stage: ContractLifecycleStage::Deployed,
        network_id: "ethereum-mainnet".to_owned(),
        expected_chain_id: 1,
        signer_ref: "deployer".to_owned(),
        expected_signer_address: "0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e".to_owned(),
        transactions: vec![PreparedContractTransactionEvidence {
            index: 0,
            style: PreparedContractTransactionStyle::Eip1559,
            chain_id: 1,
            nonce: 7,
            to_address: None,
            value_wei: "0".to_owned(),
            gas_limit: 21_000,
            max_fee_per_gas: Some("11".to_owned()),
            max_priority_fee_per_gas: Some("3".to_owned()),
            gas_price: None,
            data_digest:
                "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            data_len: 2,
            signing_digest:
                "0x0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            expected_transaction_hash: TEST_TRANSACTION_HASH.to_owned(),
        }],
        poll_interval_ms: 1_000,
        max_receipt_polls: 10,
    }
}

#[test]
fn prepared_invocation_guard_rejects_malformed_public_contract() {
    let mut malformed = prepared_invocation_fixture();
    malformed.transactions[0].data_digest = "not-a-digest".to_owned();
    assert!(matches!(
        ensure_prepared_invocation_public(&malformed),
        Err(EvmContractAdapterError::InvalidPreparedInvocation)
    ));

    let mut unknown = serde_json::to_value(prepared_invocation_fixture()).expect("JSON");
    unknown
        .as_object_mut()
        .expect("object")
        .insert("secret".to_owned(), json!("never"));
    assert!(serde_json::from_value::<PreparedContractInvocation>(unknown).is_err());
}

#[test]
fn direct_state_adapter_identity_and_nonce_lane_are_stable() {
    let binding = mfm_state_evm_contracts::contract_states_adapter_kind().expect("adapter kind");
    assert!(binding.as_str().contains("mfm.evm.contract"));
    assert_eq!(
        account_nonce_resource_key(
            &ContractNonceResourceScope {
                evm_network_context_ref:
                    "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
            },
            "0x000000000000000000000000000000000000dead",
        )
        .expect("resource key")
        .as_str(),
        r#"{"account":"0x000000000000000000000000000000000000dead","evm_network_context_ref":"content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
    );
}

#[test]
fn event_block_selector_preserves_supported_tags_and_rejects_unsupported_ones() {
    assert_eq!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Earliest,
            }),
            false,
        )
        .expect("earliest selector"),
        EvmBlockSelector::Number(0)
    );
    assert!(matches!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Finalized,
            }),
            false,
        ),
        Err(EvmContractAdapterError::UnsupportedBlockTag)
    ));
}

#[tokio::test]
async fn direct_deploy_preparation_and_reconstruction_preserve_the_submission_anchor() {
    let (fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    assert_eq!(prepared.evidence().transactions.len(), 1);
    assert_eq!(
        prepared.evidence().transactions[0].style,
        PreparedContractTransactionStyle::Eip1559
    );
    assert_eq!(
        prepared.signing_requests()[0].style(),
        SigningTransactionStyle::Eip1559
    );
    let reconstructed = fixture
        .adapter()
        .reconstruct_context_deploy_invocation(
            &fixture.action,
            &fixture.context,
            &fixture.artifact,
            &fixture.intent,
            prepared.evidence(),
        )
        .expect("reconstructed invocation");
    assert_eq!(reconstructed.evidence(), prepared.evidence());
}

#[tokio::test]
async fn ambiguity_recovery_observes_rebroadcasts_or_blocks_without_reusing_a_nonce() {
    for (receipt_mode, expected_submit_count) in [
        (RecoveryReceiptMode::Landed, 0),
        (RecoveryReceiptMode::Pending, 1),
    ] {
        assert_recovery_observed(receipt_mode, expected_submit_count).await;
    }

    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let occupying_hash = "0x2222222222222222222222222222222222222222222222222222222222222222"
        .parse::<B256>()
        .expect("occupying hash");
    let (decision, submit_count) = recover_with_provider(
        RecoveryReceiptMode::Pending,
        8,
        RecoveryOccupancyMode::Occupied {
            transaction_hash: occupying_hash,
        },
        &prepared,
    )
    .await;
    assert!(matches!(
        decision,
        SideEffectSubmissionDecision::NotSubmitted(_)
    ));
    assert_submit_count(&submit_count, 0);

    assert_recovery_unknown(RecoveryReceiptMode::ProviderFailure, 7).await;
}

#[tokio::test]
async fn resumed_submission_rejects_a_resigned_hash_mismatch_before_broadcast() {
    let (fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let reconstructed = fixture.reconstruct_with_hash(&prepared, MISMATCH_HASH);
    assert!(matches!(
        fixture.adapter().submit_prepared(&reconstructed).await,
        Err(EvmContractAdapterError::TransactionHashMismatch)
    ));
}

#[test]
fn evidence_only_validation_replay_rejects_tampered_result_evidence() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let result = ValidationReadResult {
        function: "owner".to_owned(),
        args: Vec::new(),
        expected: ExpectedValue::from_json_value(&json!(true)).expect("expected"),
        actual: ExpectedValue::from_json_value(&json!(true)).expect("actual"),
        passed: true,
    };
    let evidence = ValidationReadEvidence {
        source: ValidationSourceEvidence {
            network_id: "ethereum-mainnet".to_owned(),
            expected_chain_id: 1,
            observed_chain_id: 1,
            source_ref: "local".to_owned(),
            policy_id: "test".to_owned(),
        },
        result: result.clone(),
    };
    verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&result),
        std::slice::from_ref(&evidence),
        &[],
        &[],
    )
    .expect("evidence-only replay");

    let mut tampered = result;
    tampered.actual = ExpectedValue::from_json_value(&json!(false)).expect("actual");
    assert!(verify_validation_source_evidence(&context, &[tampered], &[], &[], &[]).is_err());
}

#[test]
fn code_identity_replay_recomputes_retained_bytecode_metadata() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let context_ref = mfm_values::ContextRefValue::from(context.context_ref().clone());
    let address =
        ContractAddress::new("0x1111111111111111111111111111111111111111").expect("address");
    let block_hash = EvmBlockHash::new(format!("0x{}", "42".repeat(32))).expect("block hash");
    let configured = ConfiguredContractInstance {
        lifecycle_version: 1,
        context_ref: context_ref.clone(),
        address: address.clone(),
        configured_from: ConfiguredFrom {
            deployed_context_ref: context_ref,
            deployed_address: address.clone(),
        },
        anchor: ConfiguredContractAnchor {
            block_number: 42,
            block_hash: block_hash.clone(),
        },
    };
    let runtime_bytecode = vec![0x60, 0x00];
    let observed_code_hash = EvmCodeHash::new(format!(
        "{:?}",
        alloy_primitives::keccak256(&runtime_bytecode)
    ))
    .expect("code hash");
    let evidence = ValidationCodeIdentityEvidence {
        evidence_version: 1,
        selector: ValidationCodeIdentitySelector {
            address,
            block_number: 42,
            block_hash,
            require_canonical: true,
        },
        source: ValidationSourceEvidence {
            network_id: "ethereum-mainnet".to_owned(),
            expected_chain_id: 1,
            observed_chain_id: 1,
            source_ref: "local".to_owned(),
            policy_id: "test".to_owned(),
        },
        observed_byte_len: runtime_bytecode.len() as u64,
        observed_code_hash,
        runtime_bytecode,
    };
    super::replay_adapter::verify_replayed_code_identity_evidence(&context, &configured, &evidence)
        .expect("untampered code identity evidence");

    let mut tampered_bytes = evidence.clone();
    tampered_bytes.runtime_bytecode.push(0x01);
    assert!(
        super::replay_adapter::verify_replayed_code_identity_evidence(
            &context,
            &configured,
            &tampered_bytes,
        )
        .is_err()
    );

    let mut tampered_length = evidence.clone();
    tampered_length.observed_byte_len += 1;
    assert!(
        super::replay_adapter::verify_replayed_code_identity_evidence(
            &context,
            &configured,
            &tampered_length,
        )
        .is_err()
    );

    let mut tampered_hash = evidence;
    tampered_hash.observed_code_hash =
        EvmCodeHash::new(format!("0x{}", "00".repeat(32))).expect("code hash");
    assert!(
        super::replay_adapter::verify_replayed_code_identity_evidence(
            &context,
            &configured,
            &tampered_hash,
        )
        .is_err()
    );
}

#[test]
fn adapter_source_has_no_app_or_operation_coupling() {
    let source = include_str!("lib.rs");
    for forbidden in ["mfm_app", "bin/cli", "bin/rest-api"] {
        assert!(
            !source.contains(forbidden),
            "adapter source contains {forbidden}"
        );
    }
    assert!(ContractAddress::new("0x000000000000000000000000000000000000beef").is_ok());
    assert_eq!(expected_test_signer_address("eip1559").len(), 42);
}
