use std::num::NonZeroU64;

use mfm_capabilities::ReadCapabilityContract;
use mfm_evm::{
    EvmAddress, EvmBalanceRead, EvmBalanceSource, EvmBlockAnchor, EvmHash, EvmReadEvidence,
    EvmReadIntent, EvmReadSubject, EvmReadValue, EvmTokenDecimals, EvmU256,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_values::{canonicalize_mfm_value, MfmValue};

fn route_ref(byte: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.route",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        )
        .expect("schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            DigestBytes::from_array([byte; 32]),
        ),
    )
    .expect("route ref")
}

fn source(source_id: &str, address_byte: &str) -> EvmBalanceSource {
    EvmBalanceSource::new(
        source_id,
        NonZeroU64::new(1).expect("chain"),
        EvmAddress::new(format!("0x{}", address_byte.repeat(20))).expect("address"),
        Some(EvmAddress::new(format!("0x{}", "cc".repeat(20))).expect("token")),
    )
    .expect("source")
}

fn anchor(number: u64, hash_byte: &str) -> EvmBlockAnchor {
    EvmBlockAnchor {
        number: EvmU256::from_u64(number),
        hash: EvmHash::new(format!("0x{}", hash_byte.repeat(32))).expect("hash"),
    }
}

fn intent(source: EvmBalanceSource, anchor: EvmBlockAnchor) -> EvmReadIntent {
    EvmReadIntent::new(
        NonZeroU64::new(1).expect("chain"),
        route_ref(2),
        EvmReadSubject::TokenBalance { source, anchor },
    )
    .expect("intent")
}

fn schema_id<T: MfmValue>() -> String {
    T::schema_descriptor()
        .expect("descriptor")
        .schema_id()
        .expect("schema id")
        .to_string()
}

#[test]
fn broad_evm_read_contract_ids_and_wires_are_exact() {
    let source = source("wallet.token", "aa");
    let anchor = anchor(17, "bb");
    let subject = EvmReadSubject::TokenBalance {
        source: source.clone(),
        anchor: anchor.clone(),
    };
    let intent = intent(source, anchor);
    let (_, intent_value_ref) = canonicalize_mfm_value(&intent).expect("intent ref");
    let value = EvmReadValue::TokenDecimals(EvmTokenDecimals::new(18).expect("decimals"));
    let evidence = EvmReadEvidence::returned(intent_value_ref, value.clone());

    let contracts = [
        (
            EvmReadSubject::semantic_id()
                .expect("subject semantic")
                .to_string(),
            schema_id::<EvmReadSubject>(),
            canonicalize_mfm_value(&subject)
                .expect("subject canonical")
                .0
                .to_string(),
        ),
        (
            EvmReadIntent::semantic_id()
                .expect("intent semantic")
                .to_string(),
            schema_id::<EvmReadIntent>(),
            canonicalize_mfm_value(&intent)
                .expect("intent canonical")
                .0
                .to_string(),
        ),
        (
            EvmReadValue::semantic_id()
                .expect("value semantic")
                .to_string(),
            schema_id::<EvmReadValue>(),
            canonicalize_mfm_value(&value)
                .expect("value canonical")
                .0
                .to_string(),
        ),
        (
            EvmReadEvidence::semantic_id()
                .expect("evidence semantic")
                .to_string(),
            schema_id::<EvmReadEvidence>(),
            canonicalize_mfm_value(&evidence)
                .expect("evidence canonical")
                .0
                .to_string(),
        ),
    ];
    assert_eq!(
        contracts,
        [
            (
                "semantic:mfm.derived:evm_read_subject:1:sha256-jcs-v1:8573a4ad095d835d4c21ebfa715ff1f2191304bc55615f9ed11901a2a83df8b7".to_owned(),
                "schema:mfm.derived.evm_read_subject:1:sha256-jcs-v1:5a9e130795db9885b55fe53a49eed1ea087d1bcc0a20658d7f360097b9a4faf5".to_owned(),
                "{\"kind\":\"token_balance\",\"value\":{\"anchor\":{\"hash\":\"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"number\":\"17\"},\"source\":{\"address\":\"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"chain_id\":1,\"source_id\":\"wallet.token\",\"token\":\"0xcccccccccccccccccccccccccccccccccccccccc\"}}}".to_owned(),
            ),
            (
                "semantic:mfm.derived:evm_read_intent:1:sha256-jcs-v1:20a6e535d00ed8920a14aa01f7bb8c0c52132e5fc3d38b6642ce9b054ebf8aae".to_owned(),
                "schema:mfm.derived.evm_read_intent:1:sha256-jcs-v1:2d411a262e102946452c10b3573930b45bbde3aed818f16cb746cdfcc5ac5579".to_owned(),
                "{\"chain_id\":1,\"route_ref\":{\"content_digest\":\"content:sha256-v1:0202020202020202020202020202020202020202020202020202020202020202\",\"schema_id\":\"schema:mfm.test.route:1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101\"},\"subject\":{\"kind\":\"token_balance\",\"value\":{\"anchor\":{\"hash\":\"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"number\":\"17\"},\"source\":{\"address\":\"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"chain_id\":1,\"source_id\":\"wallet.token\",\"token\":\"0xcccccccccccccccccccccccccccccccccccccccc\"}}}}".to_owned(),
            ),
            (
                "semantic:mfm.derived:evm_read_value:1:sha256-jcs-v1:d120bf19fa0e6a1444bb5c5d0130e30e16e625cc9602f4e15b5c2db842913b1c".to_owned(),
                "schema:mfm.derived.evm_read_value:1:sha256-jcs-v1:acc179a084787b567e4420e818fae2d46a73f3c002e7addd1ffaa33292fd02be".to_owned(),
                "{\"kind\":\"token_decimals\",\"value\":18}".to_owned(),
            ),
            (
                "semantic:mfm.derived:evm_read_evidence:1:sha256-jcs-v1:7bf610e5792fc9b944eebc64af471d538f749c00627920c5f02e8c7e5d3a9c11".to_owned(),
                "schema:mfm.derived.evm_read_evidence:1:sha256-jcs-v1:6f69e83def6bc3ff3ebfcf40ff8e1eca700284ecd96c54c99519050dd27b1afb".to_owned(),
                "{\"kind\":\"returned\",\"value\":{\"intent_value_ref\":{\"content_digest\":\"content:sha256-v1:50baf2fe27e048acbdd6904ce335fcb14cfb7e07de7cc80a1b3760a59e28a544\",\"schema_id\":\"schema:mfm.derived.evm_read_intent:1:sha256-jcs-v1:2d411a262e102946452c10b3573930b45bbde3aed818f16cb746cdfcc5ac5579\"},\"value\":{\"kind\":\"token_decimals\",\"value\":18}}}".to_owned(),
            ),
        ]
    );
    assert_eq!(
        EvmTokenDecimals::semantic_id()
            .expect("semantic")
            .as_str(),
        "semantic:mfm.evm:token-decimals:1:sha256-jcs-v1:468f7f926633110f12de2f32f114cac860c70f77e809dbb2e4329b4b9cb1637b"
    );
    assert_eq!(
        schema_id::<EvmTokenDecimals>(),
        "schema:mfm.evm-token-decimals:1:sha256-jcs-v1:7243346fb6e11f265ba4840d8223ee75032ad46d04bef8cd72e68219b78aa966"
    );
}

#[test]
fn broad_evidence_rejects_cross_intent_substitution() {
    let original = intent(source("wallet.one", "aa"), anchor(17, "bb"));
    let (_, original_ref) = canonicalize_mfm_value(&original).expect("original ref");
    let evidence = EvmReadEvidence::returned(
        original_ref.clone(),
        EvmReadValue::RawUnits(EvmU256::new("7").expect("units")),
    );
    EvmBalanceRead::bind_evidence(&original_ref, &original, &evidence).expect("bound evidence");

    for substituted in [
        intent(source("wallet.two", "dd"), anchor(17, "bb")),
        intent(source("wallet.one", "aa"), anchor(18, "ee")),
    ] {
        let (_, substituted_ref) = canonicalize_mfm_value(&substituted).expect("substituted ref");
        assert!(EvmBalanceRead::bind_evidence(&substituted_ref, &substituted, &evidence).is_err());
    }
}

#[test]
fn every_broad_terminal_evidence_wire_carries_the_exact_intent_ref() {
    let intent = intent(source("wallet.one", "aa"), anchor(17, "bb"));
    let (_, intent_value_ref) = canonicalize_mfm_value(&intent).expect("intent ref");
    for (evidence, kind) in [
        (
            EvmReadEvidence::rejected(intent_value_ref.clone()),
            "rejected",
        ),
        (
            EvmReadEvidence::safe_failure(intent_value_ref.clone()),
            "safe_failure",
        ),
        (
            EvmReadEvidence::integrity_blocked(intent_value_ref.clone()),
            "integrity_blocked",
        ),
    ] {
        assert_eq!(
            serde_json::to_value(&evidence).expect("evidence wire"),
            serde_json::json!({
                "kind": kind,
                "value": { "intent_value_ref": intent_value_ref }
            })
        );
    }
}

#[test]
fn broad_read_decode_rejects_unchecked_or_incomplete_values() {
    assert!(serde_json::from_str::<EvmTokenDecimals>("31").is_err());
    assert!(serde_json::from_str::<EvmReadValue>(r#"{"kind":"chain_id","value":0}"#).is_err());
    assert!(
        serde_json::from_str::<EvmReadSubject>(r#"{"kind":"chain_identity","value":null}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<EvmReadEvidence>(r#"{"kind":"safe_failure"}"#).is_err());
    let intent = intent(source("wallet.one", "aa"), anchor(17, "bb"));
    let (_, intent_value_ref) = canonicalize_mfm_value(&intent).expect("intent ref");
    let mut evidence = serde_json::to_value(EvmReadEvidence::safe_failure(intent_value_ref))
        .expect("evidence wire");
    evidence["value"]["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<EvmReadEvidence>(evidence).is_err());

    let mut intent_wire = serde_json::to_value(&intent).expect("intent wire");
    intent_wire["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<EvmReadIntent>(intent_wire).is_err());

    let mut subject_wire = serde_json::to_value(intent.subject()).expect("subject wire");
    subject_wire["value"]["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<EvmReadSubject>(subject_wire).is_err());
}
