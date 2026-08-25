use std::num::NonZeroU64;

use mfm_capabilities::ReadCapabilityContract;
use mfm_evm::{
    AnchoredContractCallCompletion, AnchoredContractCallContext, AnchoredContractCallFailure,
    AnchoredContractCallFailureReason, AnchoredContractCallResult, EvmAddress,
    EvmAnchoredContractCallRead, EvmBlockAnchor, EvmChainInstance, EvmHash, EvmReadEvidence,
    EvmReadValue, EvmTransactionRoute, EvmU256, ReadAnchoredContractCall,
    EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID, EVM_ANCHORED_CONTRACT_CALL_OPERATION_ID,
    MAX_EVM_CALL_RETURN_BYTES, READ_ANCHORED_CONTRACT_CALL_STATE_ID,
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
        context().intent().operation_and_chain_id(),
        (
            EVM_ANCHORED_CONTRACT_CALL_OPERATION_ID,
            NonZeroU64::new(1).expect("nonzero chain"),
        )
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
        "{\"caller_context\":{\"step\":4},\"intent\":{\"chain_id\":1,\"operation\":\"mfm.evm.read-anchored-contract-call@1\",\"route_ref\":{\"content_digest\":\"content:sha256-v1:5e6d16d6ccbb7892a82c6b5bc1de9beeb62d0db0a4dc915b2b6287620a65f0c5\",\"schema_id\":\"schema:mfm.evm-transaction-route:1:sha256-jcs-v1:ed1444b8cc704f9406fc89bef4d4b43a7e02a0814ee9db5ddf2adc23f8204c5a\"},\"subject\":{\"kind\":\"anchored_contract_call\",\"value\":{\"anchor\":{\"hash\":\"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"number\":\"7\"},\"calldata\":\"3q2-7w\",\"target\":\"0x3333333333333333333333333333333333333333\"}}}}"
    );
    assert_eq!(
        AnchoredContractCallContext::<ObjectContext>::semantic_id()
            .expect("object semantic")
            .as_str(),
        "semantic:mfm.evm:anchored-contract-call-context:1:sha256-jcs-v1:eabbd02f15a74bc0fa2ef813e2f22eafa3436c7ef3fb1e4ccb6597ee5cce8847"
    );
    assert_eq!(
        schema_id::<AnchoredContractCallContext<ObjectContext>>(),
        "schema:mfm.evm-anchored-contract-call-context:1:sha256-jcs-v1:91e827497f7c64089b8649e59850e5546675e5fe3b9f9bee5a340c460afeb8ca"
    );
    assert_eq!(
        AnchoredContractCallContext::<SequenceContext>::semantic_id().expect("semantic"),
        AnchoredContractCallContext::<ObjectContext>::semantic_id().expect("semantic")
    );
    assert_eq!(
        schema_id::<AnchoredContractCallContext<SequenceContext>>(),
        "schema:mfm.evm-anchored-contract-call-context:1:sha256-jcs-v1:91a3a5d7c56dbc0dfa73895ae11e4f57cc5990762e2578db6148eaaad36a27c5"
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
    let returned = EvmReadEvidence::Returned {
        value: EvmReadValue::AnchoredContractCall(result.clone()),
    };
    EvmAnchoredContractCallRead::bind_evidence(&intent_value_ref, input.intent(), &returned)
        .expect("bound result");
    for terminal in [
        EvmReadEvidence::Rejected,
        EvmReadEvidence::SafeFailure,
        EvmReadEvidence::IntegrityBlocked,
    ] {
        EvmAnchoredContractCallRead::bind_evidence(&intent_value_ref, input.intent(), &terminal)
            .expect("bound terminal evidence");
    }
    let wrong_anchor = EvmReadEvidence::Returned {
        value: EvmReadValue::AnchoredContractCall(
            AnchoredContractCallResult::new(anchor(8, "cc"), vec![]).expect("wrong anchor"),
        ),
    };
    assert!(EvmAnchoredContractCallRead::bind_evidence(
        &intent_value_ref,
        input.intent(),
        &wrong_anchor
    )
    .is_err());
    assert!(EvmAnchoredContractCallRead::bind_evidence(
        &intent_value_ref,
        input.intent(),
        &EvmReadEvidence::Returned {
            value: EvmReadValue::RawUnits(EvmU256::new("1").expect("units")),
        },
    )
    .is_err());

    let ProposedStateOutcome::Success { output } =
        <ReadAnchoredContractCall<ObjectContext> as ReadState<
            EvmAnchoredContractCallRead,
        >>::interpret(input, &returned)
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
    for (evidence, expected) in [
        (
            EvmReadEvidence::Rejected,
            AnchoredContractCallFailureReason::Rejected,
        ),
        (
            EvmReadEvidence::SafeFailure,
            AnchoredContractCallFailureReason::SafeFailure,
        ),
        (
            EvmReadEvidence::IntegrityBlocked,
            AnchoredContractCallFailureReason::IntegrityBlocked,
        ),
    ] {
        let ProposedStateOutcome::Failure { failure } =
            <ReadAnchoredContractCall<ObjectContext> as ReadState<
                EvmAnchoredContractCallRead,
            >>::interpret(context(), &evidence)
        else {
            panic!("expected failure");
        };
        assert_eq!(failure.caller_context().step, 4);
        assert_eq!(failure.reason(), expected);
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
    padded["intent"]["subject"]["value"]["calldata"] = serde_json::json!("3q2-7w=");
    assert!(serde_json::from_value::<AnchoredContractCallContext<ObjectContext>>(padded).is_err());
    let mut unknown = wire;
    unknown["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<AnchoredContractCallContext<ObjectContext>>(unknown).is_err());

    assert!(serde_json::from_str::<AnchoredContractCallFailureReason>(
        r#"{"kind":"rejected","value":null}"#
    )
    .is_err());
    assert!(serde_json::from_str::<mfm_evm::EvmReadSubject>(
        r#"{"kind":"chain_identity","value":null}"#
    )
    .is_err());
    assert!(
        serde_json::from_str::<EvmReadEvidence>(r#"{"kind":"safe_failure","value":null}"#).is_err()
    );
    let _: Option<AnchoredContractCallCompletion<ObjectContext>> = None;
    let _: Option<AnchoredContractCallFailure<ObjectContext>> = None;
}
