use super::*;
use mfm_chain::balance::BalanceSource;
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_values::Object;
#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Fact {
    text: String,
}

#[test]
fn source_revision_hex_is_checked_and_survives_canonical_value_qualification() {
    let entry = EntryPointId::new(PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID).unwrap();
    let admission = PortfolioAdmission::new(
        ConfigName::new("candidates").unwrap(),
        DigestBytes::from_array([0xab; 32]),
        entry.clone(),
        None,
    );
    let (canonical, _) = mfm_values::canonicalize_mfm_value(&admission).unwrap();
    let decoded: PortfolioAdmission = serde_json::from_slice(canonical.as_bytes()).unwrap();
    assert_eq!(decoded, admission);
    assert_eq!(
        decoded.config_digest_hex(),
        DigestBytes::from_array([0xab; 32])
    );
    for invalid in [
        "AB".repeat(32),
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
        format!("content:sha256-jcs-v1:{}", "ab".repeat(32)),
    ] {
        let mut wire = serde_json::to_value(&admission).unwrap();
        wire["config_digest_hex"] = serde_json::json!(invalid);
        assert!(serde_json::from_value::<PortfolioAdmission>(wire).is_err());
    }
    for invalid in ["", "Bad", "-daily", "daily_1", "daily-", &"x".repeat(65)] {
        let mut wire = serde_json::to_value(&admission).unwrap();
        wire["name"] = serde_json::json!(invalid);
        assert!(serde_json::from_value::<PortfolioAdmission>(wire).is_err());
    }
}

#[test]
fn enrichment_checks_required_source_membership_and_collection_coverage() {
    for (required, accepted) in [
        (vec![], false),
        (vec!["missing".into()], false),
        (vec!["source-0".into()], false),
        (vec!["source-0".into(), "source-2".into()], true),
    ] {
        let collections = 2;
        let sources_per_collection = 2;
        let scale = 0;
        let input = {
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
                        .map(|_| {
                            BalanceExecutionConfig::new(route.value_ref().clone(), route.clone())
                        })
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
        assert_eq!(
            PortfolioEnrichmentInput::new(input, required).is_ok(),
            accepted
        );
    }
}
