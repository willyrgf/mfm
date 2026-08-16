use super::*;
use mfm_canonical::PlainCanonicalJsonBytes;

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct OpaqueCallerContinuation {
    marker: String,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
#[mfm(transparent_string)]
struct FrozenCallerContext {
    value: String,
}

fn canonical_json<T: Serialize>(value: &T) -> String {
    let json = serde_json::to_string(value).expect("JSON");
    PlainCanonicalJsonBytes::from_json_str(&json)
        .expect("canonical JSON")
        .as_str()
        .to_owned()
}

fn balance_route_ref() -> ContentRef {
    nominal_contract_ref::<EvmBalanceRequest>().expect("route reference")
}

#[test]
fn generic_balance_completion_moves_a_non_clone_continuation() {
    let request = EvmBalanceRequest::new(
        vec![EvmBalanceSource {
            source_id: "source-1".to_owned(),
            chain_id: 1,
            address: "0xabc".to_owned(),
            token: None,
        }],
        18,
    )
    .expect("request");
    let continuation = OpaqueCallerContinuation {
        marker: "opaque".to_owned(),
    };
    let context = EvmBalanceContext::new(
        request.clone(),
        continuation,
        0,
        "correlation".to_owned(),
        balance_route_ref(),
    )
    .expect("full context");
    assert!(context.validate().is_ok());
    let EvmBalanceContext {
        caller_continuation,
        ..
    } = context;
    let anchor = EvmBlockAnchor::new("1".to_owned(), "0xblock".to_owned()).expect("anchor");
    let completion = EvmBalanceCollectionCompletion {
        caller_context: caller_continuation,
        collection_ordinal: 0,
        collection: EvmBalanceCollectionResult {
            chain_id: 1,
            anchor: anchor.clone(),
            balances: vec![EvmCollectedBalance::from_result(EvmBalanceResult {
                source: request.sources[0].clone(),
                decimals: 18,
                raw_units: "0".to_owned(),
                anchor,
            })],
            total_scaled: "0".to_owned(),
        },
    };
    assert!(completion.validate().is_ok());
    assert_eq!(completion.caller_context.marker, "opaque");
}

#[test]
fn balance_request_rejects_non_adjacent_duplicate_sources() {
    let source = EvmBalanceSource {
        source_id: "source-1".to_owned(),
        chain_id: 1,
        address: "0xabc".to_owned(),
        token: None,
    };
    assert!(EvmBalanceRequest::new(
        vec![
            source.clone(),
            EvmBalanceSource {
                source_id: "source-2".to_owned(),
                ..source.clone()
            },
            source,
        ],
        18,
    )
    .is_err());
}

#[test]
fn balance_failure_deserialization_reenters_domain_validation() {
    assert!(serde_json::from_str::<EvmBalanceFailure>(
        r#"{"kind":"source_unavailable","value":{"stage":"forged","collection_ordinal":0,"code":"observation_unavailable"}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmBalanceFailure>(
        r#"{"kind":"source_unavailable","value":{"stage":"check_chain_identity","collection_ordinal":0,"code":"integrity_blocked"}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmBalanceFailure>(
        r#"{"kind":"source_unavailable","value":{"stage":"read_initial_anchor","collection_ordinal":0,"code":"chain_identity_unavailable"}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmBalanceFailure>(
        r#"{"kind":"source_unavailable","value":{"stage":"check_chain_identity","collection_ordinal":0,"code":"observation_unavailable"}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmBalanceFailure>(
        r#"{"kind":"integrity_blocked","value":{"stage":"check_chain_identity","collection_ordinal":0,"code":"collection_invalid"}}"#
    )
    .is_err());
}

#[test]
fn balance_context_rejects_a_forged_non_prefix_work_item() {
    let value = serde_json::json!({
        "request": {
            "sources": [{
                "source_id": "source-1",
                "chain_id": 1,
                "address": "0xabc",
                "token": null
            }],
            "decimals": 18
        },
        "caller_continuation": {"marker": "opaque"},
        "metadata": {
            "collection_ordinal": 0,
            "correlation": "collection-0",
            "route_ref": balance_route_ref()
        },
        "completed": [],
        "work": {
            "kind": "read_native_balance",
            "source": {
                "source_id": "source-2",
                "chain_id": 1,
                "address": "0xdef",
                "token": null
            },
            "checked_chain_id": 1,
            "initial_anchor": {"number": "1", "hash": "0xblock"}
        }
    });
    assert!(serde_json::from_value::<EvmBalanceContext<OpaqueCallerContinuation>>(value).is_err());
}

#[test]
fn read_evidence_requires_the_exact_operation_subject_and_value() {
    let chain_intent = EvmReadIntent::new(
        "mfm.evm.read-chain-identity@1".to_owned(),
        1,
        EvmReadSubject::ChainIdentity,
        balance_route_ref(),
    )
    .expect("chain intent");
    let chain_evidence = EvmReadEvidence::Returned {
        value: EvmReadValue::ChainId(1),
    };
    assert!(EvmCapability::<2>::bind_evidence(&chain_intent, &chain_evidence).is_ok());
    assert!(serde_json::from_value::<EvmReadIntent>(serde_json::json!({
        "operation": "mfm.evm.read-chain-identity@1",
        "chain_id": 1,
        "subject": {"kind": "chain_identity"},
        "route_ref": null,
    }))
    .is_err());
    for subject in [
        serde_json::json!({
            "kind": "transaction_receipt",
            "value": {"transaction_hash": "0xtx"},
        }),
        serde_json::json!({"kind": "finalized_head"}),
        serde_json::json!({
            "kind": "canonical_inclusion_block",
            "value": {"number": "1"},
        }),
    ] {
        assert!(serde_json::from_value::<EvmReadIntent>(serde_json::json!({
            "operation": "mfm.evm.retired-status@1",
            "chain_id": 1,
            "subject": subject,
            "route_ref": balance_route_ref(),
        }))
        .is_err());
    }
    let mismatched_chain_intent = EvmReadIntent::new(
        "mfm.evm.read-chain-identity@1".to_owned(),
        1,
        EvmReadSubject::InitialAnchor,
        balance_route_ref(),
    )
    .expect("structurally valid intent");
    assert!(EvmCapability::<2>::bind_evidence(
        &mismatched_chain_intent,
        &EvmReadEvidence::Returned {
            value: EvmReadValue::Anchor {
                number: "1".to_owned(),
                hash: "0xblock".to_owned(),
            },
        },
    )
    .is_err());

    for value in [
        serde_json::json!({
            "kind": "receipt",
            "value": {
                "transaction_hash": "0xtx",
                "execution_disposition": "succeeded",
                "inclusion_block_number": "1",
                "inclusion_block_hash": "0xblock"
            }
        }),
        serde_json::json!({"kind": "finalized_head", "value": {"number": "1"}}),
        serde_json::json!({
            "kind": "canonical_block",
            "value": {"number": "1", "hash": "0xblock"},
        }),
    ] {
        assert!(serde_json::from_value::<EvmReadValue>(value).is_err());
    }

    let source = EvmBalanceSource {
        source_id: "source-1".to_owned(),
        chain_id: 1,
        address: "0xabc".to_owned(),
        token: None,
    };
    let anchor = EvmBlockAnchor::new("1".to_owned(), "0xblock".to_owned()).expect("anchor");
    let balance_intent = EvmReadIntent::new(
        "mfm.evm.read-native-balance@1".to_owned(),
        1,
        EvmReadSubject::NativeBalance { source, anchor },
        balance_route_ref(),
    )
    .expect("balance intent");
    let wrong_value = EvmReadEvidence::Returned {
        value: EvmReadValue::TokenDecimals(18),
    };
    assert!(EvmCapability::<7>::bind_evidence(&balance_intent, &wrong_value).is_err());
    assert!(serde_json::from_str::<EvmBlockAnchor>(r#"{"number":"01","hash":"0xblock"}"#).is_err());
    assert!(serde_json::from_str::<EvmReadValue>(r#"{"kind":"chain_id","value":0}"#).is_err());
}

#[test]
fn integrity_failure_retains_the_active_collection_ordinal() {
    let request = EvmBalanceRequest::new(
        vec![EvmBalanceSource {
            source_id: "source-1".to_owned(),
            chain_id: 1,
            address: "0xabc".to_owned(),
            token: None,
        }],
        18,
    )
    .expect("request");
    let context = EvmBalanceContext::new(
        request,
        OpaqueCallerContinuation {
            marker: "opaque".to_owned(),
        },
        7,
        "collection-7".to_owned(),
        balance_route_ref(),
    )
    .expect("context");
    assert_eq!(
        <EvmState<1, 0, OpaqueCallerContinuation> as State>::integrity_failure(&context),
        EvmBalanceFailure::IntegrityBlocked {
            stage: "check_chain_identity".to_owned(),
            collection_ordinal: 7,
            code: "integrity_blocked".to_owned(),
        }
    );
}

#[test]
fn chain_identity_rejection_uses_the_frozen_collection_failure_code() {
    let request = EvmBalanceRequest::new(
        vec![EvmBalanceSource {
            source_id: "source-1".to_owned(),
            chain_id: 1,
            address: "0xabc".to_owned(),
            token: None,
        }],
        18,
    )
    .expect("request");
    let context = EvmBalanceContext::new(
        request,
        OpaqueCallerContinuation {
            marker: "opaque".to_owned(),
        },
        0,
        "collection-0".to_owned(),
        balance_route_ref(),
    )
    .expect("context");
    let ProposedStateOutcome::Failure { failure } =
        interpret_check_chain_identity(context, &EvmReadEvidence::Rejected)
    else {
        panic!("rejected chain identity must fail");
    };
    assert_eq!(
        failure,
        EvmBalanceFailure::SourceUnavailable {
            stage: "check_chain_identity".to_owned(),
            collection_ordinal: 0,
            code: "chain_identity_unavailable".to_owned(),
        }
    );
}

#[test]
fn frozen_collection_wire_matches_the_contract_golden() {
    let completion = EvmBalanceCollectionCompletion::new(
        FrozenCallerContext {
            value: "portfolio:example".to_owned(),
        },
        0,
        EvmBalanceCollectionResult {
            chain_id: 1,
            anchor: EvmBlockAnchor::new(
                "100".to_owned(),
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            )
            .expect("anchor"),
            balances: vec![EvmCollectedBalance {
                source: EvmCollectedBalanceSource {
                    source_id: "wallet-a.native".to_owned(),
                    chain_id: 1,
                    address: "0x1111111111111111111111111111111111111111".to_owned(),
                    asset: EvmCollectedAsset::Native,
                },
                decimals: 18,
                raw_units: "1000000000000000000".to_owned(),
            }],
            total_scaled: "1000000000000000000".to_owned(),
        },
    )
    .expect("completion");
    assert_eq!(
        canonical_json(&completion),
        include_str!("../../../../docs/contracts/evm-portfolio/evm-balance-collection.json").trim()
    );
}
