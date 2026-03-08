use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::model::{LocalPackage, RegistryObservation, RegistryStatus};

const DEFAULT_CRATES_IO_API: &str = "https://crates.io/api/v1";

/// Simple crates.io observer for Phase 1 exact-version checks.
#[derive(Debug, Clone)]
pub struct CratesIoClient {
    client: reqwest::Client,
    base_url: String,
}

impl CratesIoClient {
    /// Builds the default crates.io observer.
    pub fn new() -> Result<Self, reqwest::Error> {
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
            base_url: DEFAULT_CRATES_IO_API.to_string(),
        })
    }

    /// Builds an observer targeting a custom base URL. Intended for tests.
    #[cfg(test)]
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, reqwest::Error> {
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
            base_url: base_url.into(),
        })
    }

    /// Observes all selected packages sequentially.
    pub async fn observe_packages(&self, packages: &[&LocalPackage]) -> Vec<RegistryObservation> {
        let mut tasks = JoinSet::new();
        for (index, package) in packages.iter().enumerate() {
            let client = self.clone();
            let package = (*package).clone();
            tasks.spawn(async move { (index, client.observe_package(&package).await) });
        }

        let mut observations = vec![None; packages.len()];
        while let Some(result) = tasks.join_next().await {
            let (index, observation) = result.expect("observation task panicked");
            observations[index] = Some(observation);
        }

        observations
            .into_iter()
            .map(|observation| observation.expect("all observation slots filled"))
            .collect()
    }

    /// Observes one package using the crates.io crate endpoint.
    pub async fn observe_package(&self, package: &LocalPackage) -> RegistryObservation {
        let url = format!(
            "{}/crates/{}",
            self.base_url.trim_end_matches('/'),
            package.name
        );
        let response = match self.client.get(url).send().await {
            Ok(response) => response,
            Err(_) => {
                return RegistryObservation {
                    package: package.name.clone(),
                    status: RegistryStatus::TemporaryError,
                    latest_version: None,
                    exact_version_present: false,
                };
            }
        };

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return RegistryObservation {
                package: package.name.clone(),
                status: RegistryStatus::Absent,
                latest_version: None,
                exact_version_present: false,
            };
        }

        if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            return RegistryObservation {
                package: package.name.clone(),
                status: RegistryStatus::AuthError,
                latest_version: None,
                exact_version_present: false,
            };
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return RegistryObservation {
                package: package.name.clone(),
                status: RegistryStatus::RateLimited,
                latest_version: None,
                exact_version_present: false,
            };
        }

        if status.is_server_error() {
            return RegistryObservation {
                package: package.name.clone(),
                status: RegistryStatus::TemporaryError,
                latest_version: None,
                exact_version_present: false,
            };
        }

        if !status.is_success() {
            return RegistryObservation {
                package: package.name.clone(),
                status: RegistryStatus::InvalidResponse,
                latest_version: None,
                exact_version_present: false,
            };
        }

        let payload = match response.json::<CratesIoCrateResponse>().await {
            Ok(payload) => payload,
            Err(_) => {
                return RegistryObservation {
                    package: package.name.clone(),
                    status: RegistryStatus::InvalidResponse,
                    latest_version: None,
                    exact_version_present: false,
                };
            }
        };

        let mut latest_version: Option<Version> = None;
        let mut exact_version_present = false;

        for version in payload.versions {
            let parsed = match Version::parse(&version.num) {
                Ok(parsed) => parsed,
                Err(_) => {
                    return RegistryObservation {
                        package: package.name.clone(),
                        status: RegistryStatus::InvalidResponse,
                        latest_version: None,
                        exact_version_present: false,
                    };
                }
            };
            if parsed == package.version {
                exact_version_present = true;
            }
            let replace_latest = latest_version
                .as_ref()
                .map(|current| &parsed > current)
                .unwrap_or(true);
            if replace_latest {
                latest_version = Some(parsed);
            }
        }

        RegistryObservation {
            package: package.name.clone(),
            status: RegistryStatus::Present,
            latest_version,
            exact_version_present,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CratesIoCrateResponse {
    versions: Vec<CratesIoVersion>,
}

#[derive(Debug, Deserialize)]
struct CratesIoVersion {
    num: String,
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use semver::Version;

    use super::CratesIoClient;
    use crate::model::{LocalPackage, RegistryStatus};

    #[tokio::test]
    async fn observes_present_and_exact_version() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).expect("read");
            let body = r#"{"versions":[{"num":"0.1.0"},{"num":"0.2.0"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write");
        });

        let client = CratesIoClient::with_base_url(format!("http://{addr}")).expect("client");
        let package = LocalPackage {
            name: "mfm-machine".to_string(),
            version: Version::parse("0.2.0").expect("version"),
            manifest_path: "crates/machine/Cargo.toml".into(),
            workspace_path: "crates/machine".into(),
            has_docs_target: true,
            readme: Some("README.md".into()),
            repository: Some("https://github.com/willyrgf/mfm".into()),
            license: Some("MIT".into()),
            publish: None,
            local_dependencies: Vec::new(),
        };

        let observation = client.observe_package(&package).await;
        server.join().expect("server");

        assert!(matches!(observation.status, RegistryStatus::Present));
        assert!(observation.exact_version_present);
        assert_eq!(
            observation.latest_version.expect("latest"),
            Version::parse("0.2.0").expect("version")
        );
    }

    #[tokio::test]
    async fn observes_absent_from_404() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).expect("read");
            let body = "{}";
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write");
        });

        let client = CratesIoClient::with_base_url(format!("http://{addr}")).expect("client");
        let package = LocalPackage {
            name: "mfm-machine".to_string(),
            version: Version::parse("0.1.0").expect("version"),
            manifest_path: "crates/machine/Cargo.toml".into(),
            workspace_path: "crates/machine".into(),
            has_docs_target: true,
            readme: Some("README.md".into()),
            repository: Some("https://github.com/willyrgf/mfm".into()),
            license: Some("MIT".into()),
            publish: None,
            local_dependencies: Vec::new(),
        };

        let observation = client.observe_package(&package).await;
        server.join().expect("server");

        assert!(matches!(observation.status, RegistryStatus::Absent));
        assert!(!observation.exact_version_present);
        assert!(observation.latest_version.is_none());
    }
}
