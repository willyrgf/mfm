use std::num::NonZeroU64;

use mfm_canonical::CanonicalBytes;
use mfm_capabilities::EffectCapabilityContract;
use mfm_evm::{
    Eip1559TransactionCommand, EvmAddress, EvmAuthorityEpoch, EvmBlockAnchor, EvmChainInstance,
    EvmHash, EvmTransactionBinding, EvmTransactionCompletion, EvmTransactionContext,
    EvmTransactionEffect, EvmTransactionOutcome, EvmTransactionReceipt, EvmTransactionReversion,
    EvmTransactionRoute, EvmTransactionSettlement, EvmTransactionSuccess, EvmU256,
    ExecuteEvmTransaction, EVM_TRANSACTION_EFFECT_CAPABILITY_ID, EXECUTE_EVM_TRANSACTION_STATE_ID,
    MAX_EVM_CALLDATA_BYTES, MAX_EVM_INITCODE_BYTES,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, EffectId, SchemaId};
use mfm_program::{CapabilityInjection, EffectState, ProposedStateOutcome, State};
use mfm_program_derive::MfmValue;
use mfm_values::{canonicalize_mfm_value, MfmValue as MfmValueTrait};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test",
    name = "object-context",
    version = "1",
    schema = "mfm.test-object-context"
)]
struct ObjectContext {
    step: u8,
}

fn content_ref() -> ContentRef {
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

fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).expect("nonzero fixture")
}

fn route() -> EvmTransactionRoute {
    EvmTransactionRoute::new(
        EvmChainInstance::new(nonzero(1), EvmHash::from_bytes([0xaa; 32])),
        content_ref(),
    )
}

fn binding() -> EvmTransactionBinding {
    EvmTransactionBinding::new(
        route(),
        EvmAuthorityEpoch::new([0x11; 32]),
        EvmAddress::new("0x7e5f4552091a69125d5dfcb7b8c2659029395bdf").expect("sender"),
    )
}

fn create_command() -> Eip1559TransactionCommand {
    Eip1559TransactionCommand::create(
        binding(),
        vec![1, 2, 3],
        EvmU256::from_u64(0),
        nonzero(2_000_000),
        EvmU256::from_u64(1_000_000_000),
        EvmU256::from_u64(10_000_000_000),
    )
    .expect("create command")
}

fn call_command() -> Eip1559TransactionCommand {
    Eip1559TransactionCommand::call(
        binding(),
        EvmAddress::from_bytes([0x33; 20]),
        vec![4, 5, 6],
        EvmU256::from_u64(7),
        nonzero(200_000),
        EvmU256::from_u64(1),
        EvmU256::from_u64(2),
    )
    .expect("call command")
}

fn anchor(number: u64) -> EvmBlockAnchor {
    EvmBlockAnchor::new(EvmU256::from_u64(number), EvmHash::from_bytes([0xdd; 32]))
}

fn transaction_hash(byte: u8) -> EvmHash {
    EvmHash::from_bytes([byte; 32])
}

fn effect_id(byte: u8) -> EffectId {
    EffectId::from_digest(DigestBytes::from_array([byte; 32]))
}

fn schema_id<T: MfmValueTrait>() -> String {
    T::schema_descriptor()
        .expect("descriptor")
        .schema_id()
        .expect("schema id")
        .to_string()
}

fn semantic_id<T: MfmValueTrait>() -> String {
    T::semantic_id().expect("semantic id").to_string()
}

fn canonical<T: MfmValueTrait>(value: &T) -> String {
    canonicalize_mfm_value(value)
        .expect("canonical value")
        .0
        .as_str()
        .to_owned()
}

fn assert_closed_object<T>(value: &T, required_field: &str)
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let mut missing = serde_json::to_value(value).expect("object wire");
    missing
        .as_object_mut()
        .expect("object")
        .remove(required_field);
    assert!(serde_json::from_value::<T>(missing).is_err());

    let mut unknown = serde_json::to_value(value).expect("object wire");
    unknown
        .as_object_mut()
        .expect("object")
        .insert("unknown".to_owned(), serde_json::Value::Bool(true));
    assert!(serde_json::from_value::<T>(unknown).is_err());
}

#[test]
fn checked_evm_primitives_reject_every_noncanonical_boundary() {
    let address = EvmAddress::from_bytes([0x11; 20]);
    assert_eq!(address.as_bytes(), &[0x11; 20]);
    assert_eq!(
        serde_json::to_string(&address).expect("address wire"),
        r#""0x1111111111111111111111111111111111111111""#
    );
    for invalid in [
        r#""0X1111111111111111111111111111111111111111""#,
        r#""0x111111111111111111111111111111111111111A""#,
        r#""0x1111""#,
    ] {
        assert!(serde_json::from_str::<EvmAddress>(invalid).is_err());
    }

    let hash = EvmHash::from_bytes([0xab; 32]);
    assert_eq!(hash.as_bytes(), &[0xab; 32]);
    assert_eq!(hash.to_string(), format!("0x{}", "ab".repeat(32)));
    assert!(EvmHash::new(format!("0x{}", "AB".repeat(32))).is_err());
    assert!(EvmHash::new(format!("0x{}", "ab".repeat(31))).is_err());

    for valid in [
        "0",
        "1",
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    ] {
        assert!(EvmU256::new(valid).is_ok());
    }
    for invalid in [
        "",
        "00",
        "01",
        "-1",
        "1.0",
        "115792089237316195423570985008687907853269984665640564039457584007913129639936",
    ] {
        assert!(EvmU256::new(invalid).is_err());
    }
    assert!(serde_json::from_str::<EvmU256>("1").is_err());

    let chain = EvmChainInstance::new(nonzero(1), EvmHash::from_bytes([0xab; 32]));
    assert_eq!(chain.chain_id(), nonzero(1));
    assert!(
        serde_json::from_value::<EvmChainInstance>(serde_json::json!({
            "chain_id": 0,
            "expected_genesis_hash": format!("0x{}", "ab".repeat(32)),
        }))
        .is_err()
    );

    assert_eq!(
        serde_json::to_string(&EvmAuthorityEpoch::new([0x11; 32])).expect("epoch"),
        r#""ERERERERERERERERERERERERERERERERERERERERERE""#
    );
    for invalid in [
        r#""ERERERERERERERERERERERERERERERERERERERERE""#,
        r#""ERERERERERERERERERERERERERERERERERERERERERE=""#,
    ] {
        assert!(serde_json::from_str::<EvmAuthorityEpoch>(invalid).is_err());
    }
}

#[test]
fn fixed_eip1559_command_has_exact_wire_and_checked_factories() {
    let command = create_command();
    assert_eq!(command.input(), [1, 2, 3]);
    assert_eq!(command.to(), None);
    assert_eq!(
        canonical(&command),
        r#"{"action":{"kind":"create","value":{"initcode":"AQID"}},"binding":{"authority_epoch":"ERERERERERERERERERERERERERERERERERERERERERE","route":{"chain_instance":{"chain_id":1,"expected_genesis_hash":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"endpoint_ref":{"content_digest":"content:sha256-v1:0202020202020202020202020202020202020202020202020202020202020202","schema_id":"schema:mfm.test.endpoint:1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101"}},"sender":"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"},"gas_limit":2000000,"max_fee_per_gas":"10000000000","max_priority_fee_per_gas":"1000000000","value":"0"}"#
    );

    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["gas_limit"] = serde_json::json!(0);
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());
    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["max_fee_per_gas"] = serde_json::json!("999999999");
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());
    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["action"]["value"]["initcode"] = serde_json::json!("AQID=");
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());
    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["action"]["value"]["initcode"] =
        serde_json::json!(CanonicalBytes::new(vec![0; MAX_EVM_INITCODE_BYTES + 1]).encoded());
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());

    assert!(Eip1559TransactionCommand::create(
        binding(),
        vec![0; MAX_EVM_INITCODE_BYTES],
        EvmU256::from_u64(0),
        nonzero(1),
        EvmU256::from_u64(1),
        EvmU256::from_u64(1),
    )
    .is_ok());
    assert!(Eip1559TransactionCommand::create(
        binding(),
        vec![0; MAX_EVM_INITCODE_BYTES + 1],
        EvmU256::from_u64(0),
        nonzero(1),
        EvmU256::from_u64(1),
        EvmU256::from_u64(1),
    )
    .is_err());
    assert!(Eip1559TransactionCommand::call(
        binding(),
        EvmAddress::from_bytes([1; 20]),
        vec![0; MAX_EVM_CALLDATA_BYTES + 1],
        EvmU256::from_u64(0),
        nonzero(1),
        EvmU256::from_u64(1),
        EvmU256::from_u64(1),
    )
    .is_err());
    let above_u128 =
        EvmU256::new("340282366920938463463374607431768211456").expect("valid u256 above u128");
    assert!(Eip1559TransactionCommand::create(
        binding(),
        Vec::new(),
        EvmU256::from_u64(0),
        nonzero(1),
        EvmU256::from_u64(0),
        above_u128,
    )
    .is_err());
}

#[test]
fn one_transaction_state_preserves_context_and_projects_checked_facts() {
    assert_eq!(
        ExecuteEvmTransaction::<ObjectContext>::state_id()
            .expect("state id")
            .as_str(),
        "mfm.evm.state.execute-transaction@1"
    );
    assert_eq!(
        EXECUTE_EVM_TRANSACTION_STATE_ID,
        "mfm.evm.state.execute-transaction@1"
    );
    assert_eq!(
        EvmTransactionEffect::contract_id()
            .expect("capability id")
            .as_str(),
        "mfm.evm.capability.execute-transaction@1"
    );
    assert_eq!(
        EVM_TRANSACTION_EFFECT_CAPABILITY_ID,
        "mfm.evm.capability.execute-transaction@1"
    );
    assert_eq!(
        <EvmTransactionEffect as CapabilityInjection<
            ExecuteEvmTransaction<ObjectContext>,
        >>::original_binding_ref(&binding())
        .expect("injection binding"),
        binding().binding_ref().expect("binding")
    );

    let creation = EvmTransactionContext::new(ObjectContext { step: 22 }, create_command())
        .expect("creation context");
    let calling = EvmTransactionContext::new(ObjectContext { step: 25 }, call_command())
        .expect("call context");
    assert_eq!(
        canonical(&creation),
        format!(
            r#"{{"caller_context":{{"step":22}},"command":{}}}"#,
            canonical(creation.command())
        )
    );
    assert_eq!(
        canonical(&calling),
        format!(
            r#"{{"caller_context":{{"step":25}},"command":{}}}"#,
            canonical(calling.command())
        )
    );
    assert_eq!(
        <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::prepare(
            &creation
        )
        .expect("prepared creation"),
        create_command()
    );
    assert_eq!(
        <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::prepare(
            &calling
        )
        .expect("prepared call"),
        call_command()
    );

    let created_address = EvmAddress::from_bytes([0x22; 20]);
    let created = EvmTransactionSettlement::created(
        effect_id(0x33),
        9,
        EvmTransactionReceipt::new(anchor(1), transaction_hash(0xcc)),
        created_address.clone(),
    );
    let called = EvmTransactionSettlement::called(
        effect_id(0x33),
        10,
        EvmTransactionReceipt::new(anchor(2), transaction_hash(0xee)),
    );
    let reverted = EvmTransactionSettlement::reverted(
        effect_id(0x33),
        11,
        EvmTransactionReceipt::new(anchor(3), transaction_hash(0xab)),
    );
    EvmTransactionEffect::bind_evidence(&effect_id(0x33), creation.command(), &created)
        .expect("bound creation");
    EvmTransactionEffect::bind_evidence(&effect_id(0x33), calling.command(), &called)
        .expect("bound call");
    EvmTransactionEffect::bind_evidence(&effect_id(0x33), creation.command(), &reverted)
        .expect("bound creation revert");
    EvmTransactionEffect::bind_evidence(&effect_id(0x33), calling.command(), &reverted)
        .expect("bound call revert");
    for (expected_effect, command, evidence) in [
        (effect_id(0x44), creation.command(), &created),
        (effect_id(0x33), creation.command(), &called),
        (effect_id(0x33), calling.command(), &created),
    ] {
        assert!(EvmTransactionEffect::bind_evidence(&expected_effect, command, evidence).is_err());
    }

    let ProposedStateOutcome::Success { output: deployment } =
        <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::interpret(
            creation, &created,
        )
    else {
        panic!("expected creation success")
    };
    assert_eq!(deployment.caller_context(), &ObjectContext { step: 22 });
    assert_eq!(deployment.binding(), &binding());
    assert_eq!(deployment.receipt(), created.receipt());
    assert_eq!(
        deployment.outcome().created_address(),
        Some(&created_address)
    );
    assert_eq!(
        canonical(deployment.outcome()),
        r#"{"kind":"created","value":{"created_address":"0x2222222222222222222222222222222222222222"}}"#
    );

    let call_target = call_command().to().expect("call target").clone();
    let ProposedStateOutcome::Success { output: configured } =
        <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::interpret(
            calling, &called,
        )
    else {
        panic!("expected call success")
    };
    assert_eq!(configured.caller_context(), &ObjectContext { step: 25 });
    assert_eq!(configured.receipt(), called.receipt());
    assert_eq!(configured.outcome().target(), Some(&call_target));
    assert_eq!(
        canonical(configured.outcome()),
        r#"{"kind":"called","value":{"target":"0x3333333333333333333333333333333333333333"}}"#
    );

    let ProposedStateOutcome::Failure { failure } =
        <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::interpret(
            EvmTransactionContext::new(ObjectContext { step: 26 }, call_command())
                .expect("reverted context"),
            &reverted,
        )
    else {
        panic!("expected reversion")
    };
    assert_eq!(failure.caller_context(), &ObjectContext { step: 26 });
    assert_eq!(failure.receipt(), reverted.receipt());

    let wires = [
        canonical(&deployment),
        canonical(&configured),
        canonical(&failure),
    ];
    assert_eq!(
        wires,
        [
            r#"{"binding":{"authority_epoch":"ERERERERERERERERERERERERERERERERERERERERERE","route":{"chain_instance":{"chain_id":1,"expected_genesis_hash":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"endpoint_ref":{"content_digest":"content:sha256-v1:0202020202020202020202020202020202020202020202020202020202020202","schema_id":"schema:mfm.test.endpoint:1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101"}},"sender":"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"},"caller_context":{"step":22},"outcome":{"kind":"created","value":{"created_address":"0x2222222222222222222222222222222222222222"}},"receipt":{"block_anchor":{"hash":"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","number":"1"},"transaction_hash":"0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}}"#,
            r#"{"binding":{"authority_epoch":"ERERERERERERERERERERERERERERERERERERERERERE","route":{"chain_instance":{"chain_id":1,"expected_genesis_hash":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"endpoint_ref":{"content_digest":"content:sha256-v1:0202020202020202020202020202020202020202020202020202020202020202","schema_id":"schema:mfm.test.endpoint:1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101"}},"sender":"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"},"caller_context":{"step":25},"outcome":{"kind":"called","value":{"target":"0x3333333333333333333333333333333333333333"}},"receipt":{"block_anchor":{"hash":"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","number":"2"},"transaction_hash":"0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}}"#,
            r#"{"caller_context":{"step":26},"receipt":{"block_anchor":{"hash":"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","number":"3"},"transaction_hash":"0xabababababababababababababababababababababababababababababababab"}}"#,
        ]
        .map(str::to_owned)
    );
    assert_closed_object(&deployment, "receipt");
    assert_closed_object(&configured, "outcome");
    assert_closed_object(&failure, "caller_context");
    assert_closed_object(&created, "effect_id");
    assert_closed_object(created.receipt(), "transaction_hash");
    assert_closed_object(
        &EvmTransactionContext::new(ObjectContext { step: 27 }, call_command())
            .expect("closed context"),
        "command",
    );
}

#[test]
fn transaction_identity_and_wire_ledger_is_frozen() {
    let receipt = EvmTransactionReceipt::new(anchor(1), transaction_hash(0xcc));
    let settlement = EvmTransactionSettlement::created(
        effect_id(0x33),
        9,
        receipt.clone(),
        EvmAddress::from_bytes([0x22; 20]),
    );
    assert_eq!(
        serde_json::to_string(&receipt).expect("receipt wire"),
        r#"{"block_anchor":{"hash":"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","number":"1"},"transaction_hash":"0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}"#
    );
    assert_eq!(
        canonical(&settlement),
        r#"{"effect_id":"effect:sha256-jcs-v1:3333333333333333333333333333333333333333333333333333333333333333","nonce":9,"outcome":{"kind":"created","value":{"created_address":"0x2222222222222222222222222222222222222222"}},"receipt":{"block_anchor":{"hash":"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","number":"1"},"transaction_hash":"0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}}"#
    );
    assert_eq!(
        serde_json::to_string(&EvmTransactionOutcome::Called).expect("called wire"),
        r#"{"kind":"called"}"#
    );
    assert_eq!(
        serde_json::to_string(&EvmTransactionOutcome::Reverted).expect("reverted wire"),
        r#"{"kind":"reverted"}"#
    );
    for hostile in [
        serde_json::json!({
            "effect_id": effect_id(0x33), "nonce": 9, "receipt": receipt,
            "outcome": { "kind": "called", "value": {} },
        }),
        serde_json::json!({
            "effect_id": effect_id(0x33), "nonce": 9,
            "receipt": EvmTransactionReceipt::new(anchor(1), transaction_hash(0xcc)),
            "outcome": { "kind": "created", "value": {} },
        }),
        serde_json::json!({
            "effect_id": effect_id(0x33), "nonce": 9,
            "receipt": EvmTransactionReceipt::new(anchor(1), transaction_hash(0xcc)),
            "outcome": { "kind": "reverted" }, "extra": true,
        }),
    ] {
        assert!(serde_json::from_value::<EvmTransactionSettlement>(hostile).is_err());
    }

    let vectors = [
        (
            "address",
            semantic_id::<EvmAddress>(),
            schema_id::<EvmAddress>(),
        ),
        ("hash", semantic_id::<EvmHash>(), schema_id::<EvmHash>()),
        ("uint256", semantic_id::<EvmU256>(), schema_id::<EvmU256>()),
        (
            "authority_epoch",
            semantic_id::<EvmAuthorityEpoch>(),
            schema_id::<EvmAuthorityEpoch>(),
        ),
        (
            "chain_instance",
            semantic_id::<EvmChainInstance>(),
            schema_id::<EvmChainInstance>(),
        ),
        (
            "transaction_route",
            semantic_id::<EvmTransactionRoute>(),
            schema_id::<EvmTransactionRoute>(),
        ),
        (
            "transaction_binding",
            semantic_id::<EvmTransactionBinding>(),
            schema_id::<EvmTransactionBinding>(),
        ),
        (
            "command",
            semantic_id::<Eip1559TransactionCommand>(),
            schema_id::<Eip1559TransactionCommand>(),
        ),
        (
            "context",
            semantic_id::<EvmTransactionContext<ObjectContext>>(),
            schema_id::<EvmTransactionContext<ObjectContext>>(),
        ),
        (
            "receipt",
            semantic_id::<EvmTransactionReceipt>(),
            schema_id::<EvmTransactionReceipt>(),
        ),
        (
            "outcome",
            semantic_id::<EvmTransactionOutcome>(),
            schema_id::<EvmTransactionOutcome>(),
        ),
        (
            "settlement",
            semantic_id::<EvmTransactionSettlement>(),
            schema_id::<EvmTransactionSettlement>(),
        ),
        (
            "success",
            semantic_id::<EvmTransactionSuccess>(),
            schema_id::<EvmTransactionSuccess>(),
        ),
        (
            "completion",
            semantic_id::<EvmTransactionCompletion<ObjectContext>>(),
            schema_id::<EvmTransactionCompletion<ObjectContext>>(),
        ),
        (
            "reversion",
            semantic_id::<EvmTransactionReversion<ObjectContext>>(),
            schema_id::<EvmTransactionReversion<ObjectContext>>(),
        ),
    ];
    let expected = [
        ("address", "semantic:mfm.evm:address:1:sha256-jcs-v1:b89c6610ee65059f35dc04f539319c6b713e5d3cef2a97c00fe876630c7f08eb", "schema:mfm.evm-address:1:sha256-jcs-v1:62ac7e52b8bd00da7f684795487a1faefc5d3b2536d50984fbcc884e0af88455"),
        ("hash", "semantic:mfm.evm:hash:1:sha256-jcs-v1:a85dc2819df3cf64358c13ddcfa39315d0e36f23f9e80ec55f288ebce11cd49e", "schema:mfm.evm-hash:1:sha256-jcs-v1:ac6987ba5dd6d4432514f834a5423e6602cf88e23e2f49f3bbaaedd4cd60cc23"),
        ("uint256", "semantic:mfm.evm:uint256:1:sha256-jcs-v1:aaeb34f822d1eba9d5e143f81f93a76aaeae38a4b6ca22b94c9222721309c4b5", "schema:mfm.evm-uint256:1:sha256-jcs-v1:b819c324714c76f55b4af1d74ef264635dedfa60b05737f300495b188bca0ba3"),
        ("authority_epoch", "semantic:mfm.evm:transaction-authority-epoch:1:sha256-jcs-v1:9717c31c3e5df1ebf3bc3f711a6e4a86fcb1e6b160a5ff8fcf0289d9cee479ae", "schema:mfm.evm-transaction-authority-epoch:1:sha256-jcs-v1:d0913666e83474f78371414f2692b19d99f787cbae3ac59d5c392433295ae14c"),
        ("chain_instance", "semantic:mfm.evm:chain-instance:1:sha256-jcs-v1:bd03cc792156c07fc5036a962689cfe59b026faa88af4dbe7b4e7cbc6dd22c67", "schema:mfm.evm-chain-instance:1:sha256-jcs-v1:378351854b8dd0c43b4b12c6d8f57f26e2614805ead0285a91ebc6416e432c3a"),
        ("transaction_route", "semantic:mfm.evm:transaction-route:1:sha256-jcs-v1:d463731df98a305d59ad98150a6f6b4f7ded383ecf51c6a1297c4a88421b978b", "schema:mfm.evm-transaction-route:1:sha256-jcs-v1:ed1444b8cc704f9406fc89bef4d4b43a7e02a0814ee9db5ddf2adc23f8204c5a"),
        ("transaction_binding", "semantic:mfm.evm:transaction-binding:1:sha256-jcs-v1:d79c1b7fabc0bc262e3067bda16dc3912bdac331532fbb59e0ba1def71db2458", "schema:mfm.evm-transaction-binding:1:sha256-jcs-v1:aa28e9a4ee8e3d5dcec3694ccfd9f20da75a781d9aa29fc694a9767d4858fbec"),
        ("command", "semantic:mfm.evm:eip1559-transaction-command:1:sha256-jcs-v1:72ec2fe60bd6430fa1a548442f9090d1f66e028f2e155959ef3bbd3aa141841c", "schema:mfm.evm-eip1559-transaction-command:1:sha256-jcs-v1:55ddb103fada5d9a721b50b725ff2b3a58ac87c8b01c602287eb1b98b9ab6107"),
        ("context", "semantic:mfm.evm:transaction-context:1:sha256-jcs-v1:c5588203372c522898f0ef8291fe4f4898941f1836d0da3b6f2e29546174b8ec", "schema:mfm.evm-transaction-context:1:sha256-jcs-v1:f16f306ed7f456006b2248075d288e92cbb354ff74edd1234611cd5e040625d5"),
        ("receipt", "semantic:mfm.evm:transaction-receipt:1:sha256-jcs-v1:b46061e3fdf0513d0649533a002e4b36a517ee050bff923affa315748c59cf71", "schema:mfm.evm-transaction-receipt:1:sha256-jcs-v1:f54298ed576a1705aa12b38528bbde9510603f38432fa9451d2c85b6f5b9e6fd"),
        ("outcome", "semantic:mfm.evm:transaction-outcome:1:sha256-jcs-v1:550b9bc834d9b97d80dc8b2d2f7fd59b2ced5d7fcb2bab7f5748d93ced6df6c9", "schema:mfm.evm-transaction-outcome:1:sha256-jcs-v1:d16d9083d15ada4afca3e14c6e8aac9d3ce8f0c1a455736cb7c645cfa10f8481"),
        ("settlement", "semantic:mfm.evm:transaction-settlement:1:sha256-jcs-v1:4c958b57f53af59186964196610ee7625069a22212f3df585b2d8ff0c58447d8", "schema:mfm.evm-transaction-settlement:1:sha256-jcs-v1:df543db8f3bce96f42c972180a02783e1143e1b46b6c75b11c090c223107e9d9"),
        ("success", "semantic:mfm.evm:transaction-success:1:sha256-jcs-v1:af476cdd76eb793656c566a966c4ed9c426f1501fed4bfc7a6bf6f82b6f06f71", "schema:mfm.evm-transaction-success:1:sha256-jcs-v1:755fea356c2a006f34f70f1d2eeb432c72e964ed76418469f25923d3971ecde8"),
        ("completion", "semantic:mfm.evm:transaction-completion:1:sha256-jcs-v1:5bb6e0c0121483b8300420aad81fbab183fb41d0e1870c7a38d08dc3947b585e", "schema:mfm.evm-transaction-completion:1:sha256-jcs-v1:9043fad1f1521c7c907ce3470a612d6d2a6ff1109ae1b7012973f7495ed4c53f"),
        ("reversion", "semantic:mfm.evm:transaction-reversion:1:sha256-jcs-v1:a7d9225a0700a95caca29d5356c339991e67935b96776e6a11830b39055ebec5", "schema:mfm.evm-transaction-reversion:1:sha256-jcs-v1:5f6c2ae01f3e71a91eb36f796cb46ddbc4c150a07aa8f179a86a523462913f2f"),
    ];
    for (actual, expected) in vectors.into_iter().zip(expected) {
        assert_eq!(
            actual,
            (expected.0, expected.1.to_owned(), expected.2.to_owned())
        );
    }
}
