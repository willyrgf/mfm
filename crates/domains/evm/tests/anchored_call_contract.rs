use std::num::NonZeroU64;

use mfm_capabilities::ReadCapabilityContract;
use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallFailureReason, AnchoredContractCallIntent,
    AnchoredContractCallResult, AnchoredObservationFacts, Called, CheckedCallPlan,
    CheckedObservationPlan, CompletedTransactionFacts, EvmAddress, EvmAnchoredContractCallRead,
    EvmAuthorityEpoch, EvmBlockAnchor, EvmChainInstance, EvmHash, EvmTransactionBinding,
    EvmTransactionReceipt, EvmTransactionRoute, EvmTransactionSettlement, EvmU256,
    ExecutedTransactionFacts, NonceDomain, ObserveAt, PreparedEvmTransactionEvidence,
    PreparedTransactionFacts, ReadAnchoredContractCall, Reservation, ReservedEvmTransaction,
    EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID, MAX_EVM_CALL_RETURN_BYTES,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, EffectId, SchemaId};
use mfm_program::{CapabilityInjection, ProposedStateOutcome, ReadState};
use mfm_program_derive::{MfmContext, MfmValue};
use mfm_values::{canonicalize_mfm_value, MfmValue as MfmValueTrait};
use serde::{Deserialize, Serialize};

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

fn intent() -> AnchoredContractCallIntent {
    CheckedObservationPlan::new(route(), vec![0xde, 0xad, 0xbe, 0xef])
        .unwrap()
        .intent_for(EvmAddress::from_bytes([0x33; 20]), anchor(7, "bb"))
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
fn anchored_intent_route_and_capability_are_exact() {
    assert_eq!(
        EvmAnchoredContractCallRead::contract_id().unwrap().as_str(),
        EVM_ANCHORED_CONTRACT_CALL_CAPABILITY_ID
    );
    assert_eq!(intent().route_ref(), &route().binding_ref().unwrap());
    assert_eq!(intent().chain_id(), NonZeroU64::new(1).unwrap());
    assert_eq!(
        <EvmAnchoredContractCallRead as CapabilityInjection<Observe>>::original_binding_ref(
            &route()
        )
        .unwrap(),
        route().binding_ref().unwrap()
    );
}

#[test]
fn anchored_capability_binds_only_the_exact_result_anchor() {
    let input = intent();
    let (_, intent_value_ref) = canonicalize_mfm_value(&input).expect("intent ref");

    let result =
        AnchoredContractCallResult::new(anchor(7, "bb"), vec![1, 2, 3]).expect("anchored result");
    let returned = AnchoredContractCallEvidence::returned(intent_value_ref.clone(), result.clone());
    EvmAnchoredContractCallRead::bind_evidence(&intent_value_ref, &input, &returned)
        .expect("bound result");
    for terminal in [
        AnchoredContractCallEvidence::rejected(intent_value_ref.clone()),
        AnchoredContractCallEvidence::safe_failure(intent_value_ref.clone()),
        AnchoredContractCallEvidence::integrity_blocked(intent_value_ref.clone()),
    ] {
        EvmAnchoredContractCallRead::bind_evidence(&intent_value_ref, &input, &terminal)
            .expect("bound terminal evidence");
    }
    let wrong_anchor = AnchoredContractCallEvidence::returned(
        intent_value_ref.clone(),
        AnchoredContractCallResult::new(anchor(8, "cc"), vec![]).expect("wrong anchor"),
    );
    assert!(
        EvmAnchoredContractCallRead::bind_evidence(&intent_value_ref, &input, &wrong_anchor)
            .is_err()
    );
    assert!(EvmAnchoredContractCallRead::bind_evidence(
        &route().binding_ref().expect("different value ref"),
        &input,
        &returned,
    )
    .is_err());
}

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

#[test]
fn anchored_evidence_rejects_every_cross_intent_substitution() {
    let original = intent();
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.observation")]
#[serde(deny_unknown_fields)]
struct Workflow<T, O> {
    transaction: T,
    observation: O,
    unrelated: u64,
}
type Initial = Workflow<CompletedTransactionFacts<Called>, CheckedObservationPlan>;
type Observe =
    ReadAnchoredContractCall<Initial, ObserveAt<WorkflowObservationSlot, WorkflowTransactionSlot>>;

fn workflow() -> Initial {
    let binding = EvmTransactionBinding::new(
        route(),
        EvmAuthorityEpoch::new([1; 32]),
        EvmAddress::from_bytes([2; 20]),
    );
    let command = CheckedCallPlan::new(
        binding,
        vec![1],
        EvmU256::from_u64(0),
        NonZeroU64::new(1).unwrap(),
        EvmU256::from_u64(1),
        EvmU256::from_u64(2),
    )
    .unwrap()
    .command_for(EvmAddress::from_bytes([0x33; 20]));
    let reservation = Reservation::new(
        EffectId::from_digest(DigestBytes::from_array([1; 32])),
        canonicalize_mfm_value(&command).unwrap().1,
        NonceDomain::from_binding(command.binding()),
        0,
    )
    .unwrap();
    let reserved = ReservedEvmTransaction::new(command, reservation).unwrap();
    let prepared = PreparedTransactionFacts::new(
        reserved,
        PreparedEvmTransactionEvidence::new(
            EffectId::from_digest(DigestBytes::from_array([2; 32])),
            EvmHash::from_bytes([3; 32]),
        ),
    );
    let executed = ExecutedTransactionFacts::new(
        prepared,
        EvmTransactionSettlement::called(
            EffectId::from_digest(DigestBytes::from_array([3; 32])),
            0,
            EvmTransactionReceipt::new(anchor(7, "bb"), EvmHash::from_bytes([3; 32])),
        ),
    )
    .unwrap();
    Workflow {
        transaction: CompletedTransactionFacts::new(executed).unwrap(),
        observation: CheckedObservationPlan::new(route(), vec![0xde, 0xad, 0xbe, 0xef]).unwrap(),
        unrelated: 4,
    }
}

#[test]
fn observation_recipe_retains_success_and_each_failure_and_rejects_local_mismatches() {
    let input = workflow();
    assert_eq!(Observe::prepare(&input).unwrap(), intent());
    let reference = canonicalize_mfm_value(&intent()).unwrap().1;
    for evidence in [
        AnchoredContractCallEvidence::returned(
            reference.clone(),
            AnchoredContractCallResult::new(anchor(7, "bb"), vec![1, 2, 3]).unwrap(),
        ),
        AnchoredContractCallEvidence::rejected(reference.clone()),
        AnchoredContractCallEvidence::safe_failure(reference.clone()),
        AnchoredContractCallEvidence::integrity_blocked(reference.clone()),
    ] {
        let expected = AnchoredObservationFacts::new(intent(), evidence.clone()).unwrap();
        let context = match Observe::interpret(input.clone(), &evidence).unwrap() {
            ProposedStateOutcome::Success { output } => {
                assert!(expected.failure_reason().is_none());
                output
            }
            ProposedStateOutcome::Failure { failure } => {
                assert_eq!(Some(failure.reason()), expected.failure_reason());
                failure.into_context()
            }
        };
        assert_eq!(context.transaction, input.transaction);
        assert_eq!(context.unrelated, 4);
        assert_eq!(context.observation, expected);
        let wire = serde_json::to_value(&context.observation).unwrap();
        assert_eq!(
            serde_json::from_value::<AnchoredObservationFacts>(wire.clone()).unwrap(),
            expected
        );
        let mut hostile = wire;
        hostile["intent"]["target"] = serde_json::json!(EvmAddress::from_bytes([8; 20]));
        assert!(serde_json::from_value::<AnchoredObservationFacts>(hostile).is_err());
    }
    let wrong_ref = AnchoredContractCallEvidence::rejected(endpoint_ref());
    assert!(Observe::interpret(input.clone(), &wrong_ref).is_err());
    let wrong_anchor = AnchoredContractCallEvidence::returned(
        reference,
        AnchoredContractCallResult::new(anchor(8, "cc"), vec![]).unwrap(),
    );
    assert!(Observe::interpret(input.clone(), &wrong_anchor).is_err());
    for route in [
        EvmTransactionRoute::new(
            route().chain_instance().clone(),
            route().binding_ref().unwrap(),
        ),
        EvmTransactionRoute::new(
            EvmChainInstance::new(NonZeroU64::new(2).unwrap(), EvmHash::from_bytes([0xaa; 32])),
            endpoint_ref(),
        ),
    ] {
        let mut mismatch = input.clone();
        mismatch.observation = CheckedObservationPlan::new(route, vec![]).unwrap();
        assert!(Observe::prepare(&mismatch).is_err());
    }
}

#[test]
fn checked_observation_plan_bounds_survive_deserialization_and_late_anchor_selection() {
    let plan = CheckedObservationPlan::new(route(), vec![0; 131_072]).unwrap();
    let target = EvmAddress::from_bytes([1; 20]);
    assert_eq!(
        plan.intent_for(target.clone(), anchor(9, "dd")),
        AnchoredContractCallIntent::new(
            NonZeroU64::new(1).unwrap(),
            route().binding_ref().unwrap(),
            anchor(9, "dd"),
            target,
            vec![0; 131_072]
        )
        .unwrap()
    );
    assert!(CheckedObservationPlan::new(route(), vec![0; 131_073]).is_err());
    let mut wire = serde_json::to_value(&plan).unwrap();
    wire["calldata"] = serde_json::json!(mfm_canonical::CanonicalBytes::new(vec![0; 131_073]));
    assert!(serde_json::from_value::<CheckedObservationPlan>(wire).is_err());
    let mut wire = serde_json::to_value(&plan).unwrap();
    wire["chain_id"] = serde_json::json!(0);
    assert!(serde_json::from_value::<CheckedObservationPlan>(wire).is_err());
}
