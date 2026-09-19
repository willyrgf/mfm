use mfm_capabilities::ReadCapabilityContract;
use mfm_chain::transaction::*;
use mfm_chain::{
    ContractArtifact, ContractLocator, LedgerIdentity, ObservationPoint, TransactionIdentity,
};
use mfm_ids::{DigestBytes, EffectId, EntryPointId, StableId};
use mfm_program::{
    Classification, ClassifyError, EffectState, ProposedStateOutcome, PureState, ReadState,
};
use mfm_values::Object;

type ReportSource = mfm_program::Operation<(mfm_program::Pure<Report>,), LifecycleDefaults>;
struct InstalledReport;
impl mfm_program::OperationDefinition for InstalledReport {
    type Body = (mfm_program::Pure<Report>,);
}
struct Resources;
impl mfm_program::ProgramEnvironment for Resources {
    // Cold discovery has no Plan implementation and does not receive lifecycle configuration.
    type Sources = mfm_program::Operation<InstalledReport, LifecycleDefaults>;
}

#[test]
fn lifecycle_report_keeps_originals_and_rejects_inconsistent_semantic_facts() {
    // Actual shared State methods with scalar native stand-ins, without Runtime or EVM execution.
    for add in [false, true] {
        let native = Object::from_value(&ConfigurationValue::new("1").unwrap()).unwrap();
        let configured_original =
            Object::from_value(&ConfigurationValue::new("2").unwrap()).unwrap();
        let read_original = Object::from_value(&ConfigurationValue::new("3").unwrap()).unwrap();
        let ledger = LedgerIdentity::new(native.clone());
        let target = ContractLocator::new(ledger.clone(), native.clone());
        let point = ObservationPoint::new(ledger.clone(), native.clone());
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
            add.then_some(3),
            add.then_some(1),
        );
        let prepared = PreparedTransaction::new(
            request.clone(),
            native.value_ref().clone(),
            native.value_ref().clone(),
            native.clone(),
        );
        let deployment = TransactionEvidence::new(
            EffectId::from_digest(DigestBytes::from_array([1; 32])),
            Object::from_value(&prepared).unwrap().value_ref().clone(),
            native.value_ref().clone(),
            TransactionIdentity::new(ledger.clone(), native.clone()),
            point.clone(),
            native.clone(),
            TransactionResult::Applied {
                output: target.clone(),
            },
        )
        .unwrap();
        let ProposedStateOutcome::Success { mut output } =
            Deploy::interpret(prepared, &deployment).unwrap()
        else {
            panic!("deployment")
        };
        if add {
            let ProposedStateOutcome::Success { output: added } =
                CheckedAddConfigurationValue::evaluate(output).unwrap()
            else {
                panic!("addition")
            };
            output = added;
        }
        let prepared = PreparedTransaction::new(
            output,
            native.value_ref().clone(),
            native.value_ref().clone(),
            native.clone(),
        );
        let configuration = TransactionEvidence::new(
            EffectId::from_digest(DigestBytes::from_array([2; 32])),
            Object::from_value(&prepared).unwrap().value_ref().clone(),
            native.value_ref().clone(),
            TransactionIdentity::new(ledger.clone(), configured_original.clone()),
            point.clone(),
            configured_original.clone(),
            TransactionResult::Applied {
                output: ConfigurationApplied,
            },
        )
        .unwrap();
        let ProposedStateOutcome::Success { output: configured } =
            Configure::interpret(prepared, &configuration).unwrap()
        else {
            panic!("configuration")
        };
        let intent = Observe::prepare(&configured).unwrap();
        assert_eq!(intent.target(), &target);
        assert_eq!(intent.at(), configuration.observed_at());
        let intent_object = Object::from_value(&intent).unwrap();
        let expected = if add { "84" } else { "42" };
        let observation = ContractValueEvidence::new(
            intent_object.value_ref().clone(),
            native.value_ref().clone(),
            read_original.clone(),
            ContractValueOutcome::Observed {
                observed_at: point.clone(),
                value: ConfigurationValue::new(expected).unwrap(),
            },
        );
        ContractRead::bind_evidence(
            intent_object.value_ref(),
            &intent,
            read_original.value_ref(),
            &observation,
        )
        .unwrap();
        assert!(ContractRead::bind_evidence(
            native.value_ref(),
            &intent,
            read_original.value_ref(),
            &observation
        )
        .is_err());
        assert!(ContractRead::bind_evidence(
            intent_object.value_ref(),
            &intent,
            native.value_ref(),
            &observation
        )
        .is_err());
        let wrong_point = ReadContractValue::new(
            native.value_ref().clone(),
            target.clone(),
            ObservationPoint::new(ledger.clone(), configured_original.clone()),
        )
        .unwrap();
        assert!(ContractRead::bind_evidence(
            intent_object.value_ref(),
            &wrong_point,
            read_original.value_ref(),
            &observation
        )
        .is_err());
        let wrong_target = ReadContractValue::new(
            native.value_ref().clone(),
            ContractLocator::new(ledger.clone(), configured_original.clone()),
            point.clone(),
        )
        .unwrap();
        let wrong_target_ref = Object::from_value(&wrong_target)
            .unwrap()
            .value_ref()
            .clone();
        let wrong_observation = ContractValueEvidence::new(
            wrong_target_ref.clone(),
            native.value_ref().clone(),
            read_original.clone(),
            ContractValueOutcome::Observed {
                observed_at: point.clone(),
                value: ConfigurationValue::new(expected).unwrap(),
            },
        );
        assert!(ObservedConfiguration::new(configured.clone(), wrong_observation).is_err());

        let mismatch = ContractValueEvidence::new(
            intent_object.value_ref().clone(),
            native.value_ref().clone(),
            read_original.clone(),
            ContractValueOutcome::Observed {
                observed_at: point.clone(),
                value: ConfigurationValue::new("9").unwrap(),
            },
        );
        let observed_mismatch =
            ObservedConfiguration::new(configured.clone(), mismatch.clone()).unwrap();
        let mismatch_object = Object::from_value(&observed_mismatch).unwrap();
        assert!(mismatch_object.decode::<ObservedConfiguration>().is_ok());
        assert!(ValidatedConfiguration::new(observed_mismatch.clone()).is_err());
        let invalid_validated = serde_json::json!({"observed": observed_mismatch});
        assert!(
            serde_json::from_str::<ValidatedConfiguration>(&invalid_validated.to_string()).is_err()
        );
        let ProposedStateOutcome::Failure { failure } =
            Validate::evaluate(observed_mismatch).unwrap()
        else {
            panic!("mismatch")
        };
        assert_eq!(failure.classify(), Classification::Permanent);
        assert!(ContractDeploymentReport::new(
            request.clone(),
            ConfigurationValue::new(expected).unwrap(),
            deployment.clone(),
            configuration.clone(),
            mismatch
        )
        .is_err());

        for (outcome, expected_failure) in [
            (ContractValueOutcome::Rejected, ObservationFailure::Rejected),
            (
                ContractValueOutcome::SafeFailure,
                ObservationFailure::SafeFailure,
            ),
            (
                ContractValueOutcome::IntegrityBlocked,
                ObservationFailure::IntegrityBlocked,
            ),
        ] {
            let failed = ContractValueEvidence::new(
                intent_object.value_ref().clone(),
                native.value_ref().clone(),
                read_original.clone(),
                outcome,
            );
            ContractRead::bind_evidence(
                intent_object.value_ref(),
                &intent,
                read_original.value_ref(),
                &failed,
            )
            .unwrap();
            let retained = Object::from_value(&failed)
                .unwrap()
                .decode::<ContractValueEvidence>()
                .unwrap();
            assert_eq!(retained.original(), &read_original);
            let ProposedStateOutcome::Failure { failure } =
                Observe::interpret(configured.clone(), &retained).unwrap()
            else {
                panic!("unsuccessful native evidence cannot produce a successful observation")
            };
            assert_eq!(failure, expected_failure);
            assert_eq!(failure.classify(), Classification::Permanent);
            assert!(ObservedConfiguration::new(configured.clone(), retained.clone()).is_err());
            assert!(ContractDeploymentReport::new(
                request.clone(),
                ConfigurationValue::new(expected).unwrap(),
                deployment.clone(),
                configuration.clone(),
                retained
            )
            .is_err());
        }

        let ProposedStateOutcome::Success { output: observed } =
            Observe::interpret(configured, &observation).unwrap()
        else {
            panic!("expected observed scalar")
        };
        let ProposedStateOutcome::Success { output: validated } =
            Validate::evaluate(observed).unwrap()
        else {
            panic!("validation")
        };
        let validated = Object::from_value(&validated)
            .unwrap()
            .decode::<ValidatedConfiguration>()
            .unwrap();
        let program = mfm_program::compile(
            EntryPointId::new("mfm.chain/report@1").unwrap(),
            &ReportSource::default(),
            &validated,
            &Resources,
            mfm_program::ProgramLimits::new(4),
        )
        .unwrap();
        let cold = mfm_program::load(program.canonical_bytes(), &Resources).unwrap();
        assert_eq!(cold.content_ref(), program.content_ref());
        let allowance = cold.declarations()[0].allowances();
        assert_eq!(allowance.retries(), if add { 3 } else { 0 });
        assert_eq!(allowance.restarts(), if add { 1 } else { 0 });
        let ProposedStateOutcome::Success { output: report } = Report::evaluate(validated).unwrap();
        assert_eq!(report.requested_value().to_string(), "42");
        assert_eq!(report.effective_value().to_string(), expected);
        assert_eq!(report.observed_value().to_string(), expected);
        let stored = Object::from_value(&report).unwrap();
        let decoded = stored.decode::<ContractDeploymentReport>().unwrap();
        for (retained, original) in [
            (decoded.deployment().original(), &native),
            (decoded.configuration().original(), &configured_original),
            (decoded.observation().original(), &read_original),
        ] {
            assert_eq!(retained.value_ref(), original.value_ref());
            assert_eq!(retained.canonical_bytes(), original.canonical_bytes());
        }
        let wire = serde_json::to_value(&report).unwrap();
        assert_eq!(wire.as_object().unwrap().len(), 5);
        let mut other_configuration_ledger = wire.clone();
        for part in ["transaction", "observed_at"] {
            other_configuration_ledger["configuration"][part]["ledger"]["native"] =
                serde_json::to_value(&configured_original).unwrap();
        }
        assert!(serde_json::from_str::<ContractDeploymentReport>(
            &other_configuration_ledger.to_string()
        )
        .is_err());
        for (path, replacement) in [
            (
                "/deployment/result",
                serde_json::json!({"rejected":{"reason":null}}),
            ),
            (
                "/configuration/result",
                serde_json::json!({"rejected":{"reason":null}}),
            ),
            (
                "/request/artifact/ledger/native",
                serde_json::to_value(&configured_original).unwrap(),
            ),
            (
                "/observation/outcome/observed/observed_at/native",
                serde_json::to_value(&configured_original).unwrap(),
            ),
            (
                "/observation/intent_ref",
                serde_json::to_value(&wrong_target_ref).unwrap(),
            ),
            (
                "/observation/outcome/observed/value",
                serde_json::json!("9"),
            ),
        ] {
            let mut invalid = wire.clone();
            *invalid.pointer_mut(path).unwrap() = replacement;
            assert!(
                serde_json::from_str::<ContractDeploymentReport>(&invalid.to_string()).is_err(),
                "{path}"
            );
        }
        let other_ledger = LedgerIdentity::new(configured_original.clone());
        assert!(ReadContractValue::new(
            native.value_ref().clone(),
            target,
            ObservationPoint::new(other_ledger, native.clone())
        )
        .is_err());
        assert_eq!(
            <ReadContractValue as mfm_values::MfmValue>::semantic_id()
                .unwrap()
                .version(),
            Some("2")
        );
        assert_eq!(
            <ContractExecutionConfig as mfm_values::MfmValue>::semantic_id()
                .unwrap()
                .version(),
            Some("2")
        );
        assert_eq!(intent.route_ref(), native.value_ref());
        let mut old_intent = serde_json::to_value(&intent).unwrap();
        old_intent.as_object_mut().unwrap().remove("route_ref");
        assert!(serde_json::from_value::<ReadContractValue>(old_intent).is_err());
        let mut old_config = serde_json::to_value(report.request().execution()).unwrap();
        old_config
            .as_object_mut()
            .unwrap()
            .remove("observation_route_ref");
        assert!(serde_json::from_value::<ContractExecutionConfig>(old_config).is_err());
        let mut wrong_route = serde_json::to_value(&report).unwrap();
        wrong_route["request"]["execution"]["observation_route_ref"] =
            serde_json::to_value(read_original.value_ref()).unwrap();
        assert!(serde_json::from_value::<ContractDeploymentReport>(wrong_route).is_err());
        let mut invalid_intent = serde_json::to_value(&intent).unwrap();
        invalid_intent["at"]["ledger"]["native"] =
            serde_json::to_value(&configured_original).unwrap();
        assert!(serde_json::from_str::<ReadContractValue>(&invalid_intent.to_string()).is_err());
    }
}
