use super::*;
use mfm_canonical::PlainCanonicalJsonBytes;
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
    let operations = [
        RecordedOperation::Admitted {
            program: object.clone(),
            initial: object.clone(),
        },
        RecordedOperation::Failed(Failure::Read {
            call: call.clone(),
            intent: object.clone(),
            original: object.clone(),
        }),
        RecordedOperation::Recovered {
            failure: Failure::Domain {
                call: StateCall::Pure(call.clone()),
                original: object.clone(),
            },
            classification: Classification::Permanent,
            request: RecoveryRequest::Stop,
            outcome: RecoveryOutcome::Stop {
                reason: StopReason::Requested,
            },
        },
    ];
    for operation in operations {
        let record = RunRecord {
            domain: RecordDomain::Current,
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
fn record_decoding_requires_current_domain_and_rejects_obsolete_fields() {
    let object = Object::from_value(&NoParams).unwrap();
    let reference = serde_json::to_string(object.value_ref()).unwrap();
    let object = serde_json::to_string(&object).unwrap();
    let operation = format!(r#"{{"admitted":{{"program":{object},"initial":{object}}}}}"#);
    let sequence = format!("[{reference},{operation},[],[],null]");
    assert!(serde_json::from_str::<RunRecord>(&sequence).is_err());
    let current = format!(
        r#"{{"domain":"mfm.runtime-record.v1","program_ref":{reference},"operation":{operation},"checkpoints":[],"usage":[],"effect_barrier":null}}"#
    );
    assert!(serde_json::from_str::<RunRecord>(&current).is_ok());
    assert!(serde_json::from_str::<RunRecord>(
        &current.replace("mfm.runtime-record.v1", "mfm.runtime-record.v0")
    )
    .is_err());
    assert!(serde_json::from_str::<RunRecord>(
        &current.replace("\"domain\":\"mfm.runtime-record.v1\",", "")
    )
    .is_err());
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
