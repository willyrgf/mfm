use std::sync::Arc;

use mfm_bitcoin::{
    BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionResponse, BitcoinBalanceSession,
    BitcoinCapabilityError, BitcoinSessionFuture, BitcoinSourceBinding,
    BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};

use super::*;

struct FakeSession;

impl BitcoinBalanceSession for FakeSession {
    fn implementation_id(&self) -> &'static str {
        BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID
    }

    fn validate_binding<'a>(
        &'a self,
        _binding: &'a BitcoinSourceBinding,
    ) -> BitcoinSessionFuture<'a, ()> {
        Box::pin(std::future::ready(Ok(())))
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
    let session: Arc<dyn BitcoinBalanceSession> = Arc::new(FakeSession);

    register_bitcoin_jsonrpc_runners(&mut registry, session, &read_factory, &adapter_factory)
        .expect("fake-session registration");
}
