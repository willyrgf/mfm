//! Managed cross-transport execution verification over production composition.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FAULT_EVM_LOCATOR_ENV: &str = "MFM_E2E_FAULT_EVM_ADAPTER_LOCATOR";
const FUNDED_RAW_UNITS: &str = "1000000000000000000000000";
// Genesis and the two leading Portfolio Pure conclusions precede the first Read.
const INTERRUPTED_HEAD_SEQUENCE: u64 = 3;
// The native two-source path executes fifteen States; its two Matches append no frames.
const TERMINAL_HEAD_SEQUENCE: u64 = 16;
const OUTPUT_CONTRACT_DIGEST: &str =
    "content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927";
const OUTPUT_SCHEMA_ID: &str = "schema:mfm.derived.portfolio_snapshot_output:1:sha256-jcs-v1:46fd4fd57bd2ba864c14b1013d3fe59815da0a01896fc097b5e4935eeec96116";

#[test]
#[ignore = "requires explicit CLI/REST binaries and managed PostgreSQL/Reth services"]
fn generated_rest_run_survives_deletion_and_cold_resumes_to_exact_live_snapshot() {
    let cli = required_path("MFM_E2E_CLI_BIN");
    let rest = required_path("MFM_E2E_REST_BIN");
    for name in [
        "MFM_E2E_RUNTIME_POSTGRES_LOCATOR",
        "MFM_E2E_ADMIN_POSTGRES_LOCATOR",
        "MFM_E2E_EVM_ADAPTER_LOCATOR",
    ] {
        std::env::var(name).unwrap_or_else(|_| panic!("client-e2e must supply {name}"));
    }

    let root = temporary_root();
    let xdg = root.join("xdg");
    let live_deployment = xdg.join("mfm/deployment.toml");
    let fault_deployment = root.join("fault-deployment.toml");
    let socket_dir = root.join("socket");
    let socket = socket_dir.join("mfm.sock");
    std::fs::create_dir_all(live_deployment.parent().expect("deployment parent"))
        .expect("create XDG tree");
    std::fs::create_dir(&socket_dir).expect("create socket directory");
    std::fs::write(&live_deployment, deployment("MFM_E2E_EVM_ADAPTER_LOCATOR"))
        .expect("write live deployment");
    std::fs::write(&fault_deployment, deployment(FAULT_EVM_LOCATOR_ENV))
        .expect("write fault deployment");
    let historical_document = root.join("historical.json");
    let replacement_document = root.join("replacement.json");
    std::fs::write(&historical_document, historical_config()).expect("write historical config");
    std::fs::write(&replacement_document, replacement_config()).expect("write replacement config");

    let initialized = run_cli(
        &cli,
        &xdg,
        &[
            "postgres",
            "init",
            "--admin-locator-env",
            "MFM_E2E_ADMIN_POSTGRES_LOCATOR",
        ],
    );
    assert_success(&initialized, "postgres init");

    let historical = cli_json(
        &cli,
        &xdg,
        &[
            "config",
            "import",
            "daily",
            "--from",
            utf8(&historical_document),
        ],
    );
    assert_eq!(historical["outcome"], "created");
    let historical_digest = historical["config"]["digest"]
        .as_str()
        .expect("historical digest")
        .to_owned();
    let replacement = cli_json(
        &cli,
        &xdg,
        &[
            "config",
            "import",
            "daily",
            "--from",
            utf8(&replacement_document),
        ],
    );
    assert_eq!(replacement["outcome"], "created");
    assert_ne!(replacement["config"]["digest"], historical_digest);

    let unavailable = UnavailableRpc::start();
    let mut fault_daemon = Daemon::start(
        &rest,
        &xdg,
        &socket,
        Some(&fault_deployment),
        Some(unavailable.locator()),
    );
    fault_daemon.wait_ready();

    let start_body = format!(r#"{{"config":{{"name":"daily","digest":"{historical_digest}"}}}}"#);
    let interrupted = rest_json(
        &socket,
        "POST",
        "/v1/runs/start",
        Some((&start_body, "application/json")),
    );
    assert_eq!(interrupted.0, 503);
    let run_id = interrupted.1["run_id"]
        .as_str()
        .expect("generated REST run id")
        .to_owned();
    assert_digest(&interrupted.1["run_id"], "run:sha256-jcs-v1:");
    assert_eq!(
        interrupted.1,
        serde_json::json!({
            "code": "dependency_unavailable",
            "message": "application dependency is unavailable",
            "run_id": run_id
        })
    );
    unavailable.assert_chain_identity_request();

    let runnable = rest_json(&socket, "GET", &format!("/v1/runs/{run_id}"), None);
    assert_eq!(runnable.0, 200);
    assert_eq!(runnable.1["run_id"], run_id);
    assert_eq!(runnable.1["head_sequence"], INTERRUPTED_HEAD_SEQUENCE);
    assert_eq!(
        runnable.1["state"],
        serde_json::json!({ "kind": "runnable" })
    );
    assert_digest(&runnable.1["head_digest"], "content:sha256-v1:");

    fault_daemon.stop();
    assert!(!socket.exists(), "fault daemon must remove the socket");

    let deleted = run_cli(
        &cli,
        &xdg,
        &["config", "delete", "daily", "--digest", &historical_digest],
    );
    assert_success(&deleted, "delete admitted historical revision");

    let mut live_daemon = Daemon::start(&rest, &xdg, &socket, None, None);
    live_daemon.wait_ready();
    let progressed = rest_json(
        &socket,
        "POST",
        &format!("/v1/runs/{run_id}/progress"),
        Some(("{}", "application/json")),
    );
    assert_eq!(progressed.0, 200);
    assert_exact_live_snapshot(&progressed.1, &run_id);
    assert_eq!(
        rest_json(&socket, "GET", &format!("/v1/runs/{run_id}"), None),
        progressed
    );

    live_daemon.stop();
    assert!(!socket.exists(), "live daemon must remove the socket");

    let cold_cli = cli_json(&cli, &xdg, &["run", "show", "--run-id", &run_id]);
    assert_eq!(cold_cli, progressed.1);
    let terminal_progress = cli_json(&cli, &xdg, &["run", "progress", "--run-id", &run_id]);
    assert_eq!(terminal_progress, progressed.1);

    std::fs::remove_dir_all(&root).expect("remove client e2e tree");
}

fn deployment(adapter_locator_env: &str) -> String {
    format!(
        r#"[postgres]
runtime_locator_env = "MFM_E2E_RUNTIME_POSTGRES_LOCATOR"

[[evm_routes]]
chain_id = 1337
endpoint_id = "reth-dev"
adapter_locator_env = "{adapter_locator_env}"
"#
    )
}

fn historical_config() -> &'static str {
    r#"{
  "entry_point": "mfm.portfolio/snapshot@1",
  "input": {
    "routes": [{"chain_id": 1337, "endpoint_id": "reth-dev"}],
    "selector": {"target": "portfolio-historical", "quote": "usd"},
    "portfolio": {"portfolio_id": "portfolio-historical", "quotes": ["usd"],
      "collections": [{"correlation": "native-collection",
        "request": {"sources": [
          {"source_id": "wallet.funded", "chain_id": 1337,
            "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8", "token": null},
          {"source_id": "wallet.empty", "chain_id": 1337,
            "address": "0x0000000000000000000000000000000000000001", "token": null}
        ], "decimals": 18}}]}
  }
}"#
}

fn replacement_config() -> &'static str {
    r#"{
  "entry_point": "mfm.portfolio/snapshot@1",
  "input": {
    "routes": [{"chain_id": 1337, "endpoint_id": "reth-dev"}],
    "selector": {"target": "portfolio-replacement", "quote": "usd"},
    "portfolio": {"portfolio_id": "portfolio-replacement", "quotes": ["usd"],
      "collections": [{"correlation": "replacement-collection",
        "request": {"sources": [
          {"source_id": "wallet.replacement", "chain_id": 1337,
            "address": "0x0000000000000000000000000000000000000001", "token": null}
        ], "decimals": 18}}]}
  }
}"#
}

fn assert_exact_live_snapshot(view: &serde_json::Value, run_id: &str) {
    assert_eq!(view["run_id"], run_id);
    assert_eq!(view["head_sequence"], TERMINAL_HEAD_SEQUENCE);
    assert_digest(&view["head_digest"], "content:sha256-v1:");

    let state = view["state"].as_object().expect("terminal state");
    assert_eq!(state.len(), 4);
    assert_eq!(state["kind"], "succeeded");
    assert_eq!(
        state["contract_ref"],
        serde_json::json!({
            "content_digest": OUTPUT_CONTRACT_DIGEST,
            "schema_id": OUTPUT_SCHEMA_ID
        })
    );
    assert_eq!(state["value_ref"]["schema_id"], OUTPUT_SCHEMA_ID);
    assert_digest(&state["value_ref"]["content_digest"], "content:sha256-v1:");

    let anchor = state["value"]["snapshot"]["collections"][0]["anchor"].clone();
    assert_anchor(&anchor);
    assert_eq!(
        state["value"],
        serde_json::json!({
            "snapshot": {
                "schema_version": 1,
                "portfolio_id": "portfolio-historical",
                "collections": [{
                    "collection_ordinal": 0,
                    "chain_id": 1337,
                    "anchor": anchor,
                    "holdings": [
                        {
                            "source_id": "wallet.funded",
                            "asset": { "kind": "native" },
                            "decimals": 18,
                            "raw_units": FUNDED_RAW_UNITS,
                            "amount_dec": "1000000.000000000000000000"
                        },
                        {
                            "source_id": "wallet.empty",
                            "asset": { "kind": "native" },
                            "decimals": 18,
                            "raw_units": "0",
                            "amount_dec": "0.000000000000000000"
                        }
                    ]
                }]
            },
            "report": {
                "schema_version": 1,
                "portfolio_id": "portfolio-historical",
                "quote": "usd",
                "collection_summaries": [{
                    "collection_ordinal": 0,
                    "total_value_dec": "1000000"
                }],
                "totals_by_quote": [{
                    "quote": "usd",
                    "total_value_dec": "1000000"
                }]
            }
        })
    );
}

fn assert_anchor(anchor: &serde_json::Value) {
    let anchor = anchor.as_object().expect("collection anchor");
    assert_eq!(anchor.len(), 2);
    anchor["number"]
        .as_str()
        .and_then(|number| number.parse::<u64>().ok())
        .expect("decimal anchor number");
    let hash = anchor["hash"].as_str().expect("anchor hash");
    assert!(
        hash.strip_prefix("0x").is_some_and(is_lower_hex_digest),
        "invalid anchor hash: {hash}"
    );
}

fn assert_digest(value: &serde_json::Value, prefix: &str) {
    let value = value.as_str().expect("digest string");
    assert!(
        value.strip_prefix(prefix).is_some_and(is_lower_hex_digest),
        "invalid digest: {value}"
    );
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

struct UnavailableRpc {
    locator: String,
    observed: Receiver<Vec<u8>>,
    server: JoinHandle<()>,
}

impl UnavailableRpc {
    fn start() -> Self {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("bind unavailable RPC stub");
        let address = listener.local_addr().expect("stub address");
        let (sender, observed) = mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("stub read timeout");
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .expect("stub write timeout");
            let encoded = read_http_request(&mut stream).expect("read provider request");
            sender.send(encoded).expect("retain provider request");
            stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("write unavailable response");
            stream.flush().expect("flush unavailable response");
        });
        Self {
            locator: format!("http://{address}"),
            observed,
            server,
        }
    }

    fn locator(&self) -> &str {
        &self.locator
    }

    fn assert_chain_identity_request(self) {
        let encoded = self
            .observed
            .recv_timeout(Duration::from_secs(5))
            .expect("provider request");
        self.server.join().expect("RPC stub");
        let body = http_request_body(&encoded);
        let request: serde_json::Value = serde_json::from_slice(body).expect("provider JSON-RPC");
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["method"], "eth_chainId");
        assert_eq!(request["params"], serde_json::json!([]));
    }
}

fn read_http_request(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut encoded = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Err(invalid_http());
        }
        encoded.extend_from_slice(&buffer[..read]);
        let Some(split) = encoded.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = std::str::from_utf8(&encoded[..split]).map_err(|_| invalid_http())?;
        let length = head
            .lines()
            .skip(1)
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .ok_or_else(invalid_http)?;
        if encoded.len() >= split + 4 + length {
            return Ok(encoded);
        }
    }
}

fn http_request_body(encoded: &[u8]) -> &[u8] {
    let split = encoded
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP request head");
    &encoded[split + 4..]
}

struct Daemon {
    child: Child,
    socket: PathBuf,
}

impl Daemon {
    fn start(
        binary: &Path,
        xdg: &Path,
        socket: &Path,
        deployment: Option<&Path>,
        fault_locator: Option<&str>,
    ) -> Self {
        let mut command = command(binary, xdg);
        command.arg("serve");
        if let Some(deployment) = deployment {
            command.args(["--deployment", utf8(deployment)]);
        }
        command.args(["--unix-socket", utf8(socket)]);
        if let Some(locator) = fault_locator {
            command.env(FAULT_EVM_LOCATOR_ENV, locator);
        }
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start REST daemon");
        Self {
            child,
            socket: socket.to_owned(),
        }
    }

    fn wait_ready(&mut self) {
        for _ in 0..600 {
            if let Some(status) = self.child.try_wait().expect("observe daemon") {
                let mut stderr = String::new();
                self.child
                    .stderr
                    .as_mut()
                    .expect("daemon stderr")
                    .read_to_string(&mut stderr)
                    .expect("read daemon stderr");
                panic!("REST daemon exited {status}: {stderr}");
            }
            if self.socket.exists() {
                if let Ok(response) = http(&self.socket, "GET", "/healthz", None) {
                    if response.status == 200 {
                        return;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("REST daemon did not become ready");
    }

    fn stop(&mut self) {
        // SAFETY: `kill` receives the live child PID and a valid signal number; no pointer is used.
        let result = unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
        assert_eq!(result, 0, "send SIGTERM to REST daemon");
        for _ in 0..200 {
            if let Some(status) = self.child.try_wait().expect("observe daemon shutdown") {
                assert!(status.success(), "REST daemon shutdown failed: {status}");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.child.kill().expect("kill wedged REST daemon");
        panic!("REST daemon did not stop gracefully");
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn rest_json(
    socket: &Path,
    method: &str,
    target: &str,
    body: Option<(&str, &str)>,
) -> (u16, serde_json::Value) {
    let response = http(socket, method, target, body).expect("REST request");
    let json = serde_json::from_slice(&response.body).expect("REST response JSON");
    (response.status, json)
}

fn http(
    socket: &Path,
    method: &str,
    target: &str,
    body: Option<(&str, &str)>,
) -> std::io::Result<HttpResponse> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(130)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let (body, content_type) = body.unwrap_or(("", ""));
    write!(
        stream,
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    )?;
    if !content_type.is_empty() {
        write!(stream, "Content-Type: {content_type}\r\n")?;
    }
    write!(stream, "\r\n{body}")?;
    stream.flush()?;
    let mut encoded = Vec::new();
    stream.read_to_end(&mut encoded)?;
    parse_http_response(encoded)
}

fn parse_http_response(encoded: Vec<u8>) -> std::io::Result<HttpResponse> {
    let split = encoded
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(invalid_http)?;
    let head = std::str::from_utf8(&encoded[..split]).map_err(|_| invalid_http())?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .ok_or_else(invalid_http)?;
    let body = &encoded[split + 4..];
    let chunked = head.lines().skip(1).any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
        })
    });
    let body = if chunked {
        decode_chunked(body)?
    } else {
        body.to_vec()
    };
    Ok(HttpResponse { status, body })
}

fn decode_chunked(mut encoded: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut decoded = Vec::new();
    loop {
        let end = encoded
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(invalid_http)?;
        let size = std::str::from_utf8(&encoded[..end])
            .ok()
            .and_then(|value| value.split(';').next())
            .and_then(|value| usize::from_str_radix(value, 16).ok())
            .ok_or_else(invalid_http)?;
        encoded = &encoded[end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if encoded.len() < size + 2 || &encoded[size..size + 2] != b"\r\n" {
            return Err(invalid_http());
        }
        decoded.extend_from_slice(&encoded[..size]);
        encoded = &encoded[size + 2..];
    }
}

fn invalid_http() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid HTTP response")
}

fn cli_json(binary: &Path, xdg: &Path, arguments: &[&str]) -> serde_json::Value {
    let mut complete = vec!["--output", "json"];
    complete.extend_from_slice(arguments);
    let output = run_cli(binary, xdg, &complete);
    assert_success(&output, "CLI request");
    serde_json::from_slice(&output.stdout).expect("CLI response JSON")
}

fn run_cli(binary: &Path, xdg: &Path, arguments: &[&str]) -> Output {
    command(binary, xdg)
        .args(arguments)
        .output()
        .expect("run CLI")
}

fn command(binary: &Path, xdg: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("XDG_CONFIG_HOME", xdg)
        .env("HOME", "/nonexistent-hostile-home")
        .env("MFM_DEPLOYMENT", "/nonexistent-hostile-deployment")
        .env(
            "MFM_E2E_RUNTIME_POSTGRES_LOCATOR",
            std::env::var("MFM_E2E_RUNTIME_POSTGRES_LOCATOR").expect("runtime locator"),
        )
        .env(
            "MFM_E2E_ADMIN_POSTGRES_LOCATOR",
            std::env::var("MFM_E2E_ADMIN_POSTGRES_LOCATOR").expect("admin locator"),
        )
        .env(
            "MFM_E2E_EVM_ADAPTER_LOCATOR",
            std::env::var("MFM_E2E_EVM_ADAPTER_LOCATOR").expect("EVM locator"),
        );
    command
}

fn temporary_root() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_nanos()
        ^ u128::from(std::process::id());
    PathBuf::from("/tmp").join(format!("mfm-client-e2e-{suffix:032x}"))
}

fn required_path(name: &str) -> PathBuf {
    let value = std::env::var_os(name).unwrap_or_else(|| panic!("client-e2e must supply {name}"));
    let path = PathBuf::from(value);
    assert!(path.is_absolute(), "{name} must be absolute");
    path
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("UTF-8 fixture path")
}

fn assert_success(output: &Output, label: &str) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{label} wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
