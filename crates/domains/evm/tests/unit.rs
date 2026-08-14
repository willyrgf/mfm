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

fn submission_request() -> EvmSubmissionRequest {
    EvmSubmissionRequest::planned(
        EvmSubmissionSelector::new(
            EvmTransactionTarget::new(1, "0xabc".to_owned(), "wallet-main".to_owned())
                .expect("target"),
            "request".to_owned(),
            Vec::new(),
            1,
            "1".to_owned(),
        )
        .expect("selector"),
        nominal_contract_ref::<EvmSubmissionSelector>().expect("signer reference"),
    )
    .expect("request")
}

fn submission_failure<O>(
    outcome: ProposedStateOutcome<O, EvmSubmissionFailure>,
) -> EvmSubmissionFailure {
    match outcome {
        ProposedStateOutcome::Failure { failure } => failure,
        ProposedStateOutcome::Success { .. } => panic!("expected submission failure"),
    }
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
    let context =
        EvmBalanceContext::new(request.clone(), continuation, 0, "correlation".to_owned())
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
fn admission_and_result_deserialization_reenter_domain_validation() {
    assert!(serde_json::from_str::<EvmSubmissionRequest>(
            r#"{"target":{"chain_id":1,"sender":"0xABC","nonce_domain":"main"},"idempotency_key":"request","data":[],"gas_limit":1,"max_fee":"1"}"#
        )
        .is_err());
    assert!(serde_json::from_str::<EvmSubmissionOutput>(
        r#"{"candidate_id":"candidate","transaction_hash":"","included":true}"#
    )
    .is_err());
    assert!(
        serde_json::from_str::<EvmSubmissionOutput>(r#"{"execution_disposition":"accepted"}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<EvmSubmissionFailure>(
        r#"{"kind":"destination_rejected","value":{"code":"secret=canary"}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmSubmissionFailure>(
        r#"{"kind":"unknown_submission_failure"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmBalanceFailure>(
        r#"{"kind":"source_unavailable","value":{"stage":"forged","collection_ordinal":0,"code":"observation_unavailable"}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmBalanceFailure>(
        r#"{"kind":"source_unavailable","value":{"stage":"check_chain_identity","collection_ordinal":0,"code":"integrity_blocked"}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmBalanceFailure>(
        r#"{"kind":"integrity_blocked","value":{"stage":"check_chain_identity","collection_ordinal":0,"code":"collection_invalid"}}"#
    )
    .is_err());
}

#[test]
fn transaction_data_capacity_accepts_exact_and_rejects_plus_one() {
    let target =
        EvmTransactionTarget::new(1, "0xabc".to_owned(), "wallet-main".to_owned()).expect("target");
    assert!(EvmSubmissionSelector::new(
        target.clone(),
        "capacity-exact".to_owned(),
        vec![0; EVM_TRANSACTION_DATA_LIMIT],
        1,
        "1".to_owned(),
    )
    .is_ok());
    assert!(EvmSubmissionSelector::new(
        target,
        "capacity-plus-one".to_owned(),
        vec![0; EVM_TRANSACTION_DATA_LIMIT + 1],
        1,
        "1".to_owned(),
    )
    .is_err());
}

#[test]
fn submission_progress_rejects_a_non_deterministic_candidate() {
    let request = submission_request();
    assert!(EvmSubmissionProgress::new(
        request,
        SubmissionPhase::Candidate {
            nonce: 7,
            candidate_id: "forged-candidate".to_owned(),
        },
    )
    .is_err());
}

#[test]
fn submission_failure_mapping_is_closed_and_stage_specific() {
    let request = submission_request();
    assert_eq!(
        submission_failure(interpret_reserve_wallet_nonce(
            request.clone(),
            &NonceReservationEvidence::Rejected,
        )),
        EvmSubmissionFailure::NonceAuthorityUnavailable
    );

    let candidate_id = submission_candidate_id(&request.target, 7);
    let candidate = EvmSubmissionProgress::new(
        request,
        SubmissionPhase::Candidate {
            nonce: 7,
            candidate_id: candidate_id.clone(),
        },
    )
    .expect("candidate progress");
    assert_eq!(
        submission_failure(interpret_broadcast_transaction(
            candidate,
            &BroadcastEvidence::Rejected,
        )),
        EvmSubmissionFailure::DestinationRejected
    );

    let progress =
        EvmSubmissionProgress::new(submission_request(), SubmissionPhase::Reserved { nonce: 7 })
            .expect("reserved progress");
    assert_eq!(
        <EvmState<0, 0> as State>::integrity_failure(&submission_request()),
        EvmSubmissionFailure::NonceAuthorityUnavailable
    );
    assert_eq!(
        <EvmState<0, 2> as State>::integrity_failure(&progress),
        EvmSubmissionFailure::ProviderUnavailable
    );
    assert_eq!(
        <EvmState<0, 3> as State>::integrity_failure(&progress),
        EvmSubmissionFailure::ProviderUnavailable
    );
    assert_eq!(
        <EvmState<0, 4> as State>::integrity_failure(&progress),
        EvmSubmissionFailure::ProviderUnavailable
    );
    assert_eq!(
        <EvmState<0, 5> as State>::integrity_failure(&progress),
        EvmSubmissionFailure::ProviderUnavailable
    );
    assert_eq!(
        submission_failure(derive_submission_candidate({
            let request = submission_request();
            let candidate_id = submission_candidate_id(&request.target, 7);
            EvmSubmissionProgress::new(
                request,
                SubmissionPhase::Candidate {
                    nonce: 7,
                    candidate_id,
                },
            )
            .expect("candidate progress")
        })),
        EvmSubmissionFailure::NonceLineageDiverged
    );
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
        "metadata": {"collection_ordinal": 0, "correlation": "collection-0"},
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
    )
    .expect("chain intent");
    let chain_evidence = EvmReadEvidence::Returned {
        value: EvmReadValue::ChainId(1),
    };
    assert!(EvmCapability::<2>::bind_evidence(&chain_intent, &chain_evidence).is_ok());
    let mismatched_chain_intent = EvmReadIntent::new(
        "mfm.evm.read-chain-identity@1".to_owned(),
        1,
        EvmReadSubject::InitialAnchor,
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

    for (intent, evidence) in [
        (
            EvmReadIntent::new(
                "mfm.evm.read-transaction-receipt@1".to_owned(),
                1,
                EvmReadSubject::TransactionReceipt {
                    transaction_hash: "0xtx".to_owned(),
                },
            )
            .expect("receipt intent"),
            EvmReadEvidence::Returned {
                value: EvmReadValue::Receipt {
                    transaction_hash: "0xtx".to_owned(),
                    execution_disposition: "succeeded".to_owned(),
                    inclusion_block_number: "1".to_owned(),
                    inclusion_block_hash: "0xblock".to_owned(),
                },
            },
        ),
        (
            EvmReadIntent::new(
                "mfm.evm.read-finalized-head@1".to_owned(),
                1,
                EvmReadSubject::FinalizedHead,
            )
            .expect("finalized-head intent"),
            EvmReadEvidence::Returned {
                value: EvmReadValue::FinalizedHead {
                    number: "1".to_owned(),
                },
            },
        ),
        (
            EvmReadIntent::new(
                "mfm.evm.read-canonical-inclusion-block@1".to_owned(),
                1,
                EvmReadSubject::CanonicalInclusionBlock {
                    number: "1".to_owned(),
                },
            )
            .expect("canonical-block intent"),
            EvmReadEvidence::Returned {
                value: EvmReadValue::CanonicalBlock {
                    number: "1".to_owned(),
                    hash: "0xblock".to_owned(),
                },
            },
        ),
    ] {
        assert!(EvmCapability::<3>::bind_evidence(&intent, &evidence).is_ok());
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
fn frozen_submission_and_collection_wires_match_the_contract_goldens() {
    assert_eq!(
        canonical_json(&EvmSubmissionOutput::new(
            EvmExecutionDisposition::Succeeded
        )),
        include_str!("../../../../docs/contracts/evm-portfolio/evm-submission-succeeded.json")
            .trim()
    );
    assert_eq!(
        canonical_json(&EvmSubmissionOutput::new(EvmExecutionDisposition::Reverted)),
        include_str!("../../../../docs/contracts/evm-portfolio/evm-submission-reverted.json")
            .trim()
    );
    for (failure, fixture) in [
        (
            EvmSubmissionFailure::NonceAuthorityUnavailable,
            include_str!("../../../../docs/contracts/evm-portfolio/evm-submission-failure.json"),
        ),
        (
            EvmSubmissionFailure::DestinationRejected,
            include_str!(
                "../../../../docs/contracts/evm-portfolio/evm-submission-destination-rejected.json"
            ),
        ),
        (
            EvmSubmissionFailure::ProviderUnavailable,
            include_str!(
                "../../../../docs/contracts/evm-portfolio/evm-submission-provider-unavailable.json"
            ),
        ),
        (
            EvmSubmissionFailure::NonceLineageDiverged,
            include_str!(
                "../../../../docs/contracts/evm-portfolio/evm-submission-nonce-lineage-diverged.json"
            ),
        ),
    ] {
        assert_eq!(canonical_json(&failure), fixture.trim());
        assert_eq!(
            serde_json::from_str::<EvmSubmissionFailure>(fixture).expect("closed failure wire"),
            failure
        );
    }

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
