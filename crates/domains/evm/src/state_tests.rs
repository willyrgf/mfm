use super::*;

use alloy_primitives::{B256, U256};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId};
use mfm_program::mfm_value_contract;
use mfm_values::{MfmValue, RetainedValueContract};

fn generation(byte: u8) -> crate::EvmRoutingGenerationRef {
    let schema = SchemaId::new(
        "mfm.test.evm-routing",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(b"test routing schema"),
    )
    .expect("schema");
    let digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        mfm_canonical::sha256_digest_bytes(&[byte]),
    );
    crate::EvmRoutingGenerationRef::from_content_ref(
        ContentRef::new(schema, digest).expect("content ref"),
    )
    .expect("generation")
}

fn binding() -> EvmNetworkBinding {
    EvmNetworkBinding::new("ethereum-mainnet", 1, generation(1)).expect("binding")
}

fn anchored(hash_byte: u8) -> EvmAnchoredSource {
    EvmAnchoredSource::new(
        EvmCheckedSource::new(binding(), "primary", "json-rpc-v1").expect("source"),
        EvmBlockAnchor::new(U256::from(19_000_000), B256::from([hash_byte; 32])),
    )
    .expect("anchor")
}

fn account(byte: u8) -> Address {
    Address::from([byte; 20])
}

#[test]
fn every_read_state_selects_exactly_one_protocol_operation() {
    assert_eq!(
        read_operation_id(bootstrap_execution(
            test_binding_ref(),
            contracts::<EvmChainIdentityRequest, EvmChainIdentityResponse>().0,
            contracts::<EvmChainIdentityRequest, EvmChainIdentityResponse>().1,
            safe_failure_contract(),
        )),
        "eth_chain_id"
    );
    assert_eq!(
        read_operation_id(initial_anchor_execution(
            test_binding_ref(),
            contracts::<EvmLatestAnchorRequest, EvmBlockResponse>().0,
            contracts::<EvmLatestAnchorRequest, EvmBlockResponse>().1,
            safe_failure_contract(),
        )),
        "eth_get_block_by_number_latest"
    );

    let ids = [
        read_operation_id(token_decimals_execution(
            test_binding_ref(),
            contracts::<EvmTokenDecimalsRequest, EvmTokenDecimalsResponse>().0,
            contracts::<EvmTokenDecimalsRequest, EvmTokenDecimalsResponse>().1,
            safe_failure_contract(),
        )),
        read_operation_id(native_balance_execution(
            test_binding_ref(),
            contracts::<EvmNativeBalanceRequest, EvmQuantityResponse>().0,
            contracts::<EvmNativeBalanceRequest, EvmQuantityResponse>().1,
            safe_failure_contract(),
        )),
        read_operation_id(token_balance_execution(
            test_binding_ref(),
            contracts::<EvmTokenBalanceRequest, EvmQuantityResponse>().0,
            contracts::<EvmTokenBalanceRequest, EvmQuantityResponse>().1,
            safe_failure_contract(),
        )),
        read_operation_id(confirmation_execution(
            test_binding_ref(),
            contracts::<EvmAnchorConfirmationRequest, EvmBlockResponse>().0,
            contracts::<EvmAnchorConfirmationRequest, EvmBlockResponse>().1,
            safe_failure_contract(),
        )),
    ];
    assert_eq!(
        ids,
        [
            "eth_call_erc20_decimals",
            "eth_get_balance",
            "eth_call_erc20_balance_of",
            "eth_get_block_by_number_confirm",
        ]
    );
    assert!(matches!(aggregate_execution(), StateExecution::Pure(_)));
}

fn read_operation_id<S: State>(execution: mfm_program::Result<StateExecution<S>>) -> String {
    let StateExecution::Read(read) = execution.expect("execution contract") else {
        panic!("expected audited read");
    };
    assert_eq!(
        read.operation().operation_contract_ref(),
        &evm_read_capability_contract_ref().expect("capability contract")
    );
    read.operation().operation_id().as_str().to_owned()
}

fn contracts<Request, Returned>() -> (RetainedValueContract, RetainedValueContract)
where
    Request: MfmValue,
    Returned: MfmValue,
{
    (
        mfm_value_contract::<Request>(
            StableId::new("mfm.test.request").expect("request role"),
            test_evidence_ref(),
        )
        .expect("request contract"),
        mfm_value_contract::<Returned>(
            StableId::new("mfm.test.returned").expect("returned role"),
            test_evidence_ref(),
        )
        .expect("returned contract"),
    )
}

fn safe_failure_contract() -> RetainedValueContract {
    mfm_value_contract::<EvmSafeDiagnostic>(
        StableId::new("mfm.test.safe-failure").expect("failure role"),
        test_evidence_ref(),
    )
    .expect("safe failure contract")
}

fn test_binding_ref() -> ContentRef {
    generation(0xf0)
        .to_content_ref()
        .expect("binding content reference")
}

fn test_evidence_ref() -> ContentRef {
    generation(0xe0)
        .to_content_ref()
        .expect("evidence content reference")
}

#[test]
fn safe_failure_matrix_is_exact_and_phase_checked() {
    #[derive(Clone, Copy)]
    enum Expected {
        Invalid,
        Insufficient,
        DestinationRejected,
    }

    fn assert_verdict(
        stable_code: &str,
        diagnostic: Option<EvmSafeDiagnostic>,
        entered: bool,
        expected: Expected,
    ) {
        let verdict = failure_verdict_projection::<BootstrapEvmSourceState>(
            stable_code,
            entered,
            diagnostic.as_ref(),
        );
        let matches = match expected {
            Expected::Invalid => matches!(verdict, EvidenceVerdict::InvalidEvidence),
            Expected::Insufficient => matches!(verdict, EvidenceVerdict::InsufficientEvidence),
            Expected::DestinationRejected => matches!(
                verdict,
                EvidenceVerdict::Settlement(Settlement::Failed {
                    failure: EvmReadFailure::DestinationRejected
                })
            ),
        };
        assert!(
            matches,
            "unexpected verdict for {stable_code}, {diagnostic:?}, entered={entered}"
        );
    }

    for (stable_code, diagnostic) in [
        ("routing_generation_unavailable", None),
        ("configuration_invalid", None),
        ("request_invalid", None),
        (
            "response_invalid",
            Some(EvmSafeDiagnostic::ResponseInvalid {
                kind: EvmResponseInvalidKind::MalformedEnvelope,
            }),
        ),
        (
            "response_invalid",
            Some(EvmSafeDiagnostic::ResponseInvalid {
                kind: EvmResponseInvalidKind::InvalidResult,
            }),
        ),
        (
            "response_missing_result",
            Some(EvmSafeDiagnostic::ResponseInvalid {
                kind: EvmResponseInvalidKind::MissingResult,
            }),
        ),
        (
            "response_too_large",
            Some(EvmSafeDiagnostic::ResponseInvalid {
                kind: EvmResponseInvalidKind::TooLarge,
            }),
        ),
    ] {
        assert_verdict(stable_code, diagnostic.clone(), false, Expected::Invalid);
        assert_verdict(stable_code, diagnostic, true, Expected::Invalid);
    }

    assert_verdict("access_cancelled", None, false, Expected::Insufficient);
    assert_verdict("access_cancelled", None, true, Expected::Insufficient);
    assert_verdict("transport_failed", None, false, Expected::Insufficient);
    assert_verdict("transport_failed", None, true, Expected::Insufficient);
    assert_verdict("unclassified_failure", None, false, Expected::Invalid);
    assert_verdict("unclassified_failure", None, true, Expected::Insufficient);

    let insufficient_http = [408, 425, 429, 500, 502, 503, 504, 507];
    for status in u16::MIN..=u16::MAX {
        assert_verdict(
            "http_status",
            Some(EvmSafeDiagnostic::HttpStatus { status }),
            true,
            if insufficient_http.contains(&status) {
                Expected::Insufficient
            } else {
                Expected::DestinationRejected
            },
        );
        assert_verdict(
            "http_status",
            Some(EvmSafeDiagnostic::HttpStatus { status }),
            false,
            Expected::Invalid,
        );
    }

    let insufficient_json_rpc = [-32603, -32001, -32002, -32005];
    for json_rpc_code in -40_000_i64..=40_000 {
        assert_verdict(
            "json_rpc_error",
            Some(EvmSafeDiagnostic::JsonRpcError {
                code: json_rpc_code,
            }),
            true,
            if insufficient_json_rpc.contains(&json_rpc_code) {
                Expected::Insufficient
            } else {
                Expected::DestinationRejected
            },
        );
        assert_verdict(
            "json_rpc_error",
            Some(EvmSafeDiagnostic::JsonRpcError {
                code: json_rpc_code,
            }),
            false,
            Expected::Invalid,
        );
    }
    for json_rpc_code in [i64::MIN, i64::MAX] {
        assert_verdict(
            "json_rpc_error",
            Some(EvmSafeDiagnostic::JsonRpcError {
                code: json_rpc_code,
            }),
            true,
            Expected::DestinationRejected,
        );
    }
}

#[test]
fn confirmation_distinguishes_anchor_drift_from_invalid_number_evidence() {
    let source = anchored(0x11);
    let same = EvmBlockAnchor::new(U256::from(19_000_000), B256::from([0x11; 32]));
    let changed = EvmBlockAnchor::new(U256::from(19_000_000), B256::from([0x22; 32]));
    let wrong_number = EvmBlockAnchor::new(U256::from(19_000_001), B256::from([0x11; 32]));

    assert_eq!(check_anchor_confirmation(&source, &same), Ok(true));
    assert_eq!(check_anchor_confirmation(&source, &changed), Ok(false));
    assert_eq!(check_anchor_confirmation(&source, &wrong_number), Err(()));
}

#[test]
fn confirmation_request_authorship_is_total_for_invalid_fan_in() {
    assert!(common_fanout_source(&[]).is_none());
    assert!(EvmAnchorConfirmationRequest::invalid_input()
        .source()
        .is_none());

    let first = EvmBalanceGraphResult::Balance {
        anchored_source: anchored(0x11),
        balance_source: EvmBalanceSource::new(account(1), EvmBalanceAsset::Native)
            .expect("balance source"),
        raw_units: "0".to_owned(),
    };
    let second = EvmBalanceGraphResult::Balance {
        anchored_source: anchored(0x22),
        balance_source: EvmBalanceSource::new(account(2), EvmBalanceAsset::Native)
            .expect("balance source"),
        raw_units: "0".to_owned(),
    };
    assert!(common_fanout_source(&[first, second]).is_none());
}

#[test]
fn pure_aggregate_requires_exact_coverage_and_emits_one_fact_per_balance() {
    let token = account(0x33);
    let native = EvmBalanceSource::new(account(1), EvmBalanceAsset::Native).expect("native");
    let erc20 = EvmBalanceSource::new(account(2), EvmBalanceAsset::erc20(token).expect("asset"))
        .expect("erc20");
    let config =
        EvmBalanceCollectionConfig::new(binding(), 18, vec![erc20.clone(), native.clone()])
            .expect("config");
    let source = anchored(0x11);
    let values = vec![
        EvmBalanceGraphResult::TokenDecimals {
            source: source.clone(),
            contract_address: canonical_address(token),
            decimals: 6,
        },
        EvmBalanceGraphResult::Balance {
            anchored_source: source.clone(),
            balance_source: native,
            raw_units: "1000000000000000000".to_owned(),
        },
        EvmBalanceGraphResult::Balance {
            anchored_source: source.clone(),
            balance_source: erc20,
            raw_units: "42".to_owned(),
        },
        EvmBalanceGraphResult::AnchorConfirmation {
            source: source.clone(),
        },
    ];
    let (collection, facts) = aggregate_collection(&config, &values).expect("aggregate");
    assert_eq!(collection.source(), &source);
    assert_eq!(collection.balances().len(), 2);
    assert_eq!(facts.as_slice().len(), 2);

    let mut missing = values.clone();
    missing.remove(2);
    assert!(aggregate_collection(&config, &missing).is_err());

    let mut duplicate = values;
    duplicate.push(duplicate[1].clone());
    assert!(aggregate_collection(&config, &duplicate).is_err());
}

#[test]
fn maximal_demand_has_exact_2052_node_bound() {
    let sources = (1_u32..=1024)
        .map(|index| {
            let mut raw = [0_u8; 20];
            raw[16..].copy_from_slice(&index.to_be_bytes());
            let contract = Address::from(raw);
            raw[0] = 1;
            let account = Address::from(raw);
            EvmBalanceSource::new(account, EvmBalanceAsset::erc20(contract).expect("asset"))
                .expect("source")
        })
        .collect();
    let config = EvmBalanceCollectionConfig::new(binding(), 18, sources).expect("config");
    assert_eq!(config.sources().len(), 1024);
    assert_eq!(config.token_contracts().len(), 1024);
    assert_eq!(config.node_count(), EVM_BALANCE_COLLECTION_NODE_LIMIT);
}
