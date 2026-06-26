#![warn(missing_docs)]
//! Shared helpers for MFM integration tests.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use mfm_core::keystore::{Keystore, KeystoreConfig};
use mfm_ids::RunId;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

/// In-memory REST app state used by integration tests.
pub type InMemoryRestAppState = mfm_rest_api::AppState<store::AsyncInMemoryRunStore>;

/// Builds in-memory REST app state.
pub fn in_memory_rest_app_state() -> InMemoryRestAppState {
    mfm_rest_api::AppState {
        store: store::AsyncInMemoryRunStore::default(),
    }
}

/// Builds a JSON POST request for REST integration tests.
pub fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let payload = serde_json::to_string(&body).expect("request body serializes");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .expect("request")
}

/// Builds an empty-body POST request for REST integration tests.
pub fn empty_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// Parses an Axum response body as JSON for REST integration tests.
pub async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("response json")
}

/// Calls a JSON-RPC endpoint and returns the response `result`.
pub async fn rpc_call(rpc_url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .expect("send json-rpc request")
        .error_for_status()
        .expect("json-rpc http status");
    let payload: serde_json::Value = response.json().await.expect("json-rpc response json");
    if let Some(error) = payload.get("error") {
        panic!("json-rpc {method} returned error: {error}");
    }
    payload
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("json-rpc {method} response missing result: {payload}"))
}

/// Environment variable carrying process-local EVM RPC source configuration.
pub const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";

/// Restores an environment variable to its previous test value when dropped.
pub struct EnvVarRestore {
    name: &'static str,
    previous: Option<String>,
}

impl Drop for EnvVarRestore {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.name, value),
            None => std::env::remove_var(self.name),
        }
    }
}

/// Starts a local JSON-RPC mock for portfolio balance reads.
pub async fn start_portfolio_rpc_mock(expected_chain_id: u64) -> String {
    let app = axum::Router::new()
        .route("/", axum::routing::post(portfolio_rpc_handler))
        .with_state(expected_chain_id);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind rpc mock");
    let addr = listener.local_addr().expect("rpc mock addr");
    listener
        .set_nonblocking(true)
        .expect("set rpc mock nonblocking");
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rpc mock runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("tokio rpc listener");
            axum::serve(listener, app).await.expect("rpc mock serve");
        });
    });
    format!("http://{addr}")
}

async fn portfolio_rpc_handler(
    axum::extract::State(expected_chain_id): axum::extract::State<u64>,
    axum::Json(request): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let id = request
        .get("id")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(1));
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .expect("json-rpc method");
    let result = match method {
        "eth_chainId" => serde_json::json!(format!("0x{expected_chain_id:x}")),
        "eth_getBlockByNumber" => serde_json::json!({
            "number": "0x64",
            "hash": "0x1111111111111111111111111111111111111111111111111111111111111111"
        }),
        "eth_getBalance" => serde_json::json!("0xde0b6b3a7640000"),
        other => panic!("unexpected rpc method {other}"),
    };
    axum::Json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
}

/// Sets the EVM RPC source registry env var for one test network and restores it on drop.
pub fn set_evm_rpc_sources_env_for_test(
    network_id: &str,
    expected_chain_id: u64,
    rpc_url: String,
) -> EnvVarRestore {
    let previous = std::env::var(ENV_EVM_RPC_SOURCES_JSON).ok();
    std::env::set_var(
        ENV_EVM_RPC_SOURCES_JSON,
        serde_json::json!({
            "sources": [
                {
                    "id": network_id,
                    "expected_chain_id": expected_chain_id,
                    "rpc_url": rpc_url,
                    "authorization": null
                }
            ],
            "policies": [
                {
                    "id": network_id,
                    "ordered_sources": [network_id]
                }
            ]
        })
        .to_string(),
    );
    EnvVarRestore {
        name: ENV_EVM_RPC_SOURCES_JSON,
        previous,
    }
}

/// Prepares a portfolio snapshot entry-point launch against the supplied store trust scope.
pub async fn prepare_portfolio_launch_for_store<S>(
    store: &S,
    config: &serde_json::Value,
    distinct_run_key: Option<&str>,
) -> mfm_app::PreparedEntryPointRunLaunch
where
    S: store::TrustScopeStore,
{
    prepare_entry_point_launch_for_store(
        store,
        "portfolio_snapshot",
        None,
        config,
        distinct_run_key,
    )
    .await
}

/// Prepares an entry-point launch against the supplied store trust scope.
pub async fn prepare_entry_point_launch_for_store<S>(
    store: &S,
    op_name: &str,
    op_version: Option<mfm_app::OpVersion>,
    config: &serde_json::Value,
    distinct_run_key: Option<&str>,
) -> mfm_app::PreparedEntryPointRunLaunch
where
    S: store::TrustScopeStore,
{
    let entry_point_registry = mfm_app::production_entry_point_op_registry().expect("entrypoints");
    let certification_registry = mfm_app::production_certification_registry().expect("cert");
    let trust_scope_id = store
        .load_trust_scope_id()
        .await
        .unwrap_or_else(|error| panic!("trust scope: {error}"));
    let authored_config = mfm_authored_config::AuthoredConfig::new(
        mfm_authored_config::AuthoredConfigFormat::Json,
        serde_json::to_vec(config).expect("entry-point config json"),
    )
    .expect("authored config");
    mfm_app::prepare_entry_point_run_launch(mfm_app::EntryPointRunLaunchInput {
        entry_point_registry: &entry_point_registry,
        public_op_name: mfm_app::PublicOpName::new(op_name).expect("op name"),
        op_version,
        authored_config,
        certification_registry: &certification_registry,
        trust_scope_id,
        distinct_run_key: distinct_run_key
            .map(mfm_app::DistinctRunKey::new)
            .transpose()
            .expect("distinct run key"),
    })
    .expect("prepared entry-point launch")
}

/// Admits a portfolio run without driving it so tests can append history before resume.
pub async fn admit_portfolio_run_without_driving<S>(
    store: &S,
    config: &serde_json::Value,
) -> (RunId, mfm_certify::CertifiedTypedSpec)
where
    S: store::RunEventStore
        + store::TrustScopeStore
        + store::RetainedArtifactReadProvider
        + Clone
        + Send
        + Sync
        + 'static,
{
    let prepared = prepare_portfolio_launch_for_store(store, config, None).await;
    let run_id = prepared.request.run_id.clone();
    let certified = prepared.request.certified_spec.clone();
    let runners = mfm_app::production_runner_registry(
        mfm_app::artifact_read_provider_from_retained(store.clone()),
    )
    .expect("production runners");
    let scheduler = mfm_runtime::SerialTypedScheduler::new(runners, Arc::new(store.clone()));
    let runtime_spec = mfm_runtime::CertifiedRuntimeSpec::new(prepared.request.certified_spec)
        .expect("runtime spec");
    let launch = scheduler
        .prepare_run_launch(
            &runtime_spec,
            prepared.request.identity_material,
            prepared.request.evidence,
            store
                .expected_next_seq(&run_id)
                .await
                .unwrap_or_else(|error| panic!("expected next seq: {error}")),
        )
        .expect("prepared launch");
    scheduler
        .start_run(store, launch)
        .await
        .expect("start fixture run");
    (run_id, certified)
}

/// Asserts framework attempts start before their terminal evidence appears in stream JSON.
pub fn assert_framework_started_before_terminal_evidence(
    stream_events: &[serde_json::Value],
    attempts: &[serde_json::Value],
    nodes: &[spec::NodeSpec],
    run_id: &RunId,
) {
    for node in nodes.iter().filter(|node| node.framework.is_some()) {
        let required_kind = match &node.framework {
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => "public_output_render",
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                "project_retention_manifest"
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(_)) => "complete_run",
            _ => continue,
        };
        let attempt = attempts
            .iter()
            .find(|attempt| {
                attempt["node_id"].as_str() == Some(node.node_id.as_str())
                    && attempt["disposition"].as_str() == Some("completed")
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing completed {required_kind} framework attempt for {}",
                    node.node_id
                )
            });
        let attempt_id = attempt["attempt_id"].as_str().expect("attempt id");
        let attempt_key = format!("attempt:{}:{}", node.node_id, attempt_id);
        let start_index = stream_event_position(
            stream_events,
            |event| {
                event["logical_key"].as_str() == Some(attempt_key.as_str())
                    && event["event_schema_id"]
                        .as_str()
                        .is_some_and(|schema| schema.contains("state_attempt_started"))
            },
            &format!("framework start {attempt_key}"),
        );
        let completed_index = stream_event_position(
            stream_events,
            |event| {
                event["logical_key"].as_str() == Some(attempt_key.as_str())
                    && event["event_schema_id"]
                        .as_str()
                        .is_some_and(|schema| schema.contains("state_attempt_completed"))
            },
            &format!("framework completion {attempt_key}"),
        );
        assert!(
            start_index < completed_index,
            "framework StateAttemptStarted must precede StateAttemptCompleted for {attempt_key}"
        );

        match &node.framework {
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => {
                let public_output_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"]
                            .as_str()
                            .is_some_and(|key| key.starts_with("public_output:"))
                            && event["event_schema_id"]
                                .as_str()
                                .is_some_and(|schema| schema.contains("public_output_produced"))
                    },
                    "public-output terminal evidence",
                );
                assert!(
                    start_index < public_output_index,
                    "public-output framework start must precede public output evidence"
                );
            }
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                let retention_prefix = format!("retention:{}:manifest:", run_id);
                let retention_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"]
                            .as_str()
                            .is_some_and(|key| key.starts_with(&retention_prefix))
                            && event["event_schema_id"].as_str().is_some_and(|schema| {
                                schema.contains("retention_manifest_projected")
                            })
                    },
                    "retention manifest terminal evidence",
                );
                assert!(
                    start_index < retention_index,
                    "retention framework start must precede retention manifest evidence"
                );
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(_)) => {
                let completed_run_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"].as_str() == Some("run:complete")
                            && event["event_schema_id"]
                                .as_str()
                                .is_some_and(|schema| schema.contains("run_completed"))
                    },
                    "run completion terminal evidence",
                );
                assert!(
                    start_index < completed_run_index,
                    "complete-run framework start must precede run completion evidence"
                );
            }
            _ => {}
        }
    }
}

fn stream_event_position(
    events: &[serde_json::Value],
    predicate: impl Fn(&serde_json::Value) -> bool,
    label: &str,
) -> usize {
    events
        .iter()
        .position(predicate)
        .unwrap_or_else(|| panic!("missing stream event for {label}"))
}

/// Ephemeral funded keystore wallet used by reth-backed EVM parity tests.
pub struct FundedRethKeystoreWallet {
    temp_dir: tempfile::TempDir,
    keystore_env: String,
    password_file_env: String,
    /// Funded sender address derived from the keystore entry.
    pub from: String,
    /// Stable keystore entry id used by EVM signer config fixtures.
    pub entry_id: String,
    keystore_path: PathBuf,
    password_file_path: PathBuf,
}

impl FundedRethKeystoreWallet {
    /// Returns the typed workflow signer intent for this wallet.
    pub fn signer_json(&self) -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": self.from,
        })
    }

    /// Returns the process-local runtime signer registry JSON for this wallet.
    pub fn runtime_signer_registry_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "signer_ref": "deployer",
                "entry_id": self.entry_id,
                "keystore_env": self.keystore_env,
                "unlock_file_env": self.password_file_env,
            }
        ])
    }

    /// Returns the EVM source registry JSON that routes one local source to this reth node.
    pub fn runtime_source_registry_json(
        &self,
        source_id: &str,
        expected_chain_id: u64,
        endpoint_url: &str,
    ) -> serde_json::Value {
        let mut source = serde_json::json!({
            "id": source_id,
            "expected_chain_id": expected_chain_id,
        });
        let source_object = source.as_object_mut().expect("source object");
        source_object.insert(
            ["rpc", "_url"].concat(),
            serde_json::Value::String(endpoint_url.to_owned()),
        );
        source_object.insert(["author", "ization"].concat(), serde_json::Value::Null);
        serde_json::json!({
            "sources": [
                source
            ],
            "policies": [
                {
                    "id": source_id,
                    "ordered_sources": [source_id]
                }
            ]
        })
    }

    /// Returns true when `rendered` exposes process-local signer registry details.
    pub fn rendered_contains_runtime_signer_config(&self, rendered: &str) -> bool {
        rendered.contains(&self.entry_id)
            || rendered.contains(&self.keystore_env)
            || rendered.contains(&self.password_file_env)
    }

    /// Returns true when `rendered` contains test-local secret-bearing file paths.
    pub fn rendered_contains_secret_path(&self, rendered: &str) -> bool {
        rendered.contains(&self.keystore_path.display().to_string())
            || rendered.contains(&self.password_file_path.display().to_string())
    }
}

impl Drop for FundedRethKeystoreWallet {
    fn drop(&mut self) {
        std::env::remove_var(&self.keystore_env);
        std::env::remove_var(&self.password_file_env);
        let _ = self.temp_dir.path();
    }
}

/// Creates an ephemeral keystore wallet from a reth dev pre-funded account and verifies balance.
pub async fn funded_reth_keystore_wallet(
    rpc_url: &str,
    account_index: u32,
) -> FundedRethKeystoreWallet {
    const TEST_PASSWORD: &str = "reth-parity-test-password-123";
    const RETH_DEV_MNEMONIC: &str = "test test test test test test test test test test test junk";
    assert!(
        account_index < 20,
        "reth --dev prefunds 20 mnemonic accounts"
    );
    let derivation_path = format!("m/44'/60'/0'/0/{account_index}");

    let temp_dir = tempfile::tempdir().expect("reth keystore wallet tempdir");
    let keystore_path = temp_dir.path().join("reth-parity.keystore");
    let password_file_path = temp_dir.path().join("reth-parity.password");
    fs::write(&password_file_path, TEST_PASSWORD).expect("write reth parity password file");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::insecure_integration_test())
            .expect("create reth parity keystore");
    keystore
        .unlock(TEST_PASSWORD)
        .expect("unlock reth parity keystore");
    let entry_id = keystore
        .import_mnemonic(
            Some("reth-parity-funded-signer".to_owned()),
            RETH_DEV_MNEMONIC,
            &derivation_path,
            None,
        )
        .expect("import reth parity key");
    let key_info = keystore
        .list_keys()
        .expect("list reth parity keys")
        .into_iter()
        .find(|key| key.id == entry_id)
        .expect("imported reth parity key info");
    let from = format!("{:?}", key_info.address);

    let suffix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .to_ascii_uppercase();
    let keystore_env = format!("MFM_EVM_PARITY_KEYSTORE_{suffix}");
    let password_file_env = format!("MFM_EVM_PARITY_KEYSTORE_PASSWORD_FILE_{suffix}");
    std::env::set_var(&keystore_env, &keystore_path);
    std::env::set_var(&password_file_env, &password_file_path);

    // Reth dev nodes prefund this deterministic key set, but recent releases do
    // not expose the dev accounts through `eth_accounts`. The balance assertion
    // below is the funding contract these tests actually need.
    let balance = rpc_call(
        rpc_url,
        "eth_getBalance",
        serde_json::json!([from.clone(), "latest"]),
    )
    .await;
    let balance = balance
        .as_str()
        .map(parse_u128_hex_quantity)
        .expect("reth funded balance hex");
    assert!(balance > 0, "reth parity keystore wallet must be funded");

    FundedRethKeystoreWallet {
        temp_dir,
        keystore_env,
        password_file_env,
        from,
        entry_id: entry_id.to_string(),
        keystore_path,
        password_file_path,
    }
}

fn parse_u128_hex_quantity(raw: &str) -> u128 {
    let trimmed = raw
        .strip_prefix("0x")
        .expect("hex quantity must start with 0x");
    u128::from_str_radix(trimmed, 16).expect("hex quantity must parse as u128")
}
