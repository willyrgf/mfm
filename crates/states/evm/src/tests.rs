use super::*;
use mfm_facts::{FactFieldExposure, FactFieldExtraction, FactFieldValueType};
use mfm_program::MfmFactType;

fn valid_subject() -> EvmAddressNativeBalanceSubject {
    EvmAddressNativeBalanceSubject::new(
        "ethereum-mainnet",
        1,
        "0x0000000000000000000000000000000000000001",
    )
    .expect("subject")
}

#[test]
fn subject_rejects_noncanonical_account() {
    let error = EvmAddressNativeBalanceSubject::new(
        "ethereum-mainnet",
        1,
        "0x00000000000000000000000000000000000000AA",
    )
    .expect_err("mixed-case account must be rejected");

    assert!(error.to_string().contains("normalized lowercase"));
}

#[test]
fn descriptor_declares_kind_anchors_coverage_and_store_commit_order() {
    let descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor().expect("descriptor");
    assert_eq!(
        descriptor.fact_kind().as_str(),
        "evm.address_native_balance_snapshot"
    );
    for field_id in [
        "subject.network",
        "subject.chain_id",
        "subject.account",
        "result.block_number",
        "result.block_hash",
        "result.raw_wei",
        "result.decimals",
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
        field.field_id().as_str() == "result.block_number"
            && field.value_type() == FactFieldValueType::UnsignedInteger
            && field.sortable()
    }));
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "metadata.store_commit_order"
            && matches!(field.extraction(), FactFieldExtraction::Metadata(_))
            && field.sortable()
    }));
}

#[test]
fn write_admission_requires_hash_and_admissible_coverage_status() {
    assert!(EvmAddressNativeBalanceResponse::new(
        1,
        "",
        "0",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .is_err());
    assert!(
        EvmAddressNativeBalanceResponse::new(
            1,
            "not-a-hash",
            "0",
            18,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .is_err(),
        "malformed nonempty hash must fail write admission"
    );
    assert!(EvmAddressNativeBalanceResponse::new(
        1,
        "0x".to_owned() + &"ab".repeat(32),
        "not-digits",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .is_err());
    assert!(EvmAddressNativeBalanceResponse::new(
        1,
        "0x".to_owned() + &"ab".repeat(32),
        "1000",
        18,
        CoverageStatus::Truncated,
        HoldingSourceStatus::Ok,
    )
    .is_err());
    let response = EvmAddressNativeBalanceResponse::new(
        21_000_000,
        "0x".to_owned() + &"cd".repeat(32),
        "1000000000000000000",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("admissible");
    let fact = EvmAddressNativeBalanceSnapshotFact::new(valid_subject(), response);
    let normalized = normalize_evm_address_native_balance_fact(&fact).expect("normalize");
    assert_eq!(normalized.raw_wei, "1000000000000000000");
    assert_eq!(normalized.decimals, 18);
    assert_eq!(normalized.block_number, 21_000_000);
    assert_eq!(normalized.chain_id, 1);
}

#[test]
fn block_hashes_are_canonicalized_for_anchor_intersection() {
    let response = EvmAddressNativeBalanceResponse::new(
        21_000_000,
        format!("0X{}", "AB".repeat(32)),
        "1",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("uppercase hash is valid");
    assert_eq!(
        response.block_hash(),
        "0xabababababababababababababababababababababababababababababababab"
    );
}

#[test]
fn normalize_rejects_malformed_decoded_payloads() {
    let missing_hash: EvmAddressNativeBalanceResponse = serde_json::from_value(serde_json::json!({
        "block_number": 10,
        "block_hash": "",
        "raw_wei": "1",
        "decimals": 18,
        "coverage": "configured_only",
        "source_status": "ok",
    }))
    .expect("decode");
    assert!(normalize_evm_address_native_balance(&valid_subject(), &missing_hash).is_err());

    let failed: EvmAddressNativeBalanceResponse = serde_json::from_value(serde_json::json!({
        "block_number": 10,
        "block_hash": "0xabababababababababababababababababababababababababababababababab",
        "raw_wei": "1",
        "decimals": 18,
        "coverage": "configured_only",
        "source_status": "failed",
    }))
    .expect("decode");
    assert!(normalize_evm_address_native_balance(&valid_subject(), &failed).is_ok());
}

#[test]
fn response_json_round_trip_has_no_floats_or_secrets() {
    let response = EvmAddressNativeBalanceResponse::new(
        1,
        "0x".to_owned() + &"11".repeat(32),
        "42",
        18,
        CoverageStatus::CompleteAtAnchor,
        HoldingSourceStatus::Ok,
    )
    .expect("response");
    let fact = EvmAddressNativeBalanceSnapshotFact::new(valid_subject(), response);
    let value = serde_json::to_value(&fact).expect("json");
    let text = serde_json::to_string(&value).expect("text");
    for forbidden in ["rpc_url", "password", "http://", "wallet_id", "symbol_id"] {
        assert!(
            !text.contains(forbidden),
            "leaked forbidden token {forbidden}: {text}"
        );
    }
    fn assert_no_floats(value: &serde_json::Value) {
        match value {
            serde_json::Value::Number(n) => assert!(n.is_u64() || n.is_i64(), "{n}"),
            serde_json::Value::Array(items) => items.iter().for_each(assert_no_floats),
            serde_json::Value::Object(map) => map.values().for_each(assert_no_floats),
            _ => {}
        }
    }
    assert_no_floats(&value);
    let decoded: EvmAddressNativeBalanceSnapshotFact =
        serde_json::from_value(value).expect("round-trip");
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
        DigestBytes::from_array([0x51; 32]),
    ));
    let plan = platform_native_balance_candidate_plan(&store_scope, scope_decision, &subject)
        .expect("plan");
    assert_eq!(plan.query_scope().audience(), FactAudience::Platform);
    assert_eq!(plan.limit(), None);
    assert_eq!(plan.ordering().name().as_str(), "result.block_number.desc");
    let query = plan.canonical_query().as_str();
    assert!(query.contains("metadata.store_commit_order"));
    assert!(query.contains("result.block_hash"));
}
