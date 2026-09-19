use mfm_chain::transaction::{
    CheckedAddConfigurationValue, ConfigurationApplied, ConfigurationValue, Configure,
    ConfiguredContract, ContractExecutionConfig, DeployedContract, DeploymentRequest,
    PreparedTransaction, TransactionEffect, TransactionEvidence, TransactionResult,
};
use mfm_chain::{
    ContractArtifact, ContractLocator, LedgerIdentity, ObservationPoint, TransactionIdentity,
};
use mfm_ids::{DigestBytes, EffectId, StableId};
use mfm_program::{
    effect_capability_contract_ref, Classification, ClassifyError, EffectState,
    ProposedStateOutcome, PureState,
};
use mfm_values::Object;

#[test]
fn configuration_preserves_request_and_originals_for_direct_composed_and_overflow_paths() {
    // Scalar native payloads isolate actual semantic States; this is not Runtime or EVM evidence.
    for (initial, add, expected) in [
        ("42", false, Some("42")),
        ("42", true, Some("84")),
        (
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            true,
            None,
        ),
    ] {
        let native = Object::from_value(&ConfigurationValue::new("1").unwrap()).unwrap();
        let ledger = LedgerIdentity::new(native.clone());
        let request = DeploymentRequest::new(
            ContractArtifact::new(ledger.clone(), native.clone()),
            ContractExecutionConfig::new(
                StableId::new("transaction@1").unwrap(),
                StableId::new("read@1").unwrap(),
                native.value_ref().clone(),
                native.clone(),
            ),
            ConfigurationValue::new(initial).unwrap(),
            ConfigurationValue::new("42").unwrap(),
            Some(3),
            Some(1),
        );
        let deployment = TransactionEvidence::new(
            EffectId::from_digest(DigestBytes::from_array([1; 32])),
            native.value_ref().clone(),
            native.value_ref().clone(),
            TransactionIdentity::new(ledger.clone(), native.clone()),
            ObservationPoint::new(ledger.clone(), native.clone()),
            native.clone(),
            TransactionResult::Applied {
                output: ContractLocator::new(ledger.clone(), native.clone()),
            },
        )
        .unwrap();
        let mut deployed = DeployedContract::new(
            request.clone(),
            request.requested().clone(),
            deployment.clone(),
        )
        .unwrap();
        if add {
            match CheckedAddConfigurationValue::evaluate(deployed).unwrap() {
                ProposedStateOutcome::Success { output } => deployed = output,
                ProposedStateOutcome::Failure { failure } => {
                    assert!(expected.is_none());
                    assert_eq!(failure.classify(), Classification::Permanent);
                    continue;
                }
            }
        }
        let expected = expected.expect("overflow must produce its typed failure");
        assert_eq!(deployed.effective().to_string(), expected);
        assert_eq!(deployed.request(), &request);
        assert_eq!(deployed.deployment(), &deployment);
        let prepared = PreparedTransaction::new(
            deployed.clone(),
            native.value_ref().clone(),
            native.value_ref().clone(),
            native.clone(),
        );
        let command = Configure::prepare(&prepared).unwrap();
        let command_object = Object::from_value(&command).unwrap();
        let command = command_object
            .decode::<PreparedTransaction<DeployedContract>>()
            .unwrap();
        let evidence = TransactionEvidence::new(
            EffectId::from_digest(DigestBytes::from_array([2; 32])),
            command_object.value_ref().clone(),
            native.value_ref().clone(),
            TransactionIdentity::new(ledger.clone(), native.clone()),
            ObservationPoint::new(ledger, native.clone()),
            native.clone(),
            TransactionResult::Applied {
                output: ConfigurationApplied,
            },
        )
        .unwrap();
        let configured = match Configure::interpret(command.clone(), &evidence).unwrap() {
            ProposedStateOutcome::Success { output } => output,
            _ => panic!("applied configuration must succeed"),
        };
        assert_eq!(configured.configured(), &deployed);
        assert_eq!(configured.configuration(), &evidence);
        let persisted = Object::from_value(&configured).unwrap();
        assert_eq!(
            persisted.decode::<ConfiguredContract>().unwrap(),
            configured
        );
        let mut rejected_wire = serde_json::to_value(&evidence).unwrap();
        rejected_wire["result"] = serde_json::json!({"rejected": {"reason": null}});
        let rejected: TransactionEvidence<ConfigurationApplied> =
            serde_json::from_str(&rejected_wire.to_string()).unwrap();
        assert!(ConfiguredContract::new(deployed, rejected.clone()).is_err());
        match Configure::interpret(command, &rejected).unwrap() {
            ProposedStateOutcome::Failure { failure } => {
                assert_eq!(failure.classify(), Classification::Permanent)
            }
            _ => panic!("rejected configuration must fail"),
        }
        let mut invalid = serde_json::to_value(configured).unwrap();
        invalid["configuration"] = rejected_wire;
        assert!(serde_json::from_str::<ConfiguredContract>(&invalid.to_string()).is_err());
    }
}

#[test]
fn applied_unit_and_request_specializations_have_exact_distinct_contracts() {
    let unit = Object::from_value(&ConfigurationApplied).unwrap();
    assert_eq!(unit.canonical_bytes(), b"null");
    assert_eq!(
        unit.decode::<ConfigurationApplied>().unwrap(),
        ConfigurationApplied
    );
    assert!(serde_json::from_str::<ConfigurationApplied>("{}").is_err());
    assert_ne!(
        effect_capability_contract_ref::<TransactionEffect<DeploymentRequest>>().unwrap(),
        effect_capability_contract_ref::<TransactionEffect<DeployedContract>>().unwrap()
    );
}
