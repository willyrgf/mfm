use super::*;
use mfm_ids::{DigestBytes, RunId};

// A full candidate portfolio must remain representable through enrichment and publication
// without losing its selected routes, config or provenance.
#[tokio::test]
async fn maximum_candidate_output_and_published_config_fit_the_existing_document_ceiling() {
    {
        let collection_count = 64;
        let mut routes = Vec::new();
        let collections = (0..collection_count).map(|i| {
            let endpoint_id = "\"".repeat(256);
            let chain_id = u64::MAX - (collection_count - 1 - i) as u64;
            routes.push((chain_id, endpoint_id));
            serde_json::json!({
                "correlation": format!("{i:02}{}", "\"".repeat(254)),
                "request": {"decimals": 30, "sources": (0..64/collection_count).map(|j| serde_json::json!({
                    "source_id": format!("{:02}{}", i*64/collection_count+j, "\"".repeat(254)),
                    "chain_id": chain_id, "address": format!("0x{}", "f".repeat(40)), "token": null
                })).collect::<Vec<_>>()}
            })
        }).collect::<Vec<_>>();
        let portfolio = serde_json::json!({"portfolio_id": "\"".repeat(256), "quotes": ["usd", "eur"], "collections": collections});
        let selector = serde_json::json!({"target": "\"".repeat(256), "quote": "usd"});
        let candidate = serde_json::json!({"entry_point": PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID, "input": {
            "portfolio": portfolio, "selector": selector, "routes": routes.iter().map(|(chain_id, endpoint_id)| serde_json::json!({"chain_id": chain_id, "endpoint_id": endpoint_id})).collect::<Vec<_>>()
        }});
        let candidate_bytes = serde_json::to_vec(&candidate).unwrap();
        assert!(candidate_bytes.len() < 128 * 1024);
        let document = ConfigDocument::new(candidate_bytes).await.unwrap();
        let admitted = native::admit_enrichment(document.native(), None).unwrap();
        use mfm_chain::balance::{
            CandidateBalance, ConsolidateBalanceCollection, DecimalScale, PreparedBalance,
        };
        use mfm_portfolio::{
            EnterEnrichmentCollection, InitializeEnrichment, ResolvePortfolioAssets,
            ResumeEnrichmentCollection,
        };
        use mfm_program::{ProposedStateOutcome, PureState};
        let ProposedStateOutcome::Success {
            output: mut progress,
        } = InitializeEnrichment::evaluate(admitted).unwrap();
        for _ in 0..collection_count {
            let ProposedStateOutcome::Success { output: context } =
                EnterEnrichmentCollection::evaluate(progress).unwrap();
            let anchor = mfm_evm::EvmBlockPoint::new(
                mfm_evm::EvmU256::new("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap(),
                mfm_evm::EvmHash::from_bytes([255; 32]),
            );
            let point = mfm_chain::ObservationPoint::new(
                context.request().sources()[0].target().ledger().clone(),
                mfm_values::Object::from_value(&anchor).unwrap(),
            );
            let prepared =
                PreparedBalance::new(context, point, DecimalScale::new(18).unwrap()).unwrap();
            let completed = CandidateBalance::new(prepared, mfm_values::Unsigned256::from_u64(1))
                .append_confirmed()
                .unwrap()
                .unwrap();
            let ProposedStateOutcome::Success { output: completion } =
                ConsolidateBalanceCollection::evaluate(completed).unwrap()
            else {
                panic!("completion")
            };
            let ProposedStateOutcome::Success { output } =
                ResumeEnrichmentCollection::evaluate(completion).unwrap();
            progress = output;
        }
        let ProposedStateOutcome::Success { output } =
            ResolvePortfolioAssets::evaluate(progress).unwrap();
        let (canonical, reference) = mfm_values::canonicalize_mfm_value(&output).unwrap();
        assert!(canonical.as_bytes().len() < MAX_CONFIG_DOCUMENT_BYTES);
        let provenance = EnrichmentProvenance::new(
            RunId::from_digest(DigestBytes::from_array([255; 32])),
            mfm_ids::ContentDigest::from_digest(
                mfm_ids::DigestAlgorithm::Sha256V1,
                DigestBytes::from_array([255; 32]),
            ),
            reference,
        )
        .unwrap();
        let published = ConfigDocument::from_enrichment(output, provenance.clone()).unwrap();
        assert!(published.canonical.as_bytes().len() < 128 * 1024);
        assert!(published
            .matches_enrichment(&serde_json::from_slice(canonical.as_bytes()).unwrap())
            .unwrap());
        assert_eq!(
            serde_json::to_value(&published.wire).unwrap()["input"]["portfolio"],
            portfolio
        );
        assert_eq!(
            serde_json::to_value(&published.wire).unwrap()["input"]["selector"],
            selector
        );
        assert_eq!(published.enrichment(), Some(&provenance));
    }
}
