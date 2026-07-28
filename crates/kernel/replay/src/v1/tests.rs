use super::*;

#[path = "tests/behavior.rs"]
mod behavior;
#[path = "tests/fixtures.rs"]
mod fixtures;

#[test]
fn replay_error_codes_remain_stable() {
    assert_eq!(
        ReplayErrorKind::InvalidRunJournal.code(),
        "MFM_REPLAY_JOURNAL_INVALID"
    );
    assert_eq!(
        ReplayErrorKind::LiveCapabilityRequest.code(),
        "MFM_REPLAY_LIVE_CAPABILITY_REQUEST"
    );
}
