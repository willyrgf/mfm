//! Pure native recipe checks using the managed first-party compiler output, without chain IO.
use mfm_capabilities::EffectImplementation;
use mfm_chain::transaction::*;
use mfm_chain::{
    ContractArtifact, ContractLocator, LedgerIdentity, ObservationPoint, TransactionIdentity,
};
use mfm_evm::*;
use mfm_ids::{DigestBytes, EffectId, StableId};
use mfm_program::{ProposedStateOutcome, PureState, ResolveEffectBinding};
use mfm_values::Object;
use std::num::NonZeroU64;

#[test]
fn gas_fee_options_preserve_full_width_and_reject_invalid_decoding() {
    assert!(Eip1559Options::new(NonZeroU64::new(1).unwrap(), 2, 1).is_err());
    let options = Eip1559Options::new(NonZeroU64::new(1).unwrap(), u128::MAX, u128::MAX).unwrap();
    let object = Object::from_value(&options).unwrap();
    assert_eq!(
        object.decode::<Eip1559Options>().unwrap().max_fee_per_gas(),
        u128::MAX
    );
    for wire in [
        r#"{"gas_limit":0,"fees":{"priority":"1","maximum":"2"}}"#,
        r#"{"gas_limit":1,"fees":{"priority":"2","maximum":"1"}}"#,
        r#"{"gas_limit":1,"fees":{"priority":"01","maximum":"2"}}"#,
    ] {
        assert!(serde_json::from_str::<Eip1559Options>(wire).is_err());
    }
    assert!(EvmScalarContractArtifact::new(vec![0]).is_err());
    assert!(serde_json::from_str::<EvmScalarContractArtifact>(r#"{"initcode":"AA"}"#).is_err());
}

#[test]
#[ignore = "requires pinned solc fixture output in MFM_EFFECT_E2E_INITCODE_PATH"]
fn supported_scalar_recipes_encode_effective_value_and_reject_foreign_artifacts_and_ledgers() {
    let path =
        std::env::var_os("MFM_EFFECT_E2E_INITCODE_PATH").expect("managed compiler output path");
    let text = std::fs::read_to_string(path).unwrap();
    let text = text.trim();
    assert_eq!(text.len() % 2, 0);
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&text[offset..offset + 2], 16).unwrap())
        .collect();
    let artifact = EvmScalarContractArtifact::new(bytes.clone()).unwrap();
    let mut unsupported = bytes.clone();
    unsupported[0] ^= 1;
    assert!(EvmScalarContractArtifact::new(unsupported).is_err());
    let artifact = Object::from_value(&artifact).unwrap();
    assert_eq!(
        artifact
            .decode::<EvmScalarContractArtifact>()
            .unwrap()
            .initcode(),
        bytes
    );

    let endpoint = Object::from_value(&EvmU256::new("1").unwrap()).unwrap();
    let chain = EvmChainInstance {
        chain_id: NonZeroU64::new(1).unwrap(),
        expected_genesis_hash: EvmHash::from_bytes([1; 32]),
    };
    let binding = EvmTransactionBinding {
        authority_epoch: EvmAuthorityEpoch::new([2; 32]),
        route: EvmTransactionRoute {
            chain_instance: chain.clone(),
            endpoint_ref: endpoint.value_ref().clone(),
        },
        sender: EvmAddress::from_bytes([3; 20]),
    };
    let config = EvmContractExecutionConfig::new(
        binding.clone(),
        Eip1559Options::new(NonZeroU64::new(1_000_000).unwrap(), 1, 2).unwrap(),
        Eip1559Options::new(NonZeroU64::new(100_000).unwrap(), 1, 3).unwrap(),
    );
    let ledger = LedgerIdentity::new(Object::from_value(&chain).unwrap());
    let selected = <EvmTransactionImplementation as EffectImplementation<
        TransactionEffect<DeploymentRequest>,
    >>::implementation_id()
    .unwrap();
    let native_config = Object::from_value(&config).unwrap();
    let maximum = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    for (requested, increment, expected_word) in [
        ("42", "42", {
            let mut word = [0; 32];
            word[31] = 84;
            word
        }),
        ("0", "0", [0; 32]),
        (maximum, "0", [255; 32]),
    ] {
        let request = DeploymentRequest::new(
            ContractArtifact::new(ledger.clone(), artifact.clone()),
            ContractExecutionConfig::new(
                selected.clone(),
                StableId::new("mfm.evm.scalar-contract-read@1").unwrap(),
                Object::from_value(&binding.route)
                    .unwrap()
                    .value_ref()
                    .clone(),
                native_config.clone(),
            ),
            ConfigurationValue::new(requested).unwrap(),
            ConfigurationValue::new(increment).unwrap(),
            None,
            None,
        );
        assert_eq!(
            <EvmTransactionImplementation as ResolveEffectBinding<
                DeploymentRequest,
                TransactionEffect<DeploymentRequest>,
            >>::binding(&request)
            .unwrap(),
            binding
        );
        let creation = request.command().unwrap();
        assert!(creation.to().is_none());
        assert_eq!(creation.input(), bytes);
        assert!(creation.value().to_u128() == Some(0));
        let target = EvmAddress::from_bytes([4; 20]);
        let original = Object::from_value(&EvmHash::from_bytes([5; 32])).unwrap();
        let point = ObservationPoint::new(
            ledger.clone(),
            Object::from_value(&EvmBlockPoint::new(
                EvmU256::from_u64(1),
                EvmHash::from_bytes([6; 32]),
            ))
            .unwrap(),
        );
        let deployed = DeployedContract::new(
            request.clone(),
            request.requested().clone(),
            TransactionEvidence::new(
                EffectId::from_digest(DigestBytes::from_array([7; 32])),
                endpoint.value_ref().clone(),
                mfm_program::effect_implementation_ref::<
                    TransactionEffect<DeploymentRequest>,
                    EvmTransactionImplementation,
                >()
                .unwrap(),
                TransactionIdentity::new(ledger.clone(), original.clone()),
                point,
                original,
                TransactionResult::Applied {
                    output: ContractLocator::new(
                        ledger.clone(),
                        Object::from_value(&target).unwrap(),
                    ),
                },
            )
            .unwrap(),
        )
        .unwrap();
        let ProposedStateOutcome::Success { output: deployed } =
            CheckedAddConfigurationValue::evaluate(deployed).unwrap()
        else {
            panic!("checked addition")
        };
        let command = deployed.command().unwrap();
        assert_eq!(command.to(), Some(&target));
        assert_eq!(&command.input()[..4], &[0x1e, 0xb2, 0x5e, 0x0a]);
        assert_eq!(&command.input()[4..], &expected_word);
        assert_eq!(command.value().to_u128(), Some(0));
        assert_eq!(command.binding(), &binding);
        let mut wrong = serde_json::to_value(&request).unwrap();
        wrong["artifact"]["native"] = serde_json::to_value(&endpoint).unwrap();
        let wrong: DeploymentRequest = serde_json::from_str(&wrong.to_string()).unwrap();
        assert!(wrong.command().is_err());
        assert!(<EvmTransactionImplementation as ResolveEffectBinding<
            DeploymentRequest,
            TransactionEffect<DeploymentRequest>,
        >>::binding(&wrong)
        .is_err());
        let other_chain = EvmChainInstance {
            chain_id: NonZeroU64::new(2).unwrap(),
            ..chain.clone()
        };
        let mut wrong = serde_json::to_value(&request).unwrap();
        wrong["artifact"]["ledger"]["native"] =
            serde_json::to_value(Object::from_value(&other_chain).unwrap()).unwrap();
        assert!(
            serde_json::from_str::<DeploymentRequest>(&wrong.to_string())
                .unwrap()
                .command()
                .is_err()
        );
    }
}
