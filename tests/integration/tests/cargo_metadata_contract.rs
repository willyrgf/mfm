use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

const EXPECTED_KERNEL_MANIFESTS: &[&str] = &[
    "crates/kernel/ids/Cargo.toml",
    "crates/kernel/canonical/Cargo.toml",
    "crates/kernel/values/Cargo.toml",
    "crates/kernel/effects/Cargo.toml",
    "crates/kernel/capabilities/Cargo.toml",
    "crates/kernel/program/Cargo.toml",
    "crates/kernel/program-derive/Cargo.toml",
    "crates/kernel/replay/Cargo.toml",
    "crates/kernel/runtime/Cargo.toml",
    "crates/kernel/spec/Cargo.toml",
    "crates/kernel/certify/Cargo.toml",
    "crates/kernel/events/Cargo.toml",
    "crates/kernel/store/Cargo.toml",
    "crates/kernel/test-support/Cargo.toml",
];

#[test]
fn kernel_workspace_crates_stay_inside_kernel_dependency_boundary() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);

    validate_kernel_dependency_boundary(&metadata, &root).expect("kernel dependency boundary");
}

#[test]
fn kernel_dependency_boundary_rejects_non_kernel_path_dependency_fixture() {
    let root = repo_root();
    let mut metadata = workspace_metadata(&root);
    let packages = metadata
        .get_mut("packages")
        .and_then(Value::as_array_mut)
        .expect("metadata packages");
    let ids = packages
        .iter_mut()
        .find(|package| package.get("name").and_then(Value::as_str) == Some("mfm-ids"))
        .expect("mfm-ids package");
    ids.get_mut("dependencies")
        .and_then(Value::as_array_mut)
        .expect("mfm-ids dependencies")
        .push(json!({
            "name": "mfm-machine",
            "source": null,
            "req": "*",
            "kind": null,
            "rename": null,
            "optional": false,
            "uses_default_features": true,
            "features": [],
            "target": null,
            "registry": null,
            "path": root.join("crates/machine").to_string_lossy(),
        }));

    let error =
        validate_kernel_dependency_boundary(&metadata, &root).expect_err("fixture must fail");
    assert!(
        error.contains("mfm-machine") && error.contains("crates/machine"),
        "unexpected error: {error}"
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("integration crate lives under tests/integration")
        .to_path_buf()
}

fn workspace_metadata(root: &Path) -> Value {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse cargo metadata")
}

fn validate_kernel_dependency_boundary(metadata: &Value, root: &Path) -> Result<(), String> {
    let workspace_members = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing workspace_members".to_owned())?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing packages".to_owned())?;

    let mut found_kernel_manifests = BTreeSet::new();
    for package in packages {
        let Some(package_id) = package.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !workspace_members.contains(package_id) {
            continue;
        }
        let Some(manifest_path) = package.get("manifest_path").and_then(Value::as_str) else {
            continue;
        };
        let manifest_rel = repo_relative(root, manifest_path);
        if is_kernel_manifest(&manifest_rel) {
            found_kernel_manifests.insert(manifest_rel);
        }
    }

    for expected in EXPECTED_KERNEL_MANIFESTS {
        if !found_kernel_manifests.contains(*expected) {
            return Err(format!("missing expected kernel crate manifest={expected}"));
        }
    }

    for package in packages {
        let Some(package_id) = package.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !workspace_members.contains(package_id) {
            continue;
        }
        let package_name = package
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let Some(manifest_path) = package.get("manifest_path").and_then(Value::as_str) else {
            continue;
        };
        let manifest_rel = repo_relative(root, manifest_path);
        if !is_kernel_manifest(&manifest_rel) {
            continue;
        }
        let Some(dependencies) = package.get("dependencies").and_then(Value::as_array) else {
            continue;
        };

        for dependency in dependencies {
            let Some(dependency_path) = dependency.get("path").and_then(Value::as_str) else {
                continue;
            };
            let dependency_rel = repo_relative(root, dependency_path);
            if !dependency_rel.starts_with("crates/kernel/") {
                let dependency_name = dependency
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                return Err(format!(
                    "kernel crate dependency violation crate={package_name} manifest={manifest_rel} dependency={dependency_name} path={dependency_rel}"
                ));
            }
        }
    }

    Ok(())
}

fn is_kernel_manifest(path: &str) -> bool {
    path.starts_with("crates/kernel/") && path.ends_with("/Cargo.toml")
}

fn repo_relative(root: &Path, path: &str) -> String {
    let path = Path::new(path);
    let rel = if path.is_absolute() {
        path.strip_prefix(root).unwrap_or(path)
    } else {
        path
    };
    rel.to_string_lossy().replace('\\', "/")
}
