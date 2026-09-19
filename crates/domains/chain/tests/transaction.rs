use mfm_capabilities::EffectCapabilityContract;
use mfm_chain::transaction::{
    ConfigurationValue, ContractExecutionConfig, DeploymentRequest, PreparedTransaction,
    TransactionEffect, TransactionEvidence, TransactionResult,
};
use mfm_chain::transaction::{Deploy, DeployedContract};
use mfm_chain::{
    ContractArtifact, ContractLocator, LedgerIdentity, ObservationPoint, TransactionIdentity,
};
use mfm_ids::{DigestBytes, EffectId, StableId};
use mfm_program::{Classification, ClassifyError, EffectState, ProposedStateOutcome};
use mfm_values::{Object, Unsigned256};

#[test]
fn deployment_settlement_retains_original_and_binds_every_available_provenance_reference() {
    // Native scalar stand-ins isolate shared semantic binding; they prove no EVM interpretation.
    let native = Object::from_value(&Unsigned256::new("1").unwrap()).unwrap();
    let other = Object::from_value(&Unsigned256::new("2").unwrap()).unwrap();
    let ledger = LedgerIdentity::new(native.clone());
    let request = DeploymentRequest::new(
        ContractArtifact::new(ledger.clone(), native.clone()),
        ContractExecutionConfig::new(
            StableId::new("transaction@1").unwrap(),
            StableId::new("read@1").unwrap(),
            native.value_ref().clone(),
            native.clone(),
        ),
        ConfigurationValue::new("42").unwrap(),
        ConfigurationValue::new("42").unwrap(),
        Some(2),
        None,
    );
    let command = PreparedTransaction::new(
        request,
        native.value_ref().clone(),
        other.value_ref().clone(),
        native.clone(),
    );
    let command_object = Object::from_value(&command).unwrap();
    let command = command_object
        .decode::<PreparedTransaction<DeploymentRequest>>()
        .unwrap();
    assert_eq!(command.request().requested().to_string(), "42");
    assert_eq!(command.request().retry_allowance(), Some(2));
    let effect = EffectId::from_digest(DigestBytes::from_array([1; 32]));
    let evidence = TransactionEvidence::new(
        effect.clone(),
        command_object.value_ref().clone(),
        command.implementation_ref().clone(),
        TransactionIdentity::new(ledger.clone(), native.clone()),
        ObservationPoint::new(ledger.clone(), other.clone()),
        native.clone(),
        TransactionResult::Applied {
            output: ContractLocator::new(ledger, other.clone()),
        },
    )
    .unwrap();
    let persisted = Object::from_value(&evidence).unwrap();
    let decoded = persisted
        .decode::<TransactionEvidence<ContractLocator>>()
        .unwrap();
    assert_eq!(decoded.original(), &native);
    let deployed = match Deploy::interpret(Deploy::prepare(&command).unwrap(), &decoded).unwrap() {
        ProposedStateOutcome::Success { output } => output,
        _ => panic!("applied deployment must succeed"),
    };
    assert_eq!(deployed.effective().to_string(), "42");
    assert_eq!(deployed.contract().unwrap().native(), &other);
    let stored_context = Object::from_value(&deployed).unwrap();
    assert_eq!(
        stored_context.decode::<DeployedContract>().unwrap(),
        deployed
    );
    let mut rejected_wire = serde_json::to_value(&decoded).unwrap();
    rejected_wire["result"] = serde_json::json!({"rejected": {"reason": null}});
    let rejected: TransactionEvidence<ContractLocator> =
        serde_json::from_str(&rejected_wire.to_string()).unwrap();
    let failure = match Deploy::interpret(command.clone(), &rejected).unwrap() {
        ProposedStateOutcome::Failure { failure } => failure,
        _ => panic!("authenticated rejection must fail deployment"),
    };
    assert_eq!(failure.classify(), Classification::Permanent);
    assert!(DeployedContract::new(
        command.request().clone(),
        ConfigurationValue::new("42").unwrap(),
        rejected
    )
    .is_err());
    let mut invalid_context = serde_json::to_value(&deployed).unwrap();
    invalid_context["deployment"] = rejected_wire;
    assert!(serde_json::from_str::<DeployedContract>(&invalid_context.to_string()).is_err());
    TransactionEffect::<DeploymentRequest>::bind_evidence(
        &effect,
        command_object.value_ref(),
        &command,
        native.value_ref(),
        &decoded,
    )
    .unwrap();
    let different_effect = EffectId::from_digest(DigestBytes::from_array([2; 32]));
    for (label, effect, command_ref, original_ref) in [
        (
            "effect",
            &different_effect,
            command_object.value_ref(),
            native.value_ref(),
        ),
        ("command", &effect, other.value_ref(), native.value_ref()),
        (
            "original",
            &effect,
            command_object.value_ref(),
            other.value_ref(),
        ),
    ] {
        assert!(
            TransactionEffect::<DeploymentRequest>::bind_evidence(
                effect,
                command_ref,
                &command,
                original_ref,
                &decoded
            )
            .is_err(),
            "{label}"
        );
    }
    let mismatched = PreparedTransaction::new(
        command.into_request(),
        other.value_ref().clone(),
        other.value_ref().clone(),
        native.clone(),
    );
    assert!(TransactionEffect::<DeploymentRequest>::bind_evidence(
        &effect,
        command_object.value_ref(),
        &mismatched,
        native.value_ref(),
        &decoded
    )
    .is_err());

    let mut wire = serde_json::to_value(&decoded).unwrap();
    wire["observed_at"]["ledger"]["native"] = serde_json::to_value(&other).unwrap();
    assert!(
        serde_json::from_str::<TransactionEvidence<ContractLocator>>(&wire.to_string()).is_err()
    );
}

#[test]
fn rejected_settlement_preserves_unavailable_reason_and_rejects_mixed_ledgers() {
    let native = Object::from_value(&Unsigned256::new("1").unwrap()).unwrap();
    let other = Object::from_value(&Unsigned256::new("2").unwrap()).unwrap();
    let ledger = LedgerIdentity::new(native.clone());
    let effect = EffectId::from_digest(DigestBytes::from_array([3; 32]));
    let evidence = TransactionEvidence::<ContractLocator>::new(
        effect.clone(),
        native.value_ref().clone(),
        native.value_ref().clone(),
        TransactionIdentity::new(ledger.clone(), native.clone()),
        ObservationPoint::new(ledger.clone(), other.clone()),
        native.clone(),
        TransactionResult::Rejected { reason: None },
    )
    .unwrap();
    let stored = Object::from_value(&evidence).unwrap();
    let decoded = stored
        .decode::<TransactionEvidence<ContractLocator>>()
        .unwrap();
    assert_eq!(
        decoded.result(),
        &TransactionResult::Rejected { reason: None }
    );
    assert_eq!(decoded.original(), &native);
    assert!(TransactionEvidence::<ContractLocator>::new(
        effect,
        native.value_ref().clone(),
        native.value_ref().clone(),
        TransactionIdentity::new(ledger, native.clone()),
        ObservationPoint::new(LedgerIdentity::new(other.clone()), other),
        native,
        TransactionResult::Rejected { reason: None }
    )
    .is_err());
}
