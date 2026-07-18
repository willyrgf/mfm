use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use alloy_primitives::{Address, Bytes, U256};
use mfm_evm_capabilities::{
    EvmBlock, EvmBlockSelector, EvmCall, EvmCode, EvmSessionEvidence, EvmSessionFuture,
};

struct MissingArtifacts;

impl store::RetainedArtifactReadProvider for MissingArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        let artifact_id = requirement.artifact_id.clone();
        Box::pin(async move { Err(store::StoreError::MissingArtifact { artifact_id }) })
    }
}

struct CountingReadSession {
    evidence: EvmSessionEvidence,
    uses: AtomicUsize,
}

impl CountingReadSession {
    fn with_chain_id(chain_id: u64) -> Self {
        let binding = EvmNetworkBinding::new(
            mfm_ids::LocalPublicId::new("ethereum-mainnet").expect("network id"),
            chain_id,
        )
        .expect("network binding");
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("primary").expect("source ref"),
                mfm_ids::LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            uses: AtomicUsize::new(0),
        }
    }

    fn used<'a, T: Send + 'a>(&'a self) -> EvmSessionFuture<'a, T> {
        self.uses.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Err(
            EvmCapabilityError::provider_failure(mfm_evm_capabilities::evm_diagnostic(
                ProviderDiagnosticCode::ProviderConfigurationInvalid,
            )),
        )))
    }
}

impl EvmReadSession for CountingReadSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(&'a self, _selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock> {
        self.used()
    }

    fn read_balance<'a>(
        &'a self,
        _account: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        self.used()
    }

    fn read_code<'a>(
        &'a self,
        _address: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        self.used()
    }

    fn call<'a>(&'a self, _request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        self.used()
    }
}

#[tokio::test]
async fn wrong_session_binding_fails_before_using_validation_authority() {
    let requested = EvmNetworkBinding::new(
        mfm_ids::LocalPublicId::new("ethereum-mainnet").expect("network id"),
        1,
    )
    .expect("requested binding");
    let session = Arc::new(CountingReadSession::with_chain_id(2));
    let bound_session = Arc::clone(&session);
    let capabilities = EvmValidationRunnerCapabilities::new(
        Arc::new(MissingArtifacts),
        |_| Ok(()),
        move |_| {
            let session = Arc::clone(&bound_session);
            Box::pin(async move { Ok(session as Arc<dyn EvmReadSession>) })
        },
    );

    let error = match capabilities.bind(requested).await {
        Ok(_) => panic!("wrong session binding must fail"),
        Err(error) => error,
    };

    assert_eq!(
        error.redacted_diagnostic().expect("diagnostic").code(),
        ProviderDiagnosticCode::ProviderConfigurationInvalid
    );
    assert_eq!(session.uses.load(Ordering::SeqCst), 0);
}
