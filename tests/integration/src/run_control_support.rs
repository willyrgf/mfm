use std::collections::BTreeMap;
use std::path::Path;

use mfm_program::{CanonicalSeed, TypedProgramDraft};
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

/// Starts one JSON-RPC mock serving the EVM and Bitcoin calls used by the production collector
/// registry integration test.
pub async fn start_collectors_rpc_mock() -> String {
    let app = axum::Router::new().route("/", axum::routing::post(collectors_rpc_handler));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind collector rpc mock");
    let addr = listener.local_addr().expect("collector rpc mock addr");
    listener
        .set_nonblocking(true)
        .expect("set collector rpc mock nonblocking");
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("collector rpc mock runtime");
        runtime.block_on(async move {
            let listener =
                tokio::net::TcpListener::from_std(listener).expect("tokio collector rpc listener");
            axum::serve(listener, app)
                .await
                .expect("collector rpc mock serve");
        });
    });
    format!("http://{addr}")
}

async fn collectors_rpc_handler(
    axum::Json(request): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let id = request
        .get("id")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(1));
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .expect("collector json-rpc method");
    let result = match method {
        "eth_chainId" => serde_json::json!("0x1"),
        "web3_clientVersion" => serde_json::json!("mfm-test-rpc"),
        "eth_getBlockByNumber" => serde_json::json!({
            "number": format!("0x{:x}", 21_000_000u64),
            "hash": format!("0x{}", "cd".repeat(32))
        }),
        "eth_getBlockByHash" => serde_json::json!({
            "number": format!("0x{:x}", 21_000_000u64),
            "hash": format!("0x{}", "cd".repeat(32))
        }),
        "eth_getBalance" => serde_json::json!("0xde0b6b3a7640000"),
        "getblockchaininfo" => serde_json::json!({
            "blocks": 850_100u64,
            "bestblockhash": "abababababababababababababababababababababababababababababababab",
            "chain": "main",
            "initialblockdownload": false
        }),
        "getblockhash" => {
            serde_json::json!("abababababababababababababababababababababababababababababababab")
        }
        "getblockheader" => serde_json::json!({
            "hash": "abababababababababababababababababababababababababababababababab",
            "height": 850_100u64,
            "time": 1_720_000_000u64
        }),
        "scantxoutset" => serde_json::json!({
            "success": true,
            "height": 850_100u64,
            "bestblock": "abababababababababababababababababababababababababababababababab",
            "total_amount": 0.001
        }),
        other => {
            return axum::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("unsupported test method {other}") }
            }));
        }
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

/// Writes a runtime config containing both collector routes for the production registry test.
pub fn write_collectors_runtime_config_for_test(dir: &Path, rpc_url: &str) -> std::path::PathBuf {
    let config_path = dir.join("runtime.toml");
    let config = format!(
        r#"
[evm.sources.ethereum-mainnet]
rpc_url = {rpc_url}

[evm.routes.ethereum-mainnet]
source_ref = "ethereum-mainnet"

[btc.routes.public-bitcoin-core]
rpc_url = {rpc_url}
"#,
        rpc_url = toml_string(rpc_url),
    );
    std::fs::write(&config_path, config).expect("write collector runtime config");
    config_path
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

/// Prepares a typed Bitcoin collector launch against the supplied store scope.
pub async fn prepare_btc_balance_launch_for_store<S>(
    store: &S,
    config: &serde_json::Value,
    invocation_key: Option<&str>,
) -> mfm_app::RunLaunchRequest
where
    S: store::StoreScopeStore,
{
    let config = serde_json::from_value(config.clone()).expect("Bitcoin collector config");
    let draft = mfm_op_btc_collectors::btc_address_balance_program_draft(config)
        .expect("Bitcoin collector program draft");
    let seed_material = draft
        .seeds()
        .iter()
        .map(|seed| {
            let bytes = CanonicalSeed::from_value(
                &mfm_op_btc_collectors::BtcAddressBalanceObservationContext {
                    observed_at_unix_ms: None,
                },
            )
            .expect("Bitcoin observation seed")
            .canonical_json()
            .clone();
            (seed.seed_id.clone(), bytes)
        })
        .collect();
    prepare_typed_launch_for_store(store, draft, seed_material, invocation_key).await
}

async fn prepare_typed_launch_for_store<S>(
    store: &S,
    draft: TypedProgramDraft,
    seed_material: BTreeMap<mfm_ids::SeedId, mfm_canonical::PlainCanonicalJsonBytes>,
    invocation_key: Option<&str>,
) -> mfm_app::RunLaunchRequest
where
    S: store::StoreScopeStore,
{
    let certification_registry = mfm_app::production_certification_registry().expect("cert");
    let store_scope_id = store
        .load_store_scope_id()
        .await
        .unwrap_or_else(|error| panic!("store scope: {error}"));
    mfm_app::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        &certification_registry,
        store_scope_id,
        invocation_key
            .map(mfm_app::InvocationKey::new)
            .transpose()
            .expect("invocation key"),
    )
    .expect("prepared typed launch")
}
