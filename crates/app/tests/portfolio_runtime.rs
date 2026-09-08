use std::future::Future;
use std::num::NonZeroU64;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_app::{
    Application, BoundCapabilitySet, ComposedRuntime, ConfigDocument, ConfigDocumentError,
    ConfigSelection, ImportOutcome, PublicBindingView, RequestError, RunPageLimit, RunRecovery,
    SerializableRunView, MAX_EVM_BINDINGS,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_config::{
    ConfigDigest, ConfigFuture, ConfigImportResult, ConfigName, ConfigRepository,
    ConfigRepositoryError, ConfigRevision, MemoryConfigRepository, MAX_CONFIG_DOCUMENT_BYTES,
};
use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallIntent, EvmBlockAnchor, EvmEndpoint, EvmHash,
    EvmReadEvidence, EvmReadIntent, EvmReadSubject, EvmReadValue, EvmTokenDecimals, EvmU256,
};
use mfm_evm_live::{EvmReadProvider, ProviderFuture};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, RunId};
use mfm_runtime::{AdapterError, RunViewState};
use mfm_store::{AppendResult, MemoryStore, Store, StoreError};

const ANCHOR: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Provider {
    chain_id: u64,
    calls: AtomicUsize,
    available: std::sync::atomic::AtomicBool,
}

impl EvmReadProvider for Provider {
    fn observe<'a>(
        &'a self,
        intent_value_ref: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> ProviderFuture<'a, EvmReadEvidence> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !self.available.load(Ordering::SeqCst) {
                return Err(AdapterError::Unavailable);
            }
            let value = match intent.subject() {
                EvmReadSubject::ChainIdentity => EvmReadValue::ChainId(
                    NonZeroU64::new(self.chain_id).ok_or(AdapterError::Internal)?,
                ),
                EvmReadSubject::InitialAnchor | EvmReadSubject::ConfirmAnchor { .. } => {
                    EvmReadValue::Anchor(EvmBlockAnchor {
                        number: EvmU256::new("100").expect("number"),
                        hash: EvmHash::new(ANCHOR).expect("hash"),
                    })
                }
                EvmReadSubject::NativeBalance { .. } => {
                    EvmReadValue::RawUnits(EvmU256::new("1000000000000000000").expect("units"))
                }
                EvmReadSubject::TokenDecimals { .. } => {
                    EvmReadValue::TokenDecimals(EvmTokenDecimals::new(6).expect("decimals"))
                }
                EvmReadSubject::TokenBalance { .. } => {
                    EvmReadValue::RawUnits(EvmU256::new("2500000").expect("units"))
                }
            };
            Ok(EvmReadEvidence::returned(intent_value_ref.clone(), value))
        })
    }

    fn observe_anchored_call<'a>(
        &'a self,
        _intent_value_ref: &'a ContentRef,
        _intent: &'a AnchoredContractCallIntent,
    ) -> ProviderFuture<'a, AnchoredContractCallEvidence> {
        Box::pin(async { Err(AdapterError::Internal) })
    }
}

fn provider(chain_id: u64) -> Arc<Provider> {
    Arc::new(Provider {
        chain_id,
        calls: AtomicUsize::new(0),
        available: std::sync::atomic::AtomicBool::new(true),
    })
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
}

fn config_name(value: &str) -> ConfigName {
    ConfigName::new(value).expect("config name")
}

fn selection(outcome: &ImportOutcome) -> ConfigSelection {
    ConfigSelection::new(
        outcome.config().name().clone(),
        outcome.config().digest().clone(),
    )
}

fn native_collection(chain_id: u64, suffix: &str) -> serde_json::Value {
    serde_json::json!({
        "correlation": format!("native-{suffix}"),
        "request": {
            "sources": [{
                "source_id": format!("wallet-{suffix}.native"),
                "chain_id": chain_id,
                "address": "0x1111111111111111111111111111111111111111",
                "token": null
            }],
            "decimals": 18
        }
    })
}

fn document_value(routes: Vec<(u64, &str)>, portfolio_id: &str) -> serde_json::Value {
    let collections = routes
        .iter()
        .enumerate()
        .map(|(index, (chain_id, _))| native_collection(*chain_id, &index.to_string()))
        .collect::<Vec<_>>();
    let routes = routes
        .into_iter()
        .map(|(chain_id, endpoint_id)| {
            serde_json::json!({ "chain_id": chain_id, "endpoint_id": endpoint_id })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "entry_point": "mfm.portfolio/snapshot@1",
        "input": {
            "routes": routes,
            "selector": { "target": portfolio_id, "quote": "usd" },
            "portfolio": {
                "portfolio_id": "portfolio-example",
                "quotes": ["usd"],
                "collections": collections
            }
        }
    })
}

async fn document(routes: Vec<(u64, &str)>) -> ConfigDocument {
    ConfigDocument::new(
        serde_json::to_vec(&document_value(routes, "portfolio-example")).expect("document JSON"),
    )
    .await
    .expect("config document")
}

fn application(routes: &[(u64, &str, Arc<Provider>)]) -> Application {
    application_with_backend(routes, Arc::new(FaultStore::new()))
}

fn application_with_backend(
    routes: &[(u64, &str, Arc<Provider>)],
    backend: Arc<FaultStore>,
) -> Application {
    let bindings = routes
        .iter()
        .map(|(chain_id, endpoint_id, provider)| {
            (
                *chain_id,
                EvmEndpoint::new(*endpoint_id).expect("endpoint"),
                provider.clone() as Arc<dyn EvmReadProvider>,
            )
        })
        .collect();
    let composed = ComposedRuntime::compose(
        backend,
        BoundCapabilitySet::new(bindings).expect("bindings"),
    )
    .expect("composition");
    Application::from_parts(composed, Arc::new(MemoryConfigRepository::default()))
}

struct HostileConfigRepository {
    revision: ConfigRevision,
}

impl ConfigRepository for HostileConfigRepository {
    fn import_config<'a>(
        &'a self,
        _revision: &'a ConfigRevision,
    ) -> ConfigFuture<'a, ConfigImportResult> {
        Box::pin(async { Err(ConfigRepositoryError::Corrupt) })
    }

    fn load_config<'a>(
        &'a self,
        _name: &'a ConfigName,
        _digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, Option<ConfigRevision>> {
        Box::pin(async { Ok(Some(self.revision.clone())) })
    }

    fn list_configs(&self) -> ConfigFuture<'_, Vec<ConfigRevision>> {
        Box::pin(async { Err(ConfigRepositoryError::Corrupt) })
    }

    fn delete_config<'a>(
        &'a self,
        _name: &'a ConfigName,
        _digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, ()> {
        Box::pin(async { Err(ConfigRepositoryError::Corrupt) })
    }
}

fn application_with_config_repository(revision: ConfigRevision) -> Application {
    let composed = ComposedRuntime::compose(
        Arc::new(FaultStore::new()),
        BoundCapabilitySet::new(Vec::new()).expect("bindings"),
    )
    .expect("composition");
    Application::from_parts(composed, Arc::new(HostileConfigRepository { revision }))
}

struct FaultStore {
    inner: MemoryStore,
    indeterminate_next_append: std::sync::atomic::AtomicBool,
}

impl FaultStore {
    fn new() -> Self {
        Self {
            inner: MemoryStore::new(),
            indeterminate_next_append: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn fail_next_append(&self) {
        self.indeterminate_next_append.store(true, Ordering::SeqCst);
    }
}

impl Store for FaultStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<mfm_journal::StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        self.inner.load_run(run_id)
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        if self.indeterminate_next_append.swap(false, Ordering::SeqCst) {
            Box::pin(async { Err(StoreError::Indeterminate) })
        } else {
            self.inner.append_run(frame)
        }
    }
}

impl mfm_store::RunIndex for FaultStore {
    fn list_runs<'a>(
        &'a self,
        after: Option<&'a RunId>,
        limit: RunPageLimit,
    ) -> Pin<
        Box<dyn Future<Output = Result<mfm_store::RunPage, mfm_store::RunIndexError>> + Send + 'a>,
    > {
        mfm_store::RunIndex::list_runs(&self.inner, after, limit)
    }
}

#[test]
fn composed_runtime_owns_one_checked_multi_route_truth() {
    let first = provider(1);
    let second = provider(1);
    let third = provider(2);
    let app = application(&[
        (1, "alpha", first),
        (1, "beta", second),
        (2, "alpha", third),
    ]);
    assert_eq!(app.bindings().len(), 3);
    for (view, (expected_chain, expected_endpoint)) in
        app.bindings()
            .iter()
            .zip([(1, "alpha"), (1, "beta"), (2, "alpha")])
    {
        let PublicBindingView::Evm {
            chain_id,
            endpoint_id,
            binding_ref,
        } = view;
        assert_eq!(*chain_id, expected_chain);
        assert_eq!(endpoint_id, expected_endpoint);
        let endpoint_ref = EvmEndpoint::new(expected_endpoint)
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("endpoint ref");
        let target = mfm_evm::EvmPhysicalTarget {
            chain_id: NonZeroU64::new(expected_chain).expect("nonzero chain"),
            endpoint_ref,
        };
        assert_eq!(binding_ref, &target.binding_ref().expect("binding ref"));
    }

    let duplicate = vec![
        (
            1,
            EvmEndpoint::new("alpha").expect("endpoint"),
            provider(1) as Arc<dyn EvmReadProvider>,
        ),
        (
            1,
            EvmEndpoint::new("alpha").expect("endpoint"),
            provider(1) as Arc<dyn EvmReadProvider>,
        ),
    ];
    assert!(BoundCapabilitySet::new(duplicate).is_err());
    let over_capacity = (0..=MAX_EVM_BINDINGS)
        .map(|index| {
            (
                (index + 1) as u64,
                EvmEndpoint::new("endpoint").expect("endpoint"),
                provider((index + 1) as u64) as Arc<dyn EvmReadProvider>,
            )
        })
        .collect();
    assert!(BoundCapabilitySet::new(over_capacity).is_err());
}

#[tokio::test]
async fn config_document_boundary_is_strict_and_canonical() {
    let reordered = br#"{
      "input":{"portfolio":{"quotes":["usd"],"portfolio_id":"portfolio-example","collections":[{"request":{"sources":[{"token":null,"source_id":"wallet-0.native","chain_id":1,"address":"0x1111111111111111111111111111111111111111"}],"decimals":18},"correlation":"native-0"}]},"selector":{"quote":"usd","target":"portfolio-example"},"routes":[{"endpoint_id":"alpha","chain_id":1}]},
      "entry_point":"mfm.portfolio/snapshot@1"}"#;
    let checked = ConfigDocument::new(reordered.to_vec())
        .await
        .expect("reordered document");
    let app = application(&[(1, "alpha", provider(1))]);
    let imported = app
        .import_config(config_name("daily"), checked)
        .await
        .expect("import");
    assert_eq!(
        imported.config().digest().as_str(),
        "content:sha256-jcs-v1:0723d5ccf638cfcf6bb85d96de269e93adcef0847147ace563d09464c8c12250"
    );
    let listed = app.list_configs().await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        ConfigDigest::parse(listed[0].digest().as_str()).expect("digest"),
        *listed[0].digest()
    );

    for malformed in [
        br#"{"entry_point":"x","entry_point":"y"}"#.as_slice(),
        br#"{"value":1.5}"#,
        b"\xff",
    ] {
        assert_eq!(
            ConfigDocument::new(malformed.to_vec()).await.err(),
            Some(ConfigDocumentError::Malformed)
        );
    }
    assert_eq!(
        ConfigDocument::new(vec![b'x'; MAX_CONFIG_DOCUMENT_BYTES + 1])
            .await
            .err(),
        Some(ConfigDocumentError::TooLarge)
    );
    let mut unknown = document_value(vec![(1, "alpha")], "portfolio-example");
    unknown["input"]["unknown"] = serde_json::json!(true);
    assert_eq!(
        ConfigDocument::new(serde_json::to_vec(&unknown).expect("JSON"))
            .await
            .err(),
        Some(ConfigDocumentError::Invalid)
    );
    for routes in [
        vec![],
        vec![(2, "beta"), (1, "alpha")],
        vec![(1, "a"), (1, "b")],
    ] {
        assert!(ConfigDocument::new(
            serde_json::to_vec(&document_value(routes, "portfolio-example")).expect("JSON")
        )
        .await
        .is_err());
    }
}

#[tokio::test]
async fn stored_config_lifecycle_uses_exact_revisions_and_preserves_admitted_runs() {
    let first = provider(1);
    let second = provider(2);
    let app = application(&[(1, "alpha", first.clone()), (2, "beta", second.clone())]);
    let name = config_name("daily");
    let created = app
        .import_config(
            name.clone(),
            document(vec![(1, "alpha"), (2, "beta")]).await,
        )
        .await
        .expect("created");
    let digest = created.config().digest().clone();
    let unchanged = app
        .import_config(
            name.clone(),
            document(vec![(1, "alpha"), (2, "beta")]).await,
        )
        .await
        .expect("unchanged");
    assert!(matches!(unchanged, ImportOutcome::Unchanged { .. }));
    assert_eq!(unchanged.config(), created.config());

    let started = app
        .start_run(run_id(10), &selection(&created))
        .await
        .expect("exact start");
    assert_eq!(started.config().digest(), &digest);
    assert!(matches!(started.run().state(), RunViewState::Succeeded(_)));
    assert_eq!(first.calls.load(Ordering::SeqCst), 4);
    assert_eq!(second.calls.load(Ordering::SeqCst), 4);
    let rendered = serde_json::to_value(SerializableRunView::new(started.run())).expect("JSON");
    assert_eq!(rendered["state"]["kind"], "succeeded");
    assert_eq!(
        rendered["state"]["value"]["snapshot"]["collections"]
            .as_array()
            .expect("snapshot collections")
            .len(),
        2
    );
    assert_eq!(
        rendered["state"]["value"]["report"]["totals_by_quote"][0]["total_value_dec"],
        "2"
    );

    let second_revision = app
        .import_config(name.clone(), document(vec![(1, "alpha")]).await)
        .await
        .expect("second revision");
    assert!(matches!(second_revision, ImportOutcome::Created { .. }));
    assert_ne!(second_revision.config().digest(), &digest);

    let exact_second = app
        .start_run(run_id(11), &selection(&second_revision))
        .await
        .expect("exact start");
    assert_eq!(exact_second.config(), second_revision.config());
    let exact_first = app
        .start_run(
            run_id(12),
            &ConfigSelection::new(name.clone(), digest.clone()),
        )
        .await
        .expect("first exact start");
    assert_eq!(exact_first.config().digest(), &digest);
    let wrong = ConfigDigest::new(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([9; 32]),
    ))
    .expect("digest");
    assert!(matches!(
        app.start_run(run_id(13), &ConfigSelection::new(name.clone(), wrong),)
            .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::ConfigAbsent
        ))
    ));

    let configs = app.list_configs().await.expect("configs");
    assert_eq!(configs.len(), 2);
    assert!(configs
        .iter()
        .any(|config| config.digest() == second_revision.config().digest()));
    assert!(configs.iter().any(|config| config.digest() == &digest));
    app.delete_config(&name, &digest)
        .await
        .expect("delete first");
    app.delete_config(&name, &digest)
        .await
        .expect("idempotent delete");
    assert!(matches!(
        app.start_run(run_id(14), &ConfigSelection::new(name, digest.clone()),)
            .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::ConfigAbsent
        ))
    ));
    assert_eq!(app.list_configs().await.expect("retained configs").len(), 1);
    let runs = app
        .list_runs(None, RunPageLimit::default())
        .await
        .expect("run page");
    assert_eq!(runs.items().len(), 3);

    let retained = app
        .read_run(&run_id(10))
        .await
        .expect("admitted run survives config deletion");
    assert_eq!(retained.head_digest(), started.run().head_digest());
}

#[tokio::test]
async fn start_revalidates_hostile_config_rows_before_binding_errors() {
    let name = config_name("hostile-digest");
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&document_value(vec![(1, "alpha")], "portfolio-example"))
            .expect("JSON"),
    )
    .expect("canonical document");
    let actual_digest = ConfigDigest::new(canonical.content_digest()).expect("actual digest");
    let false_digest = ConfigDigest::new(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([9; 32]),
    ))
    .expect("false digest");
    let revision =
        ConfigRevision::new(name.clone(), false_digest, canonical.to_vec()).expect("revision");
    assert!(matches!(
        application_with_config_repository(revision)
            .start_run(run_id(14), &ConfigSelection::new(name, actual_digest),)
            .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::InvalidRetainedConfig
        ))
    ));

    let name = config_name("hostile-plan");
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&document_value(vec![(1, "alpha")], "other-portfolio"))
            .expect("JSON"),
    )
    .expect("canonical document");
    let digest = ConfigDigest::new(canonical.content_digest()).expect("digest");
    let revision =
        ConfigRevision::new(name.clone(), digest.clone(), canonical.to_vec()).expect("revision");
    assert!(matches!(
        application_with_config_repository(revision)
            .start_run(run_id(15), &ConfigSelection::new(name, digest))
            .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::InvalidRetainedConfig
        ))
    ));
}

#[tokio::test]
async fn invalid_or_unbound_config_fails_before_run_store_io() {
    let app = application(&[(1, "alpha", provider(1))]);
    let invalid_name = config_name("invalid");
    let invalid = ConfigDocument::new(
        serde_json::to_vec(&document_value(vec![(1, "alpha")], "other-portfolio")).expect("JSON"),
    )
    .await
    .expect("typed document");
    assert_eq!(
        app.import_config(invalid_name, invalid).await,
        Err(RequestError::InvalidConfigDocument)
    );

    let unbound_name = config_name("unbound");
    let imported = app
        .import_config(
            unbound_name.clone(),
            document(vec![(u64::MAX, "upper-half")]).await,
        )
        .await
        .expect("deployment-independent import");
    assert!(matches!(
        app.start_run(run_id(20), &selection(&imported)).await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::BindingUnbound
        ))
    ));
    assert!(app
        .list_runs(None, RunPageLimit::default())
        .await
        .expect("run page")
        .items()
        .is_empty());
}

#[tokio::test]
async fn ambiguous_run_appends_carry_exact_start_and_progress_recovery_sums() {
    let start_backend = Arc::new(FaultStore::new());
    let start_app =
        application_with_backend(&[(1, "alpha", provider(1))], Arc::clone(&start_backend));
    let start_name = config_name("start-recovery");
    let imported = start_app
        .import_config(start_name.clone(), document(vec![(1, "alpha")]).await)
        .await
        .expect("import");
    start_backend.fail_next_append();
    let Err(error) = start_app.start_run(run_id(40), &selection(&imported)).await else {
        panic!("start append must be ambiguous");
    };
    assert_eq!(error.code(), "run_append_indeterminate");
    assert!(matches!(
        error.recovery(),
        Some(RunRecovery::Start { run_id: retained, config })
            if retained == &run_id(40) && config == imported.config()
    ));

    let progress_backend = Arc::new(FaultStore::new());
    let progress_provider = provider(1);
    progress_provider.available.store(false, Ordering::SeqCst);
    let progress_app = application_with_backend(
        &[(1, "alpha", Arc::clone(&progress_provider))],
        Arc::clone(&progress_backend),
    );
    let progress_name = config_name("progress-recovery");
    let progress_config = progress_app
        .import_config(progress_name.clone(), document(vec![(1, "alpha")]).await)
        .await
        .expect("import");
    assert!(matches!(
        progress_app
            .start_run(run_id(41), &selection(&progress_config))
            .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::DependencyUnavailable
        ))
    ));
    progress_provider.available.store(true, Ordering::SeqCst);
    progress_backend.fail_next_append();
    let Err(error) = progress_app.progress_run(&run_id(41)).await else {
        panic!("progress append must be ambiguous");
    };
    assert!(matches!(
        error.recovery(),
        Some(RunRecovery::Progress { run_id: retained }) if retained == &run_id(41)
    ));
}
