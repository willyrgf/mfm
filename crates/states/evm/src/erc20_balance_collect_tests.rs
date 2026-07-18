use super::*;
use alloy_primitives::{Address, Bytes, B256, U256};
use mfm_evm_capabilities::{EvmBlock, EvmNetworkBinding, EvmSessionEvidence};
use mfm_ids::LocalPublicId;
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_portfolio_model::ids::NormalizedEvmAddress;
use mfm_program::{MfmFactType, StateSpec};
use mfm_values::NonEmpty;

const HASH_A: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const HASH_B: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const TOKEN_A: &str = "0x0000000000000000000000000000000000000001";
const TOKEN_B: &str = "0x0000000000000000000000000000000000000002";
const ACCT_A: &str = "0x0000000000000000000000000000000000000011";
const ACCT_B: &str = "0x0000000000000000000000000000000000000022";

fn semantic_evm_network(network: &str, chain_id: u64) -> NetworkConfig {
    NetworkConfig::new(
        network.to_owned(),
        NetworkFamilyConfig::Evm,
        Some(chain_id),
        Some(18),
        None,
        None,
        std::collections::BTreeMap::new(),
    )
    .expect("EVM network")
}

fn normalized(value: &str) -> NormalizedEvmAddress {
    NormalizedEvmAddress::new(value, "test_address").expect("normalized address")
}

fn metadata_config() -> ObserveErc20TokenMetadataConfig {
    ObserveErc20TokenMetadataConfig {
        network: semantic_evm_network("ethereum-mainnet", 1),
        contract_address: normalized(TOKEN_A),
        max_source_reads: NonZeroU64::new(EVM_ERC20_METADATA_OBSERVE_SOURCE_READS)
            .expect("non-zero source reads"),
    }
}

fn balance_config(account: &str) -> ObserveErc20BalanceConfig {
    ObserveErc20BalanceConfig {
        network: semantic_evm_network("ethereum-mainnet", 1),
        contract_address: normalized(TOKEN_A),
        account: normalized(account),
        max_source_reads: NonZeroU64::new(EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS)
            .expect("non-zero source reads"),
    }
}

fn evidence() -> EvmSessionEvidence {
    evidence_for("primary", "default")
}

fn evidence_for(source_ref: &str, implementation_id: &str) -> EvmSessionEvidence {
    evidence_for_binding("ethereum-mainnet", 1, source_ref, implementation_id)
}

fn evidence_for_binding(
    network: &str,
    chain_id: u64,
    source_ref: &str,
    implementation_id: &str,
) -> EvmSessionEvidence {
    let binding = EvmNetworkBinding::new(LocalPublicId::new(network).expect("net"), chain_id)
        .expect("binding");
    EvmSessionEvidence::new(
        &binding,
        LocalPublicId::new(source_ref).expect("source"),
        LocalPublicId::new(implementation_id).expect("implementation"),
    )
}

fn source_binding() -> RedactedEvmSessionEvidence {
    RedactedEvmSessionEvidence::from_session(&evidence()).expect("source binding")
}

fn source_binding_for(source_ref: &str) -> RedactedEvmSessionEvidence {
    RedactedEvmSessionEvidence::from_session(&evidence_for(source_ref, "default"))
        .expect("source binding")
}

fn block_response(number: u64, hash: &str) -> EvmBlock {
    EvmBlock {
        number: U256::from(number),
        hash: B256::from_str(hash.strip_prefix("0x").expect("prefix")).expect("hash"),
    }
}

fn call_response(return_data: impl Into<Vec<u8>>) -> Bytes {
    return_data.into().into()
}

fn tip() -> EvmJointTip {
    EvmJointTip::new("ethereum-mainnet", 1, 100, HASH_A, source_binding()).expect("tip")
}

fn metadata(decimals: u8) -> EvmErc20TokenMetadata {
    let config = metadata_config();
    let tip = tip();
    let request = erc20_metadata_call_request(&config, &tip).expect("request");
    let mut result = [0u8; 32];
    result[31] = decimals;
    normalize_erc20_token_metadata_from_capability(
        &config,
        &tip,
        &request,
        &call_response(result),
        &block_response(100, HASH_A),
        &evidence(),
    )
    .expect("metadata")
}

#[test]
fn metadata_request_is_exact_hash_selected_decimals_call() {
    let config = metadata_config();
    let tip = tip();
    let request = erc20_metadata_call_request(&config, &tip).expect("request");

    assert_eq!(format!("{:#x}", request.to()), TOKEN_A);
    assert_eq!(request.input().as_ref(), &[0x31, 0x3c, 0xe5, 0x67]);
    assert!(matches!(
        request.block(),
        EvmBlockSelector::ExactHash(hash) if format!("{hash:#x}") == HASH_A
    ));

    for decimals in [0, u8::MAX] {
        let mut result = [0u8; 32];
        result[31] = decimals;
        let output = normalize_erc20_token_metadata_from_capability(
            &config,
            &tip,
            &request,
            &call_response(result),
            &block_response(100, HASH_A),
            &evidence(),
        )
        .expect("metadata output");
        assert_eq!(output.decimals(), decimals);
        assert_eq!(output.network(), "ethereum-mainnet");
        assert_eq!(output.chain_id(), 1);
        assert_eq!(output.contract_address(), TOKEN_A);
        assert_eq!(output.block_number(), 100);
        assert_eq!(output.block_hash(), HASH_A);
        assert_eq!(
            output.source_read_count(),
            EVM_ERC20_METADATA_OBSERVE_SOURCE_READS
        );
    }
}

#[test]
fn metadata_rejects_malformed_words_and_anchor_drift() {
    let config = metadata_config();
    let tip = tip();
    let request = erc20_metadata_call_request(&config, &tip).expect("request");

    let mut high_padding = [0u8; 32];
    high_padding[0] = 1;
    let malformed = vec![Vec::new(), vec![0; 31], vec![0; 33], high_padding.to_vec()];
    for bytes in malformed {
        assert!(
            normalize_erc20_token_metadata_from_capability(
                &config,
                &tip,
                &request,
                &call_response(bytes),
                &block_response(100, HASH_A),
                &evidence(),
            )
            .is_err(),
            "malformed decimals result must fail"
        );
    }

    let mut valid = [0u8; 32];
    valid[31] = 18;
    assert!(normalize_erc20_token_metadata_from_capability(
        &config,
        &tip,
        &request,
        &call_response(valid),
        &block_response(101, HASH_A),
        &evidence(),
    )
    .is_err());
    assert!(normalize_erc20_token_metadata_from_capability(
        &config,
        &tip,
        &request,
        &call_response(valid),
        &block_response(100, HASH_B),
        &evidence(),
    )
    .is_err());
}

#[test]
fn erc20_reads_and_metadata_reject_substituted_provider_source() {
    let metadata_config = metadata_config();
    let tip = tip();
    let metadata_request = erc20_metadata_call_request(&metadata_config, &tip).expect("request");
    let mut decimals = [0u8; 32];
    decimals[31] = 18;
    for substituted_evidence in [
        evidence_for("secondary", "default"),
        evidence_for("primary", "secondary"),
    ] {
        assert!(normalize_erc20_token_metadata_from_capability(
            &metadata_config,
            &tip,
            &metadata_request,
            &call_response(decimals),
            &block_response(100, HASH_A),
            &substituted_evidence,
        )
        .is_err());
    }
    assert!(normalize_erc20_token_metadata_from_capability(
        &metadata_config,
        &tip,
        &metadata_request,
        &call_response(decimals),
        &block_response(100, HASH_A),
        &evidence_for("secondary", "default"),
    )
    .is_err());

    let metadata = metadata(18);
    assert_eq!(metadata.source_binding(), tip.source_binding());
    let balance_config = balance_config(ACCT_A);
    let input = ObserveErc20BalanceInput {
        joint_tip: tip.clone(),
        metadata,
    };
    let balance_request = erc20_balance_call_request(&balance_config, &input).expect("request");
    assert!(normalize_erc20_balance_from_capability(
        &balance_config,
        &input,
        &balance_request,
        &call_response([0u8; 32]),
        &block_response(100, HASH_A),
        &evidence_for("secondary", "default"),
    )
    .is_err());
    assert!(normalize_erc20_balance_from_capability(
        &balance_config,
        &input,
        &balance_request,
        &call_response([0u8; 32]),
        &block_response(100, HASH_A),
        &evidence_for("secondary", "default"),
    )
    .is_err());

    let substituted_metadata = EvmErc20TokenMetadata::new(
        "ethereum-mainnet",
        1,
        TOKEN_A,
        18,
        100,
        HASH_A,
        source_binding_for("secondary"),
    )
    .expect("metadata");
    let substituted_input = ObserveErc20BalanceInput {
        joint_tip: tip,
        metadata: substituted_metadata,
    };
    assert!(erc20_balance_call_request(&balance_config, &substituted_input).is_err());
}

#[test]
fn balance_request_has_exact_padding_and_binding() {
    let config = balance_config(ACCT_A);
    let input = ObserveErc20BalanceInput {
        joint_tip: tip(),
        metadata: metadata(18),
    };
    let request = erc20_balance_call_request(&config, &input).expect("request");
    assert_eq!(format!("{:#x}", request.to()), TOKEN_A);
    assert_eq!(
        request.input().as_ref(),
        &[
            0x70, 0xa0, 0x82, 0x31, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11,
        ]
    );
    assert!(matches!(
        request.block(),
        EvmBlockSelector::ExactHash(hash) if format!("{hash:#x}") == HASH_A
    ));

    let wrong_token = EvmCall::new(
        request.from(),
        Address::from_str(TOKEN_B).expect("token"),
        request.value(),
        request.input().clone(),
        request.gas_limit(),
        request.access_list().clone(),
        request.block().clone(),
    )
    .expect("wrong-token call");
    let mut zero = [0u8; 32];
    assert!(normalize_erc20_balance_from_capability(
        &config,
        &input,
        &wrong_token,
        &call_response(zero),
        &block_response(100, HASH_A),
        &evidence(),
    )
    .is_err());

    let wrong_account_calldata =
        mfm_evm_core::hex::hex_to_bytes(&mfm_evm_core::encoding::encode_erc20_balance_of(
            &Address::from_str(ACCT_B).expect("account"),
        ))
        .expect("calldata");
    let wrong_account = EvmCall::new(
        request.from(),
        request.to(),
        request.value(),
        wrong_account_calldata.into(),
        request.gas_limit(),
        request.access_list().clone(),
        request.block().clone(),
    )
    .expect("wrong-account call");
    zero[31] = 1;
    assert!(normalize_erc20_balance_from_capability(
        &config,
        &input,
        &wrong_account,
        &call_response(zero),
        &block_response(100, HASH_A),
        &evidence(),
    )
    .is_err());
}

#[test]
fn balance_rejects_network_chain_and_metadata_tip_mismatches() {
    let input = ObserveErc20BalanceInput {
        joint_tip: tip(),
        metadata: metadata(18),
    };
    let config = balance_config(ACCT_A);
    let request = erc20_balance_call_request(&config, &input).expect("request");
    let zero = [0u8; 32];

    let mut wrong_network = balance_config(ACCT_A);
    wrong_network.network = semantic_evm_network("other-network", 1);
    assert!(erc20_balance_call_request(&wrong_network, &input).is_err());

    let mut wrong_chain = balance_config(ACCT_A);
    wrong_chain.network = semantic_evm_network("ethereum-mainnet", 2);
    assert!(erc20_balance_call_request(&wrong_chain, &input).is_err());

    let wrong_metadata = EvmErc20TokenMetadata::new(
        "ethereum-mainnet",
        1,
        TOKEN_A,
        18,
        99,
        HASH_A,
        source_binding(),
    )
    .expect("metadata");
    let wrong_tip_input = ObserveErc20BalanceInput {
        joint_tip: tip(),
        metadata: wrong_metadata,
    };
    assert!(erc20_balance_call_request(&config, &wrong_tip_input).is_err());

    let wrong_source = call_response(zero);
    assert!(normalize_erc20_balance_from_capability(
        &config,
        &input,
        &request,
        &wrong_source,
        &block_response(100, HASH_A),
        &evidence_for_binding("ethereum-mainnet", 2, "primary", "default"),
    )
    .is_err());
}

#[test]
fn balance_preserves_zero_and_maximum_uint256_with_closed_fact_semantics() {
    let config = balance_config(ACCT_A);
    let input = ObserveErc20BalanceInput {
        joint_tip: tip(),
        metadata: metadata(255),
    };
    let request = erc20_balance_call_request(&config, &input).expect("request");

    let zero_observation = normalize_erc20_balance_from_capability(
        &config,
        &input,
        &request,
        &call_response([0u8; 32]),
        &block_response(100, HASH_A),
        &evidence(),
    )
    .expect("zero is a successful observation");
    assert_eq!(zero_observation.response().raw_units(), "0");
    assert_eq!(zero_observation.response().coverage(), "complete_at_anchor");
    assert_eq!(zero_observation.response().source_status(), "ok");
    assert_eq!(
        zero_observation.source_read_count(),
        EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS
    );
    let fact = zero_observation.try_to_fact().expect("zero fact");
    assert_eq!(fact.response().raw_units(), "0");
    assert_eq!(
        fact.response().coverage_status().expect("coverage"),
        CoverageStatus::CompleteAtAnchor
    );
    assert_eq!(
        fact.response().holding_source_status().expect("status"),
        HoldingSourceStatus::Ok
    );

    let max_observation = normalize_erc20_balance_from_capability(
        &config,
        &input,
        &request,
        &call_response([u8::MAX; 32]),
        &block_response(100, HASH_A),
        &evidence(),
    )
    .expect("maximum uint256");
    assert_eq!(
        max_observation.response().raw_units(),
        U256::MAX.to_string()
    );
}

#[test]
fn balance_rejects_malformed_return_words() {
    let config = balance_config(ACCT_A);
    let input = ObserveErc20BalanceInput {
        joint_tip: tip(),
        metadata: metadata(18),
    };
    let request = erc20_balance_call_request(&config, &input).expect("request");
    for malformed in [Vec::new(), vec![0; 31], vec![0; 33]] {
        assert!(normalize_erc20_balance_from_capability(
            &config,
            &input,
            &request,
            &call_response(malformed),
            &block_response(100, HASH_A),
            &evidence(),
        )
        .is_err());
    }
}

#[test]
fn record_state_advertises_erc20_fact_and_fact_shape_is_source_near() {
    let descriptors = RecordErc20BalanceFactState::emitted_fact_descriptors().expect("descriptors");
    assert_eq!(descriptors.len(), 1);
    let descriptor = EvmAddressErc20BalanceSnapshotFact::descriptor().expect("descriptor");
    assert_eq!(
        descriptor.fact_kind().as_str(),
        "evm.address_erc20_balance_snapshot"
    );

    let subject = EvmAddressErc20BalanceSubject::new("ethereum-mainnet", 1, TOKEN_A, ACCT_A)
        .expect("subject");
    let fact =
        EvmAddressErc20BalanceSnapshotFact::try_new(subject, 100, HASH_A, "0", 18).expect("fact");
    let value = serde_json::to_value(&fact).expect("json");
    let subject_keys = value["subject"]
        .as_object()
        .expect("subject object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let response_keys = value["response"]
        .as_object()
        .expect("response object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        subject_keys,
        std::collections::BTreeSet::from(["account", "chain_id", "contract_address", "network",])
    );
    assert_eq!(
        response_keys,
        std::collections::BTreeSet::from([
            "block_hash",
            "block_number",
            "coverage",
            "decimals",
            "raw_units",
            "source_status",
        ])
    );
    let text = serde_json::to_string(&value).expect("text");
    for forbidden in [
        "wallet_id",
        "symbol_id",
        "valuation",
        "rpc_url",
        "password",
        "http://",
    ] {
        assert!(!text.contains(forbidden), "forbidden field {forbidden}");
    }
}

#[test]
fn configs_require_exact_state_owned_read_budgets() {
    let mut metadata = metadata_config();
    metadata.max_source_reads = NonZeroU64::new(1).expect("non-zero");
    assert!(validate_observe_erc20_token_metadata_config(&metadata).is_err());

    let mut balance = balance_config(ACCT_A);
    balance.max_source_reads = NonZeroU64::new(1).expect("non-zero");
    assert!(validate_observe_erc20_balance_config(&balance).is_err());
}

#[test]
fn erc20_receipt_preserves_zero_and_rejects_tampered_material() {
    let subject = EvmAddressErc20BalanceSubject::new("ethereum-mainnet", 1, TOKEN_A, ACCT_A)
        .expect("subject");
    let fact = EvmAddressErc20BalanceSnapshotFact::try_new(subject, 100, HASH_A, "0", 18)
        .expect("zero fact");
    let receipt =
        assemble_evm_erc20_balance_batch_receipt(AssembleEvmErc20BalanceBatchReceiptInput {
            joint_tip: tip(),
            balance_facts: NonEmpty::try_from_vec(vec![fact.clone()]).expect("fact"),
        })
        .expect("receipt");
    assert_eq!(receipt.entries().len(), 1);
    assert_eq!(receipt.source_binding(), tip().source_binding());
    assert_eq!(receipt.entries()[0].coverage(), EVM_ERC20_BALANCE_COVERAGE);
    assert_eq!(
        receipt.entries()[0].source_status(),
        EVM_ERC20_BALANCE_SOURCE_STATUS
    );

    let mut tampered_identity = serde_json::to_value(&receipt).expect("receipt JSON");
    tampered_identity["entries"][0]["fact_content_identity"]["response_hash"] = serde_json::json!(
        "sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert!(serde_json::from_value::<EvmErc20BalanceBatchReceipt>(tampered_identity).is_err());

    let mut tampered_fact = serde_json::to_value(&receipt).expect("receipt JSON");
    tampered_fact["entries"][0]["verified_fact"]["response"]["raw_units"] = serde_json::json!("1");
    assert!(serde_json::from_value::<EvmErc20BalanceBatchReceipt>(tampered_fact).is_err());

    assert!(
        assemble_evm_erc20_balance_batch_receipt(AssembleEvmErc20BalanceBatchReceiptInput {
            joint_tip: tip(),
            balance_facts: NonEmpty::try_from_vec(vec![fact.clone(), fact]).expect("facts"),
        },)
        .is_err()
    );
}
