use super::*;
use mfm_ids::{StatePosition, VisitId};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct TextFailure {
    detail: String,
}

fn cause(bytes: usize) -> FailureCauseView {
    fn value(bytes: usize) -> ValueView {
        let (canonical, value_ref) = mfm_values::canonicalize_mfm_value(&TextFailure {
            detail: "x".repeat(bytes),
        })
        .unwrap();
        ValueView {
            contract_ref: mfm_program::nominal_contract_ref::<TextFailure>().unwrap(),
            value_ref,
            canonical,
        }
    }
    FailureCauseView::Domain {
        original: value(0),
        root: value(bytes),
    }
}

#[test]
fn exact_inline_report_limit_and_one_more_byte_are_distinguished() {
    let position = ExecutionPosition {
        state: StatePosition::new(0).unwrap(),
        visit: VisitId::new(0),
    };
    let usage = RecoveryUsage {
        state_retries: 0,
        state_restarts: 0,
        run_decisions: 0,
    };
    let overhead = FailureReport::new(position, StopCode::Requested, usage, cause(0))
        .unwrap()
        .canonical_bytes()
        .len();
    let payload = 33_554_432 - overhead;
    let report = FailureReport::new(position, StopCode::Requested, usage, cause(payload)).unwrap();
    assert_eq!(report.canonical_bytes().len(), 33_554_432);
    drop(report);
    let error = match FailureReport::new(position, StopCode::Requested, usage, cause(payload + 1)) {
        Err(error) => error,
        Ok(_) => panic!("oversized report accepted"),
    };
    let (resource, size) = error.size_limit().unwrap();
    assert_eq!(resource, crate::SizeResource::FailureReport);
    assert_eq!(size.actual(), 33_554_433);
    assert_eq!(size.limit(), 33_554_432);
}
