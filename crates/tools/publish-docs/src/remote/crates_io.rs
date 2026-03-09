use semver::Version;
use serde::Deserialize;

use crate::model::{
    LocalPackage, RegistryFreshness, RegistryObservation, RegistryObservationSource, RegistryStatus,
};
use crate::remote::http::{HttpExecutor, HttpExecutorConfig};
use crate::remote::observer::{observe_many_ordered, RegistryObserver};

const DEFAULT_CRATES_IO_API: &str = "https://crates.io/api/v1";

/// crates.io HTTP API observer used for fallback and diagnostics.
#[derive(Debug, Clone)]
pub struct CratesIoClient {
    http: HttpExecutor,
    base_url: String,
}

impl CratesIoClient {
    /// Builds the default crates.io API observer.
    pub fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: HttpExecutor::new(HttpExecutorConfig::default())?,
            base_url: DEFAULT_CRATES_IO_API.to_string(),
        })
    }

    /// Builds an observer with an explicit executor.
    pub fn with_http(http: HttpExecutor, base_url: impl Into<String>) -> Self {
        Self {
            http,
            base_url: base_url.into(),
        }
    }

    /// Builds an observer targeting a custom base URL. Intended for tests.
    #[cfg(test)]
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: HttpExecutor::new(HttpExecutorConfig::default())?,
            base_url: base_url.into(),
        })
    }

    /// Observes one package using the crates.io crate endpoint.
    pub async fn observe_package(&self, package: &LocalPackage) -> RegistryObservation {
        let url = format!(
            "{}/crates/{}",
            self.base_url.trim_end_matches('/'),
            package.name
        );
        let host = self.http.host_for_url(&url);
        let result = self
            .http
            .execute(&host, || self.http.client().get(url.clone()))
            .await;

        let observed_at = Some(self.http.now_rfc3339());
        let base = RegistryObservation {
            package: package.name.clone(),
            status: RegistryStatus::TemporaryError,
            latest_version: None,
            exact_version_present: false,
            source: RegistryObservationSource::Api,
            freshness: RegistryFreshness::Unavailable,
            observed_at,
            diagnostic_code: None,
        };

        let Some(result) = result else {
            return RegistryObservation {
                diagnostic_code: Some("registry-api-transport-error".to_string()),
                ..base
            };
        };

        let status = result.response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return RegistryObservation {
                status: RegistryStatus::Absent,
                freshness: RegistryFreshness::Fresh,
                diagnostic_code: None,
                ..base
            };
        }
        if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            return RegistryObservation {
                status: RegistryStatus::AuthError,
                diagnostic_code: Some("registry-api-auth-error".to_string()),
                ..base
            };
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return RegistryObservation {
                status: RegistryStatus::RateLimited,
                diagnostic_code: Some("registry-api-rate-limited".to_string()),
                ..base
            };
        }
        if status.is_server_error() {
            return RegistryObservation {
                status: RegistryStatus::TemporaryError,
                diagnostic_code: Some("registry-api-server-error".to_string()),
                ..base
            };
        }
        if !status.is_success() {
            return RegistryObservation {
                status: RegistryStatus::InvalidResponse,
                diagnostic_code: Some("registry-api-http-status".to_string()),
                ..base
            };
        }

        let payload = match result.response.json::<CratesIoCrateResponse>().await {
            Ok(payload) => payload,
            Err(_) => {
                return RegistryObservation {
                    status: RegistryStatus::InvalidResponse,
                    diagnostic_code: Some("registry-api-json-invalid".to_string()),
                    ..base
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
                        status: RegistryStatus::InvalidResponse,
                        diagnostic_code: Some("registry-api-version-invalid".to_string()),
                        ..base
                    };
                }
            };
            if parsed == package.version {
                exact_version_present = true;
            }
            if latest_version
                .as_ref()
                .map(|current| parsed > *current)
                .unwrap_or(true)
            {
                latest_version = Some(parsed);
            }
        }

        RegistryObservation {
            status: RegistryStatus::Present,
            latest_version,
            exact_version_present,
            freshness: RegistryFreshness::Fresh,
            ..base
        }
    }

    /// Observes many packages while preserving input order.
    pub async fn observe_packages(&self, packages: &[&LocalPackage]) -> Vec<RegistryObservation> {
        let inputs = packages
            .iter()
            .map(|package| (*package).clone())
            .collect::<Vec<_>>();
        observe_many_ordered(inputs, self.http.max_in_flight(), {
            let client = self.clone();
            move |package| {
                let client = client.clone();
                async move { client.observe_package(&package).await }
            }
        })
        .await
    }
}

impl RegistryObserver for CratesIoClient {
    fn observe_packages<'a>(
        &'a self,
        packages: &'a [&'a LocalPackage],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<RegistryObservation>> + Send + 'a>>
    {
        Box::pin(async move { self.observe_packages(packages).await })
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
    use crate::model::{LocalPackage, RegistryFreshness, RegistryStatus};

    fn local_package(version: &str) -> LocalPackage {
        LocalPackage {
            name: "mfm-machine".to_string(),
            version: Version::parse(version).expect("version"),
            manifest_path: "crates/machine/Cargo.toml".into(),
            workspace_path: "crates/machine".into(),
            has_docs_target: true,
            readme: Some("README.md".into()),
            repository: Some("https://github.com/willyrgf/mfm".into()),
            license: Some("MIT".into()),
            publish: None,
            local_dependencies: Vec::new(),
        }
    }

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
        let observation = client.observe_package(&local_package("0.2.0")).await;
        server.join().expect("server");

        assert!(matches!(observation.status, RegistryStatus::Present));
        assert_eq!(observation.freshness, RegistryFreshness::Fresh);
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
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
            )
            .expect("write");
        });

        let client = CratesIoClient::with_base_url(format!("http://{addr}")).expect("client");
        let observation = client.observe_package(&local_package("0.1.0")).await;
        server.join().expect("server");

        assert!(matches!(observation.status, RegistryStatus::Absent));
        assert_eq!(observation.freshness, RegistryFreshness::Fresh);
    }
}
