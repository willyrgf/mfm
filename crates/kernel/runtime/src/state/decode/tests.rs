use super::*;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::DigestBytes;
use mfm_program::NoParams;

// Wire qualification is independent of the Program's current-state relationships, which the
// public Runtime integration tests exercise. These cases cover each inline enum alternative.
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
    let settled = Settlement {
        effect: effect.clone(),
        evidence: object.clone(),
    };
    let read_failure = ReadFailure {
        call: call.clone(),
        intent: object.clone(),
        original: object.clone(),
    };
    let mut cases = vec![
        (
            Phase::Runnable(call.clone()),
            OperationFacts::Admitted {
                program: object.clone(),
                initial: object.clone(),
            },
        ),
        (
            Phase::Succeeded(object.clone()),
            OperationFacts::Succeeded {
                call: StateCall::Pure(call.clone()),
                output: object.clone(),
            },
        ),
        (
            Phase::EffectPending(effect.clone()),
            OperationFacts::EffectPrepared(effect.clone()),
        ),
        (
            Phase::AwaitingInterpretation(settled.clone()),
            OperationFacts::EffectSettled(settled.clone()),
        ),
    ];
    let calls = [
        StateCall::Pure(call.clone()),
        StateCall::Read {
            call: call.clone(),
            intent: object.clone(),
            evidence: object.clone(),
        },
        StateCall::Effect(settled),
    ];
    let mut failures = vec![
        Failure::Read(read_failure.clone()),
        Failure::PendingEffect {
            effect,
            original: object.clone(),
        },
    ];
    for call in calls {
        let failure = DomainFailure {
            call,
            original: object.clone(),
        };
        cases.push((
            Phase::Failed(TerminalFailure::Domain {
                failure: failure.clone(),
                reason: StopReason::Requested,
                root: object.clone(),
            }),
            OperationFacts::Recovered {
                failure: Failure::Domain(failure.clone()),
                classification: Classification::Permanent,
                request: RecoveryRequest::Stop,
                decision: RecoveryDecision::Stop {
                    reason: StopReason::Requested,
                },
            },
        ));
        failures.push(Failure::Domain(failure));
    }
    for failure in failures {
        cases.push((
            Phase::AwaitingRecovery(failure.clone()),
            OperationFacts::Failed(failure),
        ));
    }
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
        cases.push((
            Phase::Failed(TerminalFailure::Read {
                failure: read_failure.clone(),
                reason,
            }),
            OperationFacts::Recovered {
                failure: Failure::Read(read_failure.clone()),
                classification: Classification::OutcomeUnknown,
                request: RecoveryRequest::Stop,
                decision: RecoveryDecision::Stop { reason },
            },
        ));
    }
    for (classification, request, decision) in [
        (
            Classification::Retryable,
            RecoveryRequest::RetryState,
            RecoveryDecision::Retry,
        ),
        (
            Classification::InputInvalidated,
            serde_json::from_str(r#"{"restart":0}"#).unwrap(),
            RecoveryDecision::Restart {
                checkpoint: call.position.state,
            },
        ),
    ] {
        cases.push((
            Phase::Runnable(call.clone()),
            OperationFacts::Recovered {
                failure: Failure::Read(read_failure.clone()),
                classification,
                request,
                decision,
            },
        ));
    }
    for (phase, facts) in cases {
        let commit = RunCommit {
            program_ref: object.value_ref().clone(),
            state: RunState {
                phase,
                checkpoints: vec![Checkpoint {
                    position: call.position.state,
                    input: object.clone(),
                }],
                usage: vec![StateUsage {
                    retries: 1,
                    restarts: 2,
                }],
                effect_barrier: Some(call.position.state),
            },
            facts,
        };
        let wire = PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&commit).unwrap())
            .unwrap();
        let mut decoder = serde_json::Deserializer::from_slice(wire.as_bytes());
        let decoded = RunCommitSeed.deserialize(&mut decoder).unwrap().unwrap();
        decoder.end().unwrap();
        assert!(decoded == commit);
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            serde_json::to_string(&commit).unwrap()
        );
    }
}

#[test]
fn phase_wire_rejects_unknown_or_multiple_variants_and_out_of_range_restart() {
    let object = Object::from_value(&NoParams).unwrap();
    let multiple = format!(
        r#"{{"succeeded":{},"runnable":null}}"#,
        serde_json::to_string(&object).unwrap()
    );
    for wire in [r#"{"unknown":null}"#, multiple.as_str()] {
        assert!(PhaseSeed
            .deserialize(&mut serde_json::Deserializer::from_str(wire))
            .is_err());
    }
    assert!(serde_json::from_str::<RecoveryRequest>(r#"{"restart":65536}"#).is_err());
}

#[test]
fn checkpoint_native_failure_survives_discarding_the_remaining_array() {
    let object = Object::from_value(&NoParams).unwrap();
    let wire = format!(
        r#"[{{"input":{{"canonical":true,"value_ref":{}}},"position":0}},{{"unknown":true}}]"#,
        serde_json::to_string(object.value_ref()).unwrap()
    );
    let mut decoder = serde_json::Deserializer::from_str(&wire);
    let cause = CheckpointsSeed
        .deserialize(&mut decoder)
        .unwrap()
        .err()
        .unwrap();
    decoder.end().unwrap();
    assert!(matches!(
        cause.downcast_ref::<mfm_values::ValueError>(),
        Some(mfm_values::ValueError::ArtifactTypeMismatch {
            field: "content_digest",
            ..
        })
    ));
}
