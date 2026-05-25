use crate::remote::http::{HttpExecutor, HttpExecutorConfig};
use crate::remote::observer::observe_many_ordered;
use crate::{
    error::PublishDocsError,
    model::{
        CatalogPackage, DocsPolicy, DocsRsObservation, DocsRsStatus, LocalPackage,
        RegistryFreshness, RegistryObservation, RegistryStatus,
    },
};

const DEFAULT_DOCS_RS_BASE: &str = "https://docs.rs";

/// docs.rs observer for versioned page availability.
#[derive(Debug, Clone)]
pub(crate) struct DocsRsClient {
    http: HttpExecutor,
    base_url: String,
}

impl DocsRsClient {
    /// Builds the default docs.rs observer.
    pub(crate) fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: HttpExecutor::new(HttpExecutorConfig::default())?,
            base_url: DEFAULT_DOCS_RS_BASE.to_string(),
        })
    }

    /// Builds an observer targeting a custom base URL. Intended for tests.
    #[cfg(test)]
    pub(crate) fn with_base_url(base_url: impl Into<String>) -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: HttpExecutor::new_without_proxy(HttpExecutorConfig::default())?,
            base_url: base_url.into(),
        })
    }

    /// Observes docs.rs for one package using the crate version landing page.
    pub(crate) async fn observe_package(
        &self,
        local: &LocalPackage,
        catalog: &CatalogPackage,
        registry: &RegistryObservation,
    ) -> Result<DocsRsObservation, PublishDocsError> {
        tracing::debug!(
            target: "mfm_publish_docs",
            package = %local.name,
            local_version = %local.version,
            registry_status = ?registry.status,
            registry_freshness = ?registry.freshness,
            "observing docs.rs package"
        );
        if !matches!(catalog.docs_policy, DocsPolicy::DocsRs) {
            return Ok(finish_docs_observation(DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::NotExpected,
                latest_available_version: None,
                exact_version_available: false,
            }));
        }

        match registry.status {
            RegistryStatus::AuthError | RegistryStatus::InvalidResponse => {
                return Ok(finish_docs_observation(DocsRsObservation {
                    package: local.name.clone(),
                    status: DocsRsStatus::TemporaryError,
                    latest_available_version: None,
                    exact_version_available: false,
                }));
            }
            RegistryStatus::TemporaryError | RegistryStatus::RateLimited => {
                return Ok(finish_docs_observation(DocsRsObservation {
                    package: local.name.clone(),
                    status: DocsRsStatus::TemporaryError,
                    latest_available_version: None,
                    exact_version_available: false,
                }));
            }
            RegistryStatus::Absent | RegistryStatus::Present => {}
        }

        if matches!(
            registry.freshness,
            RegistryFreshness::Cached | RegistryFreshness::Unavailable
        ) {
            return Ok(finish_docs_observation(DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::TemporaryError,
                latest_available_version: None,
                exact_version_available: false,
            }));
        }

        if !registry.exact_version_visible_for_planning() {
            return Ok(finish_docs_observation(DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::Absent,
                latest_available_version: None,
                exact_version_available: false,
            }));
        }

        let url = format!(
            "{}/crate/{}/{}",
            self.base_url.trim_end_matches('/'),
            local.name,
            local.version
        );
        let host = self.http.host_for_url(&url);
        let Some(result) = self
            .http
            .execute(&host, || self.http.client().get(url.clone()))
            .await?
        else {
            return Ok(finish_docs_observation(DocsRsObservation {
                package: local.name.clone(),
                status: DocsRsStatus::TemporaryError,
                latest_available_version: None,
                exact_version_available: false,
            }));
        };

        let status = result.response.status();
        if let Some(status) = docs_status_from_response_status(status) {
            return Ok(finish_docs_observation(DocsRsObservation {
                package: local.name.clone(),
                status,
                latest_available_version: None,
                exact_version_available: false,
            }));
        }

        let body = match result.response.text().await {
            Ok(body) => body,
            Err(_) => {
                return Ok(finish_docs_observation(DocsRsObservation {
                    package: local.name.clone(),
                    status: DocsRsStatus::TemporaryError,
                    latest_available_version: None,
                    exact_version_available: false,
                }));
            }
        };

        let docs_status = docs_status_from_success_body(&body);

        Ok(finish_docs_observation(DocsRsObservation {
            package: local.name.clone(),
            status: docs_status,
            latest_available_version: Some(local.version.clone()),
            exact_version_available: matches!(docs_status, DocsRsStatus::Available),
        }))
    }

    /// Observes docs.rs for many packages while preserving input order.
    pub(crate) async fn observe_packages(
        &self,
        inputs: &[(&LocalPackage, &CatalogPackage, &RegistryObservation)],
    ) -> Result<Vec<DocsRsObservation>, PublishDocsError> {
        let inputs = inputs
            .iter()
            .map(|(local, catalog, registry)| {
                ((*local).clone(), (*catalog).clone(), (*registry).clone())
            })
            .collect::<Vec<_>>();
        observe_many_ordered(inputs, self.http.max_in_flight(), {
            let client = self.clone();
            move |(local, catalog, registry)| {
                let client = client.clone();
                async move { client.observe_package(&local, &catalog, &registry).await }
            }
        })
        .await
    }
}

fn docs_status_from_response_status(status: reqwest::StatusCode) -> Option<DocsRsStatus> {
    if status.is_success() {
        None
    } else if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        Some(DocsRsStatus::TemporaryError)
    } else {
        Some(DocsRsStatus::Pending)
    }
}

fn docs_status_from_success_body(body: &str) -> DocsRsStatus {
    let body_lower = body.to_ascii_lowercase();
    if body_lower.contains("failed to build")
        || body_lower.contains("failed to compile")
        || body_lower.contains("docs.rs failed")
    {
        DocsRsStatus::Failed
    } else {
        DocsRsStatus::Available
    }
}

fn finish_docs_observation(observation: DocsRsObservation) -> DocsRsObservation {
    tracing::debug!(
        target: "mfm_publish_docs",
        package = %observation.package,
        status = ?observation.status,
        latest_available_version = ?observation.latest_available_version,
        exact_version_available = observation.exact_version_available,
        "completed docs.rs package observation"
    );
    observation
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{docs_status_from_response_status, docs_status_from_success_body, DocsRsClient};
    use crate::model::{
        CatalogPackage, CatalogSection, DocsPolicy, DocsRsStatus, LocalPackage, RegistryFreshness,
        RegistryObservation, RegistryObservationSource, RegistryStatus, UmbrellaPolicy, Visibility,
    };

    fn local_package() -> LocalPackage {
        LocalPackage {
            name: "mfm-runtime".into(),
            version: Version::parse("0.1.0").expect("version"),
            manifest_path: "crates/kernel/runtime/Cargo.toml".into(),
            workspace_path: "crates/kernel/runtime".into(),
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
            name: "mfm-runtime".into(),
            workspace_path: "crates/kernel/runtime".into(),
            visibility: Visibility::Public,
            section: CatalogSection::Core,
            summary: "typed runtime".into(),
            docs_policy: DocsPolicy::DocsRs,
            umbrella_policy: UmbrellaPolicy::WhenPublished,
            release_priority: 100,
            allow_yank: false,
            owners: Vec::new(),
            notes: String::new(),
        }
    }

    fn fresh_present_registry() -> RegistryObservation {
        RegistryObservation {
            package: "mfm-runtime".into(),
            status: RegistryStatus::Present,
            latest_version: Some(Version::parse("0.1.0").expect("version")),
            exact_version_present: true,
            source: RegistryObservationSource::Index,
            freshness: RegistryFreshness::Fresh,
            observed_at: None,
            diagnostic_code: None,
        }
    }

    #[test]
    fn pending_when_docs_page_missing() {
        assert!(matches!(
            docs_status_from_response_status(reqwest::StatusCode::NOT_FOUND),
            Some(DocsRsStatus::Pending)
        ));
        assert!(matches!(
            docs_status_from_response_status(reqwest::StatusCode::FOUND),
            Some(DocsRsStatus::Pending)
        ));
        assert!(matches!(
            docs_status_from_response_status(reqwest::StatusCode::TOO_MANY_REQUESTS),
            Some(DocsRsStatus::TemporaryError)
        ));
        assert!(docs_status_from_response_status(reqwest::StatusCode::OK).is_none());
    }

    #[tokio::test]
    async fn treats_cached_registry_state_as_temporary_error() {
        let client = DocsRsClient::with_base_url("http://127.0.0.1:1").expect("client");
        let mut registry = fresh_present_registry();
        registry.freshness = RegistryFreshness::Cached;
        let observation = client
            .observe_package(&local_package(), &catalog_package(), &registry)
            .await
            .expect("observe package");
        assert!(matches!(observation.status, DocsRsStatus::TemporaryError));
    }

    #[test]
    fn available_when_versioned_page_exists() {
        assert!(matches!(
            docs_status_from_success_body("<html><body>docs ready</body></html>"),
            DocsRsStatus::Available
        ));
        assert!(matches!(
            docs_status_from_success_body("<html><body>docs.rs failed to build</body></html>"),
            DocsRsStatus::Failed
        ));
    }
}
