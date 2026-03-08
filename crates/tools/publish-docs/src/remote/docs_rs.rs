use std::time::Duration;

use crate::model::{
    CatalogPackage, DocsPolicy, DocsRsObservation, DocsRsStatus, LocalPackage, RegistryObservation,
};
use tokio::task::JoinSet;

const DEFAULT_DOCS_RS_BASE: &str = "https://docs.rs";

/// Simple docs.rs observer for versioned page availability.
#[derive(Debug, Clone)]
pub struct DocsRsClient {
    client: reqwest::Client,
    base_url: String,
}

impl DocsRsClient {
    /// Builds the default docs.rs observer.
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
            base_url: DEFAULT_DOCS_RS_BASE.to_string(),
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

    /// Observes docs.rs for one package using the crate version landing page.
    pub async fn observe_package(
        &self,
        local: &LocalPackage,
        catalog: &CatalogPackage,
        registry: &RegistryObservation,
    ) -> DocsRsObservation {
        if !matches!(catalog.docs_policy, DocsPolicy::DocsRs) {
            return DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::NotExpected,
                latest_available_version: None,
                exact_version_available: false,
            };
        }

        match registry.status {
            crate::model::RegistryStatus::TemporaryError
            | crate::model::RegistryStatus::RateLimited
            | crate::model::RegistryStatus::AuthError
            | crate::model::RegistryStatus::InvalidResponse => {
                return DocsRsObservation {
                    package: local.name.clone(),
                    status: DocsRsStatus::TemporaryError,
                    latest_available_version: None,
                    exact_version_available: false,
                };
            }
            crate::model::RegistryStatus::Absent | crate::model::RegistryStatus::Present => {}
        }

        if !registry.exact_version_present {
            return DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::Absent,
                latest_available_version: None,
                exact_version_available: false,
            };
        }

        let url = format!(
            "{}/crate/{}/{}",
            self.base_url.trim_end_matches('/'),
            local.name,
            local.version
        );
        let response = match self.client.get(url).send().await {
            Ok(response) => response,
            Err(_) => {
                return DocsRsObservation {
                    package: local.name.clone(),
                    status: DocsRsStatus::TemporaryError,
                    latest_available_version: None,
                    exact_version_available: false,
                };
            }
        };

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::Pending,
                latest_available_version: None,
                exact_version_available: false,
            };
        }
        if status.is_server_error() {
            return DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::TemporaryError,
                latest_available_version: None,
                exact_version_available: false,
            };
        }
        if !status.is_success() {
            return DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::Pending,
                latest_available_version: None,
                exact_version_available: false,
            };
        }

        let body = match response.text().await {
            Ok(body) => body,
            Err(_) => {
                return DocsRsObservation {
                    package: local.name.clone(),
                    status: DocsRsStatus::TemporaryError,
                    latest_available_version: None,
                    exact_version_available: false,
                };
            }
        };

        let body_lower = body.to_ascii_lowercase();
        let docs_status = if body_lower.contains("failed to build")
            || body_lower.contains("failed to compile")
            || body_lower.contains("docs.rs failed")
        {
            DocsRsStatus::Failed
        } else {
            DocsRsStatus::Available
        };

        DocsRsObservation {
            package: local.name.clone(),
            status: docs_status,
            latest_available_version: Some(local.version.clone()),
            exact_version_available: matches!(docs_status, DocsRsStatus::Available),
        }
    }

    /// Observes docs.rs for many packages in parallel while preserving input order.
    pub async fn observe_packages(
        &self,
        inputs: &[(&LocalPackage, &CatalogPackage, &RegistryObservation)],
    ) -> Vec<DocsRsObservation> {
        let mut tasks = JoinSet::new();
        for (index, (local, catalog, registry)) in inputs.iter().enumerate() {
            let client = self.clone();
            let local = (*local).clone();
            let catalog = (*catalog).clone();
            let registry = (*registry).clone();
            tasks.spawn(async move {
                (
                    index,
                    client.observe_package(&local, &catalog, &registry).await,
                )
            });
        }

        let mut observations = vec![None; inputs.len()];
        while let Some(result) = tasks.join_next().await {
            let (index, observation) = result.expect("observation task panicked");
            observations[index] = Some(observation);
        }

        observations
            .into_iter()
            .map(|observation| observation.expect("all observation slots filled"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use semver::Version;

    use super::DocsRsClient;
    use crate::model::{
        CatalogPackage, CatalogSection, DocsPolicy, DocsRsStatus, LocalPackage,
        RegistryObservation, RegistryStatus, UmbrellaPolicy, Visibility,
    };

    fn local_package() -> LocalPackage {
        LocalPackage {
            name: "mfm-machine".into(),
            version: Version::parse("0.1.0").expect("version"),
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

    fn catalog_package() -> CatalogPackage {
        CatalogPackage {
            name: "mfm-machine".into(),
            workspace_path: "crates/machine".into(),
            visibility: Visibility::Public,
            section: CatalogSection::EngineSdk,
            summary: "runtime".into(),
            docs_policy: DocsPolicy::DocsRs,
            umbrella_policy: UmbrellaPolicy::WhenPublished,
            release_priority: 100,
            allow_yank: false,
            owners: Vec::new(),
            notes: String::new(),
        }
    }

    #[tokio::test]
    async fn pending_when_registry_present_but_docs_page_missing() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).expect("read");
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n\r\n"
            )
            .expect("write");
        });

        let client = DocsRsClient::with_base_url(format!("http://{addr}")).expect("client");
        let observation = client
            .observe_package(
                &local_package(),
                &catalog_package(),
                &RegistryObservation {
                    package: "mfm-machine".into(),
                    status: RegistryStatus::Present,
                    latest_version: Some(Version::parse("0.1.0").expect("version")),
                    exact_version_present: true,
                },
            )
            .await;
        server.join().expect("server");

        assert!(matches!(observation.status, DocsRsStatus::Pending));
    }

    #[tokio::test]
    async fn available_when_versioned_page_exists() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).expect("read");
            let body = "<html><body>docs ready</body></html>";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write");
        });

        let client = DocsRsClient::with_base_url(format!("http://{addr}")).expect("client");
        let observation = client
            .observe_package(
                &local_package(),
                &catalog_package(),
                &RegistryObservation {
                    package: "mfm-machine".into(),
                    status: RegistryStatus::Present,
                    latest_version: Some(Version::parse("0.1.0").expect("version")),
                    exact_version_present: true,
                },
            )
            .await;
        server.join().expect("server");

        assert!(matches!(observation.status, DocsRsStatus::Available));
        assert!(observation.exact_version_available);
    }
}
