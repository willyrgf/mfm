use super::*;
use mfm_chain::balance::ConsolidateBalanceCollection;

use mfm_chain::balance::BalanceSource;
use mfm_chain::balance::{CandidateBalance, PreparedBalance};
use mfm_chain::ObservationPoint;
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_values::{Object, Unsigned256};

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Fact {
    text: String,
}

#[test]
fn maximum_public_fields_preserve_completed_prefix_snapshot_and_enrichment() {
    let maximum_units =
        "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    for collection_count in [1, 64] {
        let input = {
            let collections = collection_count;
            let sources_per_collection = 64 / collection_count;
            let scale = 30;
            let label = |index: usize| format!("{index:02}{}", "\"".repeat(254));
            let ledger = LedgerIdentity::new(
                Object::from_value(&Fact {
                    text: "independent-ledger".into(),
                })
                .unwrap(),
            );
            let route = Object::from_value(&Fact {
                text: "\"".repeat(256),
            })
            .unwrap();
            let collections = (0..collections)
                .map(|ordinal| {
                    let sources = (0..sources_per_collection)
                        .map(|source| {
                            BalanceSource::new(
                                label(ordinal * sources_per_collection + source),
                                BalanceTarget::new(
                                    ledger.clone(),
                                    Object::from_value(&Fact {
                                        text: label(source),
                                    })
                                    .unwrap(),
                                ),
                            )
                            .unwrap()
                        })
                        .collect();
                    let request =
                        BalanceRequest::new(sources, DecimalScale::new(scale).unwrap()).unwrap();
                    let executions = (0..sources_per_collection)
                        .map(|_| {
                            BalanceExecutionConfig::new(route.value_ref().clone(), route.clone())
                        })
                        .collect();
                    PortfolioCollectionDemand::new(label(ordinal), request, executions).unwrap()
                })
                .collect();
            PortfolioSnapshotInput::new(
                PortfolioId {
                    value: "\"".repeat(256),
                },
                collections,
                QuoteCode::Usd,
                vec![QuoteCode::Usd, QuoteCode::Eur],
                None,
            )
            .unwrap()
        };
        let required: Vec<_> = input
            .collections
            .iter()
            .flat_map(|collection| {
                collection
                    .request
                    .sources()
                    .iter()
                    .map(|source| source.source_id().to_owned())
            })
            .collect();
        let ProposedStateOutcome::Success {
            output: mut continuation,
        } = InitializePortfolio::evaluate(input).unwrap();
        for _ in 0..collection_count {
            let ProposedStateOutcome::Success {
                output: mut context,
            } = EnterPortfolioCollection::evaluate(continuation).unwrap();
            for _ in 0..context.request().sources().len() {
                let point = ObservationPoint::new(
                    context.request().sources()[0].target().ledger().clone(),
                    Object::from_value(&Fact {
                        text: "9".repeat(80),
                    })
                    .unwrap(),
                );
                context = CandidateBalance::new(
                    PreparedBalance::new(context, point, DecimalScale::new(30).unwrap()).unwrap(),
                    Unsigned256::new(maximum_units).unwrap(),
                )
                .append_confirmed()
                .unwrap()
                .unwrap();
            }
            let ProposedStateOutcome::Success { output: complete } =
                ConsolidateBalanceCollection::evaluate(context).unwrap()
            else {
                panic!("expected success");
            };
            let ProposedStateOutcome::Success { output } =
                ResumePortfolioCollection::evaluate(complete).unwrap();
            continuation = output;
        }
        mfm_values::canonicalize_mfm_value(&continuation).unwrap();
        let enriched = serde_json::json!({"progress":continuation,"required_sources":required});
        let enriched = serde_json::from_slice::<EnrichmentContinuation>(
            &serde_json::to_vec(&enriched).unwrap(),
        )
        .unwrap();
        let ProposedStateOutcome::Success { output: resolved } =
            ResolvePortfolioAssets::evaluate(enriched).unwrap();
        mfm_values::canonicalize_mfm_value(&resolved).unwrap();
        assert_eq!(resolved.collections().len(), collection_count);
        let ProposedStateOutcome::Success { output } =
            ConsolidatePortfolio::evaluate(continuation).unwrap()
        else {
            panic!("expected success");
        };
        mfm_values::canonicalize_mfm_value(&output).unwrap();
        assert_eq!(output.snapshot.collections.len(), collection_count);
        let wire = serde_json::to_value(&output).unwrap();
        assert!(serde_json::from_value::<PortfolioSnapshotOutput>(wire.clone()).is_ok());
        if collection_count == 64 {
            let mut duplicate_correlation = wire.clone();
            duplicate_correlation["snapshot"]["collections"][1]["metadata"]["correlation"] =
                duplicate_correlation["snapshot"]["collections"][0]["metadata"]["correlation"]
                    .clone();
            assert_output_wire_rejected(duplicate_correlation);

            let mut duplicate_source = wire;
            duplicate_source["snapshot"]["collections"][1]["balances"][0]["source"]["source_id"] =
                duplicate_source["snapshot"]["collections"][0]["balances"][0]["source"]
                    ["source_id"]
                    .clone();
            assert_output_wire_rejected(duplicate_source);
        } else {
            // Each child remains valid at 64 sources; only the aggregate exceeds its bound.
            let mut excessive_sources = wire;
            let mut second = excessive_sources["snapshot"]["collections"][0].clone();
            second["metadata"]["collection_ordinal"] = 1.into();
            second["metadata"]["correlation"] = "second".into();
            for (ordinal, balance) in second["balances"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .enumerate()
            {
                balance["source"]["source_id"] = format!("second-{ordinal}").into();
            }
            excessive_sources["snapshot"]["collections"]
                .as_array_mut()
                .unwrap()
                .push(second);
            let mut summary = excessive_sources["report"]["collection_summaries"][0].clone();
            summary["collection_ordinal"] = 1.into();
            let total = summary["total_value_dec"].as_str().unwrap().to_owned();
            excessive_sources["report"]["collection_summaries"]
                .as_array_mut()
                .unwrap()
                .push(summary);
            excessive_sources["report"]["totals_by_quote"][0]["total_value_dec"] =
                sum_decimal_values(&[total.clone(), total]).unwrap().into();
            assert_output_wire_rejected(excessive_sources);
        }
    }
}

fn assert_output_wire_rejected(wire: serde_json::Value) {
    for collection in wire["snapshot"]["collections"].as_array().unwrap() {
        serde_json::from_value::<PortfolioSnapshotCollection>(collection.clone()).unwrap();
    }
    assert!(serde_json::from_value::<PortfolioSnapshotOutput>(wire).is_err());
}
