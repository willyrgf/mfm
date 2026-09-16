use std::future::Future;
use std::num::NonZeroU64;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use mfm_app::{
    Application, BoundCapabilitySet, ComposedRuntime, ConfigDocument, ConfigDocumentError,
    ConfigSelection, ImportOutcome, PublicBindingView, RequestError, RunPageLimit, RunRecovery,
    SerializableRunView, MAX_CONFIG_DOCUMENT_BYTES, MAX_EVM_BINDINGS,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::AdapterError;
use mfm_config::{
    ConfigDigest, ConfigFuture, ConfigImportResult, ConfigRepository, ConfigRepositoryError,
    ConfigRevision, MemoryConfigRepository,
};
use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallIntent, EvmBalanceFailure, EvmBlockAnchor,
    EvmEndpoint, EvmHash, EvmOperationalError, EvmOperationalKind, EvmReadEvidence, EvmReadIntent,
    EvmReadSubject, EvmReadValue, EvmTokenDecimals, EvmU256,
};
use mfm_evm_live::{EvmReadProvider, ProviderFuture};
use mfm_ids::{ConfigName, ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, RunId};
use mfm_portfolio::{PortfolioEnrichmentOutput, PortfolioSnapshotFailure, PortfolioSnapshotOutput};
use mfm_runtime::{Failure, RunViewState};
use mfm_store::{AppendResult, MemoryStore, Store, StoreError};
use mfm_values::{MfmValue, Object};

const ANCHOR: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NATIVE_SNAPSHOT: &str = r#"{
  "entry_point": "mfm.portfolio/snapshot@1",
  "input": {
    "routes": [{"chain_id": 1, "endpoint_id": "alpha"}],
    "selector": {"target": "portfolio-example", "quote": "usd"},
    "portfolio": {
      "portfolio_id": "portfolio-example",
      "quotes": ["usd"],
      "collections": [{
        "correlation": "native-0",
        "request": {
          "decimals": 18,
          "sources": [{
            "source_id": "wallet-a.native",
            "chain_id": 1,
            "address": "0x1111111111111111111111111111111111111111",
            "token": null
          }]
        }
      }]
    }
  }
}"#;

#[derive(Clone, Copy)]
enum ProviderMode {
    Ready,
    Blocked,
    Timeout,
    RejectBalance,
    TimeoutBalance,
}

struct Provider {
    chain_id: u64,
    mode: std::sync::Mutex<ProviderMode>,
    entered: tokio::sync::Notify,
}

impl EvmReadProvider for Provider {
    fn observe<'a>(
        &'a self,
        intent_value_ref: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> ProviderFuture<'a, EvmReadEvidence> {
        Box::pin(async move {
            let mode = *self.mode.lock().unwrap();
            match (mode, intent.subject()) {
                (ProviderMode::Blocked, _) => {
                    self.entered.notify_one();
                    return std::future::pending().await;
                }
                (ProviderMode::Timeout, _)
                | (
                    ProviderMode::TimeoutBalance,
                    EvmReadSubject::NativeBalance { .. }
                    | EvmReadSubject::TokenDecimals { .. }
                    | EvmReadSubject::TokenBalance { .. },
                ) => {
                    return Err(AdapterError::Operational(EvmOperationalError::new(
                        EvmOperationalKind::Timeout,
                        serde_json::from_value(serde_json::json!({
                            "method": "chain_id",
                            "stage": "send",
                            "failure": {"kind": "client"},
                            "diagnostics": {
                                "response": null,
                                "sources": {"layers": [], "end": "unavailable"},
                                "omissions": [],
                                "omissions_truncated": false
                            }
                        }))
                        .unwrap(),
                    )));
                }
                (
                    ProviderMode::RejectBalance,
                    EvmReadSubject::NativeBalance { .. } | EvmReadSubject::TokenBalance { .. },
                ) => {
                    return Ok(EvmReadEvidence::rejected(intent_value_ref.clone()));
                }
                _ => {}
            }
            let value = match intent.subject() {
                EvmReadSubject::ChainIdentity => {
                    EvmReadValue::ChainId(NonZeroU64::new(self.chain_id).expect("nonzero chain"))
                }
                EvmReadSubject::InitialAnchor | EvmReadSubject::ConfirmAnchor { .. } => {
                    EvmReadValue::Anchor(EvmBlockAnchor {
                        number: EvmU256::new("100").expect("number"),
                        hash: EvmHash::new(ANCHOR).expect("hash"),
                    })
                }
                EvmReadSubject::NativeBalance { source, .. } if source.source_id() == "native" => {
                    EvmReadValue::RawUnits(EvmU256::from_u64(0))
                }
                EvmReadSubject::NativeBalance { .. } => {
                    EvmReadValue::RawUnits(EvmU256::new("1000000000000000000").expect("units"))
                }
                EvmReadSubject::TokenDecimals { .. } => {
                    EvmReadValue::TokenDecimals(EvmTokenDecimals::new(18).expect("decimals"))
                }
                EvmReadSubject::TokenBalance { source, .. } if source.source_id() == "funded" => {
                    EvmReadValue::RawUnits(EvmU256::from_u64(1))
                }
                EvmReadSubject::TokenBalance { source, .. } if source.source_id() == "empty" => {
                    EvmReadValue::RawUnits(EvmU256::from_u64(0))
                }
                EvmReadSubject::TokenBalance { .. } => {
                    EvmReadValue::RawUnits(EvmU256::new("1000000000000000000").expect("units"))
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
        Box::pin(async {
            Err(AdapterError::Invariant(
                mfm_values::InvocationDiagnostic::from_fields(
                    "state_internal",
                    "observe_anchored_call",
                    &(mfm_evm::EvmDomainError::InvalidValue),
                    None,
                ),
            ))
        })
    }
}

fn provider(chain_id: u64) -> Arc<Provider> {
    Arc::new(Provider {
        chain_id,
        mode: std::sync::Mutex::new(ProviderMode::Ready),
        entered: tokio::sync::Notify::new(),
    })
}

fn open<B>(
    routes: &[(u64, &str, Arc<Provider>)],
    backend: Arc<B>,
    configs: Arc<dyn ConfigRepository>,
) -> Application
where
    B: Store + mfm_store::RunIndex + 'static,
{
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
    Application::from_parts(
        ComposedRuntime::compose(
            backend,
            BoundCapabilitySet::new(bindings).expect("bindings"),
        )
        .expect("composition"),
        configs,
    )
}

struct FaultStore {
    inner: MemoryStore,
    indeterminate_next_append: AtomicBool,
}

impl FaultStore {
    fn new() -> Self {
        Self {
            inner: MemoryStore::new(),
            indeterminate_next_append: AtomicBool::new(false),
        }
    }

    fn fail_next_append(&self) {
        self.indeterminate_next_append
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Store for FaultStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
        probe_sequence: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<mfm_store::LoadedRun>, StoreError>> + Send + 'a>>
    {
        self.inner.load_run(run_id, probe_sequence)
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        if self
            .indeterminate_next_append
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            Box::pin(async {
                Err(StoreError::Indeterminate(
                    mfm_values::DiagnosticEvidence::from_value(
                        serde_json::json!({"operation": "test.store", "injected": "Indeterminate"}),
                    ),
                ))
            })
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

#[derive(Default)]
struct LostPublicationAck {
    inner: MemoryConfigRepository,
    lose_next: AtomicBool,
}

impl ConfigRepository for LostPublicationAck {
    fn import_config<'a>(
        &'a self,
        revision: &'a ConfigRevision,
    ) -> ConfigFuture<'a, ConfigImportResult> {
        Box::pin(async move {
            let result = self.inner.import_config(revision).await?;
            if self
                .lose_next
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                Err(ConfigRepositoryError::Indeterminate)
            } else {
                Ok(result)
            }
        })
    }

    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, Option<ConfigRevision>> {
        self.inner.load_config(name, digest)
    }

    fn list_configs(&self) -> ConfigFuture<'_, Vec<ConfigRevision>> {
        self.inner.list_configs()
    }

    fn delete_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, ()> {
        self.inner.delete_config(name, digest)
    }
}

#[test]
fn bound_routes_reject_duplicates_and_over_capacity() {
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
async fn snapshot_starts_from_exact_revision_and_returns_holdings() {
    let app = open(
        &[(1, "alpha", provider(1))],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let name = ConfigName::new("daily").expect("name");
    let document = ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
        .await
        .expect("document");
    let created = app
        .import_config(name.clone(), document)
        .await
        .expect("created");
    assert!(matches!(created, ImportOutcome::Created { .. }));
    let unchanged = app
        .import_config(
            name.clone(),
            ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
                .await
                .expect("document"),
        )
        .await
        .expect("unchanged");
    assert!(matches!(unchanged, ImportOutcome::Unchanged { .. }));
    assert_eq!(unchanged.config(), created.config());

    let PublicBindingView::Evm {
        chain_id,
        endpoint_id,
        binding_ref,
    } = &app.bindings()[0];
    assert_eq!(*chain_id, 1);
    assert_eq!(endpoint_id, "alpha");
    assert_eq!(
        binding_ref,
        &EvmEndpoint::new("alpha")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .and_then(|endpoint_ref| mfm_evm::EvmPhysicalTarget {
                chain_id: NonZeroU64::new(1).expect("chain"),
                endpoint_ref,
            }
            .binding_ref())
            .expect("binding")
    );

    let run_id = RunId::from_digest(DigestBytes::from_array([10; 32]));
    let started = app
        .start_run(
            run_id.clone(),
            &ConfigSelection::new(name, created.config().digest().clone()),
        )
        .await
        .expect("start");
    assert_eq!(started.config().digest(), created.config().digest());
    let RunViewState::Succeeded(value) = started.run().state() else {
        panic!("snapshot must succeed");
    };
    let output = value.decode::<PortfolioSnapshotOutput>().expect("output");
    assert_eq!(
        serde_json::to_value(&output).expect("json"),
        serde_json::from_str::<serde_json::Value>(include_str!(
            "../../../docs/contracts/evm-portfolio/portfolio-snapshot.json"
        ))
        .expect("frozen snapshot")
    );
    let listed = app
        .list_runs(None, RunPageLimit::default())
        .await
        .expect("runs");
    assert_eq!(listed.items()[0].run_id(), &run_id);
}

#[tokio::test]
async fn snapshot_progresses_after_an_interrupted_read() {
    let provider = provider(1);
    *provider.mode.lock().unwrap() = ProviderMode::Blocked;
    let app = open(
        &[(1, "alpha", provider.clone())],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let imported = app
        .import_config(
            ConfigName::new("daily").expect("name"),
            ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    let run_id = RunId::from_digest(DigestBytes::from_array([11; 32]));
    {
        let selected = ConfigSelection::new(
            imported.config().name().clone(),
            imported.config().digest().clone(),
        );
        let start = app.start_run(run_id.clone(), &selected);
        tokio::pin!(start);
        tokio::select! {
            result = &mut start => panic!("blocked Read completed: {}", result.is_ok()),
            _ = provider.entered.notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => panic!("Read was not entered"),
        }
    }
    let retained = app.read_run(&run_id).await.expect("prefix");
    assert!(matches!(retained.state(), RunViewState::Runnable { .. }));
    *provider.mode.lock().unwrap() = ProviderMode::Ready;
    let finished = app.progress_run(&run_id).await.expect("progress");
    let RunViewState::Succeeded(value) = finished.state() else {
        panic!("progress must succeed");
    };
    value
        .decode::<PortfolioSnapshotOutput>()
        .expect("snapshot output");
}

#[tokio::test]
async fn snapshot_records_a_durable_provider_failure() {
    let provider = provider(1);
    *provider.mode.lock().unwrap() = ProviderMode::Timeout;
    let app = open(
        &[(1, "alpha", provider)],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let imported = app
        .import_config(
            ConfigName::new("typed-failure").expect("name"),
            ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    let run_id = RunId::from_digest(DigestBytes::from_array([70; 32]));
    let started = app
        .start_run(
            run_id.clone(),
            &ConfigSelection::new(
                imported.config().name().clone(),
                imported.config().digest().clone(),
            ),
        )
        .await
        .expect("start");
    let model = serde_json::to_value(SerializableRunView::new(started.run()).unwrap()).unwrap();
    assert_eq!(model["state"]["kind"], "failed");
    assert_eq!(model["state"]["report"]["cause"]["kind"], "adapter");
    assert_eq!(
        model["state"]["report"]["cause"]["error"]["canonical"]["kind"],
        "timeout"
    );
    let cold = app.read_run(&run_id).await.expect("cold");
    assert_eq!(
        serde_json::to_value(SerializableRunView::new(&cold).unwrap()).unwrap(),
        model
    );
    let error = app
        .read_run(&RunId::from_digest(DigestBytes::from_array([71; 32])))
        .await
        .err()
        .unwrap();
    assert_eq!(error.code(), "run_absent");
}

#[tokio::test]
async fn snapshot_token_holdings_and_typed_read_failures() {
    let token_document = serde_json::json!({
        "entry_point": "mfm.portfolio/snapshot@1",
        "input": {
            "routes": [{"chain_id": 1, "endpoint_id": "alpha"}],
            "selector": {"target": "portfolio", "quote": "usd"},
            "portfolio": {
                "portfolio_id": "portfolio",
                "quotes": ["usd"],
                "collections": [{
                    "correlation": "collection",
                    "request": {
                        "decimals": 18,
                        "sources": [{
                            "source_id": "wallet",
                            "chain_id": 1,
                            "address": "0x0000000000000000000000000000000000000001",
                            "token": "0x0000000000000000000000000000000000000002"
                        }]
                    }
                }]
            }
        }
    });

    let token_provider = provider(1);
    let token_app = open(
        &[(1, "alpha", token_provider)],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let token_imported = token_app
        .import_config(
            ConfigName::new("token").expect("name"),
            ConfigDocument::new(serde_json::to_vec(&token_document).unwrap())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    let token_started = token_app
        .start_run(
            RunId::from_digest(DigestBytes::from_array([2; 32])),
            &ConfigSelection::new(
                token_imported.config().name().clone(),
                token_imported.config().digest().clone(),
            ),
        )
        .await
        .expect("start");
    let RunViewState::Succeeded(value) = token_started.run().state() else {
        panic!("token snapshot must succeed");
    };
    let output = serde_json::to_value(value.decode::<PortfolioSnapshotOutput>().unwrap()).unwrap();
    assert_eq!(
        output["snapshot"]["collections"][0]["holdings"][0]["amount_dec"],
        "1.000000000000000000"
    );
    assert_eq!(
        output["snapshot"]["collections"][0]["holdings"][0]["asset"]["kind"],
        "token"
    );

    let reject_provider = provider(1);
    *reject_provider.mode.lock().unwrap() = ProviderMode::RejectBalance;
    let reject_app = open(
        &[(1, "alpha", reject_provider)],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let reject_imported = reject_app
        .import_config(
            ConfigName::new("reject").expect("name"),
            ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    let rejected = reject_app
        .start_run(
            RunId::from_digest(DigestBytes::from_array([3; 32])),
            &ConfigSelection::new(
                reject_imported.config().name().clone(),
                reject_imported.config().digest().clone(),
            ),
        )
        .await
        .expect("start");
    let RunViewState::Failed(report) = rejected.run().state() else {
        panic!("rejected evidence must fail");
    };
    let Failure::Domain { original, .. } = report.failure() else {
        panic!("domain failure");
    };
    assert!(matches!(
        original.decode::<EvmBalanceFailure>().unwrap(),
        EvmBalanceFailure::SourceUnavailable {
            collection_ordinal: 0,
            ..
        }
    ));
    assert!(matches!(
        report
            .root()
            .unwrap()
            .decode::<PortfolioSnapshotFailure>()
            .unwrap(),
        PortfolioSnapshotFailure::CollectionFailed { ordinal: 0, .. }
    ));
    let cold = reject_app
        .read_run(&RunId::from_digest(DigestBytes::from_array([3; 32])))
        .await
        .expect("cold");
    let RunViewState::Failed(cold_report) = cold.state() else {
        panic!("cold failure");
    };
    assert_eq!(cold_report.value_ref(), report.value_ref());

    let timeout_provider = provider(1);
    *timeout_provider.mode.lock().unwrap() = ProviderMode::TimeoutBalance;
    let timeout_app = open(
        &[(1, "alpha", timeout_provider)],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let timeout_imported = timeout_app
        .import_config(
            ConfigName::new("timeout-token").expect("name"),
            ConfigDocument::new(serde_json::to_vec(&token_document).unwrap())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    let timed_out = timeout_app
        .start_run(
            RunId::from_digest(DigestBytes::from_array([4; 32])),
            &ConfigSelection::new(
                timeout_imported.config().name().clone(),
                timeout_imported.config().digest().clone(),
            ),
        )
        .await
        .expect("start");
    let RunViewState::Failed(report) = timed_out.run().state() else {
        panic!("token timeout must fail");
    };
    let incident = report.failure();
    assert!(matches!(incident, Failure::Read { .. }));
    assert_eq!(
        incident
            .original()
            .decode::<EvmOperationalError>()
            .unwrap()
            .kind(),
        EvmOperationalKind::Timeout
    );
    let Failure::Read { call, .. } = incident else {
        panic!("Read facts");
    };
    let wire = serde_json::from_slice::<serde_json::Value>(call.input().canonical_bytes()).unwrap();
    assert_eq!(wire["metadata"]["collection_ordinal"], 0);
}

#[tokio::test]
async fn config_rejects_malformed_unbound_and_forged_rows_before_admission() {
    let reordered = br#"{
      "input":{"portfolio":{"quotes":["usd"],"portfolio_id":"portfolio-example","collections":[{"request":{"sources":[{"token":null,"source_id":"wallet-0.native","chain_id":1,"address":"0x1111111111111111111111111111111111111111"}],"decimals":18},"correlation":"native-0"}]},"selector":{"quote":"usd","target":"portfolio-example"},"routes":[{"endpoint_id":"alpha","chain_id":1}]},
      "entry_point":"mfm.portfolio/snapshot@1"}"#;
    let checked = ConfigDocument::new(reordered.to_vec())
        .await
        .expect("reordered document");
    let app = open(
        &[(1, "alpha", provider(1))],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let imported = app
        .import_config(ConfigName::new("daily").expect("name"), checked)
        .await
        .expect("import");
    assert_eq!(
        imported.config().digest().as_str(),
        "content:sha256-jcs-v1:0723d5ccf638cfcf6bb85d96de269e93adcef0847147ace563d09464c8c12250"
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

    let invalid = ConfigDocument::new(
        serde_json::to_vec(&serde_json::json!({
            "entry_point": "mfm.portfolio/snapshot@1",
            "input": {
                "routes": [{"chain_id": 1, "endpoint_id": "alpha"}],
                "selector": {"target": "other-portfolio", "quote": "usd"},
                "portfolio": {
                    "portfolio_id": "portfolio-example",
                    "quotes": ["usd"],
                    "collections": [{
                        "correlation": "native-0",
                        "request": {
                            "decimals": 18,
                            "sources": [{
                                "source_id": "wallet-a.native",
                                "chain_id": 1,
                                "address": "0x1111111111111111111111111111111111111111",
                                "token": null
                            }]
                        }
                    }]
                }
            }
        }))
        .unwrap(),
    )
    .await
    .expect("typed document");
    assert_eq!(
        app.import_config(ConfigName::new("invalid").expect("name"), invalid)
            .await,
        Err(RequestError::InvalidConfigDocument)
    );

    let unbound = app
        .import_config(
            ConfigName::new("unbound").expect("name"),
            ConfigDocument::new(
                serde_json::to_vec(&serde_json::json!({
                    "entry_point": "mfm.portfolio/snapshot@1",
                    "input": {
                        "routes": [{"chain_id": 18446744073709551615_u64, "endpoint_id": "upper-half"}],
                        "selector": {"target": "portfolio-example", "quote": "usd"},
                        "portfolio": {
                            "portfolio_id": "portfolio-example",
                            "quotes": ["usd"],
                            "collections": [{
                                "correlation": "native-0",
                                "request": {
                                    "decimals": 18,
                                    "sources": [{
                                        "source_id": "wallet-a.native",
                                        "chain_id": 18446744073709551615_u64,
                                        "address": "0x1111111111111111111111111111111111111111",
                                        "token": null
                                    }]
                                }
                            }]
                        }
                    }
                }))
                .unwrap(),
            )
            .await
            .expect("document"),
        )
        .await
        .expect("deployment-independent import");
    assert!(matches!(
        app.start_run(
            RunId::from_digest(DigestBytes::from_array([20; 32])),
            &ConfigSelection::new(
                unbound.config().name().clone(),
                unbound.config().digest().clone()
            )
        )
        .await,
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

    let canonical = PlainCanonicalJsonBytes::from_json_str(NATIVE_SNAPSHOT).expect("canonical");
    let false_digest = ConfigDigest::new(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([9; 32]),
    ))
    .expect("false digest");
    let name = ConfigName::new("hostile-digest").expect("name");
    let revision =
        ConfigRevision::new(name.clone(), false_digest, canonical.to_vec()).expect("revision");
    let hostile = open(
        &[],
        Arc::new(MemoryStore::new()),
        Arc::new(HostileConfigRepository { revision }),
    );
    let actual_digest = ConfigDigest::new(canonical.content_digest()).expect("actual digest");
    assert!(matches!(
        hostile
            .start_run(
                RunId::from_digest(DigestBytes::from_array([14; 32])),
                &ConfigSelection::new(name, actual_digest),
            )
            .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::InvalidRetainedConfig
        ))
    ));

    let mut invalid = serde_json::from_str::<serde_json::Value>(NATIVE_SNAPSHOT).unwrap();
    invalid["input"]["selector"]["target"] = serde_json::json!("other-portfolio");
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&invalid.to_string()).expect("canonical");
    let digest = ConfigDigest::new(canonical.content_digest()).expect("digest");
    let name = ConfigName::new("hostile-plan").expect("name");
    let revision =
        ConfigRevision::new(name.clone(), digest.clone(), canonical.to_vec()).expect("revision");
    assert!(matches!(
        open(
            &[],
            Arc::new(MemoryStore::new()),
            Arc::new(HostileConfigRepository { revision }),
        )
        .start_run(
            RunId::from_digest(DigestBytes::from_array([15; 32])),
            &ConfigSelection::new(name, digest),
        )
        .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::InvalidRetainedConfig
        ))
    ));
}

#[tokio::test]
async fn config_delete_does_not_revoke_an_admitted_run() {
    let app = open(
        &[(1, "alpha", provider(1))],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let name = ConfigName::new("daily").expect("name");
    let created = app
        .import_config(
            name.clone(),
            ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
                .await
                .expect("document"),
        )
        .await
        .expect("created");
    let digest = created.config().digest().clone();
    let started = app
        .start_run(
            RunId::from_digest(DigestBytes::from_array([10; 32])),
            &ConfigSelection::new(name.clone(), digest.clone()),
        )
        .await
        .expect("start");
    let wrong = ConfigDigest::new(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([9; 32]),
    ))
    .expect("digest");
    assert!(matches!(
        app.start_run(
            RunId::from_digest(DigestBytes::from_array([13; 32])),
            &ConfigSelection::new(name.clone(), wrong),
        )
        .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::ConfigAbsent
        ))
    ));
    app.delete_config(&name, &digest).await.expect("delete");
    app.delete_config(&name, &digest)
        .await
        .expect("idempotent delete");
    assert!(matches!(
        app.start_run(
            RunId::from_digest(DigestBytes::from_array([14; 32])),
            &ConfigSelection::new(name, digest),
        )
        .await,
        Err(mfm_app::RunRequestError::Request(
            RequestError::ConfigAbsent
        ))
    ));
    let retained = app
        .read_run(&RunId::from_digest(DigestBytes::from_array([10; 32])))
        .await
        .expect("admitted run survives config deletion");
    assert_eq!(retained.head_digest(), started.run().head_digest());
}

#[tokio::test]
async fn indeterminate_start_and_progress_carry_recovery_identity() {
    let start_backend = Arc::new(FaultStore::new());
    let start_app = open(
        &[(1, "alpha", provider(1))],
        Arc::clone(&start_backend),
        Arc::new(MemoryConfigRepository::default()),
    );
    let imported = start_app
        .import_config(
            ConfigName::new("start-recovery").expect("name"),
            ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    start_backend.fail_next_append();
    let run_id = RunId::from_digest(DigestBytes::from_array([40; 32]));
    let Err(error) = start_app
        .start_run(
            run_id.clone(),
            &ConfigSelection::new(
                imported.config().name().clone(),
                imported.config().digest().clone(),
            ),
        )
        .await
    else {
        panic!("start append must be ambiguous");
    };
    assert_eq!(error.code(), "run_append_indeterminate");
    let serialized = serde_json::to_value(
        mfm_app::SerializableClientError::for_run(&error, &error.to_string()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serialized["invocation"]["cause"]["recording"]["failure"]["store"]["cause"]
            ["indeterminate"],
        serde_json::json!({"operation": "test.store", "injected": "Indeterminate"})
    );
    assert!(matches!(
        error.recovery(),
        Some(RunRecovery::Start { run_id: retained, config })
            if retained == &run_id && config == imported.config()
    ));

    let progress_backend = Arc::new(FaultStore::new());
    let progress_provider = provider(1);
    *progress_provider.mode.lock().unwrap() = ProviderMode::Blocked;
    let progress_app = open(
        &[(1, "alpha", Arc::clone(&progress_provider))],
        Arc::clone(&progress_backend),
        Arc::new(MemoryConfigRepository::default()),
    );
    let progress_config = progress_app
        .import_config(
            ConfigName::new("progress-recovery").expect("name"),
            ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    let progress_id = RunId::from_digest(DigestBytes::from_array([41; 32]));
    {
        let selected = ConfigSelection::new(
            progress_config.config().name().clone(),
            progress_config.config().digest().clone(),
        );
        let start = progress_app.start_run(progress_id.clone(), &selected);
        tokio::pin!(start);
        tokio::select! {
            result = &mut start => panic!("blocked Read completed: {}", result.is_ok()),
            _ = progress_provider.entered.notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => panic!("Read was not entered"),
        }
    }
    let retained = progress_app.read_run(&progress_id).await.unwrap();
    assert!(matches!(retained.state(), RunViewState::Runnable { .. }));
    *progress_provider.mode.lock().unwrap() = ProviderMode::Ready;
    progress_backend.fail_next_append();
    let Err(error) = progress_app.progress_run(&progress_id).await else {
        panic!("progress append must be ambiguous");
    };
    assert!(matches!(
        error.recovery(),
        Some(RunRecovery::Progress { run_id: retained }) if retained == &progress_id
    ));
    let serialized = serde_json::to_value(
        mfm_app::SerializableClientError::for_run(&error, &error.to_string()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serialized["invocation"]["last_observed"]["state"]["kind"],
        "runnable"
    );
}

#[tokio::test]
async fn enrichment_keeps_native_and_nonzero_candidates() {
    let document = serde_json::json!({
        "entry_point": "mfm.portfolio/enrich@1",
        "input": {
            "routes": [{"chain_id": 1, "endpoint_id": "alpha"}],
            "selector": {"target": "candidates", "quote": "usd"},
            "portfolio": {
                "portfolio_id": "candidates",
                "quotes": ["eur", "usd"],
                "collections": [{
                    "correlation": "wallet",
                    "request": {
                        "decimals": 18,
                        "sources": [
                            {
                                "source_id": "funded",
                                "chain_id": 1,
                                "address": "0x0000000000000000000000000000000000000001",
                                "token": "0x0000000000000000000000000000000000000002"
                            },
                            {
                                "source_id": "native",
                                "chain_id": 1,
                                "address": "0x0000000000000000000000000000000000000001",
                                "token": null
                            },
                            {
                                "source_id": "empty",
                                "chain_id": 1,
                                "address": "0x0000000000000000000000000000000000000001",
                                "token": "0x0000000000000000000000000000000000000003"
                            }
                        ]
                    }
                }]
            }
        }
    });
    let app = open(
        &[(1, "alpha", provider(1))],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let imported = app
        .import_config(
            ConfigName::new("candidates").expect("name"),
            ConfigDocument::new(serde_json::to_vec(&document).unwrap())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    let started = app
        .start_run(
            RunId::from_digest(DigestBytes::from_array([90; 32])),
            &ConfigSelection::new(
                imported.config().name().clone(),
                imported.config().digest().clone(),
            ),
        )
        .await
        .expect("start");
    let RunViewState::Succeeded(value) = started.run().state() else {
        panic!("enrichment must succeed");
    };
    let output = value
        .decode::<PortfolioEnrichmentOutput>()
        .expect("enrichment");
    let wire = serde_json::to_value(&output).unwrap();
    assert_eq!(wire["quote"], "usd");
    let sources = wire["collections"][0]["config"]["request"]["sources"]
        .as_array()
        .unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["source_id"], "funded");
    assert_eq!(sources[1]["source_id"], "native");

    let timeout_provider = provider(1);
    *timeout_provider.mode.lock().unwrap() = ProviderMode::TimeoutBalance;
    let timeout_app = open(
        &[(1, "alpha", timeout_provider)],
        Arc::new(MemoryStore::new()),
        Arc::new(MemoryConfigRepository::default()),
    );
    let timeout_imported = timeout_app
        .import_config(
            ConfigName::new("failing-candidates").expect("name"),
            ConfigDocument::new(serde_json::to_vec(&document).unwrap())
                .await
                .expect("document"),
        )
        .await
        .expect("import");
    let failed = timeout_app
        .start_run(
            RunId::from_digest(DigestBytes::from_array([91; 32])),
            &ConfigSelection::new(
                timeout_imported.config().name().clone(),
                timeout_imported.config().digest().clone(),
            ),
        )
        .await
        .expect("start");
    let RunViewState::Failed(report) = failed.run().state() else {
        panic!("provider failure must not be filtered");
    };
    assert_eq!(
        report
            .failure()
            .original()
            .decode::<EvmOperationalError>()
            .unwrap()
            .kind(),
        EvmOperationalKind::Timeout
    );
}

#[tokio::test]
async fn enrichment_publish_rejects_incomplete_forged_and_lost_ack() {
    let provider = provider(1);
    let repository = Arc::new(LostPublicationAck::default());
    let app = open(
        &[(1, "alpha", provider.clone())],
        Arc::new(MemoryStore::new()),
        repository.clone(),
    );
    let mut candidate: serde_json::Value = serde_json::from_str(NATIVE_SNAPSHOT).unwrap();
    candidate["entry_point"] = serde_json::json!("mfm.portfolio/enrich@1");
    let imported = app
        .import_config(
            ConfigName::new("candidates").expect("name"),
            ConfigDocument::new(serde_json::to_vec(&candidate).unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();
    let enrichment_id = RunId::from_digest(DigestBytes::from_array([71; 32]));
    *provider.mode.lock().unwrap() = ProviderMode::Blocked;
    {
        let selected = ConfigSelection::new(
            imported.config().name().clone(),
            imported.config().digest().clone(),
        );
        let starting = app.start_run(enrichment_id.clone(), &selected);
        tokio::pin!(starting);
        tokio::select! {
            _ = provider.entered.notified() => {}
            _ = &mut starting => panic!("blocked enrichment must remain in flight"),
        }
    }
    let pending = app
        .publish_enrichment(ConfigName::new("unresolved").expect("name"), &enrichment_id)
        .await
        .expect_err("incomplete discovery cannot publish");
    assert_eq!(pending.code(), "invalid_enrichment");

    *provider.mode.lock().unwrap() = ProviderMode::Ready;
    let enriched = app
        .start_run(
            enrichment_id.clone(),
            &ConfigSelection::new(
                imported.config().name().clone(),
                imported.config().digest().clone(),
            ),
        )
        .await
        .unwrap();
    assert!(matches!(enriched.run().state(), RunViewState::Succeeded(_)));

    repository
        .lose_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let lost = app
        .publish_enrichment(ConfigName::new("resolved").expect("name"), &enrichment_id)
        .await
        .expect_err("lost publication acknowledgement");
    assert_eq!(lost.code(), "config_mutation_indeterminate");
    let published = app
        .publish_enrichment(ConfigName::new("resolved").expect("name"), &enrichment_id)
        .await
        .unwrap();
    assert!(matches!(published, ImportOutcome::Unchanged { .. }));

    let retained = repository
        .inner
        .load_config(published.config().name(), published.config().digest())
        .await
        .unwrap()
        .unwrap();
    for mutation in ["head", "values", "route"] {
        let mut wire: serde_json::Value =
            serde_json::from_slice(retained.canonical_bytes()).unwrap();
        match mutation {
            "head" => {
                wire["input"]["provenance"]["head"] =
                    serde_json::json!(format!("content:sha256-v1:{}", "0".repeat(64)))
            }
            "values" => {
                wire["input"]["portfolio"]["collections"][0]["request"]["sources"][0]["address"] =
                    serde_json::json!("0x2222222222222222222222222222222222222222")
            }
            _ => wire["input"]["routes"][0]["endpoint_id"] = serde_json::json!("different"),
        }
        let forged = app
            .import_config(
                ConfigName::new(mutation).expect("name"),
                ConfigDocument::new(serde_json::to_vec(&wire).unwrap())
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
        let rejected = app
            .start_run(
                RunId::from_digest(DigestBytes::from_array([74; 32])),
                &ConfigSelection::new(
                    forged.config().name().clone(),
                    forged.config().digest().clone(),
                ),
            )
            .await
            .err()
            .expect("untrusted linkage");
        assert_eq!(rejected.code(), "invalid_enrichment");
        assert!(matches!(
            app.read_run(&RunId::from_digest(DigestBytes::from_array([74; 32])))
                .await,
            Err(mfm_app::RunRequestError::Invocation(
                mfm_runtime::InvocationFailure::Execution {
                    error: mfm_runtime::RuntimeError::Absent,
                    ..
                }
            ))
        ));
    }

    let dependent = app
        .start_run(
            RunId::from_digest(DigestBytes::from_array([72; 32])),
            &ConfigSelection::new(
                published.config().name().clone(),
                published.config().digest().clone(),
            ),
        )
        .await
        .unwrap();
    assert!(matches!(
        dependent.run().state(),
        RunViewState::Succeeded(_)
    ));
    let wrong_schema = app
        .publish_enrichment(
            ConfigName::new("wrong-schema").expect("name"),
            dependent.run().run_id(),
        )
        .await
        .expect_err("snapshot is not enrichment");
    assert_eq!(wrong_schema.code(), "invalid_enrichment");
}

#[tokio::test]
async fn shipping_metadata_constructor_cases_reach_native_materialization_after_schema_admission() {
    type BalanceInput = mfm_evm::EvmBalanceContext<mfm_portfolio::PortfolioContinuation>;
    let backend = Arc::new(FaultStore::new());
    let provider = provider(1);
    *provider.mode.lock().unwrap() = ProviderMode::Blocked;
    let app = open(
        &[(1, "alpha", provider.clone())],
        backend.clone(),
        Arc::new(MemoryConfigRepository::default()),
    );
    let imported = app
        .import_config(
            ConfigName::new("native-constructor").expect("name"),
            ConfigDocument::new(NATIVE_SNAPSHOT.as_bytes().to_vec())
                .await
                .unwrap(),
        )
        .await
        .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([79; 32]));
    {
        let selected = ConfigSelection::new(
            imported.config().name().clone(),
            imported.config().digest().clone(),
        );
        let starting = app.start_run(run.clone(), &selected);
        tokio::pin!(starting);
        tokio::select! {
            _ = provider.entered.notified() => {},
            result = &mut starting => panic!("expected blocked shipping Read: {}", result.is_ok()),
        }
    }
    let loaded = backend.load_run(&run, None).await.unwrap().unwrap();
    let frame = mfm_journal::decode_frame(loaded.latest()).unwrap();
    let commit: serde_json::Value = serde_json::from_slice(frame.payload().as_bytes()).unwrap();
    let original: Object =
        serde_json::from_value(commit["operation"]["succeeded"]["output"].clone()).unwrap();
    let mut input: serde_json::Value = serde_json::from_slice(original.canonical_bytes()).unwrap();
    for (correlation, mutation) in [
        (String::new(), 0),
        ("a".repeat(257), 0),
        ("changed".into(), 1),
        ("malformed".into(), 2),
        ("wrong-slot".into(), 4),
    ] {
        input["metadata"]["correlation"] = serde_json::json!(correlation);
        let canonical =
            PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&input).unwrap())
                .unwrap();
        let object = Object::from_canonical(
            ContentRef::new(
                original.value_ref().schema_id().clone(),
                mfm_canonical::raw_content_digest(canonical.as_bytes()),
            )
            .unwrap(),
            canonical.as_bytes(),
        )
        .unwrap();
        object
            .admit(&BalanceInput::schema_descriptor().unwrap())
            .unwrap();
        let projected = if mutation != 0 {
            None
        } else {
            assert!(serde_json::from_slice::<BalanceInput>(object.canonical_bytes()).is_err());
            let native = object.decode::<BalanceInput>().err().unwrap();
            let projected = native.details().as_value().clone();
            assert_eq!(native.code(), "constructor_error");
            assert_eq!(projected["metadata"]["location"], "metadata.correlation");
            Some(projected)
        };
        let mut current = commit.clone();
        let mut replacement = serde_json::to_value(&object).unwrap();
        if mutation != 0 {
            match mutation {
                1 => replacement["value_ref"] = serde_json::to_value(original.value_ref()).unwrap(),
                2 => replacement["value_ref"] = serde_json::json!("malformed-reference"),
                4 => {
                    replacement =
                        serde_json::to_value(Object::from_value(&mfm_program::NoParams).unwrap())
                            .unwrap()
                }
                _ => unreachable!(),
            }
            current["operation"]["succeeded"]["output"] = replacement;
        } else {
            replace_object(
                &mut current,
                &serde_json::to_value(&original).unwrap(),
                &replacement,
            );
        }
        let payload =
            PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&current).unwrap())
                .unwrap();
        let fixture = mfm_journal::seal_frame(
            &run,
            frame.run_sequence(),
            frame.previous_head_digest(),
            &payload,
        )
        .unwrap();
        let head = mfm_store::RunSummary::new(
            run.clone(),
            frame.run_sequence(),
            fixture.head_digest().clone(),
            loaded.head().total_bytes() - frame.canonical_bytes().len() as u64
                + fixture.canonical_bytes().len() as u64,
        )
        .unwrap();
        let fixture_store = Arc::new(ConstructorSnapshot(
            mfm_store::LoadedRun::new(
                head.clone(),
                loaded.admission().clone(),
                Arc::from(fixture.canonical_bytes()),
                None,
            )
            .unwrap(),
        ));
        let fixture_app = open(
            &[(1, "alpha", provider.clone())],
            fixture_store,
            Arc::new(MemoryConfigRepository::default()),
        );
        if mutation != 0 {
            let failure = fixture_app.read_run(&run).await.err().unwrap();
            assert_eq!(failure.code(), "internal");
            let report = serde_json::to_value(
                mfm_app::SerializableClientError::for_run(&failure, &failure.to_string()).unwrap(),
            )
            .unwrap();
            let native = &report["invocation"]["cause"]["native"];
            assert_eq!(native["operation"], "restore");
            continue;
        }
        assert_eq!(
            fixture_app.read_run(&run).await.unwrap().head_digest(),
            head.head_digest()
        );
        let failure = fixture_app.progress_run(&run).await.err().unwrap();
        assert_eq!(failure.code(), "internal");
        let report = serde_json::to_value(
            mfm_app::SerializableClientError::for_run(&failure, &failure.to_string()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            report["invocation"]["cause"]["native"]["cause"]["details"],
            projected.unwrap()
        );
    }
}

struct ConstructorSnapshot(mfm_store::LoadedRun);

impl Store for ConstructorSnapshot {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
        probe: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<mfm_store::LoadedRun>, StoreError>> + Send + 'a>>
    {
        assert_eq!(run_id, self.0.head().run_id());
        assert!(probe.is_none());
        Box::pin(async {
            mfm_store::LoadedRun::new(
                self.0.head().clone(),
                self.0.admission().clone(),
                self.0.latest().clone(),
                None,
            )
            .map(Some)
        })
    }

    fn append_run<'a>(
        &'a self,
        _: &'a mfm_journal::EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        panic!("native constructor failure must not append")
    }
}

impl mfm_store::RunIndex for ConstructorSnapshot {
    fn list_runs<'a>(
        &'a self,
        after: Option<&'a RunId>,
        _: RunPageLimit,
    ) -> Pin<
        Box<dyn Future<Output = Result<mfm_store::RunPage, mfm_store::RunIndexError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let items = if after.is_none_or(|after| after < self.0.head().run_id()) {
                vec![self.0.head().clone()]
            } else {
                vec![]
            };
            mfm_store::RunPage::new(items, None)
        })
    }
}

fn replace_object(
    value: &mut serde_json::Value,
    original: &serde_json::Value,
    replacement: &serde_json::Value,
) {
    if value == original {
        *value = replacement.clone();
        return;
    }
    match value {
        serde_json::Value::Object(fields) => {
            for value in fields.values_mut() {
                replace_object(value, original, replacement);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                replace_object(value, original, replacement);
            }
        }
        _ => {}
    }
}
