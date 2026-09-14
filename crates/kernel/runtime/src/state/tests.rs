use super::*;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::DigestBytes;
use mfm_program::NoParams;

#[test]
fn current_payload_alternatives_roundtrip_through_canonical_json() {
    let object = Object::from_value(&NoParams).unwrap();
    let call = Call {
        position: ExecutionPosition {
            state: StatePosition::new(0).unwrap(),
            visit: VisitId::new(0),
        },
        input: object.clone(),
    };
    let effect = EffectCall {
        call: call.clone(),
        effect_id: EffectId::from_digest(DigestBytes::from_array([1; 32])),
        command: object.clone(),
    };
    let settlement = Settlement {
        effect: effect.clone(),
        evidence: object.clone(),
    };
    let mut operations = vec![
        RecordedOperation::Admitted {
            program: object.clone(),
            initial: object.clone(),
        },
        RecordedOperation::EffectPrepared(effect.clone()),
        RecordedOperation::EffectSettled(settlement.clone()),
    ];
    let mut failures = vec![
        Failure::Read {
            call: call.clone(),
            intent: object.clone(),
            original: object.clone(),
        },
        Failure::PendingEffect {
            effect,
            original: object.clone(),
        },
    ];
    for call in [
        StateCall::Pure(call.clone()),
        StateCall::Read {
            call: call.clone(),
            intent: object.clone(),
            evidence: object.clone(),
        },
        StateCall::Effect(settlement),
    ] {
        operations.push(RecordedOperation::Succeeded {
            call: call.clone(),
            output: object.clone(),
        });
        failures.push(Failure::Domain {
            call,
            original: object.clone(),
        });
    }
    for failure in failures {
        operations.push(RecordedOperation::Failed(failure.clone()));
        let mut outcomes = vec![
            RecoveryOutcome::Retry,
            RecoveryOutcome::Restart {
                checkpoint: call.position.state,
            },
        ];
        for reason in [
            StopReason::Requested,
            StopReason::Exhausted(RecoveryLimit::StateRetry),
            StopReason::Exhausted(RecoveryLimit::StateRestart),
            StopReason::Exhausted(RecoveryLimit::Run),
            StopReason::Disallowed(RecoveryDenial::PureRetry),
            StopReason::Disallowed(RecoveryDenial::CheckpointUnavailable),
            StopReason::Disallowed(RecoveryDenial::EffectBarrier),
            StopReason::Disallowed(RecoveryDenial::EffectSettled),
        ] {
            outcomes.push(RecoveryOutcome::Stop {
                reason,
                root: matches!(failure, Failure::Domain { .. }).then(|| object.clone()),
            });
        }
        for (index, outcome) in outcomes.into_iter().enumerate() {
            let (classification, request) = match index {
                0 => (Classification::Retryable, RecoveryRequest::RetryState),
                1 => (
                    Classification::InputInvalidated,
                    serde_json::from_str(r#"{"restart":0}"#).unwrap(),
                ),
                2 => (Classification::OutcomeUnknown, RecoveryRequest::Stop),
                _ => (Classification::Permanent, RecoveryRequest::Stop),
            };
            operations.push(RecordedOperation::Recovered {
                failure: failure.clone(),
                classification,
                request,
                outcome,
            });
        }
    }
    for operation in operations {
        let record = RunRecord {
            program_ref: object.value_ref().clone(),
            operation,
            checkpoints: vec![Checkpoint {
                position: call.position.state,
                input: object.clone(),
            }],
            usage: vec![StateUsage {
                retries: 1,
                restarts: 2,
            }],
            effect_barrier: Some(call.position.state),
        };
        let canonical =
            PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&record).unwrap())
                .unwrap();
        let occurrences = std::str::from_utf8(canonical.as_bytes())
            .unwrap()
            .matches("\"canonical\":")
            .count();
        assert_eq!(
            record.object_payload_bytes().unwrap(),
            (occurrences * object.canonical_bytes().len()) as u64
        );
        let decoded: RunRecord = serde_json::from_slice(canonical.as_bytes()).unwrap();
        assert!(decoded == record);
        let wire: serde_json::Value = serde_json::from_slice(canonical.as_bytes()).unwrap();
        assert!(wire.get("state").is_none());
        assert!(wire.get("facts").is_none());
        assert!(wire.get("phase").is_none());
    }
}

#[test]
fn record_decoding_rejects_obsolete_fields_and_accepts_ordinary_serde_sequences() {
    let object = Object::from_value(&NoParams).unwrap();
    let reference = serde_json::to_string(object.value_ref()).unwrap();
    let object = serde_json::to_string(&object).unwrap();
    let operation = format!(r#"{{"admitted":{{"program":{object},"initial":{object}}}}}"#);
    let sequence = format!("[{reference},{operation},[],[],null]");
    assert!(serde_json::from_str::<RunRecord>(&sequence).is_ok());
    for wire in [
        format!(r#"{{"program_ref":{reference},"state":{{}},"facts":{operation}}}"#),
        format!(
            r#"{{"program_ref":{reference},"operation":{operation},"operation":{operation},"checkpoints":[],"usage":[],"effect_barrier":null}}"#
        ),
        format!("{sequence} null"),
    ] {
        assert!(serde_json::from_str::<RunRecord>(&wire).is_err());
    }
    assert!(serde_json::from_str::<RecordedOperation>(r#"{"unknown":null}"#).is_err());
    assert!(serde_json::from_str::<RecoveryRequest>(r#"{"restart":65536}"#).is_err());
}

#[test]
fn checkpoint_rejects_invalid_nested_object_with_parser_reason() {
    let object = Object::from_value(&NoParams).unwrap();
    let wire = format!(
        r#"[{{"input":{{"canonical":true,"value_ref":{}}},"position":0}}]"#,
        serde_json::to_string(object.value_ref()).unwrap()
    );
    let error = serde_json::from_str::<Vec<Checkpoint>>(&wire)
        .err()
        .unwrap();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
    assert!(error.to_string().contains("content_digest"));
    assert_eq!(error.line(), 1);
    assert!(error.column() > 0);
}
