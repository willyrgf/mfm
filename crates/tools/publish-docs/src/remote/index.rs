use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use semver::Version;
use serde::Deserialize;
use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinSet,
    time::{sleep, Instant},
};

use crate::model::{
    LocalPackage, RegistryFreshness, RegistryObservation, RegistryObservationSource, RegistryStatus,
};

const DEFAULT_INDEX_BASE_URL: &str = "https://index.crates.io";
const MAX_CONCURRENCY: usize = 4;
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(200);
const MAX_RETRIES: usize = 2;

/// Sparse-index-backed registry observer with local cache support.
#[derive(Debug, Clone)]
pub(crate) struct IndexRegistryObserver {
    client: reqwest::Client,
    base_url: String,
    cache_root: PathBuf,
    semaphore: Arc<Semaphore>,
    next_request_at: Arc<Mutex<Instant>>,
}

impl IndexRegistryObserver {
    /// Builds the default index observer rooted under `.mfm/publish-docs/cache/registry-index`.
    pub(crate) fn new(workspace_root: &Path) -> Result<Self, reqwest::Error> {
        Self::with_base_url(
            workspace_root,
            DEFAULT_INDEX_BASE_URL.to_string(),
            ".mfm/publish-docs/cache/registry-index".into(),
        )
    }

    /// Builds an index observer using a custom base URL and cache root.
    pub(crate) fn with_base_url(
        workspace_root: &Path,
        base_url: String,
        cache_relative_path: PathBuf,
    ) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            client,
            base_url,
            cache_root: workspace_root.join(cache_relative_path),
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENCY)),
            next_request_at: Arc::new(Mutex::new(Instant::now())),
        })
    }

    /// Observes many packages while preserving input order.
    pub(crate) async fn observe_packages(
        &self,
        packages: &[&LocalPackage],
    ) -> Vec<RegistryObservation> {
        let mut tasks = JoinSet::new();
        for (index, package) in packages.iter().enumerate() {
            let observer = self.clone();
            let package = (*package).clone();
            tasks.spawn(async move { (index, observer.observe_package(&package).await) });
        }

        let mut observations = vec![None; packages.len()];
        while let Some(result) = tasks.join_next().await {
            let (index, observation) = result.expect("index observation task panicked");
            observations[index] = Some(observation);
        }

        observations
            .into_iter()
            .map(|observation| observation.expect("all observation slots filled"))
            .collect()
    }

    /// Observes one package against the sparse index, falling back to cache when possible.
    pub(crate) async fn observe_package(&self, package: &LocalPackage) -> RegistryObservation {
        let relative_path = sparse_index_relative_path(&package.name);
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            relative_path.replace('\\', "/")
        );
        tracing::debug!(
            target: "mfm_publish_docs",
            package = %package.name,
            local_version = %package.version,
            url,
            "observing registry package via sparse index"
        );

        let observation = match self.fetch_index_payload(&url).await {
            Ok(FetchOutcome::NotFound) => {
                self.fresh_observation(package, RegistryStatus::Absent, None, false, None)
            }
            Ok(FetchOutcome::Body(bytes)) => match parse_index_payload(package, &bytes) {
                Ok((latest_version, exact_version_present)) => {
                    let _ = write_cached_payload(&self.cache_path(&relative_path), &bytes);
                    self.fresh_observation(
                        package,
                        RegistryStatus::Present,
                        latest_version,
                        exact_version_present,
                        None,
                    )
                }
                Err(diagnostic_code) => self.fresh_observation(
                    package,
                    RegistryStatus::InvalidResponse,
                    None,
                    false,
                    Some(diagnostic_code),
                ),
            },
            Err((status, diagnostic_code)) => {
                match read_cached_payload(&self.cache_path(&relative_path)) {
                    Ok(Some(bytes)) => match parse_index_payload(package, &bytes) {
                        Ok((latest_version, exact_version_present)) => RegistryObservation {
                            package: package.name.clone(),
                            status,
                            latest_version,
                            exact_version_present,
                            source: RegistryObservationSource::Index,
                            freshness: RegistryFreshness::Cached,
                            observed_at: None,
                            diagnostic_code: Some(diagnostic_code.to_string()),
                        },
                        Err(cache_diagnostic_code) => RegistryObservation {
                            package: package.name.clone(),
                            status: RegistryStatus::InvalidResponse,
                            latest_version: None,
                            exact_version_present: false,
                            source: RegistryObservationSource::Index,
                            freshness: RegistryFreshness::Unavailable,
                            observed_at: None,
                            diagnostic_code: Some(cache_diagnostic_code.to_string()),
                        },
                    },
                    Ok(None) | Err(_) => RegistryObservation {
                        package: package.name.clone(),
                        status,
                        latest_version: None,
                        exact_version_present: false,
                        source: RegistryObservationSource::Index,
                        freshness: RegistryFreshness::Unavailable,
                        observed_at: None,
                        diagnostic_code: Some(diagnostic_code.to_string()),
                    },
                }
            }
        };
        finish_registry_observation(observation)
    }

    async fn fetch_index_payload(
        &self,
        url: &str,
    ) -> Result<FetchOutcome, (RegistryStatus, &'static str)> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .expect("index observer semaphore closed");
        let mut attempt = 0;

        loop {
            self.wait_for_request_slot().await;
            let response = match self.client.get(url).send().await {
                Ok(response) => response,
                Err(error) => {
                    let retryable = error.is_timeout() || error.is_connect() || error.is_request();
                    if retryable && attempt < MAX_RETRIES {
                        attempt += 1;
                        let retry_delay = backoff_for_attempt(attempt);
                        tracing::debug!(
                            target: "mfm_publish_docs",
                            url,
                            attempt,
                            max_retries = MAX_RETRIES,
                            error = %error,
                            retry_delay_ms = retry_delay.as_millis(),
                            "retrying sparse-index request after transport error"
                        );
                        sleep(retry_delay).await;
                        continue;
                    }
                    return Err((RegistryStatus::TemporaryError, "index-request-failed"));
                }
            };

            let status = response.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(FetchOutcome::NotFound);
            }
            if matches!(
                status,
                reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
            ) {
                return Err((RegistryStatus::AuthError, "index-auth-error"));
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                if attempt < MAX_RETRIES {
                    attempt += 1;
                    let retry_delay = retry_after_duration(response.headers())
                        .unwrap_or_else(|| backoff_for_attempt(attempt));
                    tracing::debug!(
                        target: "mfm_publish_docs",
                        url,
                        attempt,
                        max_retries = MAX_RETRIES,
                        status = status.as_u16(),
                        retry_delay_ms = retry_delay.as_millis(),
                        "retrying sparse-index request after rate limit"
                    );
                    sleep(retry_delay).await;
                    continue;
                }
                return Err((RegistryStatus::RateLimited, "index-rate-limited"));
            }
            if status.is_server_error() {
                if attempt < MAX_RETRIES {
                    attempt += 1;
                    let retry_delay = backoff_for_attempt(attempt);
                    tracing::debug!(
                        target: "mfm_publish_docs",
                        url,
                        attempt,
                        max_retries = MAX_RETRIES,
                        status = status.as_u16(),
                        retry_delay_ms = retry_delay.as_millis(),
                        "retrying sparse-index request after server error"
                    );
                    sleep(retry_delay).await;
                    continue;
                }
                return Err((RegistryStatus::TemporaryError, "index-server-error"));
            }
            if !status.is_success() {
                return Err((RegistryStatus::InvalidResponse, "index-unexpected-status"));
            }

            let bytes = response
                .bytes()
                .await
                .map_err(|_| (RegistryStatus::TemporaryError, "index-body-read-failed"))?;
            return Ok(FetchOutcome::Body(bytes.to_vec()));
        }
    }

    async fn wait_for_request_slot(&self) {
        let mut next_request_at = self.next_request_at.lock().await;
        let now = Instant::now();
        if *next_request_at > now {
            sleep(*next_request_at - now).await;
        }
        *next_request_at = Instant::now() + MIN_REQUEST_INTERVAL;
    }

    fn cache_path(&self, relative_path: &str) -> PathBuf {
        self.cache_root.join(relative_path)
    }

    fn fresh_observation(
        &self,
        package: &LocalPackage,
        status: RegistryStatus,
        latest_version: Option<Version>,
        exact_version_present: bool,
        diagnostic_code: Option<&'static str>,
    ) -> RegistryObservation {
        RegistryObservation {
            package: package.name.clone(),
            status,
            latest_version,
            exact_version_present,
            source: RegistryObservationSource::Index,
            freshness: RegistryFreshness::Fresh,
            observed_at: Some(now_rfc3339()),
            diagnostic_code: diagnostic_code.map(str::to_string),
        }
    }
}

#[derive(Debug)]
enum FetchOutcome {
    NotFound,
    Body(Vec<u8>),
}

fn finish_registry_observation(observation: RegistryObservation) -> RegistryObservation {
    tracing::debug!(
        target: "mfm_publish_docs",
        package = %observation.package,
        status = ?observation.status,
        freshness = ?observation.freshness,
        latest_version = ?observation.latest_version,
        exact_version_present = observation.exact_version_present,
        diagnostic_code = ?observation.diagnostic_code,
        "completed registry package observation"
    );
    observation
}

#[derive(Debug, Deserialize)]
struct IndexLine {
    vers: String,
}

fn parse_index_payload(
    package: &LocalPackage,
    bytes: &[u8],
) -> Result<(Option<Version>, bool), &'static str> {
    if bytes.is_empty() {
        return Ok((None, false));
    }

    let mut latest_version: Option<Version> = None;
    let mut exact_version_present = false;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let parsed_line: IndexLine =
            serde_json::from_slice(line).map_err(|_| "index-invalid-json-line")?;
        let version = Version::parse(&parsed_line.vers).map_err(|_| "index-invalid-version")?;
        if version == package.version {
            exact_version_present = true;
        }
        if latest_version
            .as_ref()
            .map(|current| &version > current)
            .unwrap_or(true)
        {
            latest_version = Some(version);
        }
    }

    Ok((latest_version, exact_version_present))
}

fn sparse_index_relative_path(package_name: &str) -> String {
    let normalized = package_name.to_ascii_lowercase();
    match normalized.len() {
        1 => format!("1/{normalized}"),
        2 => format!("2/{normalized}"),
        3 => format!("3/{}/{}", &normalized[..1], normalized),
        _ => format!("{}/{}/{}", &normalized[..2], &normalized[2..4], normalized),
    }
}

fn write_cached_payload(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

fn read_cached_payload(path: &Path) -> Result<Option<Vec<u8>>, std::io::Error> {
    if !path.exists() {
        return Ok(None);
    }
    fs::read(path).map(Some)
}

fn retry_after_duration(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn backoff_for_attempt(attempt: usize) -> Duration {
    Duration::from_millis(200 * (attempt as u64))
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use semver::Version;
    use tempfile::tempdir;

    use super::{sparse_index_relative_path, IndexRegistryObserver};
    use crate::model::{RegistryFreshness, RegistryStatus};

    fn local_package(name: &str, version: &str) -> crate::model::LocalPackage {
        crate::model::LocalPackage {
            name: name.into(),
            version: Version::parse(version).expect("version"),
            manifest_path: format!("crates/{name}/Cargo.toml").into(),
            workspace_path: format!("crates/{name}").into(),
            has_docs_target: true,
            readme: Some("README.md".into()),
            repository: Some("https://github.com/willyrgf/mfm".into()),
            license: Some("MIT".into()),
            publish: None,
            local_dependencies: Vec::new(),
        }
    }

    #[test]
    fn computes_sparse_index_paths() {
        assert_eq!(sparse_index_relative_path("a"), "1/a");
        assert_eq!(sparse_index_relative_path("ab"), "2/ab");
        assert_eq!(sparse_index_relative_path("abc"), "3/a/abc");
        assert_eq!(
            sparse_index_relative_path("mfm-machine"),
            "mf/m-/mfm-machine"
        );
    }

    #[tokio::test]
    async fn observes_fresh_index_payload() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).expect("read");
            let body = concat!("{\"vers\":\"0.1.0\"}\n", "{\"vers\":\"0.2.0\"}\n");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write");
        });
        let tempdir = tempdir().expect("tempdir");
        let observer = IndexRegistryObserver::with_base_url(
            tempdir.path(),
            format!("http://{addr}"),
            ".cache".into(),
        )
        .expect("observer");

        let observation = observer
            .observe_package(&local_package("mfm-machine", "0.2.0"))
            .await;
        server.join().expect("server");

        assert!(matches!(observation.status, RegistryStatus::Present));
        assert!(matches!(observation.freshness, RegistryFreshness::Fresh));
        assert!(observation.exact_version_present);
    }

    #[tokio::test]
    async fn falls_back_to_cached_payload_when_refresh_fails() {
        let tempdir = tempdir().expect("tempdir");
        let observer = IndexRegistryObserver::with_base_url(
            tempdir.path(),
            "http://127.0.0.1:1".into(),
            ".cache".into(),
        )
        .expect("observer");
        let package = local_package("mfm-machine", "0.1.0");
        let cache_path = tempdir.path().join(".cache").join("mf/m-/mfm-machine");
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).expect("cache parent");
        }
        fs::write(&cache_path, "{\"vers\":\"0.1.0\"}\n").expect("cache");

        let observation = observer.observe_package(&package).await;

        assert!(matches!(observation.freshness, RegistryFreshness::Cached));
        assert!(matches!(observation.status, RegistryStatus::TemporaryError));
        assert!(observation.exact_version_present);
    }
}
