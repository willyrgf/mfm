use super::*;
use mfm_facts::{FactFieldExposure, FactFieldExtraction, FactFieldValueType};
use mfm_program::MfmFactType;

fn valid_subject() -> BtcAddressBalanceSubject {
    BtcAddressBalanceSubject::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
    )
    .expect("subject")
}

fn response_from_json(json: serde_json::Value) -> BtcAddressBalanceResponse {
    serde_json::from_value(json).expect("response json")
}

#[test]
fn descriptor_declares_kind_anchors_coverage_and_store_commit_order() {
    let descriptor = BtcAddressBalanceSnapshotFact::descriptor().expect("descriptor");
    assert_eq!(
        descriptor.fact_kind().as_str(),
        "bitcoin.address_balance_snapshot"
    );
    for field_id in [
        "subject.network",
        "subject.bitcoin_network",
        "subject.semantic_source_identity",
        "subject.address",
        "result.anchor_height",
        "result.anchor_hash",
        "result.balance_sats",
        "result.coverage",
        "result.source_status",
    ] {
        assert!(
            descriptor.fields().iter().any(|field| {
                field.field_id().as_str() == field_id
                    && field.exposure() == FactFieldExposure::Returnable
            }),
            "missing returnable field {field_id}"
        );
    }
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "result.anchor_height"
            && field.value_type() == FactFieldValueType::UnsignedInteger
            && field.sortable()
    }));
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "metadata.store_commit_order"
            && matches!(field.extraction(), FactFieldExtraction::Metadata(_))
            && field.sortable()
    }));
    let ordering_names: Vec<_> = descriptor
        .orderings()
        .iter()
        .map(|ordering| ordering.name().as_str())
        .collect();
    assert!(ordering_names.contains(&"result.anchor_height.desc"));
    assert!(ordering_names.contains(&"metadata.store_commit_order.desc"));
}

#[test]
fn write_admission_requires_hash_and_admissible_coverage_status() {
    assert!(BtcAddressBalanceResponse::new(
        100,
        "",
        1,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .is_err());
    assert!(
        BtcAddressBalanceResponse::new(
            100,
            "not-a-hash",
            1,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .is_err(),
        "malformed nonempty hash must fail write admission"
    );
    assert!(BtcAddressBalanceResponse::new(
        100,
        "00".repeat(32),
        1,
        CoverageStatus::Truncated,
        HoldingSourceStatus::Ok,
    )
    .is_err());
    assert!(BtcAddressBalanceResponse::new(
        100,
        "00".repeat(32),
        1,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Failed,
    )
    .is_err());
    let response = BtcAddressBalanceResponse::new(
        100,
        "00".repeat(32),
        42,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("admissible");
    let fact = BtcAddressBalanceSnapshotFact::new(valid_subject(), response);
    let normalized = normalize_btc_address_balance_fact(&fact).expect("normalize");
    assert_eq!(normalized.balance_sats, 42);
    assert_eq!(normalized.anchor_height, 100);
    assert_eq!(normalized.coverage, CoverageStatus::ConfiguredOnly);
    assert_eq!(normalized.source_status, HoldingSourceStatus::Ok);
}

#[test]
fn normalize_rejects_malformed_decoded_payloads() {
    // Tampered payloads that bypassed constructor checks (e.g. direct decode).
    let missing_hash = response_from_json(serde_json::json!({
        "anchor_height": 99,
        "anchor_hash": "",
        "balance_sats": 7,
        "coverage": "configured_only",
        "source_status": "ok",
    }));
    assert!(normalize_btc_address_balance(&valid_subject(), &missing_hash).is_err());

    let malformed_hash = response_from_json(serde_json::json!({
        "anchor_height": 99,
        "anchor_hash": "zz".repeat(32),
        "balance_sats": 7,
        "coverage": "configured_only",
        "source_status": "ok",
    }));
    assert!(normalize_btc_address_balance(&valid_subject(), &malformed_hash).is_err());

    let incomplete = response_from_json(serde_json::json!({
        "anchor_height": 99,
        "anchor_hash": "aa".repeat(32),
        "balance_sats": 7,
        "coverage": "incomplete",
        "source_status": "ok",
    }));
    assert!(normalize_btc_address_balance(&valid_subject(), &incomplete).is_ok());
}

#[test]
fn response_json_round_trip_has_no_floats_or_secrets() {
    let response = BtcAddressBalanceResponse::new(
        850_000,
        "0f".repeat(32),
        100_000,
        CoverageStatus::CompleteAtAnchor,
        HoldingSourceStatus::Ok,
    )
    .expect("response");
    let fact = BtcAddressBalanceSnapshotFact::new(valid_subject(), response);
    let value = serde_json::to_value(&fact).expect("json");
    let text = serde_json::to_string(&value).expect("text");
    for forbidden in ["rpc_url", "password", "http://", "wallet_id", "symbol_id"] {
        assert!(
            !text.contains(forbidden),
            "leaked forbidden token {forbidden}: {text}"
        );
    }
    // Hashed structures must not contain JSON floats.
    fn assert_no_floats(value: &serde_json::Value) {
        match value {
            serde_json::Value::Number(n) => assert!(n.is_u64() || n.is_i64(), "{n}"),
            serde_json::Value::Array(items) => items.iter().for_each(assert_no_floats),
            serde_json::Value::Object(map) => map.values().for_each(assert_no_floats),
            _ => {}
        }
    }
    assert_no_floats(&value);
    let decoded: BtcAddressBalanceSnapshotFact = serde_json::from_value(value).expect("round-trip");
    assert_eq!(decoded, fact);
}

#[test]
fn platform_candidate_query_plan_is_exact_full_set_not_limit_one() {
    use mfm_facts::{FactAudience, ScopeDecisionEvidence, StoreScopeRef};
    use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};

    let subject = valid_subject();
    let store_scope = StoreScopeRef::new("mfm.store.default").expect("store");
    let scope_decision = ScopeDecisionEvidence::new(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x31; 32]),
    ));
    let plan = platform_address_balance_candidate_plan(&store_scope, scope_decision, &subject)
        .expect("plan");
    assert_eq!(plan.query_scope().audience(), FactAudience::Platform);
    assert_eq!(plan.limit(), None);
    assert_eq!(plan.ordering().name().as_str(), "result.anchor_height.desc");
    let query = plan.canonical_query().as_str();
    assert!(query.contains("metadata.store_commit_order"));
    assert!(query.contains("result.anchor_hash"));
}
