use super::*;
use mfm_ids::{DigestBytes, RunId};

#[tokio::test]
async fn maximum_candidate_output_and_published_config_fit_the_existing_document_ceiling() {
    for collection_count in [1, 64] {
        let mut targets = Vec::new();
        let mut routes = Vec::new();
        let collections = (0..collection_count).map(|i| {
            let endpoint_id = "\"".repeat(256);
            let chain_id = u64::MAX - (collection_count - 1 - i) as u64;
            let target = EvmRouteWire { chain_id, endpoint_id: endpoint_id.clone() }.target().unwrap();
            targets.push(target);
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
        ConfigDocument::new(candidate_bytes).await.unwrap();
        let output: PortfolioEnrichmentOutput = serde_json::from_value(serde_json::json!({
            "portfolio": portfolio, "selector": selector,
            "collections": targets.iter().map(|target| serde_json::json!({
                "chain_id": target.chain_id, "route_ref": target.binding_ref().unwrap(),
                "anchor": {"number": "9".repeat(80), "hash": "\"".repeat(256)}
            })).collect::<Vec<_>>()
        }))
        .unwrap();
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
        let published = ConfigDocument::from_enrichment(&output, &provenance, routes).unwrap();
        assert!(published.canonical.as_bytes().len() < 128 * 1024);
        assert!(published.matches_enrichment(&output));
        assert_eq!(published.enrichment(), Some(&provenance));
    }
}
