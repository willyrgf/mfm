use super::*;

#[test]
fn holding_status_tags_are_closed_and_write_admission_is_explicit() {
    assert_eq!(
        CoverageStatus::CompleteAtAnchor.as_str(),
        "complete_at_anchor"
    );
    assert_eq!(
        "configured_only".parse::<CoverageStatus>(),
        Ok(CoverageStatus::ConfiguredOnly)
    );
    assert!(CoverageStatus::ConfiguredOnly.is_admissible_for_write());
    assert!(!CoverageStatus::Truncated.is_admissible_for_write());
    assert_eq!(HoldingSourceStatus::Ok.as_str(), "ok");
    assert_eq!(
        "ok".parse::<HoldingSourceStatus>(),
        Ok(HoldingSourceStatus::Ok)
    );
    assert!(!HoldingSourceStatus::Failed.is_admissible_for_write());
}
