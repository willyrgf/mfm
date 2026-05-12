use std::{
    fs,
    path::{Path, PathBuf},
};

use mfm_publish_docs_config::{
    canonicalize_desired_catalog_authored_config, canonicalize_publish_wave_authored_config,
    parse_desired_catalog_authored_config_with_hint, parse_publish_wave_authored_config_with_hint,
};

use crate::{
    error::PublishDocsError,
    model::{
        CatalogPackage, DesiredCatalog, DocsPolicy, PackageFilter, PublishWave, UmbrellaPolicy,
        Visibility, WavePackage, WorkspaceState,
    },
};

/// Relative path to the publish wave when authored as JSON.
pub(crate) const PUBLISH_WAVE_JSON_PATH: &str = "crates/docs/publish-wave.json";
/// Alternate relative path to the publish wave when authored as TOML.
pub(crate) const PUBLISH_WAVE_TOML_PATH: &str = "crates/docs/publish-wave.toml";
/// Relative path to the desired-state catalog when authored as TOML.
pub(crate) const DESIRED_CATALOG_TOML_PATH: &str = "crates/docs/catalog.toml";
/// Alternate relative path to the desired-state catalog when authored as JSON.
pub(crate) const DESIRED_CATALOG_JSON_PATH: &str = "crates/docs/catalog.json";

/// Loads the publish wave from the workspace root.
pub(crate) fn load_publish_wave(workspace_root: &Path) -> Result<PublishWave, PublishDocsError> {
    let path = publish_wave_path(workspace_root)?;
    let bytes = fs::read_to_string(&path)?;
    let authored = parse_publish_wave_authored_config_with_hint(&bytes, Some(path.as_path()))?;
    let wave = canonicalize_publish_wave_authored_config(authored)?;
    Ok(wave)
}

/// Returns the absolute path to the publish wave for the workspace root.
pub(crate) fn publish_wave_path(workspace_root: &Path) -> Result<PathBuf, PublishDocsError> {
    let json_path = workspace_root.join(PUBLISH_WAVE_JSON_PATH);
    let toml_path = workspace_root.join(PUBLISH_WAVE_TOML_PATH);

    match (json_path.is_file(), toml_path.is_file()) {
        (true, false) => Ok(json_path),
        (false, true) => Ok(toml_path),
        (false, false) => Ok(json_path),
        (true, true) => Err(PublishDocsError::CommandFailed {
            message: format!(
                "ambiguous publish wave: both {} and {} exist",
                PUBLISH_WAVE_JSON_PATH, PUBLISH_WAVE_TOML_PATH
            ),
        }),
    }
}

/// Loads the desired-state catalog from the workspace root and validates it against the workspace
/// inventory.
pub(crate) fn load_desired_catalog(
    workspace_root: &Path,
    workspace: &WorkspaceState,
) -> Result<DesiredCatalog, PublishDocsError> {
    let path = desired_catalog_path(workspace_root)?;
    let bytes = fs::read_to_string(&path)?;
    let authored = parse_desired_catalog_authored_config_with_hint(&bytes, Some(path.as_path()))?;
    let catalog = canonicalize_desired_catalog_authored_config(authored)?;
    validate_catalog(&catalog, workspace)?;
    Ok(catalog)
}

fn desired_catalog_path(workspace_root: &Path) -> Result<PathBuf, PublishDocsError> {
    let toml_path = workspace_root.join(DESIRED_CATALOG_TOML_PATH);
    let json_path = workspace_root.join(DESIRED_CATALOG_JSON_PATH);

    match (toml_path.is_file(), json_path.is_file()) {
        (true, false) => Ok(toml_path),
        (false, true) => Ok(json_path),
        (false, false) => Ok(toml_path),
        (true, true) => Err(PublishDocsError::CommandFailed {
            message: format!(
                "ambiguous desired catalog: both {} and {} exist",
                DESIRED_CATALOG_TOML_PATH, DESIRED_CATALOG_JSON_PATH
            ),
        }),
    }
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

    for local in &workspace.packages {
        if !seen_names.contains(&local.name) {
            return Err(PublishDocsError::CommandFailed {
                message: format!(
                    "public workspace package missing from docs catalog: {} ({})",
                    local.name,
                    local.workspace_path.display()
                ),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use semver::Version;
    use tempfile::tempdir;

    use super::{load_desired_catalog, load_publish_wave, select_packages};
    use crate::model::{
        CatalogPackage, CatalogSection, DesiredCatalog, DocsPolicy, LocalPackage, PackageFilter,
        PublishWave, UmbrellaPolicy, Visibility, WavePackage, WorkspaceState,
    };

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

    fn local_package(name: &str, workspace_path: &str) -> LocalPackage {
        LocalPackage {
            name: name.into(),
            version: Version::parse("0.1.0").expect("version"),
            manifest_path: PathBuf::from(format!("/workspace/{workspace_path}/Cargo.toml")),
            workspace_path: PathBuf::from(workspace_path),
            has_docs_target: true,
            readme: None,
            repository: None,
            license: None,
            publish: None,
            local_dependencies: Vec::new(),
        }
    }

    fn workspace() -> WorkspaceState {
        WorkspaceState {
            selected: vec!["mfm-docs".into()],
            packages: vec![local_package("mfm-docs", "crates/docs")],
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

    #[test]
    fn load_desired_catalog_supports_json_when_toml_missing() {
        let dir = tempdir().expect("tempdir");
        let docs_dir = dir.path().join("crates/docs");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(
            docs_dir.join("catalog.json"),
            r#"{
                "catalog_version": 1,
                "umbrella_package": "mfm-docs",
                "packages": [{
                    "name": "mfm-docs",
                    "workspace_path": "crates/docs",
                    "visibility": "public",
                    "section": "binaries_tooling",
                    "summary": "Umbrella docs surface"
                }]
            }"#,
        )
        .expect("write catalog");

        let catalog = load_desired_catalog(dir.path(), &workspace()).expect("load catalog");

        assert_eq!(
            catalog,
            DesiredCatalog {
                catalog_version: 1,
                umbrella_package: "mfm-docs".into(),
                packages: vec![CatalogPackage {
                    name: "mfm-docs".into(),
                    workspace_path: "crates/docs".into(),
                    visibility: Visibility::Public,
                    section: CatalogSection::BinariesTooling,
                    summary: "Umbrella docs surface".into(),
                    docs_policy: DocsPolicy::RepoOnly,
                    umbrella_policy: UmbrellaPolicy::WhenPublished,
                    release_priority: 100,
                    allow_yank: false,
                    owners: Vec::new(),
                    notes: String::new(),
                }],
            }
        );
    }

    #[test]
    fn load_desired_catalog_rejects_ambiguous_toml_and_json() {
        let dir = tempdir().expect("tempdir");
        let docs_dir = dir.path().join("crates/docs");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(
            docs_dir.join("catalog.toml"),
            r#"
                catalog_version = 1
                umbrella_package = "mfm-docs"

                [[packages]]
                name = "mfm-docs"
                workspace_path = "crates/docs"
                visibility = "public"
                section = "binaries_tooling"
                summary = "Umbrella docs surface"
            "#,
        )
        .expect("write toml");
        fs::write(
            docs_dir.join("catalog.json"),
            r#"{
                "catalog_version": 1,
                "umbrella_package": "mfm-docs",
                "packages": []
            }"#,
        )
        .expect("write json");

        let err = load_desired_catalog(dir.path(), &workspace()).expect_err("ambiguous catalog");
        assert!(err.to_string().contains("ambiguous desired catalog"));
    }

    #[test]
    fn load_desired_catalog_rejects_missing_workspace_package() {
        let dir = tempdir().expect("tempdir");
        let docs_dir = dir.path().join("crates/docs");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(
            docs_dir.join("catalog.toml"),
            r#"
                catalog_version = 1
                umbrella_package = "mfm-docs"

                [[packages]]
                name = "mfm-docs"
                workspace_path = "crates/docs"
                visibility = "public"
                section = "binaries_tooling"
                summary = "Umbrella docs surface"
            "#,
        )
        .expect("write catalog");
        let mut workspace = workspace();
        workspace
            .packages
            .push(local_package("mfm-public-extra", "crates/public-extra"));

        let err = load_desired_catalog(dir.path(), &workspace).expect_err("missing catalog entry");

        assert!(err
            .to_string()
            .contains("public workspace package missing from docs catalog: mfm-public-extra"));
    }

    #[test]
    fn load_desired_catalog_accepts_explicit_private_hidden_package() {
        let dir = tempdir().expect("tempdir");
        let docs_dir = dir.path().join("crates/docs");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(
            docs_dir.join("catalog.toml"),
            r#"
                catalog_version = 1
                umbrella_package = "mfm-docs"

                [[packages]]
                name = "mfm-docs"
                workspace_path = "crates/docs"
                visibility = "public"
                section = "binaries_tooling"
                summary = "Umbrella docs surface"

                [[packages]]
                name = "mfm-private-tool"
                workspace_path = "crates/tools/private-tool"
                visibility = "private"
                section = "binaries_tooling"
                summary = "Internal release helper."
                docs_policy = "hidden"
                umbrella_policy = "never"
            "#,
        )
        .expect("write catalog");
        let mut workspace = workspace();
        workspace.packages.push(local_package(
            "mfm-private-tool",
            "crates/tools/private-tool",
        ));

        let catalog = load_desired_catalog(dir.path(), &workspace).expect("load catalog");

        let private = catalog
            .packages
            .iter()
            .find(|package| package.name == "mfm-private-tool")
            .expect("private entry");
        assert_eq!(private.visibility, Visibility::Private);
        assert_eq!(private.docs_policy, DocsPolicy::Hidden);
        assert_eq!(private.umbrella_policy, UmbrellaPolicy::Never);
    }

    #[test]
    fn load_publish_wave_supports_toml_when_json_missing() {
        let dir = tempdir().expect("tempdir");
        let docs_dir = dir.path().join("crates/docs");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(
            docs_dir.join("publish-wave.toml"),
            r#"
                wave = "docs-rs-wave-1"

                [[packages]]
                name = "a"
                group = "foundation"
                workspace_path = "crates/a"
                docs_rs = "https://docs.rs/a"
            "#,
        )
        .expect("write wave");

        let wave = load_publish_wave(dir.path()).expect("load wave");

        assert_eq!(wave.wave, "docs-rs-wave-1");
        assert_eq!(wave.packages.len(), 1);
        assert_eq!(wave.packages[0].name, "a");
    }

    #[test]
    fn load_publish_wave_rejects_ambiguous_json_and_toml() {
        let dir = tempdir().expect("tempdir");
        let docs_dir = dir.path().join("crates/docs");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(
            docs_dir.join("publish-wave.json"),
            r#"{
                "wave": "docs-rs-wave-1",
                "packages": []
            }"#,
        )
        .expect("write json");
        fs::write(
            docs_dir.join("publish-wave.toml"),
            r#"
                wave = "docs-rs-wave-1"
            "#,
        )
        .expect("write toml");

        let err = load_publish_wave(dir.path()).expect_err("ambiguous wave");
        assert!(err.to_string().contains("ambiguous publish wave"));
    }
}
