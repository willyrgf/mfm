use mfm_certify::CertificationRegistry;
use mfm_replay::v1::{ReplayBroker, Result};

type ReplayVerifier = fn(&ReplayBroker, &CertificationRegistry) -> Result<()>;

/// The one production replay verifier registry compiled into the application.
///
/// Verifiers are evidence-only domain functions. The certification registry is passed through
/// because some domain contracts validate imported certified runs in addition to this run.
pub(crate) struct ReplayVerifierRegistry {
    verifiers: &'static [ReplayVerifier],
}

impl ReplayVerifierRegistry {
    pub(crate) const fn production() -> Self {
        Self {
            verifiers: &[
                verify_portfolio,
                verify_btc,
                verify_evm,
                verify_contracts,
                verify_proof,
            ],
        }
    }

    pub(crate) fn verify(
        &self,
        broker: &ReplayBroker,
        certification_registry: &CertificationRegistry,
    ) -> Result<()> {
        for verifier in self.verifiers {
            verifier(broker, certification_registry)?;
        }
        Ok(())
    }
}

fn verify_portfolio(broker: &ReplayBroker, _registry: &CertificationRegistry) -> Result<()> {
    mfm_adapters_portfolio::verify_portfolio_replay(broker)
}

fn verify_btc(broker: &ReplayBroker, _registry: &CertificationRegistry) -> Result<()> {
    mfm_adapters_btc_jsonrpc::verify_btc_jsonrpc_replay(broker)
}

fn verify_evm(broker: &ReplayBroker, _registry: &CertificationRegistry) -> Result<()> {
    mfm_adapters_evm::verify_evm_native_balance_replay(broker)
}

fn verify_contracts(broker: &ReplayBroker, registry: &CertificationRegistry) -> Result<()> {
    mfm_adapters_evm_contracts::verify_contract_lifecycle_replay(broker, registry)
}

fn verify_proof(broker: &ReplayBroker, _registry: &CertificationRegistry) -> Result<()> {
    mfm_transports_proof::verify_deterministic_proof_replay(broker)
}
