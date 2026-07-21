use std::path::Path;

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

/// Starts one JSON-RPC mock serving the EVM and Bitcoin calls used by portfolio integration tests.
pub async fn start_portfolio_rpc_mock() -> String {
    let app = axum::Router::new().route("/", axum::routing::post(portfolio_rpc_handler));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind portfolio RPC mock");
    let addr = listener.local_addr().expect("portfolio RPC mock address");
    listener
        .set_nonblocking(true)
        .expect("set portfolio RPC mock nonblocking");
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("portfolio RPC mock runtime");
        runtime.block_on(async move {
            let listener =
                tokio::net::TcpListener::from_std(listener).expect("tokio portfolio RPC listener");
            axum::serve(listener, app)
                .await
                .expect("portfolio RPC mock serve");
        });
    });
    format!("http://{addr}")
}

async fn portfolio_rpc_handler(
    axum::Json(request): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let id = request
        .get("id")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(1));
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .expect("portfolio JSON-RPC method");
    let result = match method {
        "eth_chainId" => serde_json::json!("0x1"),
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
[evm.routes.{network}]
source_ref = {network}
rpc_url = {rpc_url}
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

/// Writes a runtime config containing the Bitcoin and EVM routes for a portfolio test.
pub fn write_portfolio_runtime_config_for_test(dir: &Path, rpc_url: &str) -> std::path::PathBuf {
    let config_path = dir.join("runtime.toml");
    let config = format!(
        r#"
[evm.routes.ethereum-mainnet]
source_ref = "ethereum-mainnet"
rpc_url = {rpc_url}

[btc.routes.public-bitcoin-core]
rpc_url = {rpc_url}
"#,
        rpc_url = toml_string(rpc_url),
    );
    std::fs::write(&config_path, config).expect("write portfolio runtime config");
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
