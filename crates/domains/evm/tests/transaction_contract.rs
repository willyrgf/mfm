use mfm_canonical::CanonicalBytes;
use mfm_capabilities::EffectCapabilityContract;
use mfm_evm::{
    Eip1559TransactionCommand, EvmAddress, EvmAuthorityEpoch, EvmBlockAnchor, EvmChainInstance,
    EvmHash, EvmNonceReservationEffect, EvmTransactionBinding, EvmTransactionOutcome,
    EvmTransactionPreparationEffect, EvmTransactionReceipt, EvmTransactionRoute,
    EvmTransactionSettlement, EvmU256, NonceDomain, PreparedEvmTransaction,
    PreparedEvmTransactionEvidence, Reservation, ReservedEvmTransaction, MAX_EVM_CALLDATA_BYTES,
    MAX_EVM_INITCODE_BYTES,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, EffectId, SchemaId};
use mfm_values::{canonicalize_mfm_value, MfmValue as MfmValueTrait};
use serde::Serialize;
use std::num::NonZeroU64;

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
    EvmTransactionRoute {
        chain_instance: EvmChainInstance {
            chain_id: nonzero(1),
            expected_genesis_hash: EvmHash::from_bytes([0xaa; 32]),
        },
        endpoint_ref: content_ref(),
    }
}

fn binding() -> EvmTransactionBinding {
    EvmTransactionBinding {
        route: route(),
        authority_epoch: EvmAuthorityEpoch::new([0x11; 32]),
        sender: EvmAddress::new("0x7e5f4552091a69125d5dfcb7b8c2659029395bdf").expect("sender"),
    }
}

fn create_command() -> Eip1559TransactionCommand {
    Eip1559TransactionCommand::create(
        binding(),
        vec![1, 2, 3],
        EvmU256::from_u64(0),
        nonzero(2_000_000),
        1_000_000_000_u128,
        10_000_000_000_u128,
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
        1_u128,
        2_u128,
    )
    .expect("call command")
}

fn anchor(number: u64) -> EvmBlockAnchor {
    EvmBlockAnchor {
        number: EvmU256::from_u64(number),
        hash: EvmHash::from_bytes([0xdd; 32]),
    }
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

// Transaction decoding and direct construction must agree on fee, gas and payload constraints
// and the canonical command format.
#[test]
fn fixed_eip1559_command_has_exact_wire_and_checked_factories() {
    let command = create_command();
    assert_eq!(
        canonical(&command),
        r#"{"action":{"kind":"create","value":{"initcode":"AQID"}},"parameters":{"binding":{"authority_epoch":"ERERERERERERERERERERERERERERERERERERERERERE","route":{"chain_instance":{"chain_id":1,"expected_genesis_hash":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"endpoint_ref":{"content_digest":"content:sha256-v1:0202020202020202020202020202020202020202020202020202020202020202","schema_id":"schema:mfm.test.endpoint:1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101"}},"sender":"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"},"fees":{"maximum":"10000000000","priority":"1000000000"},"gas_limit":2000000,"value":"0"}}"#
    );

    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["parameters"]["gas_limit"] = serde_json::json!(0);
    assert!(serde_json::from_value::<Eip1559TransactionCommand>(wire).is_err());
    let mut wire = serde_json::to_value(&command).expect("wire");
    wire["parameters"]["fees"]["maximum"] = serde_json::json!("999999999");
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
        1_u128,
        1_u128,
    )
    .is_ok());
    assert!(Eip1559TransactionCommand::create(
        binding(),
        vec![0; MAX_EVM_INITCODE_BYTES + 1],
        EvmU256::from_u64(0),
        nonzero(1),
        1_u128,
        1_u128,
    )
    .is_err());
    assert!(Eip1559TransactionCommand::call(
        binding(),
        EvmAddress::from_bytes([1; 20]),
        vec![0; MAX_EVM_CALLDATA_BYTES + 1],
        EvmU256::from_u64(0),
        nonzero(1),
        1_u128,
        1_u128,
    )
    .is_err());
}

// Independent wire and identity vectors protect the persisted transaction stages from accidental
// contract changes.
#[test]
fn transaction_identity_and_wire_ledger_is_frozen() {
    let receipt = EvmTransactionReceipt {
        block_anchor: anchor(1),
        transaction_hash: transaction_hash(0xcc),
    };
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
            "receipt": EvmTransactionReceipt { block_anchor: anchor(1), transaction_hash: transaction_hash(0xcc) },
            "outcome": { "kind": "created", "value": {} },
        }),
        serde_json::json!({
            "effect_id": effect_id(0x33), "nonce": 9,
            "receipt": EvmTransactionReceipt { block_anchor: anchor(1), transaction_hash: transaction_hash(0xcc) },
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
    ];
    let expected = [
        ("address", "semantic:mfm.evm:address:1:sha256-jcs-v1:b89c6610ee65059f35dc04f539319c6b713e5d3cef2a97c00fe876630c7f08eb", "schema:mfm.evm-address:1:sha256-jcs-v1:62ac7e52b8bd00da7f684795487a1faefc5d3b2536d50984fbcc884e0af88455"),
        ("hash", "semantic:mfm.evm:hash:1:sha256-jcs-v1:a85dc2819df3cf64358c13ddcfa39315d0e36f23f9e80ec55f288ebce11cd49e", "schema:mfm.evm-hash:1:sha256-jcs-v1:ac6987ba5dd6d4432514f834a5423e6602cf88e23e2f49f3bbaaedd4cd60cc23"),
        ("uint256", "semantic:mfm.evm:uint256:1:sha256-jcs-v1:aaeb34f822d1eba9d5e143f81f93a76aaeae38a4b6ca22b94c9222721309c4b5", "schema:mfm.evm-uint256:1:sha256-jcs-v1:b819c324714c76f55b4af1d74ef264635dedfa60b05737f300495b188bca0ba3"),
        ("authority_epoch", "semantic:mfm.evm:transaction-authority-epoch:1:sha256-jcs-v1:9717c31c3e5df1ebf3bc3f711a6e4a86fcb1e6b160a5ff8fcf0289d9cee479ae", "schema:mfm.evm-transaction-authority-epoch:1:sha256-jcs-v1:d0913666e83474f78371414f2692b19d99f787cbae3ac59d5c392433295ae14c"),
        ("chain_instance", "semantic:mfm.evm:chain-instance:1:sha256-jcs-v1:bd03cc792156c07fc5036a962689cfe59b026faa88af4dbe7b4e7cbc6dd22c67", "schema:mfm.evm-chain-instance:1:sha256-jcs-v1:378351854b8dd0c43b4b12c6d8f57f26e2614805ead0285a91ebc6416e432c3a"),
        ("transaction_route", "semantic:mfm.evm:transaction-route:1:sha256-jcs-v1:d463731df98a305d59ad98150a6f6b4f7ded383ecf51c6a1297c4a88421b978b", "schema:mfm.evm-transaction-route:1:sha256-jcs-v1:ed1444b8cc704f9406fc89bef4d4b43a7e02a0814ee9db5ddf2adc23f8204c5a"),
        ("transaction_binding", "semantic:mfm.evm:transaction-binding:1:sha256-jcs-v1:d79c1b7fabc0bc262e3067bda16dc3912bdac331532fbb59e0ba1def71db2458", "schema:mfm.evm-transaction-binding:1:sha256-jcs-v1:aa28e9a4ee8e3d5dcec3694ccfd9f20da75a781d9aa29fc694a9767d4858fbec"),
        ("command", "semantic:mfm.evm:eip1559-transaction-command:1:sha256-jcs-v1:72ec2fe60bd6430fa1a548442f9090d1f66e028f2e155959ef3bbd3aa141841c", "schema:mfm.evm-eip1559-transaction-command:1:sha256-jcs-v1:238fada95370980eebf597f62ab5b57a5beea218b86199c09096785e01cb2768"),
        ("receipt", "semantic:mfm.evm:transaction-receipt:1:sha256-jcs-v1:b46061e3fdf0513d0649533a002e4b36a517ee050bff923affa315748c59cf71", "schema:mfm.evm-transaction-receipt:1:sha256-jcs-v1:f54298ed576a1705aa12b38528bbde9510603f38432fa9451d2c85b6f5b9e6fd"),
        ("outcome", "semantic:mfm.evm:transaction-outcome:1:sha256-jcs-v1:550b9bc834d9b97d80dc8b2d2f7fd59b2ced5d7fcb2bab7f5748d93ced6df6c9", "schema:mfm.evm-transaction-outcome:1:sha256-jcs-v1:d16d9083d15ada4afca3e14c6e8aac9d3ce8f0c1a455736cb7c645cfa10f8481"),
        ("settlement", "semantic:mfm.evm:transaction-settlement:1:sha256-jcs-v1:4c958b57f53af59186964196610ee7625069a22212f3df585b2d8ff0c58447d8", "schema:mfm.evm-transaction-settlement:1:sha256-jcs-v1:df543db8f3bce96f42c972180a02783e1143e1b46b6c75b11c090c223107e9d9"),
    ];
    for (actual, expected) in vectors.into_iter().zip(expected) {
        assert_eq!(
            actual,
            (expected.0, expected.1.to_owned(), expected.2.to_owned())
        );
    }
}

// A nonce reservation must belong to the exact command and sender domain, and leave room for the
// next nonce.
#[test]
fn reservation_descriptors_reject_mismatched_commands_domains_and_exhaustion() {
    let command = create_command();
    let reference = canonicalize_mfm_value(&command).unwrap().1;
    let domain = NonceDomain::from_binding(command.binding());
    assert!(Reservation::new(effect_id(1), reference.clone(), domain.clone(), u64::MAX).is_err());
    let reservation = Reservation::new(
        effect_id(1),
        reference.clone(),
        domain.clone(),
        u64::MAX - 1,
    )
    .unwrap();
    let reserved = ReservedEvmTransaction::new(command.clone(), reservation.clone()).unwrap();
    let mut wire = serde_json::to_value(&reservation).unwrap();
    wire["nonce"] = serde_json::json!(u64::MAX);
    assert!(serde_json::from_value::<Reservation>(wire).is_err());
    assert!(ReservedEvmTransaction::new(call_command(), reservation).is_err());
    let wrong_domain = NonceDomain {
        authority_epoch: domain.authority_epoch.clone(),
        chain_instance: domain.chain_instance.clone(),
        sender: EvmAddress::from_bytes([0x99; 20]),
    };
    let wrong = Reservation::new(effect_id(1), reference, wrong_domain, 0).unwrap();
    assert!(ReservedEvmTransaction::new(command, wrong.clone()).is_err());
    let mut wire = serde_json::to_value(&reserved).unwrap();
    wire["reservation"] = serde_json::to_value(wrong).unwrap();
    assert!(serde_json::from_value::<ReservedEvmTransaction>(wire).is_err());
    assert_closed_object(&reserved, "reservation");
}

// Native qualification retains command, reservation and preparation independently of the shared
// applied-result projection. Mixed nonce/hash/action/Effect identities must fail locally.
#[test]
fn native_preparation_and_settlement_reject_hostile_combinations() {
    for command in [create_command(), call_command()] {
        let command_ref = canonicalize_mfm_value(&command).unwrap().1;
        let reservation = Reservation::new(
            effect_id(1),
            command_ref.clone(),
            NonceDomain::from_binding(command.binding()),
            7,
        )
        .unwrap();
        let reservation_ref = canonicalize_mfm_value(&reservation).unwrap().1;
        EvmNonceReservationEffect::bind_evidence(
            &effect_id(1),
            &command_ref,
            &command,
            &reservation_ref,
            &reservation,
        )
        .unwrap();
        assert!(EvmNonceReservationEffect::bind_evidence(
            &effect_id(2),
            &command_ref,
            &command,
            &reservation_ref,
            &reservation
        )
        .is_err());
        let reserved = ReservedEvmTransaction::new(command.clone(), reservation.clone()).unwrap();
        let reserved_ref = canonicalize_mfm_value(&reserved).unwrap().1;
        let preparation = PreparedEvmTransactionEvidence {
            effect_id: effect_id(2),
            transaction_hash: transaction_hash(9),
        };
        let preparation_ref = canonicalize_mfm_value(&preparation).unwrap().1;
        EvmTransactionPreparationEffect::bind_evidence(
            &effect_id(2),
            &reserved_ref,
            &reserved,
            &preparation_ref,
            &preparation,
        )
        .unwrap();
        assert!(EvmTransactionPreparationEffect::bind_evidence(
            &effect_id(1),
            &reserved_ref,
            &reserved,
            &preparation_ref,
            &preparation
        )
        .is_err());
        let prepared = PreparedEvmTransaction::new(reserved, preparation.transaction_hash.clone());
        let receipt = EvmTransactionReceipt {
            block_anchor: anchor(1),
            transaction_hash: transaction_hash(9),
        };
        let settlement = if command.to().is_none() {
            EvmTransactionSettlement::created(
                effect_id(3),
                7,
                receipt.clone(),
                EvmAddress::from_bytes([3; 20]),
            )
        } else {
            EvmTransactionSettlement::called(effect_id(3), 7, receipt.clone())
        };
        settlement.qualify(&effect_id(3), &prepared).unwrap();
        assert!(settlement.qualify(&effect_id(2), &prepared).is_err());
        let decoded: PreparedEvmTransaction =
            serde_json::from_value(serde_json::to_value(&prepared).unwrap()).unwrap();
        assert_eq!(decoded.reserved().command(), &command);
        assert_eq!(decoded.reserved().reservation(), &reservation);
        assert_eq!(decoded.transaction_hash(), &preparation.transaction_hash);
        let wire = serde_json::to_value(&settlement).unwrap();
        for (field, value) in [
            ("nonce", serde_json::json!(8)),
            (
                "receipt",
                serde_json::json!({"block_anchor":anchor(1),"transaction_hash":transaction_hash(8)}),
            ),
        ] {
            let mut changed = wire.clone();
            changed[field] = value;
            let hostile: EvmTransactionSettlement = serde_json::from_value(changed).unwrap();
            assert!(hostile.qualify(&effect_id(3), &prepared).is_err());
        }
        let bad_action = if command.to().is_none() {
            EvmTransactionSettlement::called(effect_id(3), 7, receipt.clone())
        } else {
            EvmTransactionSettlement::created(
                effect_id(3),
                7,
                receipt.clone(),
                EvmAddress::from_bytes([3; 20]),
            )
        };
        assert!(bad_action.qualify(&effect_id(3), &prepared).is_err());
        EvmTransactionSettlement::reverted(effect_id(3), 7, receipt)
            .qualify(&effect_id(3), &prepared)
            .unwrap();
    }
}

#[test]
fn shared_scalar_rejection_retains_its_cause_and_native_wire_stays_exact() {
    use mfm_evm::EvmDomainError;
    use mfm_values::{Object, Unsigned256, Unsigned256Error};

    for (input, expected) in [
        ("01", Unsigned256Error::NonCanonical),
        (
            "115792089237316195423570985008687907853269984665640564039457584007913129639936",
            Unsigned256Error::OutOfRange,
        ),
    ] {
        let error = EvmU256::new(input).unwrap_err();
        assert_eq!(
            std::error::Error::source(&error).unwrap().downcast_ref(),
            Some(&expected)
        );
        let object = Object::from_value(&error).unwrap();
        assert_eq!(
            object.decode::<EvmDomainError>().unwrap(),
            EvmDomainError::Unsigned256(expected),
        );
    }
    for text in [
        "0",
        "42",
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    ] {
        let native = Object::from_value(&EvmU256::new(text).unwrap()).unwrap();
        let shared = Object::from_value(&Unsigned256::new(text).unwrap()).unwrap();
        assert_eq!(native.canonical_bytes(), shared.canonical_bytes());
        assert_ne!(native.value_ref(), shared.value_ref());
        assert_eq!(native.decode::<EvmU256>().unwrap().as_str(), text);
        assert!(native.decode::<Unsigned256>().is_err());
        assert!(shared.decode::<EvmU256>().is_err());
    }
}
