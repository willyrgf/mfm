use mfm_certify::CertificationRegistry;
use mfm_events::v1 as events;
use mfm_ids::{StateKind, StateVersion};
use mfm_program::StateSpec;
use mfm_replay::v1::{ReplayBroker, Result};

type ReplayVerifier = fn(&ReplayBroker, &CertificationRegistry) -> Result<()>;
type ReplayStateKeyFactory = fn() -> Result<ReplayStateKey>;
type ReplayIntentMatcher = fn(&events::side_effect::IntentPersisted) -> Result<bool>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayStateKey {
    kind: StateKind,
    version: StateVersion,
}

struct ReplayVerifierRegistration {
    state_keys: &'static [ReplayStateKeyFactory],
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
                        state_key::<mfm_state_portfolio::SelectHoldingsState>,
                        state_key::<mfm_state_portfolio::AssembleSnapshotState>,
                        state_key::<mfm_state_portfolio::ProjectReportState>,
                    ],
                    intent_matcher: None,
                    verifier: verify_portfolio,
                },
                ReplayVerifierRegistration {
                    state_keys: &[state_key::<mfm_states_btc::CollectBitcoinBalancesState>],
                    intent_matcher: None,
                    verifier: verify_btc,
                },
                ReplayVerifierRegistration {
                    state_keys: &[state_key::<mfm_states_evm::CollectEvmBalancesState>],
                    intent_matcher: None,
                    verifier: verify_evm_balance_collection,
                },
            ],
        }
    }

    pub(crate) fn verify(
        &self,
        broker: &ReplayBroker,
        certification_registry: &CertificationRegistry,
    ) -> Result<()> {
        for registration in self.registrations {
            if registration.applies(broker)? {
                (registration.verifier)(broker, certification_registry)?;
            }
        }
        Ok(())
    }
}

impl ReplayVerifierRegistration {
    fn applies(&self, broker: &ReplayBroker) -> Result<bool> {
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

fn verify_portfolio(broker: &ReplayBroker, _registry: &CertificationRegistry) -> Result<()> {
    mfm_replay::v1::verify_external_read_state::<mfm_state_portfolio::SelectHoldingsState>(broker)?;
    mfm_replay::v1::verify_pure_state::<mfm_state_portfolio::AssembleSnapshotState>(broker)?;
    mfm_replay::v1::verify_pure_state::<mfm_state_portfolio::ProjectReportState>(broker)
}

fn verify_btc(broker: &ReplayBroker, _registry: &CertificationRegistry) -> Result<()> {
    mfm_adapters_btc_jsonrpc::verify_bitcoin_jsonrpc_replay(broker)
}

fn verify_evm_balance_collection(
    broker: &ReplayBroker,
    _registry: &CertificationRegistry,
) -> Result<()> {
    mfm_adapters_evm::verify_evm_balance_collection_replay(broker)
}

#[cfg(test)]
#[path = "replay_verifiers_tests.rs"]
mod tests;
