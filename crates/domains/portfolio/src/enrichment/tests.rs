use super::*;

#[test]
fn source_revision_hex_is_checked_and_survives_canonical_value_qualification() {
    let entry = EntryPointId::new(PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID).unwrap();
    let admission = PortfolioAdmission::new(
        "candidates".to_owned(),
        "ab".repeat(32),
        entry.clone(),
        None,
    )
    .unwrap();
    let (canonical, _) = mfm_values::canonicalize_mfm_value(&admission).unwrap();
    let decoded: PortfolioAdmission = serde_json::from_slice(canonical.as_bytes()).unwrap();
    assert_eq!(decoded, admission);
    assert_eq!(decoded.config_digest_hex(), "ab".repeat(32));
    for invalid in [
        "AB".repeat(32),
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
        format!("content:sha256-jcs-v1:{}", "ab".repeat(32)),
    ] {
        assert!(PortfolioAdmission::new(
            "candidates".to_owned(),
            invalid.clone(),
            entry.clone(),
            None
        )
        .is_err());
        let mut wire = serde_json::to_value(&admission).unwrap();
        wire["config_digest_hex"] = serde_json::json!(invalid);
        assert!(serde_json::from_value::<PortfolioAdmission>(wire).is_err());
    }
}

#[test]
fn enrichment_rejects_collections_without_a_native_candidate() {
    let config: PortfolioConfig = serde_json::from_value(serde_json::json!({
        "portfolio_id": "candidates", "quotes": ["usd"], "collections": [{
            "correlation": "tokens", "request": {"decimals": 18, "sources": [{
                "source_id": "token", "chain_id": 1, "address": format!("0x{}", "1".repeat(40)),
                "token": format!("0x{}", "2".repeat(40))
            }]}
        }]
    }))
    .unwrap();
    let selector =
        serde_json::from_value(serde_json::json!({"target": "candidates", "quote": "usd"}))
            .unwrap();
    let target = EvmPhysicalTarget {
        chain_id: NonZeroU64::new(1).unwrap(),
        endpoint_ref: mfm_evm::EvmEndpoint::new("candidate")
            .unwrap()
            .endpoint_ref()
            .unwrap(),
    };
    assert!(matches!(
        plan_enrichment(selector, &config, &[target], None),
        Err(PortfolioError::InvalidValue)
    ));
}
