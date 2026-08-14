use super::*;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm::EvmBalanceSource;
use mfm_values::ValidatedConfig;

fn canonical_json<T: Serialize>(value: &T) -> String {
    let json = serde_json::to_string(value).expect("JSON");
    PlainCanonicalJsonBytes::from_json_str(&json)
        .expect("canonical JSON")
        .as_str()
        .to_owned()
}

#[test]
fn shared_config_validation_rejects_domain_invalid_values() {
    assert!(ValidatedConfig::new(PortfolioConfig {
        portfolio_id: PortfolioId {
            value: "portfolio-1".to_owned(),
        },
        quotes: Vec::new(),
        collections: Vec::new(),
    })
    .is_err());
}

fn request() -> EvmBalanceRequest {
    EvmBalanceRequest::new(
        vec![mfm_evm::EvmBalanceSource {
            source_id: "source-1".to_owned(),
            chain_id: 1,
            address: "0xabc".to_owned(),
            token: None,
        }],
        18,
    )
    .expect("request")
}

fn planned_input(requests: Vec<EvmBalanceRequest>) -> PortfolioSnapshotInput {
    let route_ref = nominal_contract_ref::<EvmBalanceRequest>().expect("route ref");
    let collections = requests
        .into_iter()
        .enumerate()
        .map(|(ordinal, request)| {
            PortfolioCollectionDemand::new(
                format!("collection-{ordinal}"),
                request,
                route_ref.clone(),
            )
            .expect("demand")
        })
        .collect();
    PortfolioSnapshotInput::from_demand(
        PortfolioId {
            value: "portfolio-1".to_owned(),
        },
        collections,
        QuoteCode::Usd,
    )
    .expect("input")
}

fn result_for(request: &EvmBalanceRequest) -> PortfolioSnapshotCollection {
    let anchor = PortfolioAnchor::new("1".to_owned(), "0xblock".to_owned()).expect("anchor");
    PortfolioSnapshotCollection {
        collection_ordinal: 0,
        chain_id: 1,
        anchor,
        holdings: request
            .sources
            .iter()
            .cloned()
            .map(|source| PortfolioHolding {
                source_id: source.source_id,
                asset: match source.token {
                    None => PortfolioAsset::Native,
                    Some(contract) => PortfolioAsset::Token { contract },
                },
                decimals: request.decimals,
                raw_units: "0".to_owned(),
                amount_dec: decimal_amount("0", request.decimals),
            })
            .collect(),
    }
}

#[test]
fn continuation_derives_its_remaining_declaration_suffix() {
    let input = planned_input(vec![request()]);
    let mut continuation = PortfolioContinuation::new(input, Vec::new()).expect("continuation");
    assert_eq!(continuation.next_collection_ordinal(), Some(0));
    let mut foreign_result = result_for(&request());
    foreign_result.holdings[0].source_id = "foreign-source".to_owned();
    continuation.completed_collections.push(foreign_result);
    assert_eq!(
        continuation.validate(),
        Err(PortfolioError::InvalidContinuation)
    );

    let input = planned_input(vec![request()]);
    let mut continuation = PortfolioContinuation::new(input, Vec::new()).expect("continuation");
    let mut inconsistent_amount = result_for(&request());
    inconsistent_amount.holdings[0].amount_dec = "2".to_owned();
    continuation.completed_collections.push(inconsistent_amount);
    assert_eq!(
        continuation.validate(),
        Err(PortfolioError::InvalidContinuation)
    );
}

#[test]
fn resume_rechecks_the_evm_scaled_total_before_storing_the_snapshot_projection() {
    let request = request();
    let input = planned_input(vec![request.clone()]);
    let continuation = PortfolioContinuation::new(input, Vec::new()).expect("continuation");
    let completion: EvmBalanceCollectionCompletion<PortfolioContinuation> =
        serde_json::from_value(serde_json::json!({
            "caller_context": &continuation,
            "collection_ordinal": 0,
            "collection": {
                "chain_id": 1,
                "anchor": { "number": "1", "hash": "0xblock" },
                "balances": [{
                    "source": {
                        "source_id": &request.sources[0].source_id,
                        "chain_id": 1,
                        "address": &request.sources[0].address,
                        "asset": { "kind": "native" },
                    },
                    "decimals": 18,
                    "raw_units": "1",
                }],
                "total_scaled": "2",
            },
        }))
        .expect("syntactically valid EVM completion");
    let mfm_capabilities::ProposedStateOutcome::Failure { failure } =
        resume_portfolio_collection(completion)
    else {
        panic!("mismatched EVM total must fail");
    };
    assert_eq!(failure, PortfolioSnapshotFailure::ConsolidationFailed);
}

#[test]
fn output_rejects_report_projection_mismatches() {
    let output = PortfolioSnapshotOutput {
        snapshot: PortfolioSnapshot {
            schema_version: 1,
            portfolio_id: PortfolioId {
                value: "portfolio-1".to_owned(),
            },
            collections: vec![PortfolioSnapshotCollection {
                collection_ordinal: 0,
                chain_id: 1,
                anchor: PortfolioAnchor::new("1".to_owned(), "0xblock".to_owned()).expect("anchor"),
                holdings: vec![PortfolioHolding {
                    source_id: "source-1".to_owned(),
                    asset: PortfolioAsset::Native,
                    decimals: 18,
                    raw_units: "0".to_owned(),
                    amount_dec: "0.000000000000000000".to_owned(),
                }],
            }],
        },
        report: PortfolioReport {
            schema_version: 1,
            portfolio_id: PortfolioId {
                value: "portfolio-1".to_owned(),
            },
            quote: QuoteCode::Usd,
            collection_summaries: vec![PortfolioCollectionSummary {
                collection_ordinal: 0,
                total_value_dec: "0".to_owned(),
            }],
            totals_by_quote: vec![PortfolioQuoteTotal {
                quote: QuoteCode::Usd,
                total_value_dec: "0".to_owned(),
            }],
        },
    };
    assert_eq!(output.validate(), Ok(()));
    let mut output = output;
    output.snapshot.collections[0].holdings[0].amount_dec = "1".to_owned();
    assert_eq!(output.validate(), Err(PortfolioError::InvalidValue));
    output.snapshot.collections[0].holdings[0].amount_dec = "0.000000000000000000".to_owned();
    output.report.totals_by_quote[0].total_value_dec = "not-a-number".to_owned();
    assert_eq!(output.validate(), Err(PortfolioError::InvalidValue));
}

#[test]
fn frozen_snapshot_and_failure_wires_match_the_contract_goldens() {
    let output = PortfolioSnapshotOutput {
        snapshot: PortfolioSnapshot {
            schema_version: 1,
            portfolio_id: PortfolioId {
                value: "portfolio-example".to_owned(),
            },
            collections: vec![PortfolioSnapshotCollection {
                collection_ordinal: 0,
                chain_id: 1,
                anchor: PortfolioAnchor::new(
                    "100".to_owned(),
                    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                )
                .expect("anchor"),
                holdings: vec![PortfolioHolding {
                    source_id: "wallet-a.native".to_owned(),
                    asset: PortfolioAsset::Native,
                    decimals: 18,
                    raw_units: "1000000000000000000".to_owned(),
                    amount_dec: "1.000000000000000000".to_owned(),
                }],
            }],
        },
        report: PortfolioReport {
            schema_version: 1,
            portfolio_id: PortfolioId {
                value: "portfolio-example".to_owned(),
            },
            quote: QuoteCode::Usd,
            collection_summaries: vec![PortfolioCollectionSummary {
                collection_ordinal: 0,
                total_value_dec: "1".to_owned(),
            }],
            totals_by_quote: vec![PortfolioQuoteTotal {
                quote: QuoteCode::Usd,
                total_value_dec: "1".to_owned(),
            }],
        },
    };
    assert_eq!(output.validate(), Ok(()));
    assert_eq!(
        canonical_json(&output),
        include_str!("../../../../docs/contracts/evm-portfolio/portfolio-snapshot.json").trim()
    );
    assert_eq!(
        canonical_json(&PortfolioSnapshotFailure::CollectionFailed {
            ordinal: 0,
            code: "chain_identity_unavailable".to_owned(),
        }),
        include_str!("../../../../docs/contracts/evm-portfolio/portfolio-snapshot-failure.json")
            .trim()
    );
}

#[test]
fn portfolio_failure_decode_rejects_secret_and_out_of_range_payloads() {
    assert!(serde_json::from_str::<PortfolioSnapshotFailure>(
        r#"{"kind":"collection_failed","value":{"ordinal":0,"code":"secret=canary"}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<PortfolioSnapshotFailure>(
        r#"{"kind":"collection_failed","value":{"ordinal":64,"code":"observation_unavailable"}}"#
    )
    .is_err());
}

#[test]
fn consolidation_projects_the_frozen_decimal_snapshot_wire() {
    let request = EvmBalanceRequest::new(
        vec![EvmBalanceSource {
            source_id: "wallet-a.native".to_owned(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".to_owned(),
            token: None,
        }],
        18,
    )
    .expect("request");
    let input = PortfolioSnapshotInput::from_demand(
        PortfolioId {
            value: "portfolio-example".to_owned(),
        },
        vec![PortfolioCollectionDemand::new(
            "collection-0".to_owned(),
            request,
            nominal_contract_ref::<EvmBalanceRequest>().expect("route ref"),
        )
        .expect("demand")],
        QuoteCode::Usd,
    )
    .expect("input");
    let anchor = PortfolioAnchor::new(
        "100".to_owned(),
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
    )
    .expect("anchor");
    let collection = PortfolioSnapshotCollection {
        collection_ordinal: 0,
        chain_id: 1,
        anchor,
        holdings: vec![PortfolioHolding {
            source_id: "wallet-a.native".to_owned(),
            asset: PortfolioAsset::Native,
            decimals: 18,
            raw_units: "1000000000000000000".to_owned(),
            amount_dec: "1.000000000000000000".to_owned(),
        }],
    };
    let continuation =
        PortfolioContinuation::new(input, vec![collection]).expect("complete continuation");
    let mfm_capabilities::ProposedStateOutcome::Success { output, .. } =
        consolidate_portfolio(continuation)
    else {
        panic!("complete continuation must consolidate");
    };
    assert_eq!(
        canonical_json(&output),
        include_str!("../../../../docs/contracts/evm-portfolio/portfolio-snapshot.json").trim()
    );
}

#[test]
fn evm_integrity_failure_preserves_its_collection_ordinal() {
    let mfm_capabilities::ProposedStateOutcome::Failure { failure } =
        map_evm_balance_failure(EvmBalanceFailure::IntegrityBlocked {
            stage: "read_token_balance".to_owned(),
            collection_ordinal: 3,
            code: "integrity_blocked".to_owned(),
        })
    else {
        panic!("EVM failure must remain a Portfolio failure");
    };
    assert_eq!(
        failure,
        PortfolioSnapshotFailure::CollectionFailed {
            ordinal: 3,
            code: "integrity_blocked".to_owned(),
        }
    );
}

#[test]
fn portfolio_total_source_bound_is_exact() {
    let sources = |count: usize| {
        (0..count)
            .map(|ordinal| EvmBalanceSource {
                source_id: format!("source-{ordinal}"),
                chain_id: 1,
                address: format!("0x{ordinal:x}"),
                token: None,
            })
            .collect::<Vec<_>>()
    };
    let accepted = EvmBalanceRequest::new(sources(64), 18).expect("64 sources");
    assert!(PortfolioSnapshotInput::from_demand(
        PortfolioId {
            value: "portfolio-1".to_owned(),
        },
        vec![PortfolioCollectionDemand::new(
            "collection-0".to_owned(),
            accepted,
            nominal_contract_ref::<EvmBalanceRequest>().expect("route ref"),
        )
        .expect("demand")],
        QuoteCode::Usd,
    )
    .is_ok());

    let half = EvmBalanceRequest::new(sources(32), 18).expect("32 sources");
    let over = EvmBalanceRequest::new(sources(33), 18).expect("33 sources");
    assert!(PortfolioSnapshotInput::from_demand(
        PortfolioId {
            value: "portfolio-1".to_owned(),
        },
        vec![
            PortfolioCollectionDemand::new(
                "collection-0".to_owned(),
                half,
                nominal_contract_ref::<EvmBalanceRequest>().expect("route ref"),
            )
            .expect("demand"),
            PortfolioCollectionDemand::new(
                "collection-1".to_owned(),
                over,
                nominal_contract_ref::<EvmBalanceRequest>().expect("route ref"),
            )
            .expect("demand"),
        ],
        QuoteCode::Usd,
    )
    .is_err());

    let rejected = EvmBalanceRequest::new(sources(65), 18);
    assert!(rejected.is_err());
}

#[test]
fn admission_deserialization_reenters_nested_domain_validation() {
    let invalid = serde_json::json!({
        "portfolio_id": "portfolio-1",
        "collections": [{
            "correlation": "collection-0",
            "request": {
                "sources": [{
                    "source_id": "source-1",
                    "chain_id": 1,
                    "address": "0xABC",
                    "token": null
                }],
                "decimals": 18
            },
            "route_ref": serde_json::to_value(
                nominal_contract_ref::<EvmBalanceRequest>().expect("route ref")
            ).expect("route ref JSON")
        }],
        "quote": "usd"
    });
    assert!(serde_json::from_value::<PortfolioSnapshotInput>(invalid).is_err());
}
