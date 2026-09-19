use std::num::NonZeroU64;

use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallFailureReason, AnchoredContractCallIntent,
    AnchoredContractCallResult, EvmAddress, EvmBlockAnchor, EvmChainInstance, EvmHash,
    EvmTransactionRoute, EvmU256, MAX_EVM_CALLDATA_BYTES, MAX_EVM_CALL_RETURN_BYTES,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_values::{canonicalize_mfm_value, MfmValue as MfmValueTrait};

fn endpoint_ref() -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.endpoint",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        )
        .expect("schema"),
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([2; 32])),
    )
    .expect("endpoint ref")
}

fn route() -> EvmTransactionRoute {
    EvmTransactionRoute {
        chain_instance: EvmChainInstance {
            chain_id: NonZeroU64::new(1).expect("nonzero chain"),
            expected_genesis_hash: EvmHash::new(format!("0x{}", "aa".repeat(32))).expect("genesis"),
        },
        endpoint_ref: endpoint_ref(),
    }
}

fn anchor(number: u64, byte: &str) -> EvmBlockAnchor {
    EvmBlockAnchor {
        number: EvmU256::from_u64(number),
        hash: EvmHash::new(format!("0x{}", byte.repeat(32))).expect("block hash"),
    }
}

fn intent() -> AnchoredContractCallIntent {
    AnchoredContractCallIntent::new(
        NonZeroU64::new(1).unwrap(),
        route().binding_ref().unwrap(),
        anchor(7, "bb"),
        EvmAddress::from_bytes([0x33; 20]),
        vec![0xde, 0xad, 0xbe, 0xef],
    )
    .unwrap()
}

fn schema_id<T: MfmValueTrait>() -> String {
    T::schema_descriptor()
        .expect("descriptor")
        .schema_id()
        .expect("schema id")
        .to_string()
}

fn assert_contract<T: MfmValueTrait>(
    value: &T,
    semantic_id: &str,
    schema_id: &str,
    canonical: &str,
) {
    assert_eq!(T::semantic_id().expect("semantic id").as_str(), semantic_id);
    assert_eq!(self::schema_id::<T>(), schema_id);
    assert_eq!(
        canonicalize_mfm_value(value)
            .expect("canonical value")
            .0
            .as_str(),
        canonical
    );
}

// Even unsuccessful anchored-call evidence must identify the request it answers in the persisted
// wire.
#[test]
fn every_anchored_terminal_evidence_wire_carries_the_exact_intent_ref() {
    let (_, intent_value_ref) = canonicalize_mfm_value(&intent()).expect("intent ref");
    for (evidence, kind) in [
        (
            AnchoredContractCallEvidence::rejected(intent_value_ref.clone()),
            "rejected",
        ),
        (
            AnchoredContractCallEvidence::safe_failure(intent_value_ref.clone()),
            "safe_failure",
        ),
        (
            AnchoredContractCallEvidence::integrity_blocked(intent_value_ref.clone()),
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

// Deserialization must enforce call bounds and the current closed wire format instead of
// bypassing typed validation.
#[test]
fn anchored_bytes_and_decode_paths_enforce_every_bound_and_closed_shape() {
    assert!(
        AnchoredContractCallResult::new(anchor(1, "aa"), vec![0; MAX_EVM_CALL_RETURN_BYTES])
            .is_ok()
    );
    assert!(AnchoredContractCallResult::new(
        anchor(1, "aa"),
        vec![0; MAX_EVM_CALL_RETURN_BYTES + 1]
    )
    .is_err());

    let native = intent();
    let maximum = AnchoredContractCallIntent::new(
        native.chain_id(),
        native.route_ref().clone(),
        native.anchor().clone(),
        native.target().clone(),
        vec![0; MAX_EVM_CALLDATA_BYTES],
    )
    .unwrap();
    let maximum_wire = serde_json::to_value(&maximum).unwrap();
    assert_eq!(
        serde_json::from_value::<AnchoredContractCallIntent>(maximum_wire.clone()).unwrap(),
        maximum
    );
    let mut oversized = maximum_wire;
    oversized["calldata"] = serde_json::json!(mfm_canonical::CanonicalBytes::new(vec![
        0;
        MAX_EVM_CALLDATA_BYTES
            + 1
    ]));
    assert!(serde_json::from_value::<AnchoredContractCallIntent>(oversized).is_err());
    assert!(AnchoredContractCallIntent::new(
        native.chain_id(),
        native.route_ref().clone(),
        native.anchor().clone(),
        native.target().clone(),
        vec![0; MAX_EVM_CALLDATA_BYTES + 1]
    )
    .is_err());
    let wire = serde_json::to_value(intent()).expect("context wire");
    let mut wrong_operation = wire.clone();
    wrong_operation["operation"] = serde_json::json!("mfm.evm.read-native-balance@1");
    assert!(serde_json::from_value::<AnchoredContractCallIntent>(wrong_operation).is_err());
    let mut padded = wire.clone();
    padded["calldata"] = serde_json::json!("3q2-7w=");
    assert!(serde_json::from_value::<AnchoredContractCallIntent>(padded).is_err());
    let mut unknown = wire;
    unknown["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<AnchoredContractCallIntent>(unknown).is_err());

    assert!(serde_json::from_str::<AnchoredContractCallFailureReason>(
        r#"{"kind":"rejected","value":null}"#
    )
    .is_err());
    assert!(serde_json::from_str::<AnchoredContractCallEvidence>(
        r#"{"kind":"safe_failure","value":null}"#
    )
    .is_err());
    let (_, intent_value_ref) = canonicalize_mfm_value(&intent()).expect("intent ref");
    let mut evidence =
        serde_json::to_value(AnchoredContractCallEvidence::safe_failure(intent_value_ref))
            .expect("evidence wire");
    evidence["value"]["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<AnchoredContractCallEvidence>(evidence).is_err());
}

// Fixed schema identities and canonical bytes protect the persisted anchored-call format from
// accidental drift.
#[test]
fn anchored_value_contracts_are_exact() {
    let input = intent();
    let (_, intent_ref) = canonicalize_mfm_value(&input).expect("intent ref");
    let result = AnchoredContractCallResult::new(anchor(7, "bb"), vec![1, 2, 3]).expect("result");
    let evidence = AnchoredContractCallEvidence::returned(intent_ref.clone(), result.clone());
    assert_contract(
        &input,
        "semantic:mfm.evm:anchored-contract-call-intent:1:sha256-jcs-v1:3db24e67e0da07d64f8dd59f4de2d70f56ad8ca9e822890f453bf00aa65221ce",
        "schema:mfm.evm-anchored-contract-call-intent:1:sha256-jcs-v1:7b3476cd022d1980dec5c27632eb19f9ae9e5dcfbb0a19e7d4eb2d92bbc0bc6e",
        r#"{"anchor":{"hash":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","number":"7"},"calldata":"3q2-7w","chain_id":1,"route_ref":{"content_digest":"content:sha256-v1:5e6d16d6ccbb7892a82c6b5bc1de9beeb62d0db0a4dc915b2b6287620a65f0c5","schema_id":"schema:mfm.evm-transaction-route:1:sha256-jcs-v1:ed1444b8cc704f9406fc89bef4d4b43a7e02a0814ee9db5ddf2adc23f8204c5a"},"target":"0x3333333333333333333333333333333333333333"}"#,
    );
    assert_contract(
        &evidence,
        "semantic:mfm.evm:anchored-contract-call-evidence:1:sha256-jcs-v1:7637d00c0e8a23a5a51625ce1e9731db24c66a495bc311fefaa5a5ec5af85095",
        "schema:mfm.evm-anchored-contract-call-evidence:1:sha256-jcs-v1:26fa68e349f854eee2322b0c44d3be4023f564ff1c6d387f22402509e9a30829",
        r#"{"kind":"returned","value":{"intent_value_ref":{"content_digest":"content:sha256-v1:dd5d0386125000a092ac88c1ecacb3f7c85c4df97af9d6c4aa626593efcad43c","schema_id":"schema:mfm.evm-anchored-contract-call-intent:1:sha256-jcs-v1:7b3476cd022d1980dec5c27632eb19f9ae9e5dcfbb0a19e7d4eb2d92bbc0bc6e"},"result":{"anchor":{"hash":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","number":"7"},"return_bytes":"AQID"}}}"#,
    );
    assert_contract(
        &result,
        "semantic:mfm.evm:anchored-contract-call-result:1:sha256-jcs-v1:b71f0e91e89c9346fa9631a7346dbbb5275b3f7c111907c550478b9b56384291",
        "schema:mfm.evm-anchored-contract-call-result:1:sha256-jcs-v1:df8d201bec7917b8b215e46472b1cc66b46c0e97e14797626c7310abec7c83d0",
        r#"{"anchor":{"hash":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","number":"7"},"return_bytes":"AQID"}"#,
    );
    assert_contract(
        &AnchoredContractCallFailureReason::Rejected,
        "semantic:mfm.evm:anchored-contract-call-failure-reason:1:sha256-jcs-v1:d3699ed2c84289282413c849224b29f8c24de8bf800b3c90a061a84fb320b443",
        "schema:mfm.evm-anchored-contract-call-failure-reason:1:sha256-jcs-v1:205a413c0c19108f6624dbf83e1be39d506cc5c4887d57ae3a13a97e92b38110",
        r#"{"kind":"rejected"}"#,
    );
}
