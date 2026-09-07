use std::num::NonZeroU64;

use mfm_capabilities::ReadCapabilityContract;
use mfm_evm::{
    AnchoredContractCallCompletion, AnchoredContractCallContext, AnchoredContractCallEvidence,
    AnchoredContractCallFailure, AnchoredContractCallFailureReason, AnchoredContractCallIntent,
    AnchoredContractCallResult, EvmAddress, EvmAnchoredContractCallRead, EvmBlockAnchor,
    EvmChainInstance, EvmHash, EvmTransactionRoute, EvmU256, ReadAnchoredContractCall,
    EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID, MAX_EVM_CALL_RETURN_BYTES,
    READ_ANCHORED_CONTRACT_CALL_STATE_ID,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_program::{CapabilityInjection, ProposedStateOutcome, ReadState, State};
use mfm_program_derive::MfmValue;
use mfm_values::{canonicalize_mfm_value, MfmValue as MfmValueTrait};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test",
    name = "anchored-object-context",
    version = "1",
    schema = "mfm.test-anchored-object-context"
)]
struct ObjectContext {
    step: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test",
    name = "anchored-sequence-context",
    version = "1",
    schema = "mfm.test-anchored-sequence-context"
)]
struct SequenceContext {
    #[mfm(persisted, minimum_items = 1, maximum_items = 4)]
    labels: Vec<String>,
}

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
    EvmTransactionRoute::new(
        EvmChainInstance::new(
            NonZeroU64::new(1).expect("nonzero chain"),
            EvmHash::new(format!("0x{}", "aa".repeat(32))).expect("genesis"),
        ),
        endpoint_ref(),
    )
}

fn anchor(number: u64, byte: &str) -> EvmBlockAnchor {
    EvmBlockAnchor::new(
        EvmU256::from_u64(number),
        EvmHash::new(format!("0x{}", byte.repeat(32))).expect("block hash"),
    )
}

fn context() -> AnchoredContractCallContext<ObjectContext> {
    AnchoredContractCallContext::for_route(
        ObjectContext { step: 4 },
        &route(),
        EvmAddress::new("0x3333333333333333333333333333333333333333").expect("target"),
        vec![0xde, 0xad, 0xbe, 0xef],
        anchor(7, "bb"),
    )
    .expect("context")
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

#[test]
fn anchored_intent_wire_and_generic_descriptor_composition_are_exact() {
    assert_eq!(
        EvmAnchoredContractCallRead::contract_id()
            .expect("capability id")
            .as_str(),
        EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID
    );
    assert_eq!(
        ReadAnchoredContractCall::<ObjectContext>::state_id()
            .expect("state id")
            .as_str(),
        READ_ANCHORED_CONTRACT_CALL_STATE_ID
    );
    assert_eq!(
        context().intent().chain_id(),
        NonZeroU64::new(1).expect("nonzero chain")
    );
    assert_eq!(
        context().intent().route_ref(),
        &route().binding_ref().expect("binding")
    );
    assert_eq!(
        <EvmAnchoredContractCallRead as CapabilityInjection<
            ReadAnchoredContractCall<ObjectContext>,
        >>::original_binding_ref(&route())
        .expect("injected binding"),
        route().binding_ref().expect("route binding")
    );

    let (canonical, _) = canonicalize_mfm_value(&context()).expect("canonical context");
    assert_eq!(
        serde_json::to_string(&route().binding_ref().expect("route ref"))
            .expect("route ref wire"),
        "{\"content_digest\":\"content:sha256-v1:5e6d16d6ccbb7892a82c6b5bc1de9beeb62d0db0a4dc915b2b6287620a65f0c5\",\"schema_id\":\"schema:mfm.evm-transaction-route:1:sha256-jcs-v1:ed1444b8cc704f9406fc89bef4d4b43a7e02a0814ee9db5ddf2adc23f8204c5a\"}"
    );
    assert_eq!(
        canonical.as_str(),
        "{\"caller_context\":{\"step\":4},\"intent\":{\"anchor\":{\"hash\":\"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"number\":\"7\"},\"calldata\":\"3q2-7w\",\"chain_id\":1,\"route_ref\":{\"content_digest\":\"content:sha256-v1:5e6d16d6ccbb7892a82c6b5bc1de9beeb62d0db0a4dc915b2b6287620a65f0c5\",\"schema_id\":\"schema:mfm.evm-transaction-route:1:sha256-jcs-v1:ed1444b8cc704f9406fc89bef4d4b43a7e02a0814ee9db5ddf2adc23f8204c5a\"},\"target\":\"0x3333333333333333333333333333333333333333\"}}"
    );
    assert_eq!(
        AnchoredContractCallContext::<ObjectContext>::semantic_id()
            .expect("object semantic")
            .as_str(),
        "semantic:mfm.evm:anchored-contract-call-context:1:sha256-jcs-v1:eabbd02f15a74bc0fa2ef813e2f22eafa3436c7ef3fb1e4ccb6597ee5cce8847"
    );
    assert_eq!(
        schema_id::<AnchoredContractCallContext<ObjectContext>>(),
        "schema:mfm.evm-anchored-contract-call-context:1:sha256-jcs-v1:347e1c132cd6b3de99b616838c00cba5fd28bb186cbe33cad5b8057d4d4c833d"
    );
    assert_eq!(
        AnchoredContractCallContext::<SequenceContext>::semantic_id().expect("semantic"),
        AnchoredContractCallContext::<ObjectContext>::semantic_id().expect("semantic")
    );
    assert_eq!(
        schema_id::<AnchoredContractCallContext<SequenceContext>>(),
        "schema:mfm.evm-anchored-contract-call-context:1:sha256-jcs-v1:26732ecc8589834f6c31c4c212cfc1944b365c69d4fcac1fd7c90a43e4bf32b4"
    );
    assert_ne!(
        schema_id::<AnchoredContractCallContext<ObjectContext>>(),
        schema_id::<AnchoredContractCallContext<SequenceContext>>()
    );
    assert_ne!(
        schema_id::<AnchoredContractCallCompletion<ObjectContext>>(),
        schema_id::<AnchoredContractCallCompletion<SequenceContext>>()
    );
    assert_ne!(
        schema_id::<AnchoredContractCallFailure<ObjectContext>>(),
        schema_id::<AnchoredContractCallFailure<SequenceContext>>()
    );
}

#[test]
fn anchored_capability_binds_only_the_exact_result_anchor() {
    let input = context();
    let prepared = <ReadAnchoredContractCall<ObjectContext> as ReadState<
        EvmAnchoredContractCallRead,
    >>::prepare(&input)
    .expect("prepared intent");
    assert_eq!(&prepared, input.intent());
    let (_, intent_value_ref) = canonicalize_mfm_value(input.intent()).expect("intent ref");

    let result =
        AnchoredContractCallResult::new(anchor(7, "bb"), vec![1, 2, 3]).expect("anchored result");
    let returned = AnchoredContractCallEvidence::returned(intent_value_ref.clone(), result.clone());
    EvmAnchoredContractCallRead::bind_evidence(&intent_value_ref, input.intent(), &returned)
        .expect("bound result");
    for terminal in [
        AnchoredContractCallEvidence::rejected(intent_value_ref.clone()),
        AnchoredContractCallEvidence::safe_failure(intent_value_ref.clone()),
        AnchoredContractCallEvidence::integrity_blocked(intent_value_ref.clone()),
    ] {
        EvmAnchoredContractCallRead::bind_evidence(&intent_value_ref, input.intent(), &terminal)
            .expect("bound terminal evidence");
    }
    let wrong_anchor = AnchoredContractCallEvidence::returned(
        intent_value_ref.clone(),
        AnchoredContractCallResult::new(anchor(8, "cc"), vec![]).expect("wrong anchor"),
    );
    assert!(EvmAnchoredContractCallRead::bind_evidence(
        &intent_value_ref,
        input.intent(),
        &wrong_anchor
    )
    .is_err());
    assert!(EvmAnchoredContractCallRead::bind_evidence(
        &route().binding_ref().expect("different value ref"),
        input.intent(),
        &returned,
    )
    .is_err());

    let ProposedStateOutcome::Success { output } =
        <ReadAnchoredContractCall<ObjectContext> as ReadState<
            EvmAnchoredContractCallRead,
        >>::interpret(input, &returned).unwrap()
    else {
        panic!("expected anchored call success");
    };
    assert_eq!(output.caller_context().step, 4);
    assert_eq!(output.result(), &result);
    assert_eq!(
        serde_json::to_string(&output).expect("completion wire"),
        "{\"caller_context\":{\"step\":4},\"result\":{\"anchor\":{\"hash\":\"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"number\":\"7\"},\"return_bytes\":\"AQID\"}}"
    );
}

#[test]
fn anchored_state_preserves_context_for_each_closed_failure() {
    let (_, intent_value_ref) = canonicalize_mfm_value(context().intent()).expect("intent ref");
    for (evidence, expected) in [
        (
            AnchoredContractCallEvidence::rejected(intent_value_ref.clone()),
            AnchoredContractCallFailureReason::Rejected,
        ),
        (
            AnchoredContractCallEvidence::safe_failure(intent_value_ref.clone()),
            AnchoredContractCallFailureReason::SafeFailure,
        ),
        (
            AnchoredContractCallEvidence::integrity_blocked(intent_value_ref),
            AnchoredContractCallFailureReason::IntegrityBlocked,
        ),
    ] {
        let ProposedStateOutcome::Failure { failure } =
            <ReadAnchoredContractCall<ObjectContext> as ReadState<
                EvmAnchoredContractCallRead,
            >>::interpret(context(), &evidence).unwrap()
        else {
            panic!("expected failure");
        };
        assert_eq!(failure.caller_context().step, 4);
        assert_eq!(failure.reason(), expected);
    }
}

#[test]
fn every_anchored_terminal_evidence_wire_carries_the_exact_intent_ref() {
    let (_, intent_value_ref) = canonicalize_mfm_value(context().intent()).expect("intent ref");
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

#[test]
fn anchored_evidence_rejects_every_cross_intent_substitution() {
    let original = context().intent().clone();
    let (_, original_ref) = canonicalize_mfm_value(&original).expect("original ref");
    let evidence = AnchoredContractCallEvidence::returned(
        original_ref.clone(),
        AnchoredContractCallResult::new(original.anchor().clone(), vec![1]).expect("result"),
    );
    EvmAnchoredContractCallRead::bind_evidence(&original_ref, &original, &evidence)
        .expect("original binding");

    let alternatives = [
        AnchoredContractCallIntent::new(
            original.chain_id(),
            endpoint_ref(),
            original.anchor().clone(),
            original.target().clone(),
            original.calldata().to_vec(),
        )
        .expect("different route"),
        AnchoredContractCallIntent::new(
            original.chain_id(),
            original.route_ref().clone(),
            anchor(8, "cc"),
            original.target().clone(),
            original.calldata().to_vec(),
        )
        .expect("different anchor"),
        AnchoredContractCallIntent::new(
            original.chain_id(),
            original.route_ref().clone(),
            original.anchor().clone(),
            EvmAddress::new("0x4444444444444444444444444444444444444444")
                .expect("different target"),
            original.calldata().to_vec(),
        )
        .expect("different target intent"),
        AnchoredContractCallIntent::new(
            original.chain_id(),
            original.route_ref().clone(),
            original.anchor().clone(),
            original.target().clone(),
            vec![0xca, 0xfe],
        )
        .expect("different calldata"),
    ];

    for alternative in alternatives {
        let (_, alternative_ref) = canonicalize_mfm_value(&alternative).expect("alternative ref");
        assert!(EvmAnchoredContractCallRead::bind_evidence(
            &alternative_ref,
            &alternative,
            &evidence,
        )
        .is_err());
    }
}

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

    let wire = serde_json::to_value(context()).expect("context wire");
    let mut wrong_operation = wire.clone();
    wrong_operation["intent"]["operation"] = serde_json::json!("mfm.evm.read-native-balance@1");
    assert!(
        serde_json::from_value::<AnchoredContractCallContext<ObjectContext>>(wrong_operation)
            .is_err()
    );
    let mut padded = wire.clone();
    padded["intent"]["calldata"] = serde_json::json!("3q2-7w=");
    assert!(serde_json::from_value::<AnchoredContractCallContext<ObjectContext>>(padded).is_err());
    let mut unknown = wire;
    unknown["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<AnchoredContractCallContext<ObjectContext>>(unknown).is_err());

    assert!(serde_json::from_str::<AnchoredContractCallFailureReason>(
        r#"{"kind":"rejected","value":null}"#
    )
    .is_err());
    assert!(serde_json::from_str::<AnchoredContractCallEvidence>(
        r#"{"kind":"safe_failure","value":null}"#
    )
    .is_err());
    let (_, intent_value_ref) = canonicalize_mfm_value(context().intent()).expect("intent ref");
    let mut evidence =
        serde_json::to_value(AnchoredContractCallEvidence::safe_failure(intent_value_ref))
            .expect("evidence wire");
    evidence["value"]["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<AnchoredContractCallEvidence>(evidence).is_err());
    let _: Option<AnchoredContractCallIntent> = None;
    let _: Option<AnchoredContractCallCompletion<ObjectContext>> = None;
    let _: Option<AnchoredContractCallFailure<ObjectContext>> = None;
}

#[test]
fn anchored_value_contracts_are_exact() {
    let input = context();
    let (_, intent_ref) = canonicalize_mfm_value(input.intent()).expect("intent ref");
    let result = AnchoredContractCallResult::new(anchor(7, "bb"), vec![1, 2, 3]).expect("result");
    let evidence = AnchoredContractCallEvidence::returned(intent_ref.clone(), result.clone());
    let ProposedStateOutcome::Success { output } =
        <ReadAnchoredContractCall<ObjectContext> as ReadState<
            EvmAnchoredContractCallRead,
        >>::interpret(input.clone(), &evidence).unwrap()
    else {
        panic!("completion");
    };
    let ProposedStateOutcome::Failure { failure } =
        <ReadAnchoredContractCall<ObjectContext> as ReadState<
            EvmAnchoredContractCallRead,
        >>::interpret(
            input.clone(),
            &AnchoredContractCallEvidence::rejected(intent_ref),
        ).unwrap()
    else {
        panic!("failure");
    };

    assert_contract(
        input.intent(),
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
        &input,
        "semantic:mfm.evm:anchored-contract-call-context:1:sha256-jcs-v1:eabbd02f15a74bc0fa2ef813e2f22eafa3436c7ef3fb1e4ccb6597ee5cce8847",
        "schema:mfm.evm-anchored-contract-call-context:1:sha256-jcs-v1:347e1c132cd6b3de99b616838c00cba5fd28bb186cbe33cad5b8057d4d4c833d",
        r#"{"caller_context":{"step":4},"intent":{"anchor":{"hash":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","number":"7"},"calldata":"3q2-7w","chain_id":1,"route_ref":{"content_digest":"content:sha256-v1:5e6d16d6ccbb7892a82c6b5bc1de9beeb62d0db0a4dc915b2b6287620a65f0c5","schema_id":"schema:mfm.evm-transaction-route:1:sha256-jcs-v1:ed1444b8cc704f9406fc89bef4d4b43a7e02a0814ee9db5ddf2adc23f8204c5a"},"target":"0x3333333333333333333333333333333333333333"}}"#,
    );
    assert_contract(
        &output,
        "semantic:mfm.evm:anchored-contract-call-completion:1:sha256-jcs-v1:4e8b29c65e720cf8ccc656bfd09776e23cd127c6ccb318b1007eef6522d69dcb",
        "schema:mfm.evm-anchored-contract-call-completion:1:sha256-jcs-v1:f10407526215ebe906d239e3229ca2cb4ba2bbed2480f2659ad64b418e3e6d12",
        r#"{"caller_context":{"step":4},"result":{"anchor":{"hash":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","number":"7"},"return_bytes":"AQID"}}"#,
    );
    assert_contract(
        &failure,
        "semantic:mfm.evm:anchored-contract-call-failure:1:sha256-jcs-v1:6826539e1501fe44c4b4d93d38a83817f3f4d4fd03033feb51abd49c6163c2ed",
        "schema:mfm.evm-anchored-contract-call-failure:1:sha256-jcs-v1:719b198d54a99565c26fbdf1ff26e72853841fc57eaa1cddad977815604479b2",
        r#"{"caller_context":{"step":4},"reason":{"kind":"rejected"}}"#,
    );
    assert_contract(
        &AnchoredContractCallFailureReason::Rejected,
        "semantic:mfm.evm:anchored-contract-call-failure-reason:1:sha256-jcs-v1:d3699ed2c84289282413c849224b29f8c24de8bf800b3c90a061a84fb320b443",
        "schema:mfm.evm-anchored-contract-call-failure-reason:1:sha256-jcs-v1:205a413c0c19108f6624dbf83e1be39d506cc5c4887d57ae3a13a97e92b38110",
        r#"{"kind":"rejected"}"#,
    );
    assert_eq!(
        schema_id::<AnchoredContractCallContext<SequenceContext>>(),
        "schema:mfm.evm-anchored-contract-call-context:1:sha256-jcs-v1:26732ecc8589834f6c31c4c212cfc1944b365c69d4fcac1fd7c90a43e4bf32b4"
    );
    assert_eq!(
        schema_id::<AnchoredContractCallCompletion<SequenceContext>>(),
        "schema:mfm.evm-anchored-contract-call-completion:1:sha256-jcs-v1:e05d47e25fa38688eddfdfccee9f2e256673baa746e308a4b2e1d9e3166f5fb7"
    );
    assert_eq!(
        schema_id::<AnchoredContractCallFailure<SequenceContext>>(),
        "schema:mfm.evm-anchored-contract-call-failure:1:sha256-jcs-v1:dc951f9ab77a1b891ca2f0601ff4e9b7d63792a7ac17ffc9b7f021c1b537477c"
    );
}
