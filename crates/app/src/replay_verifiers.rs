use std::collections::BTreeSet;

use mfm_certify::CertificationRegistry;
use mfm_events::v1 as events;
use mfm_ids::{StateKind, StateVersion};
use mfm_program::StateSpec;
use mfm_replay::v1::{ReplayBroker, Result};

type ReplayVerifier = fn(&ReplayBroker<'_>, &CertificationRegistry) -> Result<()>;
type ReplayStateKeyFactory = fn() -> Result<ReplayStateKey>;
type ReplayIntentMatcher = fn(&events::side_effect::IntentPersisted) -> Result<bool>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReplayStateKey {
    kind: StateKind,
    version: StateVersion,
}

struct ReplayVerifierRegistration {
    state_keys: &'static [ReplayStateKeyFactory],
    intent_state_keys: &'static [ReplayStateKeyFactory],
    intent_matcher: Option<ReplayIntentMatcher>,
    verifier: ReplayVerifier,
}

/// The one production replay verifier registry compiled into the application.
///
/// Verifiers are evidence-only domain functions. The certification registry supplies the trusted
/// descriptor authority needed to validate the retained certified run.
pub(crate) struct ReplayVerifierRegistry {
    registrations: &'static [ReplayVerifierRegistration],
}

impl ReplayVerifierRegistry {
    pub(crate) const fn production() -> Self {
        Self {
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
                    intent_state_keys: &[],
                    intent_matcher: None,
                    verifier: verify_evm_balance_collection,
                },
            ],
        }
    }

    pub(crate) fn verify(
        &self,
        broker: &ReplayBroker<'_>,
        certification_registry: &CertificationRegistry,
    ) -> Result<()> {
        for registration in self.registrations {
            if registration.applies(broker)? {
                (registration.verifier)(broker, certification_registry)?;
            }
        }
        Ok(())
    }

    pub(crate) fn validate_authoring_catalog(
        &self,
        catalog: &mfm_certify::ProgramAuthoringCatalog,
    ) -> Result<()> {
        let expected_states = catalog
            .state_descriptors()
            .filter(|descriptor| !catalog.is_framework_state(&descriptor.descriptor_id))
            .map(|descriptor| ReplayStateKey {
                kind: descriptor.state_kind.clone(),
                version: descriptor.state_version.clone(),
            })
            .collect::<BTreeSet<_>>();
        let expected_intents = catalog
            .state_descriptors()
            .filter(|descriptor| {
                catalog
                    .side_effect_state_descriptor_ids()
                    .any(|id| id == &descriptor.descriptor_id)
            })
            .map(|descriptor| ReplayStateKey {
                kind: descriptor.state_kind.clone(),
                version: descriptor.state_version.clone(),
            })
            .collect::<BTreeSet<_>>();
        let mut actual_states = BTreeSet::new();
        let mut actual_intents = BTreeSet::new();
        for registration in self.registrations {
            let declares_intents = !registration.intent_state_keys.is_empty();
            for factory in registration.state_keys {
                let key = factory()?;
                if !actual_states.insert(key) {
                    return Err(replay_registration_error(
                        "duplicate replay state key in production registry",
                    ));
                }
            }
            for factory in registration.intent_state_keys {
                let key = factory()?;
                if !actual_intents.insert(key) {
                    return Err(replay_registration_error(
                        "duplicate replay intent key in production registry",
                    ));
                }
            }
            if registration.intent_matcher.is_some() != declares_intents {
                return Err(replay_registration_error(
                    "replay intent matcher and declared intent keys disagree",
                ));
            }
        }
        if actual_states != expected_states {
            return Err(replay_registration_error(
                "replay state keys do not equal the authoring catalog",
            ));
        }
        if actual_intents != expected_intents {
            return Err(replay_registration_error(
                "replay intent keys do not equal the authoring catalog",
            ));
        }
        Ok(())
    }
}

impl ReplayVerifierRegistration {
    fn applies(&self, broker: &ReplayBroker<'_>) -> Result<bool> {
        for state_key_factory in self.state_keys {
            let expected = state_key_factory()?;
            let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
                Ok(node.state_kind == expected.kind && node.state_version == expected.version)
            })?;
            if !frames.is_empty() {
                return Ok(true);
            }
        }
        if let Some(intent_matcher) = self.intent_matcher {
            return Ok(!broker
                .side_effect_replay_frames_matching(intent_matcher)?
                .is_empty());
        }
        Ok(false)
    }
}

fn state_key<S: StateSpec>() -> Result<ReplayStateKey> {
    Ok(ReplayStateKey {
        kind: S::kind().map_err(replay_registration_error)?,
        version: S::version().map_err(replay_registration_error)?,
    })
}

fn replay_registration_error(error: impl std::fmt::Display) -> mfm_replay::v1::ReplayError {
    mfm_replay::v1::ReplayError::new(
        mfm_replay::v1::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn verify_portfolio(broker: &ReplayBroker<'_>, _registry: &CertificationRegistry) -> Result<()> {
    mfm_replay::v1::verify_external_read_state::<mfm_portfolio::SelectHoldingsState>(broker)?;
    mfm_replay::v1::verify_pure_state::<mfm_portfolio::AssembleSnapshotState>(broker)?;
    mfm_replay::v1::verify_pure_state::<mfm_portfolio::ProjectReportState>(broker)
}

fn verify_btc(broker: &ReplayBroker<'_>, _registry: &CertificationRegistry) -> Result<()> {
    mfm_bitcoin_live::verify_bitcoin_jsonrpc_replay(broker)
}

fn verify_evm_balance_collection(
    broker: &ReplayBroker<'_>,
    _registry: &CertificationRegistry,
) -> Result<()> {
    mfm_evm_live::verify_evm_balance_collection_replay(broker)
}

#[cfg(test)]
#[path = "replay_verifiers_tests.rs"]
mod tests;
