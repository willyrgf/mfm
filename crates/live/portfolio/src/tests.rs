use super::*;
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};

struct FakeStore;

impl store::FactQueryStore for FakeStore {
    type Error = store::StoreError;

    fn fact_query_implementation_id(&self) -> &'static str {
        "mfm.test.portfolio.fact-query.v1"
    }

    fn execute_fact_queries<'a>(
        &'a self,
        _plans: &'a [mfm_facts::CanonicalFactQueryPlan],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Vec<mfm_facts::FactQueryResult>, Self::Error>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(std::future::ready(Err(store::StoreError::Identity(
            "private fake query detail".to_owned(),
        ))))
    }
}

impl store::RetainedArtifactReadProvider for FakeStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        let artifact_id = requirement.artifact_id.clone();
        Box::pin(std::future::ready(Err(
            store::StoreError::MissingArtifact { artifact_id },
        )))
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
fn registration_accepts_one_fake_store_for_both_authorities() {
    let mut registry = registry();
    let pure_factory = registry.factory_binding(
        events::RunnerFactoryId::new(PURE_FACTORY).expect("pure factory identity"),
    );
    let read_factory = registry.factory_binding(
        events::RunnerFactoryId::new(READ_FACTORY).expect("read factory identity"),
    );
    let adapter_factory = registry.factory_binding(
        events::RunnerFactoryId::new(ADAPTER_FACTORY).expect("adapter factory identity"),
    );

    register_portfolio_live(
        &mut registry,
        Arc::new(FakeStore),
        &pure_factory,
        &read_factory,
        &adapter_factory,
    )
    .expect("single fake store registration");
}

#[test]
fn fact_query_store_failures_block_without_exposing_backend_detail() {
    let error = fact_query_runtime_error("private backend detail");
    assert_eq!(
        error,
        mfm_runtime::RuntimeError::Blocked("fact-query store is unavailable".to_owned())
    );
    assert!(!error.to_string().contains("private backend detail"));
}
