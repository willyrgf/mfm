use super::*;
use alloy_primitives::{Address, B256, U256};
use mfm_evm_capabilities::{
    EvmNetworkBinding, EvmNetworkId, EvmSourcePolicyId, EvmSourceRef, RedactedEvmSourceEvidence,
};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_portfolio_model::ids::NormalizedEvmAddress;
use mfm_program::{MfmFactType, StateSpec};

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

fn evidence() -> RedactedEvmSourceEvidence {
    let binding = EvmNetworkBinding::new(EvmNetworkId::new("ethereum-mainnet").expect("net"), 1)
        .expect("binding");
    RedactedEvmSourceEvidence::from_binding(
        &binding,
        1,
        EvmSourceRef::new("primary").expect("source"),
        EvmSourcePolicyId::new("default").expect("policy"),
    )
    .expect("evidence")
}

fn block_response(number: u64, hash: &str) -> EvmBlockReadResponse {
    EvmBlockReadResponse {
        evidence: evidence(),
        block_number: number,
        block_hash: B256::from_str(hash.strip_prefix("0x").expect("prefix")).expect("hash"),
    }
}

fn call_response(return_data: impl Into<Vec<u8>>) -> EvmCallReadResponse {
    EvmCallReadResponse {
        evidence: evidence(),
        return_data: return_data.into(),
    }
}

fn tip() -> EvmJointTip {
    EvmJointTip::new("ethereum-mainnet", 1, 100, HASH_A).expect("tip")
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
    )
    .expect("metadata")
}

#[test]
fn metadata_request_is_exact_hash_selected_decimals_call() {
    let config = metadata_config();
    let tip = tip();
    let request = erc20_metadata_call_request(&config, &tip).expect("request");

    assert_eq!(format!("{:#x}", request.to()), TOKEN_A);
    assert_eq!(request.calldata(), &[0x31, 0x3c, 0xe5, 0x67]);
    assert!(matches!(
        request.block(),
        EvmBlockSelector::Hash(hash) if format!("{hash:#x}") == HASH_A
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
    )
    .is_err());
    assert!(normalize_erc20_token_metadata_from_capability(
        &config,
        &tip,
        &request,
        &call_response(valid),
        &block_response(100, HASH_B),
    )
    .is_err());
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
        request.calldata(),
        &[
            0x70, 0xa0, 0x82, 0x31, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11,
        ]
    );
    assert!(matches!(
        request.block(),
        EvmBlockSelector::Hash(hash) if format!("{hash:#x}") == HASH_A
    ));

    let wrong_token = EvmCallReadRequest::new(
        Address::from_str(TOKEN_B).expect("token"),
        request.calldata().to_vec(),
        request.block().clone(),
    );
    let mut zero = [0u8; 32];
    assert!(normalize_erc20_balance_from_capability(
        &config,
        &input,
        &wrong_token,
        &call_response(zero),
        &block_response(100, HASH_A),
    )
    .is_err());

    let wrong_account_calldata =
        mfm_evm_core::hex::hex_to_bytes(&mfm_evm_core::encoding::encode_erc20_balance_of(
            &Address::from_str(ACCT_B).expect("account"),
        ))
        .expect("calldata");
    let wrong_account = EvmCallReadRequest::new(
        request.to(),
        wrong_account_calldata,
        request.block().clone(),
    );
    zero[31] = 1;
    assert!(normalize_erc20_balance_from_capability(
        &config,
        &input,
        &wrong_account,
        &call_response(zero),
        &block_response(100, HASH_A),
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

    let wrong_metadata = EvmErc20TokenMetadata::new("ethereum-mainnet", 1, TOKEN_A, 18, 99, HASH_A)
        .expect("metadata");
    let wrong_tip_input = ObserveErc20BalanceInput {
        joint_tip: tip(),
        metadata: wrong_metadata,
    };
    assert!(erc20_balance_call_request(&config, &wrong_tip_input).is_err());

    let mut wrong_source = call_response(zero);
    wrong_source.evidence.observed_chain_id = 2;
    assert!(normalize_erc20_balance_from_capability(
        &config,
        &input,
        &request,
        &wrong_source,
        &block_response(100, HASH_A),
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
