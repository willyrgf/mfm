use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const REQUIRED_JOURNAL_CONSUMERS: [&str; 3] = ["mfm-replay", "mfm-runtime", "mfm-store"];
const REMOVED_PACKAGES: [&str; 3] = ["mfm-events", "mfm-manual-auth", "mfm-portfolio-live"];

#[test]
fn recoverability_workspace_uses_one_journal_crate() {
    let root = repository_root();
    let metadata = cargo_metadata(&root);
    let packages = workspace_packages(&metadata);

    let journal = packages
        .get("mfm-journal")
        .expect("mfm-journal must be a workspace member");
    assert_eq!(
        relative_manifest_path(&root, journal),
        "crates/kernel/journal/Cargo.toml"
    );
    assert_eq!(
        journal
            .pointer("/metadata/mfm/layer")
            .and_then(Value::as_str),
        Some("kernel")
    );
    assert_eq!(
        journal
            .pointer("/metadata/mfm/domain-facing")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        journal
            .pointer("/metadata/mfm/binary-facing")
            .and_then(Value::as_bool),
        Some(false)
    );

    for consumer_name in REQUIRED_JOURNAL_CONSUMERS {
        let consumer = packages
            .get(consumer_name)
            .unwrap_or_else(|| panic!("{consumer_name} must be a workspace member"));
        assert!(
            dependency_names(consumer).contains("mfm-journal"),
            "{consumer_name} must consume the one journal contract"
        );
    }

    let executor = packages
        .get("mfm-executor")
        .expect("mfm-executor must be a workspace member");
    assert!(
        !dependency_names(executor).contains("mfm-journal"),
        "mfm-executor must embed capability-owned values without depending on mfm-journal"
    );
}

#[test]
fn superseded_event_manual_auth_and_portfolio_live_crates_are_absent() {
    let root = repository_root();
    let metadata = cargo_metadata(&root);
    let packages = workspace_packages(&metadata);

    for removed in REMOVED_PACKAGES {
        assert!(
            !packages.contains_key(removed),
            "{removed} must not remain a workspace member"
        );

        for (consumer_name, consumer) in &packages {
            assert!(
                !dependency_names(consumer).contains(removed),
                "{consumer_name} must not depend on removed package {removed}"
            );
        }
    }

    for removed_path in [
        "crates/kernel/events/Cargo.toml",
        "crates/kernel/manual-auth/Cargo.toml",
        "crates/live/portfolio/Cargo.toml",
    ] {
        assert!(
            !root.join(removed_path).exists(),
            "{removed_path} must be deleted rather than retained as a compatibility crate"
        );
    }
}

#[test]
fn run_history_ownership_dependencies_and_sources_are_one_way() {
    let root = repository_root();
    let metadata = cargo_metadata(&root);
    let packages = workspace_packages(&metadata);

    assert_normal_dependencies(
        &packages,
        "mfm-store",
        &[],
        &["mfm-app", "mfm-runtime", "mfm-storage-postgres"],
    );
    assert_normal_dependencies(
        &packages,
        "mfm-runtime",
        &["mfm-program", "mfm-store"],
        &["mfm-app", "mfm-storage-postgres"],
    );
    assert_normal_dependencies(
        &packages,
        "mfm-replay",
        &["mfm-store"],
        &["mfm-app", "mfm-runtime", "mfm-storage-postgres"],
    );
    assert_normal_dependencies(
        &packages,
        "mfm-storage-postgres",
        &["mfm-store"],
        &["mfm-app", "mfm-runtime"],
    );
    assert_normal_dependencies(
        &packages,
        "mfm-app",
        &["mfm-replay", "mfm-runtime", "mfm-storage-postgres"],
        &[],
    );

    let production = read_source(&root, "crates/app/src/production.rs");
    for forbidden in [
        ".append_admission(",
        ".prepare_admission(",
        "QualifiedPostgresStore",
        "PreparedJournalAppend",
        "writer_pool(",
    ] {
        assert!(
            !production.contains(forbidden),
            "application production assembly must not retain `{forbidden}`"
        );
    }
    assert!(
        production.contains(".admit("),
        "application admission must enter through Runtime"
    );

    let postgres_exports = read_source(&root, "crates/storages/postgres/src/lib.rs");
    assert!(
        !postgres_exports.contains("QualifiedPostgresStore"),
        "the retired combined PostgreSQL facade must not remain exported"
    );
    let store_exports = read_source(&root, "crates/kernel/store/src/lib.rs");
    for retired in [
        "RunJournalStore",
        "SupportStore",
        "ConfiguredValueStore",
        "AdmissionSourceStore",
        "FactSelectionStore",
    ] {
        assert!(
            !store_exports.contains(retired),
            "the retired combined store facade `{retired}` must not remain exported"
        );
    }
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
        String::from_utf8_lossy(&output.stderr)
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

fn assert_normal_dependencies(
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
            "{package_name} must normally depend on {required_name}"
        );
    }
    for forbidden_name in forbidden {
        assert!(
            !dependencies.contains(forbidden_name),
            "{package_name} must not normally depend on {forbidden_name}"
        );
    }
}

fn read_source(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn relative_manifest_path(root: &Path, package: &Value) -> String {
    let manifest_path = package
        .get("manifest_path")
        .and_then(Value::as_str)
        .expect("manifest path");
    Path::new(manifest_path)
        .strip_prefix(root)
        .expect("workspace manifest must be under repository root")
        .to_string_lossy()
        .replace('\\', "/")
}
