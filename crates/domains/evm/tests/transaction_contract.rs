use mfm_capabilities::EffectCapabilityContract;
use mfm_evm::{
    AnchoredContractCallFailureReason, AnchoredContractCallResult, Eip1559TransactionCommand,
    EvmAddress, EvmAuthorityEpoch, EvmBlockAnchor, EvmChainInstance, EvmHash, EvmTransactionAction,
    EvmTransactionBinding, EvmTransactionConfirmation, EvmTransactionContext, EvmTransactionEffect,
    EvmTransactionRevert, EvmTransactionRoute, EvmTransactionSettlement, EvmU256,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test",
    name = "sequence-context",
    version = "1",
    schema = "mfm.test-sequence-context"
)]
struct SequenceContext {
    #[mfm(persisted, minimum_items = 1, maximum_items = 4)]
    labels: Vec<String>,
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

fn route() -> EvmTransactionRoute {
    EvmTransactionRoute::new(
        EvmChainInstance::new(
            1,
            EvmHash::new(format!("0x{}", "aa".repeat(32))).expect("genesis"),
        )
        .expect("chain"),
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
    Eip1559TransactionCommand::new(
        binding(),
        EvmTransactionAction::create(vec![1, 2, 3]).expect("create"),
        EvmU256::new("0").expect("value"),
        2_000_000,
        EvmU256::new("1000000000").expect("priority fee"),
        EvmU256::new("10000000000").expect("max fee"),
    )
    .expect("command")
}

fn anchor(number: u64) -> EvmBlockAnchor {
    EvmBlockAnchor::new(
        EvmU256::from_u64(number),
        EvmHash::new(format!("0x{}", "dd".repeat(32))).expect("block hash"),
    )
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

#[test]
fn checked_evm_primitives_reject_every_noncanonical_boundary() {
    assert!(EvmAddress::new("0x1111111111111111111111111111111111111111").is_ok());
    for invalid in [
        "0X1111111111111111111111111111111111111111",
        "0x111111111111111111111111111111111111111A",
        "0x1111",
    ] {
        assert!(EvmAddress::new(invalid).is_err());
    }

    assert!(EvmHash::new(format!("0x{}", "ab".repeat(32))).is_ok());
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

    let genesis = format!("0x{}", "ab".repeat(32));
    assert!(EvmChainInstance::new(1, EvmHash::new(&genesis).expect("genesis")).is_ok());
    assert!(EvmChainInstance::new(0, EvmHash::new(&genesis).expect("genesis")).is_err());
    assert!(
        serde_json::from_value::<EvmChainInstance>(serde_json::json!({
            "chain_id": 0,
            "expected_genesis_hash": genesis,
        }))
        .is_err()
    );

    let epoch_json = serde_json::to_string(&EvmAuthorityEpoch::new([0x11; 32])).expect("epoch");
    assert_eq!(
        epoch_json,
        "\"ERERERERERERERERERERERERERERERERERERERERERE\""
    );
    for invalid in [
        "\"ERERERERERERERERERERERERERERERERERERERERE\"",
        "\"ERERERERERERERERERERERERERERERERERERERERERE=\"",
    ] {
        assert!(serde_json::from_str::<EvmAuthorityEpoch>(invalid).is_err());
    }

    assert!(EvmTransactionAction::create(vec![0; MAX_EVM_INITCODE_BYTES]).is_ok());
    assert!(EvmTransactionAction::create(vec![0; MAX_EVM_INITCODE_BYTES + 1]).is_err());
    let target = EvmAddress::new("0x1111111111111111111111111111111111111111").expect("target");
    assert!(EvmTransactionAction::call(target.clone(), vec![0; MAX_EVM_CALLDATA_BYTES]).is_ok());
    assert!(EvmTransactionAction::call(target, vec![0; MAX_EVM_CALLDATA_BYTES + 1]).is_err());
}

#[test]
fn fixed_eip1559_command_has_exact_wire_and_rejects_shape_or_relationship_drift() {
    let command = create_command();
    let (canonical, _) = canonicalize_mfm_value(&command).expect("canonical command");
    assert_eq!(
        canonical.as_str(),
        "{\"action\":{\"kind\":\"create\",\"value\":{\"initcode\":\"AQID\"}},\"binding\":{\"authority_epoch\":\"ERERERERERERERERERERERERERERERERERERERERERE\",\"route\":{\"chain_instance\":{\"chain_id\":1,\"expected_genesis_hash\":\"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"},\"endpoint_ref\":{\"content_digest\":\"content:sha256-v1:0202020202020202020202020202020202020202020202020202020202020202\",\"schema_id\":\"schema:mfm.test.endpoint:1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101\"}},\"sender\":\"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf\"},\"gas_limit\":2000000,\"max_fee_per_gas\":\"10000000000\",\"max_priority_fee_per_gas\":\"1000000000\",\"value\":\"0\"}"
    );

    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["gas_limit"] = serde_json::json!(0);
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());
    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["max_fee_per_gas"] = serde_json::json!("999999999");
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());
    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["nonce"] = serde_json::json!(1);
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());
    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["action"]["value"]["initcode"] = serde_json::json!("AQID=");
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());
    assert!(serde_json::from_str::<EvmTransactionSettlement>(
        r#"{"kind":"reverted","value":null}"#
    )
    .is_err());
    assert!(serde_json::from_str::<EvmTransactionConfirmation>(
        r#"{"kind":"called","value":null}"#
    )
    .is_err());
}

#[test]
fn effect_binding_and_interpretation_are_exact_and_context_preserving() {
    assert_eq!(
        EvmTransactionEffect::contract_id()
            .expect("capability id")
            .as_str(),
        EVM_TRANSACTION_EFFECT_CAPABILITY_ID
    );
    assert_eq!(
        ExecuteEvmTransaction::<ObjectContext>::state_id()
            .expect("state id")
            .as_str(),
        EXECUTE_EVM_TRANSACTION_STATE_ID
    );
    assert_eq!(
        <EvmTransactionEffect as CapabilityInjection<
            ExecuteEvmTransaction<ObjectContext>,
        >>::original_binding_ref(&binding())
        .expect("injected binding"),
        binding().binding_ref().expect("binding ref")
    );

    let command = create_command();
    let input = EvmTransactionContext::new(ObjectContext { step: 7 }, command.clone());
    assert_eq!(
        <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::prepare(
            &input
        )
        .expect("prepared command"),
        command
    );
    let settlement = EvmTransactionSettlement::confirmed(
        effect_id(0x33),
        9,
        EvmTransactionConfirmation::Created {
            block_anchor: anchor(1),
            created_address: EvmAddress::new("0x2222222222222222222222222222222222222222")
                .expect("created address"),
            transaction_hash: EvmHash::new(format!("0x{}", "cc".repeat(32)))
                .expect("transaction hash"),
        },
    );
    EvmTransactionEffect::bind_evidence(&effect_id(0x33), &command, &settlement)
        .expect("bound evidence");
    assert!(EvmTransactionEffect::bind_evidence(&effect_id(0x44), &command, &settlement).is_err());

    let call_command = Eip1559TransactionCommand::new(
        binding(),
        EvmTransactionAction::call(
            EvmAddress::new("0x3333333333333333333333333333333333333333").expect("target"),
            vec![],
        )
        .expect("call"),
        EvmU256::new("0").expect("value"),
        200_000,
        EvmU256::new("1").expect("priority"),
        EvmU256::new("2").expect("max"),
    )
    .expect("call command");
    assert!(
        EvmTransactionEffect::bind_evidence(&effect_id(0x33), &call_command, &settlement).is_err()
    );
    let call_settlement = EvmTransactionSettlement::confirmed(
        effect_id(0x33),
        10,
        EvmTransactionConfirmation::Called {
            block_anchor: anchor(2),
            transaction_hash: EvmHash::new(format!("0x{}", "ee".repeat(32)))
                .expect("transaction hash"),
        },
    );
    EvmTransactionEffect::bind_evidence(&effect_id(0x33), &call_command, &call_settlement)
        .expect("bound call evidence");
    assert!(
        EvmTransactionEffect::bind_evidence(&effect_id(0x33), &command, &call_settlement).is_err()
    );
    let ProposedStateOutcome::Success {
        output: call_output,
    } = <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::interpret(
        EvmTransactionContext::new(ObjectContext { step: 6 }, call_command),
        &call_settlement,
    )
    else {
        panic!("expected call success");
    };
    assert_eq!(
        serde_json::to_string(&call_output).expect("call output"),
        "{\"caller_context\":{\"step\":6},\"confirmed\":{\"kind\":\"called\",\"value\":{\"block_anchor\":{\"hash\":\"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"number\":\"2\"},\"transaction_hash\":\"0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\"}}}"
    );

    let ProposedStateOutcome::Success { output } =
        <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::interpret(
            input,
            &settlement,
        )
    else {
        panic!("expected success");
    };
    let output_json = serde_json::to_string(&output).expect("output");
    assert_eq!(
        output_json,
        "{\"caller_context\":{\"step\":7},\"confirmed\":{\"kind\":\"created\",\"value\":{\"block_anchor\":{\"hash\":\"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"number\":\"1\"},\"created_address\":\"0x2222222222222222222222222222222222222222\",\"transaction_hash\":\"0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"}}}"
    );
    assert!(!output_json.contains("effect_id"));
    assert!(!output_json.contains("nonce"));
    assert!(!output_json.contains("command"));

    let reverted = EvmTransactionSettlement::reverted(
        effect_id(0x33),
        9,
        EvmTransactionRevert::new(
            anchor(2),
            EvmHash::new(format!("0x{}", "cc".repeat(32))).expect("transaction hash"),
        ),
    );
    let ProposedStateOutcome::Failure { failure } =
        <ExecuteEvmTransaction<ObjectContext> as EffectState<EvmTransactionEffect>>::interpret(
            EvmTransactionContext::new(ObjectContext { step: 8 }, command),
            &reverted,
        )
    else {
        panic!("expected revert");
    };
    assert_eq!(failure.caller_context().step, 8);
    assert_eq!(failure.reverted().block_anchor(), &anchor(2));
}

#[test]
fn public_transaction_value_identities_are_frozen() {
    let result = AnchoredContractCallResult::new(anchor(3), vec![1, 2]).expect("call result");
    let settlement = EvmTransactionSettlement::reverted(
        effect_id(0x33),
        9,
        EvmTransactionRevert::new(
            anchor(1),
            EvmHash::new(format!("0x{}", "cc".repeat(32))).expect("transaction hash"),
        ),
    );
    let confirmation: EvmTransactionConfirmation = serde_json::from_value(serde_json::json!({
        "kind": "called",
        "value": {
            "block_anchor": anchor(1),
            "transaction_hash": format!("0x{}", "cc".repeat(32)),
        }
    }))
    .expect("confirmation");
    let revert: EvmTransactionRevert = serde_json::from_value(serde_json::json!({
        "block_anchor": anchor(1),
        "transaction_hash": format!("0x{}", "cc".repeat(32)),
    }))
    .expect("revert");
    assert_eq!(
        serde_json::to_string(&settlement).expect("settlement wire"),
        "{\"kind\":\"reverted\",\"value\":{\"effect_id\":\"effect:sha256-jcs-v1:3333333333333333333333333333333333333333333333333333333333333333\",\"nonce\":9,\"revert\":{\"block_anchor\":{\"hash\":\"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"number\":\"1\"},\"transaction_hash\":\"0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"}}}"
    );
    assert_eq!(
        serde_json::to_string(&confirmation).expect("confirmation wire"),
        "{\"kind\":\"called\",\"value\":{\"block_anchor\":{\"hash\":\"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"number\":\"1\"},\"transaction_hash\":\"0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"}}"
    );
    assert_eq!(
        serde_json::to_string(&revert).expect("revert wire"),
        "{\"block_anchor\":{\"hash\":\"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"number\":\"1\"},\"transaction_hash\":\"0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"}"
    );
    assert_eq!(
        serde_json::to_string(&result).expect("anchored result wire"),
        "{\"anchor\":{\"hash\":\"0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"number\":\"3\"},\"return_bytes\":\"AQI\"}"
    );
    assert_eq!(
        serde_json::to_string(&AnchoredContractCallFailureReason::Rejected)
            .expect("failure reason wire"),
        "{\"kind\":\"rejected\"}"
    );

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
            "settlement",
            semantic_id::<EvmTransactionSettlement>(),
            schema_id::<EvmTransactionSettlement>(),
        ),
        (
            "confirmation",
            semantic_id::<EvmTransactionConfirmation>(),
            schema_id::<EvmTransactionConfirmation>(),
        ),
        (
            "revert",
            semantic_id::<EvmTransactionRevert>(),
            schema_id::<EvmTransactionRevert>(),
        ),
        (
            "anchored_result",
            semantic_id::<AnchoredContractCallResult>(),
            schema_id::<AnchoredContractCallResult>(),
        ),
        (
            "anchored_failure_reason",
            semantic_id::<AnchoredContractCallFailureReason>(),
            schema_id::<AnchoredContractCallFailureReason>(),
        ),
    ];
    let expected = [
        (
            "address",
            "semantic:mfm.evm:address:1:sha256-jcs-v1:b89c6610ee65059f35dc04f539319c6b713e5d3cef2a97c00fe876630c7f08eb",
            "schema:mfm.evm-address:1:sha256-jcs-v1:62ac7e52b8bd00da7f684795487a1faefc5d3b2536d50984fbcc884e0af88455",
        ),
        (
            "hash",
            "semantic:mfm.evm:hash:1:sha256-jcs-v1:a85dc2819df3cf64358c13ddcfa39315d0e36f23f9e80ec55f288ebce11cd49e",
            "schema:mfm.evm-hash:1:sha256-jcs-v1:ac6987ba5dd6d4432514f834a5423e6602cf88e23e2f49f3bbaaedd4cd60cc23",
        ),
        (
            "uint256",
            "semantic:mfm.evm:uint256:1:sha256-jcs-v1:aaeb34f822d1eba9d5e143f81f93a76aaeae38a4b6ca22b94c9222721309c4b5",
            "schema:mfm.evm-uint256:1:sha256-jcs-v1:b819c324714c76f55b4af1d74ef264635dedfa60b05737f300495b188bca0ba3",
        ),
        (
            "authority_epoch",
            "semantic:mfm.evm:transaction-authority-epoch:1:sha256-jcs-v1:9717c31c3e5df1ebf3bc3f711a6e4a86fcb1e6b160a5ff8fcf0289d9cee479ae",
            "schema:mfm.evm-transaction-authority-epoch:1:sha256-jcs-v1:d0913666e83474f78371414f2692b19d99f787cbae3ac59d5c392433295ae14c",
        ),
        (
            "chain_instance",
            "semantic:mfm.evm:chain-instance:1:sha256-jcs-v1:bd03cc792156c07fc5036a962689cfe59b026faa88af4dbe7b4e7cbc6dd22c67",
            "schema:mfm.evm-chain-instance:1:sha256-jcs-v1:378351854b8dd0c43b4b12c6d8f57f26e2614805ead0285a91ebc6416e432c3a",
        ),
        (
            "transaction_route",
            "semantic:mfm.evm:transaction-route:1:sha256-jcs-v1:d463731df98a305d59ad98150a6f6b4f7ded383ecf51c6a1297c4a88421b978b",
            "schema:mfm.evm-transaction-route:1:sha256-jcs-v1:ed1444b8cc704f9406fc89bef4d4b43a7e02a0814ee9db5ddf2adc23f8204c5a",
        ),
        (
            "transaction_binding",
            "semantic:mfm.evm:transaction-binding:1:sha256-jcs-v1:d79c1b7fabc0bc262e3067bda16dc3912bdac331532fbb59e0ba1def71db2458",
            "schema:mfm.evm-transaction-binding:1:sha256-jcs-v1:aa28e9a4ee8e3d5dcec3694ccfd9f20da75a781d9aa29fc694a9767d4858fbec",
        ),
        (
            "command",
            "semantic:mfm.evm:eip1559-transaction-command:1:sha256-jcs-v1:72ec2fe60bd6430fa1a548442f9090d1f66e028f2e155959ef3bbd3aa141841c",
            "schema:mfm.evm-eip1559-transaction-command:1:sha256-jcs-v1:55ddb103fada5d9a721b50b725ff2b3a58ac87c8b01c602287eb1b98b9ab6107",
        ),
        (
            "settlement",
            "semantic:mfm.evm:transaction-settlement:1:sha256-jcs-v1:4c958b57f53af59186964196610ee7625069a22212f3df585b2d8ff0c58447d8",
            "schema:mfm.evm-transaction-settlement:1:sha256-jcs-v1:420eb72f41c6174f437789953d480a70749eba108f7ad857ff2e9876620105d0",
        ),
        (
            "confirmation",
            "semantic:mfm.evm:transaction-confirmation:1:sha256-jcs-v1:85ca70680feabbb9b3c1c5057db6b11548a89d8e67e127fa42d27ebf343569c7",
            "schema:mfm.evm-transaction-confirmation:1:sha256-jcs-v1:e2c1a18ee304d660a340b237b4d1e719884339c3a1f2b9750ea6634973904a6c",
        ),
        (
            "revert",
            "semantic:mfm.evm:transaction-revert:1:sha256-jcs-v1:c747a8f580ba7ea9b8a3cb1e5073882883014ba3199ee0bdabd818925c244347",
            "schema:mfm.evm-transaction-revert:1:sha256-jcs-v1:ef32e95ebf7345a7d40cc987429febc6191efbd89eead988e4197fbbf00068be",
        ),
        (
            "anchored_result",
            "semantic:mfm.evm:anchored-contract-call-result:1:sha256-jcs-v1:b71f0e91e89c9346fa9631a7346dbbb5275b3f7c111907c550478b9b56384291",
            "schema:mfm.evm-anchored-contract-call-result:1:sha256-jcs-v1:df8d201bec7917b8b215e46472b1cc66b46c0e97e14797626c7310abec7c83d0",
        ),
        (
            "anchored_failure_reason",
            "semantic:mfm.evm:anchored-contract-call-failure-reason:1:sha256-jcs-v1:d3699ed2c84289282413c849224b29f8c24de8bf800b3c90a061a84fb320b443",
            "schema:mfm.evm-anchored-contract-call-failure-reason:1:sha256-jcs-v1:205a413c0c19108f6624dbf83e1be39d506cc5c4887d57ae3a13a97e92b38110",
        ),
    ];
    for (actual, expected) in vectors.into_iter().zip(expected) {
        assert_eq!(
            actual,
            (expected.0, expected.1.to_owned(), expected.2.to_owned())
        );
    }

    assert_eq!(
        EvmTransactionContext::<ObjectContext>::semantic_id().expect("object context semantic"),
        EvmTransactionContext::<SequenceContext>::semantic_id().expect("sequence context semantic")
    );
    assert_ne!(
        schema_id::<EvmTransactionContext<ObjectContext>>(),
        schema_id::<EvmTransactionContext<SequenceContext>>()
    );
    assert_ne!(
        schema_id::<mfm_evm::EvmTransactionCompletion<ObjectContext>>(),
        schema_id::<mfm_evm::EvmTransactionCompletion<SequenceContext>>()
    );
    assert_ne!(
        schema_id::<mfm_evm::EvmTransactionReversion<ObjectContext>>(),
        schema_id::<mfm_evm::EvmTransactionReversion<SequenceContext>>()
    );
}
