use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const REMOVED_PACKAGES: [&str; 3] = [
    "mfm-executor",
    "mfm-storage-executor-file",
    "mfm-storage-executor-postgres",
];

const GENERIC_HISTORY_PACKAGES: [&str; 7] = [
    "mfm-journal",
    "mfm-program",
    "mfm-certify",
    "mfm-store",
    "mfm-runtime",
    "mfm-replay",
    "mfm-spec",
];

#[test]
fn retired_executor_packages_and_paths_are_absent() {
    let root = repository_root();
    let metadata = cargo_metadata(&root);
    let packages = workspace_packages(&metadata);

    for removed in REMOVED_PACKAGES {
        assert!(
            !packages.contains_key(removed),
            "{removed} must not remain a workspace member",
        );
        for (consumer_name, consumer) in &packages {
            assert!(
                !dependency_names(consumer).contains(removed),
                "{consumer_name} must not depend on retired package {removed}",
            );
        }
    }

    for removed_path in [
        "crates/kernel/executor",
        "crates/storages/executor-file",
        "crates/storages/executor-postgres",
    ] {
        assert!(
            !root.join(removed_path).exists(),
            "{removed_path} must be deleted rather than retained as a compatibility path",
        );
    }
}

#[test]
fn wallet_authority_storage_is_an_explicit_storage_layer_member() {
    let root = repository_root();
    let metadata = cargo_metadata(&root);
    let packages = workspace_packages(&metadata);
    let wallet = packages
        .get("mfm-storage-evm-postgres")
        .expect("wallet authority storage must be a workspace member");

    assert_eq!(
        relative_manifest_path(&root, wallet),
        "crates/storages/evm-postgres/Cargo.toml",
    );
    assert_eq!(
        metadata_string(wallet, "/metadata/mfm/layer"),
        Some("storage")
    );
    assert_eq!(metadata_string(wallet, "/metadata/mfm/domain"), Some("evm"));
    let dependencies = normal_dependency_names(wallet);
    assert!(dependencies.contains("mfm-evm"));
    for forbidden in ["mfm-app", "mfm-runtime", "mfm-replay", "mfm-store"] {
        assert!(
            !dependencies.contains(forbidden),
            "wallet authority storage must not depend on {forbidden}",
        );
    }
}

#[test]
fn generic_structured_history_packages_remain_evm_neutral() {
    let root = repository_root();
    let metadata = cargo_metadata(&root);
    let packages = workspace_packages(&metadata);

    for package_name in GENERIC_HISTORY_PACKAGES {
        let package = packages
            .get(package_name)
            .unwrap_or_else(|| panic!("{package_name} must be a workspace member"));
        assert_eq!(
            metadata_string(package, "/metadata/mfm/layer"),
            Some("kernel")
        );
        for dependency in normal_dependency_names(package) {
            assert!(
                dependency != "mfm-evm"
                    && dependency != "mfm-evm-live"
                    && dependency != "mfm-storage-evm-postgres",
                "{package_name} must remain EVM-neutral but depends on {dependency}",
            );
        }
    }
}

#[test]
fn structured_history_ownership_edges_are_one_way() {
    let root = repository_root();
    let metadata = cargo_metadata(&root);
    let packages = workspace_packages(&metadata);

    assert_dependencies(
        &packages,
        "mfm-certify",
        &["mfm-program", "mfm-journal"],
        &["mfm-store", "mfm-runtime", "mfm-app", "mfm-storage-postgres"],
    );
    assert_dependencies(
        &packages,
        "mfm-runtime",
        &["mfm-certify", "mfm-program", "mfm-journal"],
        &["mfm-store", "mfm-app", "mfm-replay", "mfm-storage-postgres"],
    );
    assert_dependencies(
        &packages,
        "mfm-store",
        &["mfm-runtime", "mfm-certify", "mfm-journal"],
        &["mfm-app", "mfm-storage-postgres"],
    );
    assert_dependencies(
        &packages,
        "mfm-replay",
        &["mfm-journal", "mfm-store"],
        &["mfm-app", "mfm-runtime", "mfm-storage-postgres"],
    );
    assert_dependencies(
        &packages,
        "mfm-storage-postgres",
        &["mfm-journal", "mfm-store", "mfm-certify"],
        &["mfm-app", "mfm-replay"],
    );
    assert_dependencies(
        &packages,
        "mfm-app",
        &[
            "mfm-certify",
            "mfm-program",
            "mfm-replay",
            "mfm-runtime",
            "mfm-storage-evm-postgres",
            "mfm-storage-postgres",
        ],
        &[],
    );

    assert!(
        root.join("crates/app/src/production_structured.rs")
            .is_file(),
        "application assembly must use the structured production backend",
    );
    assert!(
        !root.join("crates/app/src/production.rs").exists(),
        "the graph-era production backend must be deleted",
    );
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("integration crate must live under tests/integration")
        .to_path_buf()
}

fn cargo_metadata(root: &Path) -> Value {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).expect("decode cargo metadata")
}

fn workspace_packages(metadata: &Value) -> BTreeMap<String, &Value> {
    let workspace_members = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .expect("workspace_members")
        .iter()
        .map(|member| member.as_str().expect("workspace member id"))
        .collect::<BTreeSet<_>>();

    metadata
        .get("packages")
        .and_then(Value::as_array)
        .expect("packages")
        .iter()
        .filter(|package| {
            package
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| workspace_members.contains(id))
        })
        .map(|package| {
            (
                package
                    .get("name")
                    .and_then(Value::as_str)
                    .expect("package name")
                    .to_owned(),
                package,
            )
        })
        .collect()
}

fn dependency_names(package: &Value) -> BTreeSet<&str> {
    package
        .get("dependencies")
        .and_then(Value::as_array)
        .expect("package dependencies")
        .iter()
        .map(|dependency| {
            dependency
                .get("name")
                .and_then(Value::as_str)
                .expect("dependency name")
        })
        .collect()
}

fn normal_dependency_names(package: &Value) -> BTreeSet<&str> {
    package
        .get("dependencies")
        .and_then(Value::as_array)
        .expect("package dependencies")
        .iter()
        .filter(|dependency| dependency.get("kind").is_none_or(Value::is_null))
        .map(|dependency| {
            dependency
                .get("name")
                .and_then(Value::as_str)
                .expect("dependency name")
        })
        .collect()
}

fn assert_dependencies(
    packages: &BTreeMap<String, &Value>,
    package_name: &str,
    required: &[&str],
    forbidden: &[&str],
) {
    let package = packages
        .get(package_name)
        .unwrap_or_else(|| panic!("{package_name} must be a workspace member"));
    let dependencies = normal_dependency_names(package);
    for required_name in required {
        assert!(
            dependencies.contains(required_name),
            "{package_name} must normally depend on {required_name}",
        );
    }
    for forbidden_name in forbidden {
        assert!(
            !dependencies.contains(forbidden_name),
            "{package_name} must not normally depend on {forbidden_name}",
        );
    }
}

fn metadata_string<'a>(package: &'a Value, pointer: &str) -> Option<&'a str> {
    package.pointer(pointer).and_then(Value::as_str)
}

fn relative_manifest_path(root: &Path, package: &Value) -> String {
    let manifest = package
        .get("manifest_path")
        .and_then(Value::as_str)
        .expect("manifest_path");
    Path::new(manifest)
        .strip_prefix(root)
        .expect("workspace-relative manifest")
        .to_string_lossy()
        .replace('\\', "/")
}
