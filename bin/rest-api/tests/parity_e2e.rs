//! Managed two-binary parity verification over the production Unix-socket listener.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
#[ignore = "requires explicit CLI/REST binaries and managed PostgreSQL/Reth services"]
fn cli_and_rest_are_one_live_application_surface() {
    let cli = required_path("MFM_E2E_CLI_BIN");
    let rest = required_path("MFM_E2E_REST_BIN");
    for name in [
        "MFM_E2E_RUNTIME_STORE_LOCATOR",
        "MFM_E2E_ADMIN_STORE_LOCATOR",
        "MFM_E2E_EVM_ADAPTER_LOCATOR",
    ] {
        std::env::var(name).unwrap_or_else(|_| panic!("rest-e2e must supply {name}"));
    }

    let root = temporary_root();
    let xdg = root.join("xdg");
    let deployment = xdg.join("mfm/deployment.toml");
    let socket_dir = root.join("socket");
    let socket = socket_dir.join("mfm.sock");
    std::fs::create_dir_all(deployment.parent().expect("deployment parent"))
        .expect("create XDG tree");
    std::fs::create_dir(&socket_dir).expect("create socket directory");
    std::fs::write(
        &deployment,
        r#"[store]
runtime_locator_env = "MFM_E2E_RUNTIME_STORE_LOCATOR"

[[evm_routes]]
chain_id = 1337
endpoint_id = "reth-dev"
adapter_locator_env = "MFM_E2E_EVM_ADAPTER_LOCATOR"
"#,
    )
    .expect("write deployment");
    let document = root.join("daily.json");
    std::fs::write(&document, config_document()).expect("write config document");

    let initialized = run_cli(
        &cli,
        &xdg,
        &[
            "store",
            "init",
            "--admin-store-locator-env",
            "MFM_E2E_ADMIN_STORE_LOCATOR",
        ],
    );
    assert_success(&initialized, "store init");

    let mut daemon = Daemon::start(&rest, &xdg, &socket);
    daemon.wait_ready();

    assert_eq!(
        cli_json(&cli, &xdg, &["entry-point", "list"]),
        rest_json(&socket, "GET", "/v1/entry-points", None).1
    );
    assert_eq!(
        cli_json(&cli, &xdg, &["binding", "list"]),
        rest_json(&socket, "GET", "/v1/bindings", None).1
    );

    let imported = cli_json(
        &cli,
        &xdg,
        &["config", "import", "daily", "--from", utf8(&document)],
    );
    assert_eq!(imported["outcome"], "created");
    let digest = imported["config"]["digest"]
        .as_str()
        .expect("config digest")
        .to_owned();
    let rest_import = rest_json(
        &socket,
        "PUT",
        "/v1/configs/daily",
        Some((&config_document(), "application/json")),
    );
    assert_eq!(rest_import.0, 200);
    assert_eq!(rest_import.1["outcome"], "unchanged");
    assert_eq!(rest_import.1["config"], imported["config"]);
    assert_eq!(
        cli_json(&cli, &xdg, &["config", "show", "daily"]),
        rest_json(&socket, "GET", "/v1/configs/daily", None).1
    );
    assert_eq!(
        cli_json(&cli, &xdg, &["config", "list", "--limit", "50"]),
        rest_json(&socket, "GET", "/v1/configs?limit=50", None).1
    );

    let current_id = run_id(0x11);
    let cli_current = cli_json(
        &cli,
        &xdg,
        &["run", "start", "--config", "daily", "--run-id", &current_id],
    );
    let current_body = r#"{"config":{"kind":"current","name":"daily"}}"#;
    let rest_current = rest_json(
        &socket,
        "POST",
        &format!("/v1/runs/{current_id}/start"),
        Some((current_body, "application/json")),
    );
    assert_eq!(rest_current.0, 200);
    assert_eq!(rest_current.1, cli_current);
    assert_eq!(rest_current.1["config"]["digest"], digest);
    assert_eq!(rest_current.1["run"]["state"]["kind"], "succeeded");
    assert!(rest_current.1["run"]["state"]["value"].is_object());

    let exact_id = run_id(0x22);
    let exact_body =
        format!(r#"{{"config":{{"kind":"exact","name":"daily","digest":"{digest}"}}}}"#);
    let rest_exact = rest_json(
        &socket,
        "POST",
        &format!("/v1/runs/{exact_id}/start"),
        Some((&exact_body, "application/json")),
    );
    assert_eq!(rest_exact.0, 200);
    let cli_exact = cli_json(
        &cli,
        &xdg,
        &[
            "run",
            "start",
            "--config",
            "daily",
            "--config-digest",
            &digest,
            "--run-id",
            &exact_id,
        ],
    );
    assert_eq!(rest_exact.1, cli_exact);

    assert_eq!(
        cli_json(&cli, &xdg, &["run", "show", "--run-id", &current_id]),
        rest_json(&socket, "GET", &format!("/v1/runs/{current_id}"), None).1
    );
    assert_eq!(
        cli_json(&cli, &xdg, &["run", "list", "--limit", "50"]),
        rest_json(&socket, "GET", "/v1/runs?limit=50", None).1
    );

    let wrong_digest =
        "content:sha256-jcs-v1:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let mismatch_id = run_id(0x33);
    let cli_mismatch = run_cli(
        &cli,
        &xdg,
        &[
            "--output",
            "json",
            "run",
            "start",
            "--config",
            "daily",
            "--config-digest",
            wrong_digest,
            "--run-id",
            &mismatch_id,
        ],
    );
    assert_eq!(cli_mismatch.status.code(), Some(2));
    let rest_mismatch_body =
        format!(r#"{{"config":{{"kind":"exact","name":"daily","digest":"{wrong_digest}"}}}}"#);
    let rest_mismatch = rest_json(
        &socket,
        "POST",
        &format!("/v1/runs/{mismatch_id}/start"),
        Some((&rest_mismatch_body, "application/json")),
    );
    assert_eq!(rest_mismatch.0, 409);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&cli_mismatch.stderr).expect("CLI error JSON"),
        rest_mismatch.1
    );

    daemon.stop();
    assert!(!socket.exists(), "graceful shutdown must remove the socket");
    std::fs::remove_dir_all(&root).expect("remove parity tree");
}

struct Daemon {
    child: Child,
    socket: PathBuf,
}

impl Daemon {
    fn start(binary: &Path, xdg: &Path, socket: &Path) -> Self {
        let child = command(binary, xdg)
            .args([
                "serve",
                "--unix-socket",
                utf8(socket),
                "--max-in-flight-runs",
                "4",
                "--run-timeout",
                "120",
            ])
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
            "MFM_E2E_RUNTIME_STORE_LOCATOR",
            std::env::var("MFM_E2E_RUNTIME_STORE_LOCATOR").expect("runtime locator"),
        )
        .env(
            "MFM_E2E_ADMIN_STORE_LOCATOR",
            std::env::var("MFM_E2E_ADMIN_STORE_LOCATOR").expect("admin locator"),
        )
        .env(
            "MFM_E2E_EVM_ADAPTER_LOCATOR",
            std::env::var("MFM_E2E_EVM_ADAPTER_LOCATOR").expect("EVM locator"),
        );
    command
}

fn config_document() -> String {
    r#"{
  "entry_point": "mfm.portfolio/snapshot@1",
  "input": {
    "routes": [{"chain_id": 1337, "endpoint_id": "reth-dev"}],
    "selector": {"target": "portfolio-parity", "quote": "usd"},
    "portfolio": {"portfolio_id": "portfolio-parity", "quotes": ["usd"],
      "collections": [{"correlation": "native-collection",
        "request": {"sources": [{"source_id": "wallet.native", "chain_id": 1337,
          "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8", "token": null}],
          "decimals": 18}}]}
  }
}"#
    .to_owned()
}

fn run_id(byte: u8) -> String {
    format!("run:sha256-jcs-v1:{byte:02x}{}", "0".repeat(62))
}

fn temporary_root() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_nanos()
        ^ u128::from(std::process::id());
    PathBuf::from("/tmp").join(format!("mfm-rest-parity-{suffix:032x}"))
}

fn required_path(name: &str) -> PathBuf {
    let value = std::env::var_os(name).unwrap_or_else(|| panic!("rest-e2e must supply {name}"));
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
