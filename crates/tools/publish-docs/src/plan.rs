use std::collections::BTreeMap;

use semver::Version;

use crate::{
    catalog::catalog_entry,
    error::PublishDocsError,
    model::{
        DesiredCatalog, DocsPolicy, DocsRsObservation, DocsRsStatus, Mode, Plan, PlanAction,
        PlannedPackage, RegistryFreshness, RegistryObservation, RegistryObservationSource,
        RegistryStatus, ReleaseLedger, SummaryCounts, WorkspaceState,
    },
    workspace::selected_packages,
};

/// Builds the planner output for the current run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_plan(
    run_id: String,
    wave_name: String,
    mode: Mode,
    selection_from: Option<String>,
    selection_only: Option<String>,
    resumed_from_run_id: Option<String>,
    workspace: &WorkspaceState,
    catalog: &DesiredCatalog,
    _ledger: &ReleaseLedger,
    registry_observations: &[RegistryObservation],
    docs_observations: &[DocsRsObservation],
    changed_since_release: &BTreeMap<String, bool>,
    umbrella_synced: bool,
) -> Result<Plan, PublishDocsError> {
    let registry_by_name: BTreeMap<_, _> = registry_observations
        .iter()
        .map(|observation| (observation.package.as_str(), observation))
        .collect();
    let docs_by_name: BTreeMap<_, _> = docs_observations
        .iter()
        .map(|observation| (observation.package.as_str(), observation))
        .collect();

    let packages = selected_packages(workspace)?
        .into_iter()
        .map(|package| {
            let remote = registry_by_name.get(package.name.as_str()).copied();
            let docs = docs_by_name.get(package.name.as_str()).copied();
            let changed = changed_since_release
                .get(package.name.as_str())
                .copied()
                .unwrap_or(false);

            let Some(catalog_entry) = catalog_entry(catalog, &package.name) else {
                return PlannedPackage {
                    name: package.name.clone(),
                    local_version: package.version.clone(),
                    remote_version: remote
                        .and_then(|observation| observation.latest_version.clone()),
                    docs_status: docs.map(|observation| observation.status),
                    action: PlanAction::ManualReview,
                    reason: "missing-catalog-entry".to_string(),
                    blocking_dependencies: Vec::new(),
                    synthetic: false,
                };
            };

            let Some(remote) = remote else {
                return PlannedPackage {
                    name: package.name.clone(),
                    local_version: package.version.clone(),
                    remote_version: None,
                    docs_status: docs.map(|observation| observation.status),
                    action: PlanAction::ManualReview,
                    reason: "missing-registry-observation".to_string(),
                    blocking_dependencies: Vec::new(),
                    synthetic: false,
                };
            };

            let blocking_dependencies: Vec<String> = package
                .local_dependencies
                .iter()
                .filter(|dependency| {
                    registry_by_name
                        .get(dependency.as_str())
                        .map(|observation| !observation.exact_version_visible_for_planning())
                        .unwrap_or(true)
                })
                .cloned()
                .collect();

            let docs_status = docs.map(|observation| observation.status);
            let (action, reason) = classify_action(
                package.name.as_str(),
                &package.version,
                catalog.umbrella_package.as_str(),
                matches!(catalog_entry.docs_policy, DocsPolicy::DocsRs),
                remote,
                docs_status,
                changed,
                !blocking_dependencies.is_empty(),
                umbrella_synced,
            );

            PlannedPackage {
                name: package.name.clone(),
                local_version: package.version.clone(),
                remote_version: remote.latest_version.clone(),
                docs_status,
                action,
                reason,
                blocking_dependencies,
                synthetic: false,
            }
        })
        .collect();

    Ok(Plan {
        schema_version: 1,
        run_id,
        mode,
        wave: wave_name,
        selection_from,
        selection_only,
        resumed_from_run_id,
        packages,
    })
}

#[allow(clippy::too_many_arguments)]
fn classify_action(
    package_name: &str,
    local_version: &Version,
    umbrella_package: &str,
    expects_docs_rs: bool,
    remote: &RegistryObservation,
    docs_status: Option<DocsRsStatus>,
    changed_since_release: bool,
    has_blocking_dependencies: bool,
    umbrella_synced: bool,
) -> (PlanAction, String) {
    if package_name == umbrella_package && !umbrella_synced {
        return (
            PlanAction::RefreshUmbrella,
            "umbrella-sync-required".to_string(),
        );
    }

    if !matches!(remote.source, RegistryObservationSource::Index) {
        return match remote.status {
            RegistryStatus::AuthError => {
                (PlanAction::ManualReview, "registry-auth-error".to_string())
            }
            RegistryStatus::InvalidResponse => (
                PlanAction::ManualReview,
                "registry-invalid-response".to_string(),
            ),
            RegistryStatus::RateLimited => (
                PlanAction::WaitRegistry,
                "registry-rate-limited".to_string(),
            ),
            RegistryStatus::TemporaryError | RegistryStatus::Absent | RegistryStatus::Present => (
                PlanAction::WaitRegistry,
                "registry-refresh-failed".to_string(),
            ),
        };
    }

    match remote.freshness {
        RegistryFreshness::Cached => {
            return (PlanAction::WaitRegistry, "registry-cached-only".to_string());
        }
        RegistryFreshness::Unavailable => match remote.status {
            RegistryStatus::RateLimited => {
                return (
                    PlanAction::WaitRegistry,
                    "registry-rate-limited".to_string(),
                );
            }
            RegistryStatus::TemporaryError | RegistryStatus::Absent | RegistryStatus::Present => {
                return (
                    PlanAction::WaitRegistry,
                    "registry-refresh-failed".to_string(),
                );
            }
            RegistryStatus::AuthError => {
                return (PlanAction::ManualReview, "registry-auth-error".to_string());
            }
            RegistryStatus::InvalidResponse => {
                return (
                    PlanAction::ManualReview,
                    "registry-invalid-response".to_string(),
                );
            }
        },
        RegistryFreshness::Fresh => {}
    }

    match remote.status {
        RegistryStatus::RateLimited => {
            return (
                PlanAction::WaitRegistry,
                "registry-rate-limited".to_string(),
            );
        }
        RegistryStatus::TemporaryError => {
            return (
                PlanAction::WaitRegistry,
                "registry-refresh-failed".to_string(),
            );
        }
        RegistryStatus::AuthError => {
            return (PlanAction::ManualReview, "registry-auth-error".to_string());
        }
        RegistryStatus::InvalidResponse => {
            return (
                PlanAction::ManualReview,
                "registry-invalid-response".to_string(),
            );
        }
        RegistryStatus::Absent | RegistryStatus::Present => {}
    }

    if matches!(remote.status, RegistryStatus::Present) && remote.latest_version.is_none() {
        return (
            PlanAction::ManualReview,
            "registry-missing-version-data".to_string(),
        );
    }

    if let Some(remote_version) = &remote.latest_version {
        if !remote.exact_version_present && remote_version > local_version {
            return (
                PlanAction::ManualReview,
                "remote-newer-than-local".to_string(),
            );
        }

        if !remote.exact_version_present && remote_version == local_version {
            return (
                PlanAction::ManualReview,
                "contradictory-registry-state".to_string(),
            );
        }
    }

    if remote.exact_version_visible_for_planning() {
        if changed_since_release {
            return (
                PlanAction::NeedsVersionBump,
                "changed-since-last-release".to_string(),
            );
        }

        if expects_docs_rs {
            match docs_status {
                Some(DocsRsStatus::Available) | Some(DocsRsStatus::NotExpected) => {
                    return (PlanAction::Noop, "exact-version-present".to_string());
                }
                Some(DocsRsStatus::Pending) | Some(DocsRsStatus::Absent) => {
                    return (PlanAction::WaitDocsRs, "docs-rs-pending".to_string());
                }
                Some(DocsRsStatus::TemporaryError) => {
                    return (
                        PlanAction::ManualReview,
                        "docs-rs-temporary-error".to_string(),
                    );
                }
                Some(DocsRsStatus::Failed) => {
                    return (PlanAction::ManualReview, "docs-rs-failed".to_string());
                }
                None => {
                    return (
                        PlanAction::ManualReview,
                        "missing-docs-observation".to_string(),
                    );
                }
            }
        }

        return (PlanAction::Noop, "exact-version-present".to_string());
    }

    if has_blocking_dependencies {
        return (
            PlanAction::WaitDependencies,
            "waiting-on-selected-dependencies".to_string(),
        );
    }

    (PlanAction::Publish, "target-version-absent".to_string())
}

/// Converts planner output into aggregate summary counts.
pub(crate) fn summarize_plan(plan: &Plan) -> SummaryCounts {
    let mut summary = SummaryCounts::default();
    for package in &plan.packages {
        match package.action {
            PlanAction::Noop => summary.noop += 1,
            PlanAction::Publish => summary.publish += 1,
            PlanAction::WaitDependencies => summary.wait_dependencies += 1,
            PlanAction::WaitRegistry => summary.wait_registry += 1,
            PlanAction::WaitDocsRs => summary.wait_docs_rs += 1,
            PlanAction::NeedsVersionBump => summary.needs_version_bump += 1,
            PlanAction::RefreshUmbrella => summary.refresh_umbrella += 1,
            PlanAction::Yank => summary.yank += 1,
            PlanAction::ManualReview => summary.manual_review += 1,
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semver::Version;

    use super::{build_plan, summarize_plan};
    use crate::model::{
        CatalogPackage, CatalogSection, DesiredCatalog, DocsPolicy, DocsRsObservation,
        DocsRsStatus, LocalPackage, Mode, PlanAction, RegistryFreshness, RegistryObservation,
        RegistryObservationSource, RegistryStatus, ReleaseLedger, UmbrellaPolicy, Visibility,
        WorkspaceState,
    };

    fn registry_observation(
        package: &str,
        status: RegistryStatus,
        latest_version: Option<Version>,
        exact_version_present: bool,
    ) -> RegistryObservation {
        RegistryObservation {
            package: package.into(),
            status,
            latest_version,
            exact_version_present,
            source: RegistryObservationSource::Index,
            freshness: RegistryFreshness::Fresh,
            observed_at: Some("2026-03-09T00:00:00Z".into()),
            diagnostic_code: None,
        }
    }

    fn catalog() -> DesiredCatalog {
        DesiredCatalog {
            catalog_version: 1,
            umbrella_package: "mfm-docs".into(),
            packages: vec![
                CatalogPackage {
                    name: "mfm-runtime".into(),
                    workspace_path: "crates/kernel/runtime".into(),
                    visibility: Visibility::Public,
                    section: CatalogSection::EngineSdk,
                    summary: "runtime".into(),
                    docs_policy: DocsPolicy::DocsRs,
                    umbrella_policy: UmbrellaPolicy::WhenPublished,
                    release_priority: 100,
                    allow_yank: false,
                    owners: Vec::new(),
                    notes: String::new(),
                },
                CatalogPackage {
                    name: "mfm-replay".into(),
                    workspace_path: "crates/kernel/replay".into(),
                    visibility: Visibility::Public,
                    section: CatalogSection::EngineSdk,
                    summary: "sdk".into(),
                    docs_policy: DocsPolicy::DocsRs,
                    umbrella_policy: UmbrellaPolicy::WhenPublished,
                    release_priority: 90,
                    allow_yank: false,
                    owners: Vec::new(),
                    notes: String::new(),
                },
                CatalogPackage {
                    name: "mfm-docs".into(),
                    workspace_path: "crates/docs".into(),
                    visibility: Visibility::Public,
                    section: CatalogSection::BinariesTooling,
                    summary: "umbrella".into(),
                    docs_policy: DocsPolicy::DocsRs,
                    umbrella_policy: UmbrellaPolicy::Never,
                    release_priority: 80,
                    allow_yank: false,
                    owners: Vec::new(),
                    notes: String::new(),
                },
            ],
        }
    }

    fn local_package(name: &str, workspace_path: &str, dependencies: Vec<String>) -> LocalPackage {
        LocalPackage {
            name: name.into(),
            version: Version::parse("0.1.0").expect("version"),
            manifest_path: format!("{workspace_path}/Cargo.toml").into(),
            workspace_path: workspace_path.into(),
            has_docs_target: true,
            readme: Some("README.md".into()),
            repository: Some("https://github.com/willyrgf/mfm".into()),
            license: Some("MIT".into()),
            publish: None,
            local_dependencies: dependencies,
        }
    }

    #[test]
    fn classifies_wait_dependencies_when_selected_dependency_is_missing() {
        let workspace = WorkspaceState {
            selected: vec!["mfm-runtime".into(), "mfm-replay".into()],
            packages: vec![
                local_package("mfm-runtime", "crates/kernel/runtime", Vec::new()),
                local_package(
                    "mfm-replay",
                    "crates/kernel/replay",
                    vec!["mfm-runtime".into()],
                ),
            ],
        };
        let observations = vec![
            registry_observation("mfm-runtime", RegistryStatus::Absent, None, false),
            registry_observation("mfm-replay", RegistryStatus::Absent, None, false),
        ];

        let plan = build_plan(
            "run_1".into(),
            "docs-rs-wave-1".into(),
            Mode::Plan,
            None,
            None,
            None,
            &workspace,
            &catalog(),
            &ReleaseLedger {
                schema_version: 1,
                packages: BTreeMap::new(),
            },
            &observations,
            &[],
            &BTreeMap::new(),
            false,
        )
        .expect("plan");

        assert_eq!(plan.packages[0].action, PlanAction::Publish);
        assert_eq!(plan.packages[1].action, PlanAction::WaitDependencies);
        assert_eq!(
            plan.packages[1].blocking_dependencies,
            vec!["mfm-runtime".to_string()]
        );

        let summary = summarize_plan(&plan);
        assert_eq!(summary.publish, 1);
        assert_eq!(summary.wait_dependencies, 1);
    }

    #[test]
    fn classifies_wait_docs_rs_for_published_docs_rs_crate() {
        let workspace = WorkspaceState {
            selected: vec!["mfm-runtime".into()],
            packages: vec![local_package(
                "mfm-runtime",
                "crates/kernel/runtime",
                Vec::new(),
            )],
        };
        let observations = vec![registry_observation(
            "mfm-runtime",
            RegistryStatus::Present,
            Some(Version::parse("0.1.0").expect("version")),
            true,
        )];
        let docs = vec![DocsRsObservation {
            package: "mfm-runtime".into(),
            status: DocsRsStatus::Pending,
            latest_available_version: None,
            exact_version_available: false,
        }];

        let plan = build_plan(
            "run_1".into(),
            "docs-rs-wave-1".into(),
            Mode::Plan,
            None,
            None,
            None,
            &workspace,
            &catalog(),
            &ReleaseLedger {
                schema_version: 1,
                packages: BTreeMap::new(),
            },
            &observations,
            &docs,
            &BTreeMap::new(),
            false,
        )
        .expect("plan");

        assert_eq!(plan.packages[0].action, PlanAction::WaitDocsRs);
    }

    #[test]
    fn classifies_needs_version_bump_when_changed_since_release() {
        let workspace = WorkspaceState {
            selected: vec!["mfm-runtime".into()],
            packages: vec![local_package(
                "mfm-runtime",
                "crates/kernel/runtime",
                Vec::new(),
            )],
        };
        let observations = vec![registry_observation(
            "mfm-runtime",
            RegistryStatus::Present,
            Some(Version::parse("0.1.0").expect("version")),
            true,
        )];
        let docs = vec![DocsRsObservation {
            package: "mfm-runtime".into(),
            status: DocsRsStatus::Available,
            latest_available_version: Some(Version::parse("0.1.0").expect("version")),
            exact_version_available: true,
        }];
        let changed_since_release = BTreeMap::from([("mfm-runtime".to_string(), true)]);

        let plan = build_plan(
            "run_1".into(),
            "docs-rs-wave-1".into(),
            Mode::Plan,
            None,
            None,
            None,
            &workspace,
            &catalog(),
            &ReleaseLedger {
                schema_version: 1,
                packages: BTreeMap::new(),
            },
            &observations,
            &docs,
            &changed_since_release,
            false,
        )
        .expect("plan");

        assert_eq!(plan.packages[0].action, PlanAction::NeedsVersionBump);
    }

    #[test]
    fn classifies_remote_newer_than_local_as_manual_review() {
        let workspace = WorkspaceState {
            selected: vec!["mfm-runtime".into()],
            packages: vec![local_package(
                "mfm-runtime",
                "crates/kernel/runtime",
                Vec::new(),
            )],
        };
        let observations = vec![registry_observation(
            "mfm-runtime",
            RegistryStatus::Present,
            Some(Version::parse("0.2.0").expect("version")),
            false,
        )];

        let plan = build_plan(
            "run_1".into(),
            "docs-rs-wave-1".into(),
            Mode::Plan,
            None,
            None,
            None,
            &workspace,
            &catalog(),
            &ReleaseLedger {
                schema_version: 1,
                packages: BTreeMap::new(),
            },
            &observations,
            &[],
            &BTreeMap::new(),
            false,
        )
        .expect("plan");

        assert_eq!(plan.packages[0].action, PlanAction::ManualReview);
        assert_eq!(plan.packages[0].reason, "remote-newer-than-local");
    }

    #[test]
    fn classifies_umbrella_refresh_when_readme_is_stale() {
        let workspace = WorkspaceState {
            selected: vec!["mfm-docs".into()],
            packages: vec![local_package("mfm-docs", "crates/docs", Vec::new())],
        };
        let observations = vec![registry_observation(
            "mfm-docs",
            RegistryStatus::Absent,
            None,
            false,
        )];

        let plan = build_plan(
            "run_1".into(),
            "docs-rs-wave-1".into(),
            Mode::Plan,
            None,
            Some("mfm-docs".into()),
            None,
            &workspace,
            &catalog(),
            &ReleaseLedger {
                schema_version: 1,
                packages: BTreeMap::new(),
            },
            &observations,
            &[],
            &BTreeMap::new(),
            false,
        )
        .expect("plan");

        assert_eq!(plan.packages[0].action, PlanAction::RefreshUmbrella);
    }

    #[test]
    fn classifies_cached_registry_visibility_as_wait_registry() {
        let workspace = WorkspaceState {
            selected: vec!["mfm-runtime".into()],
            packages: vec![local_package(
                "mfm-runtime",
                "crates/kernel/runtime",
                Vec::new(),
            )],
        };
        let observations = vec![RegistryObservation {
            freshness: RegistryFreshness::Cached,
            ..registry_observation(
                "mfm-runtime",
                RegistryStatus::Present,
                Some(Version::parse("0.1.0").expect("version")),
                true,
            )
        }];

        let plan = build_plan(
            "run_1".into(),
            "docs-rs-wave-1".into(),
            Mode::Plan,
            None,
            None,
            None,
            &workspace,
            &catalog(),
            &ReleaseLedger {
                schema_version: 1,
                packages: BTreeMap::new(),
            },
            &observations,
            &[],
            &BTreeMap::new(),
            false,
        )
        .expect("plan");

        assert_eq!(plan.packages[0].action, PlanAction::WaitRegistry);
        assert_eq!(plan.packages[0].reason, "registry-cached-only");
    }

    #[test]
    fn classifies_unavailable_registry_refresh_as_wait_registry() {
        let workspace = WorkspaceState {
            selected: vec!["mfm-runtime".into()],
            packages: vec![local_package(
                "mfm-runtime",
                "crates/kernel/runtime",
                Vec::new(),
            )],
        };
        let observations = vec![RegistryObservation {
            freshness: RegistryFreshness::Unavailable,
            diagnostic_code: Some("registry-timeout".into()),
            ..registry_observation("mfm-runtime", RegistryStatus::TemporaryError, None, false)
        }];

        let plan = build_plan(
            "run_1".into(),
            "docs-rs-wave-1".into(),
            Mode::Plan,
            None,
            None,
            None,
            &workspace,
            &catalog(),
            &ReleaseLedger {
                schema_version: 1,
                packages: BTreeMap::new(),
            },
            &observations,
            &[],
            &BTreeMap::new(),
            false,
        )
        .expect("plan");

        assert_eq!(plan.packages[0].action, PlanAction::WaitRegistry);
        assert_eq!(plan.packages[0].reason, "registry-refresh-failed");
    }
}
