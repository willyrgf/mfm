use std::path::Path;
use std::sync::Arc;

use mfm_ids::RunId;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

/// Environment variable carrying the runtime config file path.
pub const ENV_RUNTIME_CONFIG_FILE: &str = mfm_app::MFM_RUNTIME_CONFIG_FILE;

/// Runtime signer binding used by test runtime config files.
pub struct RuntimeConfigSignerBinding<'a> {
    /// Workflow signer reference.
    pub signer_ref: &'a str,
    /// Keystore entry id.
    pub entry_id: &'a str,
    /// Keystore file path.
    pub keystore_path: &'a Path,
    /// Unlock password file path.
    pub unlock_file: &'a Path,
}

/// Restores an environment variable to its previous test value when dropped.
pub struct EnvVarRestore {
    previous: Vec<(&'static str, Option<String>)>,
    _temp_dirs: Vec<tempfile::TempDir>,
}

impl Drop for EnvVarRestore {
    fn drop(&mut self) {
        for (name, previous) in self.previous.drain(..) {
            match previous {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
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

/// Writes a runtime config file for one EVM source/route and returns its path.
pub fn write_evm_runtime_config_for_test(
    dir: &Path,
    network_id: &str,
    rpc_url: &str,
    signer: Option<RuntimeConfigSignerBinding<'_>>,
) -> std::path::PathBuf {
    let config_path = dir.join("runtime.toml");
    let mut config = format!(
        r#"
[evm.sources.{network}]
rpc_url = {rpc_url}

[evm.routes.{network}]
source_ref = {network}
"#,
        network = toml_string(network_id),
        rpc_url = toml_string(rpc_url),
    );
    if let Some(signer) = signer {
        config.push_str(&format!(
            r#"
[keystores.default]
keystore_path = {keystore_path}
unlock_file = {unlock_file}

[signers.{signer_ref}]
provider = "keystore"
keystore_ref = "default"
entry_id = {entry_id}
"#,
            signer_ref = toml_string(signer.signer_ref),
            entry_id = toml_string(signer.entry_id),
            keystore_path = toml_string(&signer.keystore_path.display().to_string()),
            unlock_file = toml_string(&signer.unlock_file.display().to_string()),
        ));
    }
    std::fs::write(&config_path, config).expect("write runtime config");
    config_path
}

/// Sets the runtime config env var for one test EVM route and restores it on drop.
pub fn set_evm_runtime_config_env_for_test(network_id: &str, rpc_url: &str) -> EnvVarRestore {
    set_evm_runtime_config_env_with_signer_for_test(network_id, rpc_url, None)
}

/// Sets the runtime config env var for one EVM route plus optional signer binding.
pub fn set_evm_runtime_config_env_with_signer_for_test(
    network_id: &str,
    rpc_url: &str,
    signer: Option<RuntimeConfigSignerBinding<'_>>,
) -> EnvVarRestore {
    let previous = vec![(
        ENV_RUNTIME_CONFIG_FILE,
        std::env::var(ENV_RUNTIME_CONFIG_FILE).ok(),
    )];
    let temp_dir = tempfile::tempdir().expect("runtime config tempdir");
    let config_path =
        write_evm_runtime_config_for_test(temp_dir.path(), network_id, rpc_url, signer);
    std::env::set_var(ENV_RUNTIME_CONFIG_FILE, config_path);
    EnvVarRestore {
        previous,
        _temp_dirs: vec![temp_dir],
    }
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("toml-compatible string")
}

/// Prepares a portfolio snapshot entry-point launch against the supplied store scope.
pub async fn prepare_portfolio_launch_for_store<S>(
    store: &S,
    config: &serde_json::Value,
    invocation_key: Option<&str>,
) -> mfm_app::PreparedEntryPointRunLaunch
where
    S: store::StoreScopeStore,
{
    prepare_entry_point_launch_for_store(store, "portfolio_snapshot", None, config, invocation_key)
        .await
}

/// Prepares an entry-point launch against the supplied store scope.
pub async fn prepare_entry_point_launch_for_store<S>(
    store: &S,
    op_name: &str,
    op_version: Option<mfm_app::OpVersion>,
    config: &serde_json::Value,
    invocation_key: Option<&str>,
) -> mfm_app::PreparedEntryPointRunLaunch
where
    S: store::StoreScopeStore,
{
    let entry_point_registry = mfm_app::production_entry_point_op_registry().expect("entrypoints");
    let certification_registry = mfm_app::production_certification_registry().expect("cert");
    let store_scope_id = store
        .load_store_scope_id()
        .await
        .unwrap_or_else(|error| panic!("store scope: {error}"));
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
        store_scope_id,
        invocation_key: invocation_key
            .map(mfm_app::InvocationKey::new)
            .transpose()
            .expect("invocation key"),
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
        + store::StoreScopeStore
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
        Arc::new(store.clone()),
        mfm_app::ProjectionFactIndexProvider::empty_arc(),
        None,
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
