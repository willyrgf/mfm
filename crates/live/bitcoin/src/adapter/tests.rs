use std::sync::Arc;

use mfm_bitcoin::{
    BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionResponse, BitcoinBalanceSession,
    BitcoinCapabilityError, BitcoinSessionFuture, BitcoinSourceBinding,
    BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};

use super::*;

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

struct FakeSession;

impl BitcoinBalanceSession for FakeSession {
    fn implementation_id(&self) -> &'static str {
        BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID
    }

    fn validate_binding(
        &self,
        _binding: &BitcoinSourceBinding,
    ) -> Result<(), BitcoinCapabilityError> {
        Ok(())
    }

    fn collect_balances<'a>(
        &'a self,
        _request: &'a BitcoinBalanceCollectionRequest,
    ) -> BitcoinSessionFuture<'a, BitcoinBalanceCollectionResponse> {
        Box::pin(std::future::ready(Err(BitcoinCapabilityError::provider(
            mfm_capabilities::ProviderDiagnosticCode::SourceUnavailable,
            "fake_session",
            true,
        ))))
    }
}

fn registry() -> ErasedRunnerRegistry {
    ErasedRunnerRegistry::new(mfm_runtime::ExecutableIdentityTemplate::new(
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x42; 32]),
        ),
    ))
}

#[test]
fn adapter_registration_accepts_a_fake_pure_session() {
    let mut registry = registry();
    let read_factory = registry.factory_binding(
        events::RunnerFactoryId::new(READ_FACTORY).expect("read factory identity"),
    );
    let adapter_factory = registry.factory_binding(
        events::RunnerFactoryId::new(ADAPTER_FACTORY).expect("adapter factory identity"),
    );
    let artifacts: Arc<dyn store::RetainedArtifactReadProvider> = Arc::new(MissingArtifacts);
    let session: Arc<dyn BitcoinBalanceSession> = Arc::new(FakeSession);

    register_bitcoin_jsonrpc_runners(
        &mut registry,
        artifacts,
        session,
        &read_factory,
        &adapter_factory,
    )
    .expect("fake-session registration");
}
