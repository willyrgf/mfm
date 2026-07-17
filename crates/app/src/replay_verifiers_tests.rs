use super::*;
use std::collections::BTreeSet;

#[test]
fn production_dispatch_scopes_are_explicit_and_unique() {
    let registry = ReplayVerifierRegistry::production();
    let mut state_keys = BTreeSet::new();
    for registration in registry.registrations {
        assert!(!registration.state_keys.is_empty() || registration.intent_matcher.is_some());
        for state_key_factory in registration.state_keys {
            let key = state_key_factory().expect("certified state key");
            assert!(state_keys.insert((key.kind, key.version)));
        }
    }

    for expected in [
        state_key::<mfm_op_portfolio_snapshot::AssemblePortfolioCollectionReceiptState>(),
        state_key::<mfm_state_portfolio::SelectHoldingsState>(),
        state_key::<mfm_state_portfolio::AssembleSnapshotState>(),
        state_key::<mfm_state_portfolio::ProjectReportState>(),
    ] {
        let expected = expected.expect("portfolio replay state key");
        assert!(state_keys.contains(&(expected.kind, expected.version)));
    }
}
