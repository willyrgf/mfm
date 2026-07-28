use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{ContentRef, DigestAlgorithm, FieldPath, SemanticTypeId, StableId};
use mfm_program::{
    config_value_contract, mfm_value_contract, state_input_value_contract, CanonicalCodec,
    CapabilityOperation, EvidenceVerdict, FactSet, NoBoundaryValue, NoContext, ObservationView,
    QualifiedInputContract, QualifiedOutputProjector, QualifiedOutputSourceContract,
    QualifiedSettlementCodecs, QualifiedSourceContract, QualifiedStateContract,
    QualifiedStateRegistration, Settlement, State, StateExecution, StateExecutionKind, StateFrame,
    VerifiedTerminalEffectView,
};
use mfm_program_derive::{MfmConfig, MfmValue, StateInput};
use mfm_spec::{
    schema_id, CertifiedInputDestination, CertifiedOutputSlot, CertifiedSettlementContract,
    CertifiedStateExecution, ComponentImplementationDescriptor, ComponentKind,
    RetainedValueContract,
};
use mfm_values::StateInput as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, MfmConfig)]
struct Config {
    value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, StateInput)]
struct Input {
    value: OutputValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, StateInput)]
struct FanInInput {
    first: OutputValue,
    second: OutputValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "program_execution_value",
    version = "1",
    schema = "mfm.test.program_execution_value"
)]
struct OutputValue {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "program_execution_failure",
    version = "1",
    schema = "mfm.test.program_execution_failure"
)]
struct Failure {
    code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "program_execution_boundary",
    version = "1",
    schema = "mfm.test.program_execution_boundary"
)]
struct Boundary {
    value: u64,
}

struct PureState;
struct ReadState;
struct EffectState;

macro_rules! common_state_types {
    () => {
        type Config = Config;
        type Context = NoContext;
        type Input = Input;
        type Output = OutputValue;
        type Failure = Failure;

        fn state_contract_ref() -> mfm_program::Result<ContentRef> {
            Ok(test_contract("state"))
        }
    };
}

impl State for PureState {
    common_state_types!();
    type Request = NoBoundaryValue;
    type Observation = NoBoundaryValue;
    type AccessFailure = NoBoundaryValue;
}

impl State for ReadState {
    common_state_types!();
    type Request = Boundary;
    type Observation = Boundary;
    type AccessFailure = Failure;
}

impl State for EffectState {
    common_state_types!();
    type Request = Boundary;
    type Observation = Boundary;
    type AccessFailure = Failure;
}

fn pure_apply(_frame: StateFrame<'_, PureState>) -> Settlement<PureState> {
    Settlement::succeeded(OutputValue { value: 1 }, FactSet::empty())
}

fn read_request(_frame: StateFrame<'_, ReadState>) -> Boundary {
    Boundary { value: 1 }
}

fn read_apply(
    _frame: StateFrame<'_, ReadState>,
    _observation: ObservationView<'_, Boundary, Failure>,
) -> EvidenceVerdict<Settlement<ReadState>> {
    EvidenceVerdict::InsufficientEvidence
}

fn effect_request(_frame: StateFrame<'_, EffectState>) -> Boundary {
    Boundary { value: 1 }
}

fn effect_settle(
    _frame: StateFrame<'_, EffectState>,
    _terminal: VerifiedTerminalEffectView<'_>,
) -> EvidenceVerdict<Settlement<EffectState>> {
    EvidenceVerdict::InvalidEvidence
}

fn pure_execution() -> StateExecution<PureState> {
    StateExecution::pure(pure_apply)
}

fn read_execution() -> mfm_program::Result<StateExecution<ReadState>> {
    Ok(StateExecution::read(
        operation("read"),
        CanonicalCodec::mfm_value(value_contract::<Boundary>("read-request")?)?,
        CanonicalCodec::mfm_value(value_contract::<Boundary>("read-observation")?)?,
        CanonicalCodec::mfm_value(value_contract::<Failure>("read-failure")?)?,
        read_request,
        read_apply,
    ))
}

fn effect_execution() -> mfm_program::Result<StateExecution<EffectState>> {
    Ok(StateExecution::effect(
        operation("effect"),
        CanonicalCodec::mfm_value(value_contract::<Boundary>("effect-request")?)?,
        value_contract::<Boundary>("effect-ensure-result")?,
        value_contract::<Boundary>("effect-terminal-evidence")?,
        value_contract::<Boundary>("effect-domain-result")?,
        effect_request,
        effect_settle,
    ))
}

fn operation(name: &str) -> CapabilityOperation {
    CapabilityOperation::new(
        test_contract("operation"),
        stable_id(name),
        test_contract("binding"),
    )
}

fn value_contract<T: mfm_values::MfmValue>(
    role: &str,
) -> mfm_program::Result<RetainedValueContract> {
    mfm_value_contract::<T>(stable_id(role), test_contract("value-evidence"))
}

fn semantic_type(name: &str) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("semantic:mfm.test:{name}:1").as_bytes()),
    )
    .expect("semantic type")
}

fn stable_id(value: &str) -> StableId {
    StableId::new(value).expect("stable id")
}

fn field_path(value: &str) -> FieldPath {
    FieldPath::new(value).expect("field path")
}

fn test_contract(label: &str) -> ContentRef {
    let json =
        serde_json::to_string(&serde_json::json!({ "test": label })).expect("test contract json");
    mfm_spec::exact_content_ref(
        schema_id("mfm.access-audit-entry.v1").expect("schema"),
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical"),
    )
    .expect("content ref")
}

fn identity(value: &OutputValue) -> &OutputValue {
    value
}

#[test]
fn state_execution_is_exactly_one_closed_case_supplied_by_qualification() {
    assert_eq!(pure_execution().kind(), StateExecutionKind::Pure);
    assert_eq!(
        read_execution().expect("read execution").kind(),
        StateExecutionKind::Read
    );
    assert_eq!(
        effect_execution().expect("effect execution").kind(),
        StateExecutionKind::Effect
    );
}

#[test]
fn callback_codec_round_trips_exact_canonical_bytes() {
    let codec = CanonicalCodec::<Boundary>::mfm_value(
        value_contract::<Boundary>("boundary").expect("boundary contract"),
    )
    .expect("boundary codec");
    let (bytes, content_ref) = codec.encode(&Boundary { value: 7 }).expect("encode");
    assert_eq!(bytes.as_str(), r#"{"value":7}"#);
    assert_eq!(content_ref.schema_id(), codec.schema_id());
    assert_eq!(
        codec.decode(bytes.as_bytes()).expect("decode"),
        Boundary { value: 7 }
    );
    assert!(codec.decode(br#"{ "value": 7 }"#).is_err());
}

#[test]
fn named_state_input_freezes_sorted_destination_paths() {
    assert_eq!(
        FanInInput::input_destination_paths().expect("destination paths"),
        vec![field_path("first"), field_path("second")]
    );
}

#[test]
fn qualified_registration_owns_the_state_execution_callbacks() {
    let state_contract_ref = PureState::state_contract_ref().expect("state contract");
    let config_contract = config_value_contract::<Config>(
        semantic_type("program-config"),
        stable_id("config"),
        test_contract("config-evidence"),
    )
    .expect("config contract");
    let input_contract = state_input_value_contract::<Input>(
        semantic_type("program-input"),
        stable_id("input"),
        test_contract("input-evidence"),
    )
    .expect("input contract");
    let output_contract = value_contract::<OutputValue>("output").expect("output contract");
    let failure_contract = value_contract::<Failure>("failure").expect("failure contract");
    let settlement_contract = CertifiedSettlementContract::new(
        Some(failure_contract.clone()),
        vec![CertifiedOutputSlot::new(
            0,
            field_path("value"),
            output_contract.clone(),
        )],
        Vec::new(),
    )
    .expect("settlement contract");
    let contract = QualifiedStateContract::new(
        state_contract_ref.clone(),
        config_contract,
        None,
        input_contract,
        vec![QualifiedInputContract::new(
            field_path("value"),
            CertifiedInputDestination::OrdinaryValue,
            QualifiedSourceContract::new(output_contract.clone(), Vec::new())
                .expect("input source contract"),
        )],
        vec![QualifiedOutputSourceContract::new(
            0,
            QualifiedSourceContract::new(output_contract.clone(), Vec::new())
                .expect("output source contract"),
        )],
        CertifiedStateExecution::Pure,
        settlement_contract,
    )
    .expect("qualified state contract");
    let descriptor = ComponentImplementationDescriptor::new(
        ComponentKind::State,
        state_contract_ref,
        test_contract("callback-surface"),
        test_contract("qualification"),
    )
    .expect("component descriptor");
    let codecs = QualifiedSettlementCodecs::new(
        vec![QualifiedOutputProjector::new(
            0,
            field_path("value"),
            CanonicalCodec::mfm_value(output_contract).expect("output codec"),
            identity,
        )],
        Some(CanonicalCodec::mfm_value(failure_contract).expect("failure codec")),
    )
    .expect("settlement codecs");

    let registration =
        QualifiedStateRegistration::new(contract, descriptor, pure_execution(), codecs)
            .expect("qualified registration");

    assert_eq!(registration.callbacks().kind(), StateExecutionKind::Pure);
    assert_eq!(
        registration.contract().input_destinations()[0].destination_field_path(),
        &field_path("value")
    );
}
