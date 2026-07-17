use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

const EXPECTED_KERNEL_MANIFESTS: &[&str] = &[
    "crates/kernel/ids/Cargo.toml",
    "crates/kernel/canonical/Cargo.toml",
    "crates/kernel/values/Cargo.toml",
    "crates/kernel/facts/Cargo.toml",
    "crates/kernel/effects/Cargo.toml",
    "crates/kernel/capabilities/Cargo.toml",
    "crates/kernel/program/Cargo.toml",
    "crates/kernel/program-derive/Cargo.toml",
    "crates/kernel/manual-auth/Cargo.toml",
    "crates/kernel/replay/Cargo.toml",
    "crates/kernel/runtime/Cargo.toml",
    "crates/kernel/spec/Cargo.toml",
    "crates/kernel/certify/Cargo.toml",
    "crates/kernel/events/Cargo.toml",
    "crates/kernel/store/Cargo.toml",
];

const APPROVED_CATEGORY_DEPENDENCY_OVERRIDES: &[(&str, &str)] = &[
    ("mfm", "mfm_core"),
    ("mfm-transports-proof", "mfm-collectors-proof"),
];

const PATH_CATEGORY_EXCEPTIONS: &[(&str, CrateCategory)] = &[
    (
        "crates/transports/btc-jsonrpc-http/Cargo.toml",
        CrateCategory::Transport,
    ),
    ("crates/collectors/proof/Cargo.toml", CrateCategory::State),
    ("crates/core/Cargo.toml", CrateCategory::SignerProvider),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CrateCategory {
    Kernel,
    CapabilityContract,
    DomainModel,
    DomainConfig,
    State,
    Operation,
    Adapter,
    Transport,
    SignerContract,
    SignerProvider,
    RuntimeConfig,
    Storage,
    App,
    Binary,
    TestSupport,
}

impl CrateCategory {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "kernel" => Some(Self::Kernel),
            "capability-contract" => Some(Self::CapabilityContract),
            "domain-model" => Some(Self::DomainModel),
            "domain-config" => Some(Self::DomainConfig),
            "state" => Some(Self::State),
            "operation" => Some(Self::Operation),
            "adapter" => Some(Self::Adapter),
            "transport" => Some(Self::Transport),
            "signer-contract" => Some(Self::SignerContract),
            "signer-provider" => Some(Self::SignerProvider),
            "runtime-config" => Some(Self::RuntimeConfig),
            "storage" => Some(Self::Storage),
            "app" => Some(Self::App),
            "binary" => Some(Self::Binary),
            "test-support" => Some(Self::TestSupport),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::CapabilityContract => "capability-contract",
            Self::DomainModel => "domain-model",
            Self::DomainConfig => "domain-config",
            Self::State => "state",
            Self::Operation => "operation",
            Self::Adapter => "adapter",
            Self::Transport => "transport",
            Self::SignerContract => "signer-contract",
            Self::SignerProvider => "signer-provider",
            Self::RuntimeConfig => "runtime-config",
            Self::Storage => "storage",
            Self::App => "app",
            Self::Binary => "binary",
            Self::TestSupport => "test-support",
        }
    }
}

#[derive(Debug, Clone)]
struct WorkspacePackage {
    name: String,
    manifest_rel: String,
    manifest_dir_rel: String,
    category: CrateCategory,
}

#[test]
fn all_workspace_crates_have_mfm_category() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);

    validate_all_workspace_crates_have_mfm_category(&metadata).expect("workspace crate category");
}

#[test]
fn workspace_categories_are_known() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);

    let packages = workspace_packages(&metadata, &root).expect("workspace package categories");
    validate_workspace_category_paths(&packages).expect("category path consistency");
}

#[test]
fn workspace_category_dependency_rules_hold_with_exact_allowlist() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);

    validate_category_dependency_rules(&metadata, &root).expect("category dependency rules");
}

#[test]
fn category_dependency_rules_reject_forbidden_edges() {
    let root = repo_root();
    let base_metadata = workspace_metadata(&root);
    for (
        name,
        source,
        dependency,
        dependency_path,
        expected_source_category,
        expected_dependency_category,
    ) in [
        (
            "state to adapter",
            "mfm-state-portfolio",
            "mfm-adapters-portfolio",
            "crates/adapters/portfolio",
            "state",
            "adapter",
        ),
        (
            "state to live transport",
            "mfm-state-portfolio",
            "mfm-transports-evm",
            "crates/transports/evm",
            "state",
            "transport",
        ),
        (
            "transport to runtime config",
            "mfm-transports-evm",
            "mfm-runtime-config",
            "crates/runtime-config",
            "transport",
            "runtime-config",
        ),
        (
            "state to runtime config",
            "mfm-state-portfolio",
            "mfm-runtime-config",
            "crates/runtime-config",
            "state",
            "runtime-config",
        ),
        (
            "operation to runtime config",
            "mfm-op-portfolio-snapshot",
            "mfm-runtime-config",
            "crates/runtime-config",
            "operation",
            "runtime-config",
        ),
        (
            "transport to operation",
            "mfm-transports-proof",
            "mfm-op-proof",
            "crates/ops/proof-op",
            "transport",
            "operation",
        ),
    ] {
        let mut metadata = base_metadata.clone();
        push_path_dependency(
            &mut metadata,
            source,
            dependency,
            &root.join(dependency_path),
        );

        let error =
            validate_category_dependency_rules(&metadata, &root).expect_err("fixture must fail");
        assert!(
            error.contains(&format!("source_category={expected_source_category}"))
                && error.contains(&format!(
                    "dependency_category={expected_dependency_category}"
                ))
                && error.contains(dependency),
            "{name}: unexpected error: {error}"
        );
    }
}

#[test]
fn category_dependency_rules_reject_forbidden_binary_edges() {
    let root = repo_root();
    let mut metadata = workspace_metadata(&root);
    push_path_dependency(
        &mut metadata,
        "mfm",
        "mfm-adapters-portfolio",
        &root.join("crates/adapters/portfolio"),
    );

    let error =
        validate_category_dependency_rules(&metadata, &root).expect_err("fixture must fail");
    assert!(
        error.contains("source_category=binary")
            && error.contains("dependency_category=adapter")
            && error.contains("mfm-adapters-portfolio"),
        "unexpected error: {error}"
    );
}

#[test]
fn category_dependency_rules_reject_unapproved_binary_platform_edges() {
    let root = repo_root();
    let base_metadata = workspace_metadata(&root);
    for (name, package, manifest, category, dependency_path) in [
        (
            "storage platform",
            "mfm-storage-fixture",
            "crates/storages/fixture/Cargo.toml",
            "storage",
            "crates/storages/fixture",
        ),
        (
            "signer-provider platform",
            "mfm-signer-provider-fixture",
            "crates/signers/fixture/Cargo.toml",
            "signer-provider",
            "crates/signers/fixture",
        ),
    ] {
        let mut metadata = base_metadata.clone();
        push_synthetic_workspace_package(&mut metadata, &root, package, manifest, category);
        push_path_dependency(&mut metadata, "mfm", package, &root.join(dependency_path));

        let error =
            validate_category_dependency_rules(&metadata, &root).expect_err("fixture must fail");
        assert!(
            error.contains("source_category=binary")
                && error.contains(&format!("dependency_category={category}"))
                && error.contains(package),
            "{name}: unexpected error: {error}"
        );
    }
}

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
    push_path_dependency(
        &mut metadata,
        "mfm-ids",
        "mfm-app",
        &root.join("crates/app"),
    );

    let error =
        validate_kernel_dependency_boundary(&metadata, &root).expect_err("fixture must fail");
    assert!(
        error.contains("mfm-app") && error.contains("crates/app"),
        "unexpected error: {error}"
    );
}

#[test]
fn configured_target_ownership_and_dependency_boundaries_are_explicit() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);
    let packages = workspace_packages(&metadata, &root).expect("workspace package categories");
    assert_eq!(
        packages.len(),
        45,
        "the configured-target portfolio snapshot workspace has 45 packages"
    );

    for removed in [
        "mfm-authored-config",
        "mfm-portfolio-config",
        "mfm-stream-store-postgres",
        "mfm-op-portfolio-tracker",
        "mfm-catalog-model",
    ] {
        assert!(
            packages.iter().all(|package| package.name != removed),
            "removed package remains in workspace metadata: {removed}"
        );
    }
    assert!(
        packages
            .iter()
            .any(|package| package.name == "mfm-op-portfolio-snapshot"),
        "the complete portfolio snapshot operation must remain a workspace package"
    );

    let by_name = packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    assert!(
        !by_name.contains_key("mfm-catalog-model"),
        "the deleted catalog-model package must not remain in workspace metadata"
    );
    assert_eq!(
        by_name
            .get("mfm-runtime-config")
            .map(|package| package.category),
        Some(CrateCategory::RuntimeConfig)
    );

    let storage = by_name
        .get("mfm-storage-postgres")
        .expect("renamed PostgreSQL storage package");
    for dependency in path_dependencies(&metadata, storage.name.as_str(), &by_name) {
        assert!(!matches!(
            dependency.category,
            CrateCategory::DomainModel | CrateCategory::DomainConfig
        ));
    }

    for package in packages.iter().filter(|package| {
        matches!(
            package.category,
            CrateCategory::Operation | CrateCategory::State
        )
    }) {
        for dependency in path_dependencies(&metadata, package.name.as_str(), &by_name) {
            assert!(
                !matches!(
                    dependency.name,
                    "mfm"
                        | "mfm-app"
                        | "mfm-rest-api"
                        | "mfm-storage-postgres"
                        | "mfm-runtime-config"
                ) && dependency.name != "sqlx",
                "{} may not depend on binaries, app, PostgreSQL, runtime config, or SQLx: {}",
                package.name,
                dependency.name
            );
        }
    }

    for package in packages
        .iter()
        .filter(|package| package.category == CrateCategory::Binary)
    {
        for dependency in path_dependencies(&metadata, package.name.as_str(), &by_name)
            .into_iter()
            .filter(|dependency| dependency.kind.is_none())
        {
            assert!(
                !matches!(
                    dependency.category,
                    CrateCategory::DomainConfig | CrateCategory::Operation
                ),
                "production binary {} must not own domain config or operation ingress: {}",
                package.name,
                dependency.name
            );
        }
    }
}

#[test]
fn configured_target_source_boundaries_are_enforced() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);
    let packages = workspace_packages(&metadata, &root).expect("workspace package categories");
    let by_name = packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect::<BTreeMap<_, _>>();

    let sources = rust_sources(&root);
    assert!(
        !root.join("crates/catalog-model/Cargo.toml").exists(),
        "the deleted catalog-model package must not retain a manifest"
    );

    let storage_root = root.join("crates/storages");
    for path in sources
        .iter()
        .filter(|path| path.starts_with(&storage_root))
    {
        let source = fs::read_to_string(path).expect("read storage Rust source");
        for forbidden in ["MfmConfig", "ValidatedConfig<"] {
            assert!(
                !source.contains(forbidden),
                "storage source {} imports or names domain type {forbidden}",
                path.display()
            );
        }
    }

    let app_root = root.join("crates/app/src");
    let configured_value_methods = [
        "publish_configured_values(",
        ".load_configured_value(",
        ".list_configured_targets(",
    ];
    for path in &sources {
        if path.starts_with(&storage_root) || path.starts_with(&app_root) {
            continue;
        }
        let source = fs::read_to_string(path).expect("read Rust source");
        assert!(
            !configured_value_methods
                .iter()
                .any(|method| source.contains(method)),
            "non-app source directly calls configured-value persistence: {}",
            path.display()
        );
    }

    let binary_root = root.join("bin");
    for path in sources.iter().filter(|path| path.starts_with(&binary_root)) {
        let source = fs::read_to_string(path).expect("read binary Rust source");
        for forbidden in [
            "SetupDocument",
            "enum SetupConfig",
            "toml::from_str",
            "ValidatedConfig<",
            "MfmConfig",
        ] {
            assert!(
                !source.contains(forbidden),
                "binary source {} owns domain setup/config construction: {forbidden}",
                path.display()
            );
        }
    }

    let composed_source =
        fs::read_to_string(root.join("crates/ops/portfolio-snapshot-op/src/lib.rs"))
            .expect("read composed operation source");
    assert!(
        !composed_source.contains("ConfiguredValue"),
        "portfolio composition and its certified graph helpers must not retain configured storage"
    );
    assert!(
        composed_source.contains("type Config = PortfolioConfig"),
        "portfolio composition must take the aggregate PortfolioConfig as its only authority"
    );
    assert!(
        composed_source.contains("struct AssemblePortfolioCollectionReceiptState"),
        "composition must own its operation-local exact receipt fan-in state"
    );
    assert!(
        composed_source.contains("struct PortfolioSnapshotOperation"),
        "one operation must own the complete portfolio snapshot objective"
    );
    assert!(
        composed_source.contains("portfolio_snapshot_program_draft"),
        "the snapshot operation must expose its single production root-draft helper"
    );
    for forbidden in ["mfm-events", "mfm-replay", "mfm-spec", "mfm-store"] {
        assert!(
            path_dependencies(&metadata, "mfm-op-portfolio-snapshot", &by_name)
                .iter()
                .all(|dependency| dependency.name != forbidden),
            "portfolio operation must remain replay/store independent: {forbidden}"
        );
    }
    assert!(
        !root
            .join("crates/ops/portfolio-snapshot-op/src/replay.rs")
            .exists(),
        "portfolio operation must not retain a replay module"
    );
    let app_portfolio_snapshot_source =
        fs::read_to_string(root.join("crates/app/src/portfolio_snapshot_replay.rs"))
            .expect("read private app portfolio replay binding");
    assert!(
        app_portfolio_snapshot_source.contains("verify_portfolio_collection_receipt_replay"),
        "app replay dispatch must own the operation-local receipt verification binding"
    );
    let portfolio_state_root = root.join("crates/states/portfolio/src");
    for path in sources
        .iter()
        .filter(|path| path.starts_with(&portfolio_state_root))
    {
        let source = fs::read_to_string(path).expect("read portfolio state source");
        assert!(
            !source.contains("AssemblePortfolioCollectionReceiptState"),
            "portfolio state source must not duplicate operation-local receipt fan-in: {}",
            path.display()
        );
    }
}

#[derive(Debug)]
struct PathDependency<'a> {
    name: &'a str,
    category: CrateCategory,
    kind: Option<&'a str>,
}

fn path_dependencies<'a>(
    metadata: &'a Value,
    source_name: &str,
    by_name: &'a BTreeMap<&'a str, &'a WorkspacePackage>,
) -> Vec<PathDependency<'a>> {
    metadata_packages(metadata)
        .expect("metadata packages")
        .iter()
        .find(|package| package.get("name").and_then(Value::as_str) == Some(source_name))
        .and_then(|package| package.get("dependencies").and_then(Value::as_array))
        .into_iter()
        .flatten()
        .filter_map(|dependency| {
            let name = dependency.get("name").and_then(Value::as_str)?;
            let package = by_name.get(name)?;
            Some(PathDependency {
                name,
                category: package.category,
                kind: dependency.get("kind").and_then(Value::as_str),
            })
        })
        .collect()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("integration crate lives under tests/integration")
        .to_path_buf()
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for directory in [root.join("crates"), root.join("bin")] {
        collect_rust_sources(&directory, &mut sources);
    }
    sources.sort();
    sources
}

fn collect_rust_sources(path: &Path, sources: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    for entry in entries {
        let entry = entry.expect("read source directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            sources.push(path);
        }
    }
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

fn validate_all_workspace_crates_have_mfm_category(metadata: &Value) -> Result<(), String> {
    let workspace_members = workspace_members(metadata)?;
    let packages = metadata_packages(metadata)?;
    let mut missing = Vec::new();

    for package in packages {
        let Some(package_id) = package.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !workspace_members.contains(package_id) {
            continue;
        }
        if package_category_raw(package).is_none() {
            missing.push(
                package
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
                    .to_owned(),
            );
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "workspace crates missing package.metadata.mfm.category: {}",
            missing.join(", ")
        ))
    }
}

fn validate_category_dependency_rules(metadata: &Value, root: &Path) -> Result<(), String> {
    let packages = workspace_packages(metadata, root)?;
    let by_manifest_dir = packages
        .iter()
        .map(|package| (package.manifest_dir_rel.clone(), package))
        .collect::<BTreeMap<_, _>>();
    let by_name = packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    let mut used_allowlist = BTreeSet::new();

    for source in metadata_packages(metadata)? {
        let Some(source_name) = source.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(source_package) = by_name.get(source_name) else {
            continue;
        };
        let Some(dependencies) = source.get("dependencies").and_then(Value::as_array) else {
            continue;
        };

        for dependency in dependencies {
            // Category rules describe runtime ownership boundaries. Test-only dependencies may
            // exercise another binary surface without becoming a production binary edge.
            if dependency.get("kind").and_then(Value::as_str) == Some("dev") {
                continue;
            }
            let Some(dependency_path) = dependency.get("path").and_then(Value::as_str) else {
                continue;
            };
            let dependency_rel = repo_relative(root, dependency_path);
            let Some(dependency_package) = by_manifest_dir.get(&dependency_rel) else {
                continue;
            };
            if category_dependency_allowed(source_package.category, dependency_package.category) {
                continue;
            }

            let edge = (
                source_package.name.as_str(),
                dependency_package.name.as_str(),
            );
            if APPROVED_CATEGORY_DEPENDENCY_OVERRIDES.contains(&edge) {
                used_allowlist
                    .insert((source_package.name.clone(), dependency_package.name.clone()));
                continue;
            }

            return Err(format!(
                "category dependency violation source={} source_category={} dependency={} dependency_category={} dependency_path={}",
                source_package.name,
                source_package.category.as_str(),
                dependency_package.name,
                dependency_package.category.as_str(),
                dependency_package.manifest_dir_rel
            ));
        }
    }

    for &(source, dependency) in APPROVED_CATEGORY_DEPENDENCY_OVERRIDES {
        if !used_allowlist.contains(&(source.to_owned(), dependency.to_owned())) {
            return Err(format!(
                "stale category dependency override source={source} dependency={dependency}"
            ));
        }
    }

    Ok(())
}

fn workspace_packages(metadata: &Value, root: &Path) -> Result<Vec<WorkspacePackage>, String> {
    let workspace_members = workspace_members(metadata)?;
    let packages = metadata_packages(metadata)?;
    let mut workspace_packages = Vec::new();

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
        let manifest_path = package
            .get("manifest_path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("workspace package missing manifest path: {package_name}"))?;
        let manifest_rel = repo_relative(root, manifest_path);
        let manifest_dir_rel = Path::new(&manifest_rel)
            .parent()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let category = parse_package_category(package).map_err(|error| {
            format!("workspace package category invalid package={package_name}: {error}")
        })?;

        workspace_packages.push(WorkspacePackage {
            name: package_name.to_owned(),
            manifest_rel,
            manifest_dir_rel,
            category,
        });
    }

    Ok(workspace_packages)
}

fn push_path_dependency(
    metadata: &mut Value,
    source_name: &str,
    dependency_name: &str,
    path: &Path,
) {
    let packages = metadata
        .get_mut("packages")
        .and_then(Value::as_array_mut)
        .expect("metadata packages");
    let source = packages
        .iter_mut()
        .find(|package| package.get("name").and_then(Value::as_str) == Some(source_name))
        .expect("source package");
    source
        .get_mut("dependencies")
        .and_then(Value::as_array_mut)
        .expect("source dependencies")
        .push(json!({
            "name": dependency_name,
            "source": null,
            "req": "*",
            "kind": null,
            "rename": null,
            "optional": false,
            "uses_default_features": true,
            "features": [],
            "target": null,
            "registry": null,
            "path": path.to_string_lossy(),
        }));
}

fn push_synthetic_workspace_package(
    metadata: &mut Value,
    root: &Path,
    name: &str,
    manifest_rel: &str,
    category: &str,
) {
    let manifest_path = root.join(manifest_rel);
    let package_id = format!("path+file://{}#{name}@0.0.0", manifest_path.display());
    metadata
        .get_mut("workspace_members")
        .and_then(Value::as_array_mut)
        .expect("workspace members")
        .push(json!(package_id.clone()));
    metadata
        .get_mut("packages")
        .and_then(Value::as_array_mut)
        .expect("metadata packages")
        .push(json!({
            "name": name,
            "id": package_id,
            "manifest_path": manifest_path.to_string_lossy(),
            "metadata": {
                "mfm": {
                    "category": category,
                },
            },
            "dependencies": [],
        }));
}

fn validate_workspace_category_paths(packages: &[WorkspacePackage]) -> Result<(), String> {
    let mut used_exceptions = BTreeSet::new();

    for package in packages {
        if validate_category_path(package, &mut used_exceptions)? {
            continue;
        }
    }

    for &(manifest, category) in PATH_CATEGORY_EXCEPTIONS {
        if !used_exceptions.contains(&(manifest.to_owned(), category)) {
            return Err(format!(
                "stale category path exception manifest={manifest} category={}",
                category.as_str()
            ));
        }
    }

    Ok(())
}

fn validate_category_path(
    package: &WorkspacePackage,
    used_exceptions: &mut BTreeSet<(String, CrateCategory)>,
) -> Result<bool, String> {
    let exception = (package.manifest_rel.as_str(), package.category);
    if PATH_CATEGORY_EXCEPTIONS.contains(&exception) {
        used_exceptions.insert((package.manifest_rel.clone(), package.category));
        return Ok(true);
    }

    let ok = match package.category {
        CrateCategory::Kernel => package.manifest_rel.starts_with("crates/kernel/"),
        CrateCategory::CapabilityContract => {
            package.manifest_rel.starts_with("crates/")
                && package.manifest_dir_rel.ends_with("-capabilities")
        }
        CrateCategory::DomainModel => {
            package.manifest_dir_rel.ends_with("-model")
                || package.manifest_dir_rel.ends_with("/model")
                || package.manifest_dir_rel == "crates/evm-core"
        }
        CrateCategory::DomainConfig => package.manifest_dir_rel.ends_with("-config"),
        CrateCategory::State => package.manifest_rel.starts_with("crates/states/"),
        CrateCategory::Operation => package.manifest_rel.starts_with("crates/ops/"),
        CrateCategory::Adapter => package.manifest_rel.starts_with("crates/adapters/"),
        CrateCategory::Transport => package.manifest_rel.starts_with("crates/transports/"),
        CrateCategory::SignerContract => {
            matches!(
                package.manifest_dir_rel.as_str(),
                "crates/signing" | "crates/evm-signing"
            )
        }
        CrateCategory::SignerProvider => package.manifest_rel.starts_with("crates/signers/"),
        CrateCategory::RuntimeConfig => package.manifest_rel == "crates/runtime-config/Cargo.toml",
        CrateCategory::Storage => package.manifest_rel.starts_with("crates/storages/"),
        CrateCategory::App => package.manifest_rel == "crates/app/Cargo.toml",
        CrateCategory::Binary => package.manifest_rel.starts_with("bin/"),
        CrateCategory::TestSupport => package.manifest_rel.starts_with("tests/"),
    };

    if ok {
        Ok(true)
    } else {
        Err(format!(
            "category path violation package={} category={} manifest={}",
            package.name,
            package.category.as_str(),
            package.manifest_rel
        ))
    }
}

fn category_dependency_allowed(source: CrateCategory, dependency: CrateCategory) -> bool {
    use CrateCategory::{
        Adapter, App, Binary, CapabilityContract, DomainConfig, DomainModel, Kernel, Operation,
        RuntimeConfig, SignerContract, SignerProvider, State, Storage, TestSupport, Transport,
    };

    match source {
        Kernel => dependency == Kernel,
        CapabilityContract => matches!(dependency, Kernel | CapabilityContract | DomainModel),
        DomainModel => matches!(dependency, Kernel | DomainModel),
        DomainConfig => matches!(
            dependency,
            Kernel | DomainModel | DomainConfig | SignerContract
        ),
        State => matches!(
            dependency,
            Kernel | CapabilityContract | DomainModel | DomainConfig | SignerContract
        ),
        Operation => matches!(
            dependency,
            Kernel
                | CapabilityContract
                | DomainModel
                | DomainConfig
                | State
                | Operation
                | SignerContract
        ),
        Adapter => matches!(
            dependency,
            Kernel
                | CapabilityContract
                | DomainModel
                | DomainConfig
                | State
                | SignerContract
                | Transport
        ),
        Transport => matches!(dependency, Kernel | CapabilityContract | DomainModel),
        SignerContract => matches!(dependency, Kernel | DomainModel | SignerContract),
        SignerProvider => matches!(
            dependency,
            Kernel | DomainModel | SignerContract | SignerProvider
        ),
        RuntimeConfig => matches!(dependency, Kernel | CapabilityContract | SignerContract),
        Storage => matches!(dependency, Kernel | CapabilityContract),
        App => !matches!(dependency, Binary | TestSupport),
        Binary => matches!(
            dependency,
            Kernel | App | Operation | DomainModel | DomainConfig | RuntimeConfig
        ),
        TestSupport => true,
    }
}

fn parse_package_category(package: &Value) -> Result<CrateCategory, String> {
    let raw = package_category_raw(package).ok_or_else(|| "missing mfm.category".to_owned())?;
    CrateCategory::parse(raw).ok_or_else(|| format!("unknown category={raw}"))
}

fn package_category_raw(package: &Value) -> Option<&str> {
    package
        .get("metadata")
        .and_then(|metadata| metadata.get("mfm"))
        .and_then(|mfm| mfm.get("category"))
        .and_then(Value::as_str)
}

fn workspace_members(metadata: &Value) -> Result<BTreeSet<&str>, String> {
    metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing workspace_members".to_owned())
        .map(|members| members.iter().filter_map(Value::as_str).collect())
}

fn metadata_packages(metadata: &Value) -> Result<&Vec<Value>, String> {
    metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing packages".to_owned())
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
