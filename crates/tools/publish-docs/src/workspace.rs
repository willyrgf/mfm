use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context};
use semver::Version;
use serde::Deserialize;

use crate::{
    catalog::{PUBLISH_WAVE_JSON_PATH, PUBLISH_WAVE_TOML_PATH},
    error::PublishDocsError,
    model::{LocalPackage, WavePackage, WorkspaceState},
};

/// Finds the workspace root by honoring `MFM_WORKSPACE_ROOT` first and then walking upward.
pub(crate) fn find_workspace_root(start_dir: &Path) -> Result<PathBuf, PublishDocsError> {
    if let Some(root) = env::var_os("MFM_WORKSPACE_ROOT") {
        let root = PathBuf::from(root);
        if is_workspace_root(&root) {
            return Ok(root);
        }
    }

    let mut current = start_dir.to_path_buf();
    loop {
        if is_workspace_root(&current) {
            return Ok(current);
        }

        if !current.pop() {
            break;
        }
    }

    Err(PublishDocsError::WorkspaceRootNotFound {
        start_dir: start_dir.to_path_buf(),
    })
}

/// Enforces a clean working tree unless the caller explicitly allows dirty state.
pub(crate) fn ensure_clean_worktree(workspace_root: &Path) -> Result<(), PublishDocsError> {
    let unstaged = Command::new("git")
        .args(["diff", "--quiet", "--ignore-submodules", "HEAD", "--"])
        .current_dir(workspace_root)
        .status()?;
    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet", "--ignore-submodules", "--"])
        .current_dir(workspace_root)
        .status()?;

    if unstaged.success() && staged.success() {
        Ok(())
    } else {
        Err(PublishDocsError::DirtyWorkingTree)
    }
}

/// Loads local workspace facts for the selected wave packages from `cargo metadata`.
pub(crate) fn load_workspace_state(
    workspace_root: &Path,
    selected_packages: &[WavePackage],
) -> Result<WorkspaceState, PublishDocsError> {
    let metadata = load_metadata(workspace_root)?;
    let metadata_by_name: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .cloned()
        .map(|package| (package.name.clone(), package))
        .collect();
    let selected_names: BTreeSet<_> = selected_packages
        .iter()
        .map(|package| package.name.clone())
        .collect();

    for package in selected_packages {
        if !metadata_by_name.contains_key(&package.name) {
            return Err(PublishDocsError::PackageMissingFromMetadata {
                package: package.name.clone(),
            });
        }
    }

    Ok(WorkspaceState {
        selected: selected_packages
            .iter()
            .map(|package| package.name.clone())
            .collect(),
        packages: metadata
            .packages
            .into_iter()
            .map(|package| local_package_from_metadata(workspace_root, &selected_names, package))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// Returns selected packages from the workspace state in wave order.
pub(crate) fn selected_packages(
    workspace: &WorkspaceState,
) -> Result<Vec<&LocalPackage>, PublishDocsError> {
    let packages_by_name: BTreeMap<_, _> = workspace
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();
    workspace
        .selected
        .iter()
        .map(|name| {
            packages_by_name.get(name.as_str()).copied().ok_or_else(|| {
                PublishDocsError::PackageMissingFromMetadata {
                    package: name.clone(),
                }
            })
        })
        .collect()
}

/// Returns a workspace package by cargo package name.
pub(crate) fn workspace_package<'a>(
    workspace: &'a WorkspaceState,
    name: &str,
) -> Option<&'a LocalPackage> {
    workspace
        .packages
        .iter()
        .find(|package| package.name == name)
}

fn is_workspace_root(path: &Path) -> bool {
    path.join("Cargo.toml").is_file()
        && (path.join(PUBLISH_WAVE_JSON_PATH).is_file()
            || path.join(PUBLISH_WAVE_TOML_PATH).is_file())
}

fn load_metadata(workspace_root: &Path) -> Result<CargoMetadata, PublishDocsError> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(workspace_root)
        .output()?;
    if !output.status.success() {
        return Err(PublishDocsError::CommandFailed {
            message: format!(
                "cargo metadata failed with status {}",
                output.status.code().unwrap_or(1)
            ),
        });
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

fn local_package_from_metadata(
    workspace_root: &Path,
    selected_names: &BTreeSet<String>,
    metadata_package: CargoPackage,
) -> Result<LocalPackage, PublishDocsError> {
    let manifest_path = metadata_package.manifest_path.clone();
    let workspace_path = manifest_path
        .parent()
        .ok_or_else(|| anyhow!("manifest path had no parent: {}", manifest_path.display()))?
        .strip_prefix(workspace_root)
        .with_context(|| {
            format!(
                "manifest path {} was outside workspace root {}",
                manifest_path.display(),
                workspace_root.display()
            )
        })?
        .to_path_buf();

    let local_dependencies = metadata_package
        .dependencies
        .iter()
        .filter(|dependency| dependency.path.is_some())
        .map(|dependency| dependency.name.clone())
        .filter(|dependency| selected_names.contains(dependency))
        .collect();

    Ok(LocalPackage {
        name: metadata_package.name.clone(),
        version: Version::parse(&metadata_package.version)?,
        manifest_path,
        workspace_path,
        has_docs_target: metadata_package.targets.iter().any(|target| {
            target
                .kind
                .iter()
                .any(|kind| kind == "lib" || kind == "proc-macro")
        }),
        readme: metadata_package.readme,
        repository: metadata_package.repository,
        license: metadata_package.license,
        publish: metadata_package.publish,
        local_dependencies,
    })
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
}

#[derive(Debug, Clone, Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    manifest_path: PathBuf,
    dependencies: Vec<CargoDependency>,
    targets: Vec<CargoTarget>,
    readme: Option<String>,
    repository: Option<String>,
    license: Option<String>,
    publish: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct CargoDependency {
    name: String,
    path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
struct CargoTarget {
    kind: Vec<String>,
}
