use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    error::PublishDocsError,
    model::{
        CatalogPackage, CatalogSection, DesiredCatalog, DocsPolicy, PackageFilter, PublishWave,
        UmbrellaPolicy, Visibility, WavePackage, WorkspaceState,
    },
};

/// Relative path to the current publish wave file.
pub(crate) const PUBLISH_WAVE_PATH: &str = "crates/docs/publish-wave.json";
/// Relative path to the desired-state catalog.
pub(crate) const DESIRED_CATALOG_PATH: &str = "crates/docs/catalog.toml";

/// Loads `publish-wave.json` from the workspace root.
pub(crate) fn load_publish_wave(workspace_root: &Path) -> Result<PublishWave, PublishDocsError> {
    let path = publish_wave_path(workspace_root);
    let bytes = fs::read(&path)?;
    let wave = serde_json::from_slice(&bytes)?;
    Ok(wave)
}

/// Returns the absolute path to `publish-wave.json` for the workspace root.
pub(crate) fn publish_wave_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(PUBLISH_WAVE_PATH)
}

/// Loads `catalog.toml` from the workspace root and validates it against the workspace inventory.
pub(crate) fn load_desired_catalog(
    workspace_root: &Path,
    workspace: &WorkspaceState,
) -> Result<DesiredCatalog, PublishDocsError> {
    let path = workspace_root.join(DESIRED_CATALOG_PATH);
    let bytes = fs::read_to_string(path)?;
    let raw: RawCatalog =
        toml::from_str(&bytes).map_err(|error| PublishDocsError::Other(error.into()))?;
    let catalog = DesiredCatalog {
        catalog_version: raw.catalog_version,
        umbrella_package: raw.umbrella_package,
        packages: raw
            .packages
            .into_iter()
            .map(|package| CatalogPackage {
                name: package.name,
                workspace_path: package.workspace_path,
                visibility: package.visibility,
                section: package.section,
                summary: package.summary,
                docs_policy: package.docs_policy.unwrap_or(DocsPolicy::RepoOnly),
                umbrella_policy: package
                    .umbrella_policy
                    .unwrap_or(UmbrellaPolicy::WhenPublished),
                release_priority: package.release_priority.unwrap_or(100),
                allow_yank: package.allow_yank.unwrap_or(false),
                owners: package.owners.unwrap_or_default(),
                notes: package.notes.unwrap_or_default(),
            })
            .collect(),
    };
    validate_catalog(&catalog, workspace)?;
    Ok(catalog)
}

/// Applies `--only` and `--from` selection semantics to the wave.
pub(crate) fn select_packages(
    wave: &PublishWave,
    filter: &PackageFilter,
) -> Result<Vec<WavePackage>, PublishDocsError> {
    let selected = if let Some(only) = &filter.only {
        let package = wave
            .packages
            .iter()
            .find(|package| package.name == *only)
            .cloned()
            .ok_or_else(|| PublishDocsError::UnknownPackage {
                package: only.clone(),
            })?;
        vec![package]
    } else if let Some(from) = &filter.from {
        let index = wave
            .packages
            .iter()
            .position(|package| package.name == *from)
            .ok_or_else(|| PublishDocsError::UnknownPackage {
                package: from.clone(),
            })?;
        wave.packages[index..].to_vec()
    } else {
        wave.packages.clone()
    };

    if selected.is_empty() {
        return Err(PublishDocsError::EmptySelection);
    }

    Ok(selected)
}

/// Returns a catalog entry by package name.
pub(crate) fn catalog_entry<'a>(
    catalog: &'a DesiredCatalog,
    name: &str,
) -> Option<&'a CatalogPackage> {
    catalog.packages.iter().find(|package| package.name == name)
}

fn validate_catalog(
    catalog: &DesiredCatalog,
    workspace: &WorkspaceState,
) -> Result<(), PublishDocsError> {
    if catalog.catalog_version != 1 {
        return Err(PublishDocsError::CommandFailed {
            message: format!("unsupported catalog version: {}", catalog.catalog_version),
        });
    }

    let mut seen_names = std::collections::BTreeSet::new();
    let mut seen_paths = std::collections::BTreeSet::new();
    let workspace_by_name: std::collections::BTreeMap<_, _> = workspace
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    let mut umbrella_found = false;
    for package in &catalog.packages {
        if !seen_names.insert(package.name.clone()) {
            return Err(PublishDocsError::CommandFailed {
                message: format!("duplicate catalog package name: {}", package.name),
            });
        }
        if !seen_paths.insert(package.workspace_path.clone()) {
            return Err(PublishDocsError::CommandFailed {
                message: format!(
                    "duplicate catalog workspace path: {}",
                    package.workspace_path
                ),
            });
        }
        if package.summary.trim().is_empty() {
            return Err(PublishDocsError::CommandFailed {
                message: format!("catalog summary is empty for package={}", package.name),
            });
        }

        let local = workspace_by_name
            .get(package.name.as_str())
            .ok_or_else(|| PublishDocsError::PackageMissingFromMetadata {
                package: package.name.clone(),
            })?;
        if local.workspace_path != Path::new(&package.workspace_path) {
            return Err(PublishDocsError::CommandFailed {
                message: format!(
                    "catalog workspace path mismatch package={} catalog={} metadata={}",
                    package.name,
                    package.workspace_path,
                    local.workspace_path.display()
                ),
            });
        }
        if matches!(package.docs_policy, DocsPolicy::DocsRs) && !local.has_docs_target {
            return Err(PublishDocsError::CommandFailed {
                message: format!(
                    "docs-rs policy requires a docs target package={}",
                    package.name
                ),
            });
        }
        if !matches!(package.umbrella_policy, UmbrellaPolicy::Never)
            && !matches!(package.visibility, Visibility::Public)
        {
            return Err(PublishDocsError::CommandFailed {
                message: format!(
                    "umbrella policy requires public visibility package={}",
                    package.name
                ),
            });
        }
        if package.name == catalog.umbrella_package {
            umbrella_found = true;
        }
    }

    if !umbrella_found {
        return Err(PublishDocsError::CommandFailed {
            message: format!(
                "umbrella package missing from catalog: {}",
                catalog.umbrella_package
            ),
        });
    }

    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct RawCatalog {
    catalog_version: u32,
    umbrella_package: String,
    packages: Vec<RawCatalogPackage>,
}

#[derive(Debug, serde::Deserialize)]
struct RawCatalogPackage {
    name: String,
    workspace_path: String,
    visibility: Visibility,
    section: CatalogSection,
    summary: String,
    docs_policy: Option<DocsPolicy>,
    umbrella_policy: Option<UmbrellaPolicy>,
    release_priority: Option<i64>,
    allow_yank: Option<bool>,
    owners: Option<Vec<String>>,
    notes: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::select_packages;
    use crate::model::{PackageFilter, PublishWave, WavePackage};

    fn wave() -> PublishWave {
        PublishWave {
            wave: "docs-rs-wave-1".into(),
            packages: vec![
                WavePackage {
                    name: "a".into(),
                    group: "foundation".into(),
                    workspace_path: "crates/a".into(),
                    docs_rs: "https://docs.rs/a".into(),
                },
                WavePackage {
                    name: "b".into(),
                    group: "foundation".into(),
                    workspace_path: "crates/b".into(),
                    docs_rs: "https://docs.rs/b".into(),
                },
                WavePackage {
                    name: "c".into(),
                    group: "foundation".into(),
                    workspace_path: "crates/c".into(),
                    docs_rs: "https://docs.rs/c".into(),
                },
            ],
        }
    }

    #[test]
    fn selects_only_package() {
        let selected = select_packages(
            &wave(),
            &PackageFilter {
                from: None,
                only: Some("b".into()),
            },
        )
        .expect("selection");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "b");
    }

    #[test]
    fn selects_from_package() {
        let selected = select_packages(
            &wave(),
            &PackageFilter {
                from: Some("b".into()),
                only: None,
            },
        )
        .expect("selection");
        assert_eq!(
            selected
                .into_iter()
                .map(|package| package.name)
                .collect::<Vec<_>>(),
            vec!["b".to_string(), "c".to_string()]
        );
    }
}
