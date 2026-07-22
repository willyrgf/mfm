use std::path::Path;
use std::sync::{Arc, Mutex};

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

/// Running JSON-RPC mock with a complete record of requested method names.
pub struct PortfolioRpcMock {
    url: String,
    methods: Arc<Mutex<Vec<String>>>,
}

impl PortfolioRpcMock {
    /// Returns the mock's loopback endpoint.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the requested JSON-RPC methods in observed order.
    pub fn methods(&self) -> Vec<String> {
        self.methods
            .lock()
            .expect("portfolio RPC method record")
            .clone()
    }
}

/// Starts one counted JSON-RPC mock serving the portfolio EVM and Bitcoin calls.
pub async fn start_counted_portfolio_rpc_mock() -> PortfolioRpcMock {
    let methods = Arc::new(Mutex::new(Vec::new()));
    let app = axum::Router::new()
        .route("/", axum::routing::post(portfolio_rpc_handler))
        .with_state(Arc::clone(&methods));
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
    PortfolioRpcMock {
        url: format!("http://{addr}"),
        methods,
    }
}

/// Starts one JSON-RPC mock serving the EVM and Bitcoin calls used by portfolio integration tests.
pub async fn start_portfolio_rpc_mock() -> String {
    start_counted_portfolio_rpc_mock().await.url
}

async fn portfolio_rpc_handler(
    axum::extract::State(methods): axum::extract::State<Arc<Mutex<Vec<String>>>>,
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
    methods
        .lock()
        .expect("portfolio RPC method record")
        .push(method.to_owned());
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
        "scantxoutset" => serde_json::json!({
            "success": true,
            "txouts": 1,
            "height": 850_100u64,
            "bestblock": "abababababababababababababababababababababababababababababababab",
            "unspents": [{
                "txid": "1111111111111111111111111111111111111111111111111111111111111111",
                "vout": 0,
                "scriptPubKey": "00149c0a9f112688c85fcb4417b05c8dec775e4cab36",
                "desc": "addr(bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw)",
                "amount": 0.001,
                "height": 850_100u64
            }],
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
rpc_url = {{ direct = {rpc_url} }}
"#,
        network = toml_string(network_id),
        rpc_url = toml_string(rpc_url),
    );
    if let Some(signer) = signer {
        config.push_str(&format!(
            r#"
[keystores.default]
keystore_path = {{ direct = {keystore_path} }}
unlock_file_path = {{ direct = {unlock_file} }}

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
rpc_url = {{ direct = {rpc_url} }}

[bitcoin.routes.public-bitcoin-core]
rpc_url = {{ direct = {rpc_url} }}
scan_timeout_seconds = 30
"#,
        rpc_url = toml_string(rpc_url),
    );
    std::fs::write(&config_path, config).expect("write portfolio runtime config");
    config_path
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("toml-compatible string")
}
