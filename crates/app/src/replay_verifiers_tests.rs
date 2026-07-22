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
        state_key::<mfm_portfolio::SelectHoldingsState>(),
        state_key::<mfm_portfolio::AssembleSnapshotState>(),
        state_key::<mfm_portfolio::ProjectReportState>(),
    ] {
        let expected = expected.expect("portfolio replay state key");
        assert!(state_keys.contains(&(expected.kind, expected.version)));
    }

    let validation =
        state_key::<mfm_evm::ValidateEvmContractState>().expect("validation state key");
    assert!(!state_keys.contains(&(validation.kind, validation.version)));
    assert!(
        registry
            .registrations
            .iter()
            .all(|registration| registration.intent_matcher.is_none()),
        "production replay must not register transaction intent dispatch"
    );
    assert!(registry
        .registrations
        .iter()
        .all(|registration| registration.intent_state_keys.is_empty()));
}

#[test]
fn production_replay_registry_equals_the_published_authoring_catalog() {
    let catalog = crate::production_authoring_catalog().expect("production authoring catalog");
    ReplayVerifierRegistry::production()
        .validate_authoring_catalog(&catalog)
        .expect("exact replay catalog coverage");

    let missing = ReplayVerifierRegistry { registrations: &[] };
    assert!(missing.validate_authoring_catalog(&catalog).is_err());

    let duplicate = ReplayVerifierRegistry {
        registrations: &[
            ReplayVerifierRegistration {
                state_keys: &[state_key::<mfm_portfolio::SelectHoldingsState>],
                intent_state_keys: &[],
                intent_matcher: None,
                verifier: verify_portfolio,
            },
            ReplayVerifierRegistration {
                state_keys: &[state_key::<mfm_portfolio::SelectHoldingsState>],
                intent_state_keys: &[],
                intent_matcher: None,
                verifier: verify_portfolio,
            },
        ],
    };
    assert!(duplicate.validate_authoring_catalog(&catalog).is_err());

    let extra = ReplayVerifierRegistry {
        registrations: &[
            ReplayVerifierRegistration {
                state_keys: &[
                    state_key::<mfm_portfolio::SelectHoldingsState>,
                    state_key::<mfm_portfolio::AssembleSnapshotState>,
                    state_key::<mfm_portfolio::ProjectReportState>,
                ],
                intent_state_keys: &[],
                intent_matcher: None,
                verifier: verify_portfolio,
            },
            ReplayVerifierRegistration {
                state_keys: &[state_key::<mfm_bitcoin::CollectBitcoinBalancesState>],
                intent_state_keys: &[],
                intent_matcher: None,
                verifier: verify_btc,
            },
            ReplayVerifierRegistration {
                state_keys: &[
                    state_key::<mfm_evm::CollectEvmBalancesState>,
                    state_key::<mfm_evm::ValidateEvmContractState>,
                ],
                intent_state_keys: &[],
                intent_matcher: None,
                verifier: verify_evm_balance_collection,
            },
        ],
    };
    assert!(extra.validate_authoring_catalog(&catalog).is_err());

    let extra_intent = ReplayVerifierRegistry {
        registrations: &[
            ReplayVerifierRegistration {
                state_keys: &[
                    state_key::<mfm_portfolio::SelectHoldingsState>,
                    state_key::<mfm_portfolio::AssembleSnapshotState>,
                    state_key::<mfm_portfolio::ProjectReportState>,
                ],
                intent_state_keys: &[],
                intent_matcher: None,
                verifier: verify_portfolio,
            },
            ReplayVerifierRegistration {
                state_keys: &[state_key::<mfm_bitcoin::CollectBitcoinBalancesState>],
                intent_state_keys: &[],
                intent_matcher: None,
                verifier: verify_btc,
            },
            ReplayVerifierRegistration {
                state_keys: &[state_key::<mfm_evm::CollectEvmBalancesState>],
                intent_state_keys: &[state_key::<mfm_evm::SubmitEvmTransactionState>],
                intent_matcher: Some(mfm_evm_live::is_evm_transaction_replay_intent),
                verifier: verify_evm_balance_collection,
            },
        ],
    };
    assert!(extra_intent.validate_authoring_catalog(&catalog).is_err());
}
