use super::*;
use mfm_ids::{StatePosition, VisitId};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct TextFailure {
    detail: String,
}

fn value(bytes: usize) -> Object {
    Object::from_value(&TextFailure {
        detail: "x".repeat(bytes),
    })
    .unwrap()
}

#[test]
fn exact_inline_report_limit_and_one_more_byte_are_distinguished() {
    let position = ExecutionPosition {
        state: StatePosition::new(0).unwrap(),
        visit: VisitId::new(0),
    };
    let failure = crate::Failure::Domain {
        call: crate::StateCall::Pure(crate::Call {
            position,
            input: value(0),
        }),
        original: value(0),
    };
    let usage = RecoveryUsage {
        state_retries: 0,
        state_restarts: 0,
        run_decisions: 0,
    };
    let overhead = FailureReport::new(
        failure.clone(),
        StopReason::Requested,
        usage,
        Some(value(0)),
    )
    .unwrap()
    .canonical_bytes()
    .len();
    let payload = 33_554_432 - overhead;
    let report = FailureReport::new(
        failure.clone(),
        StopReason::Requested,
        usage,
        Some(value(payload)),
    )
    .unwrap();
    assert_eq!(report.canonical_bytes().len(), 33_554_432);
    drop(report);
    let error = match FailureReport::new(
        failure.clone(),
        StopReason::Requested,
        usage,
        Some(value(payload + 1)),
    ) {
        Err(error) => error,
        Ok(_) => panic!("oversized report accepted"),
    };
    assert!(
        matches!(error.size_limit().unwrap(), crate::SizeViolation::SerializationBound {
        resource: crate::SizeResource::FailureReport,
        observed_at_least,
        limit: 33_554_432,
    } if observed_at_least > 33_554_432)
    );
}
