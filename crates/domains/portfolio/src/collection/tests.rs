use super::*;
use mfm_chain::balance::{
    CandidateBalance, ConsolidateBalanceCollection, DecimalScale, PreparedBalance,
};
use mfm_chain::ObservationPoint;
use mfm_values::{Object, Unsigned256};

use mfm_chain::balance::BalanceSource;
use mfm_chain::{BalanceTarget, LedgerIdentity};

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Fact {
    text: String,
}

#[test]
fn resumption_preserves_product_fields_and_rejects_substituted_handoff_facts() {
    let input = {
        let collections = 1;
        let sources_per_collection = 2;
        let scale = 2;
        let label = |index: usize| format!("source-{index}");
        let ledger = LedgerIdentity::new(
            Object::from_value(&Fact {
                text: "independent-ledger".into(),
            })
            .unwrap(),
        );
        let route = Object::from_value(&Fact {
            text: "independent-route".into(),
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
                    .map(|_| BalanceExecutionConfig::new(route.value_ref().clone(), route.clone()))
                    .collect();
                PortfolioCollectionDemand::new(label(ordinal), request, executions).unwrap()
            })
            .collect();
        PortfolioSnapshotInput::new(
            PortfolioId {
                value: "portfolio".into(),
            },
            collections,
            QuoteCode::Usd,
            vec![QuoteCode::Usd, QuoteCode::Eur],
            None,
        )
        .unwrap()
    };
    let point = ObservationPoint::new(
        input.collections[0].request.sources()[0]
            .target()
            .ledger()
            .clone(),
        Object::from_value(&Fact {
            text: "18446744073709551616".into(),
        })
        .unwrap(),
    );
    let ProposedStateOutcome::Success {
        output: continuation,
    } = InitializePortfolio::evaluate(input).unwrap()
    else {
        panic!("expected initial continuation")
    };
    let ProposedStateOutcome::Success {
        output: mut context,
    } = EnterPortfolioCollection::evaluate(continuation).unwrap()
    else {
        panic!("expected shared collection")
    };
    // Shared confirmation admission is scripted here; native IO and Runtime execution are covered
    // by separate integration work, not by this product handoff test.
    for raw in [100, 250] {
        let prepared =
            PreparedBalance::new(context, point.clone(), DecimalScale::new(2).unwrap()).unwrap();
        context = CandidateBalance::new(prepared, Unsigned256::from_u64(raw))
            .append_confirmed()
            .unwrap()
            .unwrap();
    }
    let ProposedStateOutcome::Success { output: completion } =
        ConsolidateBalanceCollection::evaluate(context).unwrap()
    else {
        panic!("expected collection completion")
    };
    let wire = serde_json::to_value(&completion).unwrap();
    for (field, value, reason) in [
        ("collection_ordinal", serde_json::json!(1), "Ordinal"),
        ("correlation", serde_json::json!("another"), "Metadata"),
        (
            "route_ref",
            serde_json::to_value(
                Object::from_value(&Fact {
                    text: "substituted-route".into(),
                })
                .unwrap()
                .value_ref(),
            )
            .unwrap(),
            "Metadata",
        ),
    ] {
        let mut changed = wire.clone();
        changed["context"]["metadata"][field] = value;
        let completion =
            serde_json::from_str::<BalanceCollectionCompletion<PortfolioContinuation>>(
                &changed.to_string(),
            )
            .unwrap();
        let error = ResumePortfolioCollection::evaluate(completion).unwrap_err();
        assert_eq!(error.operation(), "resume_portfolio_collection");
        assert!(error.details().as_value().to_string().contains(reason));
    }
    let mut changed = wire;
    changed["context"]["request"]["sources"][0]["source_id"] = serde_json::json!("substituted");
    changed["context"]["completed"][0]["source"]["source_id"] = serde_json::json!("substituted");
    let substituted = serde_json::from_str::<BalanceCollectionCompletion<PortfolioContinuation>>(
        &changed.to_string(),
    )
    .unwrap();
    let error = ResumePortfolioCollection::evaluate(substituted).unwrap_err();
    assert_eq!(error.details().as_value(), &serde_json::json!("Request"));

    let completion = Object::from_value(&completion).unwrap().decode().unwrap();
    let ProposedStateOutcome::Success { output } =
        ResumePortfolioCollection::evaluate(completion).unwrap()
    else {
        panic!("expected resumed Portfolio")
    };
    // Cold admission still checks both a child's total and its relation to retained demand.
    let wire = serde_json::to_value(&output).unwrap();
    for (path, value) in [
        ("/completed_collections/0/total_scaled", "999"),
        ("/completed_collections/0/metadata/correlation", "another"),
        ("/input/collections/0/correlation", "another"),
    ] {
        let mut changed = wire.clone();
        *changed.pointer_mut(path).unwrap() = serde_json::json!(value);
        assert!(serde_json::from_str::<PortfolioContinuation>(&changed.to_string()).is_err());
    }
    let cold = Object::from_value(&output)
        .unwrap()
        .decode::<PortfolioContinuation>()
        .unwrap();
    assert_eq!(
        cold.completed_collections[0].balances[0]
            .observed_at()
            .native()
            .decode::<Fact>()
            .unwrap()
            .text,
        "18446744073709551616"
    );
    assert_eq!(
        cold.completed_collections[0].balances[0]
            .source()
            .source_id(),
        "source-0"
    );
    assert_eq!(
        cold.completed_collections[0].balances[1]
            .raw_units()
            .as_str(),
        "250"
    );
    let ProposedStateOutcome::Success { output } = ConsolidatePortfolio::evaluate(cold).unwrap()
    else {
        panic!("expected snapshot")
    };
    assert_eq!(output.report.collection_summaries[0].total_value_dec, "3.5");
}
