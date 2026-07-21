use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Kernel,
    Domain,
    Live,
    Signing,
    SecretProvider,
    Storage,
    Assembly,
    Binary,
    Test,
}

impl Layer {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "kernel" => Some(Self::Kernel),
            "domain" => Some(Self::Domain),
            "live" => Some(Self::Live),
            "signing" => Some(Self::Signing),
            "secret-provider" => Some(Self::SecretProvider),
            "storage" => Some(Self::Storage),
            "assembly" => Some(Self::Assembly),
            "binary" => Some(Self::Binary),
            "test" => Some(Self::Test),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Domain => "domain",
            Self::Live => "live",
            Self::Signing => "signing",
            Self::SecretProvider => "secret-provider",
            Self::Storage => "storage",
            Self::Assembly => "assembly",
            Self::Binary => "binary",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DomainRole {
    Source,
    Aggregate,
}

impl DomainRole {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "source" => Some(Self::Source),
            "aggregate" => Some(Self::Aggregate),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Aggregate => "aggregate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackageSemantics {
    layer: Layer,
    domain: Option<String>,
    domain_role: Option<DomainRole>,
    domain_facing: Option<bool>,
    binary_facing: Option<bool>,
}

impl PackageSemantics {
    fn kernel(domain_facing: bool, binary_facing: bool) -> Self {
        Self {
            layer: Layer::Kernel,
            domain: None,
            domain_role: None,
            domain_facing: Some(domain_facing),
            binary_facing: Some(binary_facing),
        }
    }

    fn domain(domain: &str, role: DomainRole) -> Self {
        Self {
            layer: Layer::Domain,
            domain: Some(domain.to_owned()),
            domain_role: Some(role),
            domain_facing: None,
            binary_facing: None,
        }
    }

    fn live(domain: &str) -> Self {
        Self {
            layer: Layer::Live,
            domain: Some(domain.to_owned()),
            domain_role: None,
            domain_facing: None,
            binary_facing: None,
        }
    }

    fn plain(layer: Layer) -> Self {
        Self {
            layer,
            domain: None,
            domain_role: None,
            domain_facing: None,
            binary_facing: None,
        }
    }

    fn assembly(binary_facing: bool) -> Self {
        Self {
            layer: Layer::Assembly,
            domain: None,
            domain_role: None,
            domain_facing: None,
            binary_facing: Some(binary_facing),
        }
    }
}

#[derive(Debug, Clone)]
struct WorkspacePackage {
    name: String,
    manifest_dir_rel: String,
    semantics: PackageSemantics,
    target_kinds: Vec<BTreeSet<String>>,
}

#[test]
fn workspace_semantic_metadata_contract_holds() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);
    let packages = workspace_packages(&metadata, &root).expect("workspace package metadata");

    domain_roles(&packages).expect("domain and live metadata");
    for package in &packages {
        validate_target_coherence(package).expect("Cargo target-kind coherence");
    }
}

#[test]
fn workspace_semantic_dependency_matrix_holds() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);
    let packages = workspace_packages(&metadata, &root).expect("workspace package metadata");

    validate_dependency_matrix(&metadata, &root, &packages).expect("semantic dependency matrix");
}

#[test]
fn metadata_shape_is_closed_and_typed() {
    let accepted = [
        json!({"layer": "kernel", "domain-facing": true, "binary-facing": false}),
        json!({"layer": "domain", "domain": "bitcoin", "domain-role": "source"}),
        json!({"layer": "live", "domain": "bitcoin"}),
        json!({"layer": "signing"}),
        json!({"layer": "secret-provider"}),
        json!({"layer": "storage"}),
        json!({"layer": "assembly", "binary-facing": true}),
        json!({"layer": "binary"}),
        json!({"layer": "test"}),
    ];
    for value in accepted {
        parse_semantics("fixture", value.as_object().expect("metadata object"))
            .expect("valid semantic metadata");
    }

    for (name, rejected) in [
        ("old category", json!({"category": "kernel"})),
        ("unknown layer", json!({"layer": "adapter"})),
        (
            "missing kernel facing flag",
            json!({"layer": "kernel", "domain-facing": true}),
        ),
        (
            "domain missing role",
            json!({"layer": "domain", "domain": "bitcoin"}),
        ),
        (
            "live carrying role",
            json!({"layer": "live", "domain": "bitcoin", "domain-role": "source"}),
        ),
        (
            "binary carrying domain",
            json!({"layer": "binary", "domain": "bitcoin"}),
        ),
        (
            "unknown metadata key",
            json!({"layer": "storage", "phase": "transition"}),
        ),
        (
            "invalid domain",
            json!({"layer": "domain", "domain": "Bitcoin/Core", "domain-role": "source"}),
        ),
        (
            "invalid role",
            json!({"layer": "domain", "domain": "bitcoin", "domain-role": "leaf"}),
        ),
    ] {
        let error = parse_semantics("fixture", rejected.as_object().expect("metadata object"))
            .expect_err("metadata fixture must fail");
        assert!(!error.is_empty(), "{name} returned an empty diagnostic");
    }
}

#[test]
fn domain_metadata_rejects_role_conflicts_and_orphan_live_packages() {
    let mut packages = vec![fixture_package(
        "source",
        PackageSemantics::domain("chain", DomainRole::Source),
        &[&["lib"]],
    )];
    packages.push(fixture_package(
        "conflict",
        PackageSemantics::domain("chain", DomainRole::Aggregate),
        &[&["lib"]],
    ));
    let error = domain_roles(&packages).expect_err("conflicting domain roles must fail");
    assert!(error.contains("domain=chain") && error.contains("role conflict"));

    let orphan = vec![fixture_package(
        "orphan-live",
        PackageSemantics::live("unowned"),
        &[&["lib"]],
    )];
    let error = domain_roles(&orphan).expect_err("orphan live package must fail");
    assert!(error.contains("domain=unowned") && error.contains("no matching domain package"));
}

#[test]
fn semantic_matrix_accepts_each_layer_row() {
    let roles = fixture_domain_roles();
    let kernel_domain = PackageSemantics::kernel(true, false);
    let kernel_platform = PackageSemantics::kernel(false, false);
    let signing = PackageSemantics::plain(Layer::Signing);
    let source = PackageSemantics::domain("bitcoin", DomainRole::Source);
    let source_peer = PackageSemantics::domain("bitcoin", DomainRole::Source);
    let aggregate = PackageSemantics::domain("portfolio", DomainRole::Aggregate);
    let aggregate_peer = PackageSemantics::domain("portfolio", DomainRole::Aggregate);
    let source_live = PackageSemantics::live("bitcoin");
    let source_live_peer = PackageSemantics::live("bitcoin");
    let aggregate_live = PackageSemantics::live("portfolio");
    let aggregate_live_peer = PackageSemantics::live("portfolio");
    let secret = PackageSemantics::plain(Layer::SecretProvider);
    let storage = PackageSemantics::plain(Layer::Storage);
    let assembly = PackageSemantics::assembly(true);
    let binary = PackageSemantics::plain(Layer::Binary);
    let test = PackageSemantics::plain(Layer::Test);

    let allowed = [
        (&kernel_platform, &kernel_domain, "kernel -> kernel"),
        (&signing, &kernel_domain, "signing -> domain-facing kernel"),
        (&source, &kernel_domain, "source domain -> kernel"),
        (&source, &signing, "source domain -> signing"),
        (&source, &source_peer, "source domain -> same domain"),
        (&aggregate, &kernel_domain, "aggregate domain -> kernel"),
        (&aggregate, &signing, "aggregate domain -> signing"),
        (
            &aggregate,
            &aggregate_peer,
            "aggregate domain -> same domain",
        ),
        (&aggregate, &source, "aggregate domain -> source domain"),
        (&source_live, &kernel_platform, "source live -> kernel"),
        (&source_live, &signing, "source live -> signing"),
        (&source_live, &source, "source live -> own domain"),
        (
            &source_live,
            &source_live_peer,
            "source live -> same-domain live",
        ),
        (
            &aggregate_live,
            &kernel_platform,
            "aggregate live -> kernel",
        ),
        (&aggregate_live, &signing, "aggregate live -> signing"),
        (&aggregate_live, &aggregate, "aggregate live -> own domain"),
        (&aggregate_live, &source, "aggregate live -> source domain"),
        (
            &aggregate_live,
            &aggregate_live_peer,
            "aggregate live -> same-domain live",
        ),
        (
            &secret,
            &kernel_domain,
            "secret provider -> domain-facing kernel",
        ),
        (&secret, &signing, "secret provider -> signing"),
        (&secret, &secret, "secret provider -> secret provider"),
        (&storage, &kernel_platform, "storage -> kernel"),
        (&assembly, &aggregate_live, "assembly -> lower layer"),
        (&assembly, &assembly, "assembly -> assembly support"),
        (
            &binary,
            &PackageSemantics::kernel(true, true),
            "binary -> facing kernel",
        ),
        (&binary, &assembly, "binary -> assembly"),
        (&test, &binary, "test -> unrestricted"),
    ];

    for (source, dependency, name) in allowed {
        assert!(
            dependency_allowed(source, dependency, &roles),
            "allowed matrix row was rejected: {name}"
        );
    }
}

#[test]
fn semantic_matrix_rejects_forbidden_edges_without_exceptions() {
    let roles = fixture_domain_roles();
    let kernel_domain = PackageSemantics::kernel(true, false);
    let kernel_platform = PackageSemantics::kernel(false, false);
    let signing = PackageSemantics::plain(Layer::Signing);
    let source = PackageSemantics::domain("bitcoin", DomainRole::Source);
    let other_source = PackageSemantics::domain("evm", DomainRole::Source);
    let aggregate = PackageSemantics::domain("portfolio", DomainRole::Aggregate);
    let other_aggregate = PackageSemantics::domain("reporting", DomainRole::Aggregate);
    let source_live = PackageSemantics::live("bitcoin");
    let other_source_live = PackageSemantics::live("evm");
    let aggregate_live = PackageSemantics::live("portfolio");
    let secret = PackageSemantics::plain(Layer::SecretProvider);
    let storage = PackageSemantics::plain(Layer::Storage);
    let assembly = PackageSemantics::assembly(true);
    let binary = PackageSemantics::plain(Layer::Binary);

    let forbidden = [
        (&kernel_domain, &source, "kernel -> domain"),
        (
            &signing,
            &kernel_platform,
            "signing -> platform-only kernel",
        ),
        (
            &source,
            &kernel_platform,
            "source domain -> platform-only kernel",
        ),
        (
            &source,
            &other_source,
            "source domain -> other source domain",
        ),
        (&source, &source_live, "source domain -> live"),
        (
            &aggregate,
            &other_aggregate,
            "aggregate domain -> other aggregate domain",
        ),
        (&aggregate, &aggregate_live, "aggregate domain -> live"),
        (&source_live, &aggregate, "source live -> aggregate domain"),
        (
            &source_live,
            &other_source_live,
            "source live -> cross-domain live",
        ),
        (&source_live, &storage, "source live -> storage"),
        (
            &aggregate_live,
            &source_live,
            "aggregate live -> source live",
        ),
        (&aggregate_live, &storage, "aggregate live -> storage"),
        (
            &secret,
            &kernel_platform,
            "secret provider -> platform-only kernel",
        ),
        (&secret, &source, "secret provider -> domain"),
        (&storage, &source, "storage -> domain"),
        (&assembly, &binary, "assembly -> binary"),
        (&binary, &source, "binary -> domain"),
        (&binary, &source_live, "binary -> live"),
    ];

    for (source, dependency, name) in forbidden {
        assert!(
            !dependency_allowed(source, dependency, &roles),
            "forbidden matrix edge was accepted: {name}"
        );
    }
}

#[test]
fn cargo_target_kinds_constrain_declared_layers() {
    let mislabeled_binary = fixture_package(
        "mislabeled",
        PackageSemantics::domain("bitcoin", DomainRole::Source),
        &[&["bin"]],
    );
    let error = validate_target_coherence(&mislabeled_binary)
        .expect_err("a binary target cannot self-label as a domain");
    assert!(error.contains("bin target") && error.contains("layer=domain"));

    let mixed_binary = fixture_package(
        "mixed",
        PackageSemantics::plain(Layer::Binary),
        &[&["lib"], &["bin"], &["test"]],
    );
    validate_target_coherence(&mixed_binary).expect("a mixed binary package is valid");

    let dedicated_proc_macro = fixture_package(
        "derive",
        PackageSemantics::kernel(true, false),
        &[&["proc-macro"], &["test"]],
    );
    validate_target_coherence(&dedicated_proc_macro).expect("dedicated proc macro");

    let mixed_proc_macro = fixture_package(
        "derive-with-lib",
        PackageSemantics::kernel(true, false),
        &[&["proc-macro"], &["lib"]],
    );
    let error = validate_target_coherence(&mixed_proc_macro)
        .expect_err("a proc macro package must remain dedicated");
    assert!(error.contains("proc-macro") && error.contains("dedicated"));
}

#[test]
fn mixed_binary_library_dependencies_obey_the_binary_row() {
    let roles = fixture_domain_roles();
    let mixed_binary = fixture_package(
        "mixed",
        PackageSemantics::plain(Layer::Binary),
        &[&["lib"], &["bin"]],
    );
    validate_target_coherence(&mixed_binary).expect("mixed binary target shape");

    let facing_app = PackageSemantics::assembly(true);
    assert!(dependency_allowed(
        &mixed_binary.semantics,
        &facing_app,
        &roles
    ));

    let live_library = PackageSemantics::live("bitcoin");
    assert!(
        !dependency_allowed(&mixed_binary.semantics, &live_library, &roles),
        "a mixed package cannot hide a library dependency that violates its binary layer"
    );
}

#[test]
fn durable_source_boundaries_follow_semantic_metadata() {
    let root = repo_root();
    let metadata = workspace_metadata(&root);
    let packages = workspace_packages(&metadata, &root).expect("workspace package metadata");

    for package in packages {
        if package.semantics.layer == Layer::Test {
            continue;
        }
        let source_root = root.join(&package.manifest_dir_rel);
        if !source_root.exists() {
            continue;
        }
        let mut sources = Vec::new();
        collect_rust_sources(&source_root, &mut sources);

        for path in sources {
            let source = fs::read_to_string(&path).expect("read Rust source");
            if package.semantics.layer == Layer::Storage {
                for forbidden in ["MfmConfig", "ValidatedConfig<"] {
                    assert!(
                        !source.contains(forbidden),
                        "storage source {} imports or names domain configuration {forbidden}",
                        path.display()
                    );
                }
            }

            if package.semantics.layer == Layer::Binary {
                for forbidden in [
                    "SetupDocument",
                    "enum SetupConfig",
                    "toml::from_str",
                    "ValidatedConfig<",
                    "MfmConfig",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "binary source {} owns semantic setup/config construction: {forbidden}",
                        path.display()
                    );
                }
            }

            let owns_configured_values = package.semantics.layer == Layer::Storage
                || (package.semantics.layer == Layer::Assembly
                    && package.semantics.binary_facing == Some(true));
            if !owns_configured_values {
                for forbidden in [
                    "publish_configured_values(",
                    ".load_configured_value(",
                    ".list_configured_targets(",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "source {} bypasses app-owned configured-value services: {forbidden}",
                        path.display()
                    );
                }
            }
        }
    }
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

fn workspace_packages(metadata: &Value, root: &Path) -> Result<Vec<WorkspacePackage>, String> {
    let members = workspace_members(metadata)?;
    let mut packages = Vec::new();

    for package in metadata_packages(metadata)? {
        let Some(id) = package.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !members.contains(id) {
            continue;
        }

        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "workspace package missing name".to_owned())?;
        let manifest_path = package
            .get("manifest_path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("workspace package missing manifest path package={name}"))?;
        let manifest_rel = repo_relative(root, manifest_path);
        let manifest_dir_rel = Path::new(&manifest_rel)
            .parent()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let mfm = package
            .get("metadata")
            .and_then(|value| value.get("mfm"))
            .and_then(Value::as_object)
            .ok_or_else(|| format!("package={name} missing object package.metadata.mfm"))?;
        let semantics = parse_semantics(name, mfm)?;
        let target_kinds = package
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("package={name} missing Cargo targets"))?
            .iter()
            .map(|target| {
                target
                    .get("kind")
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("package={name} target missing kind"))?
                    .iter()
                    .map(|kind| {
                        kind.as_str()
                            .map(ToOwned::to_owned)
                            .ok_or_else(|| format!("package={name} target kind is not a string"))
                    })
                    .collect::<Result<BTreeSet<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;

        packages.push(WorkspacePackage {
            name: name.to_owned(),
            manifest_dir_rel,
            semantics,
            target_kinds,
        });
    }

    Ok(packages)
}

fn parse_semantics(name: &str, mfm: &Map<String, Value>) -> Result<PackageSemantics, String> {
    let layer_raw = mfm
        .get("layer")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("package={name} missing string mfm.layer"))?;
    let layer = Layer::parse(layer_raw)
        .ok_or_else(|| format!("package={name} has unknown mfm.layer={layer_raw}"))?;

    let expected_keys = match layer {
        Layer::Kernel => ["binary-facing", "domain-facing", "layer"].as_slice(),
        Layer::Domain => ["domain", "domain-role", "layer"].as_slice(),
        Layer::Live => ["domain", "layer"].as_slice(),
        Layer::Assembly => ["binary-facing", "layer"].as_slice(),
        Layer::Signing | Layer::SecretProvider | Layer::Storage | Layer::Binary | Layer::Test => {
            ["layer"].as_slice()
        }
    };
    let actual_keys = mfm.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_keys = expected_keys.iter().copied().collect::<BTreeSet<_>>();
    if actual_keys != expected_keys {
        return Err(format!(
            "package={name} metadata keys do not match layer={}: actual={actual_keys:?} expected={expected_keys:?}",
            layer.as_str()
        ));
    }

    let domain = mfm
        .get("domain")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if let Some(domain) = &domain {
        if !valid_domain_id(domain) {
            return Err(format!("package={name} has invalid domain={domain}"));
        }
    }
    let domain_role = mfm
        .get("domain-role")
        .and_then(Value::as_str)
        .map(|role| {
            DomainRole::parse(role)
                .ok_or_else(|| format!("package={name} has invalid domain-role={role}"))
        })
        .transpose()?;
    let domain_facing = optional_bool(name, mfm, "domain-facing")?;
    let binary_facing = optional_bool(name, mfm, "binary-facing")?;

    Ok(PackageSemantics {
        layer,
        domain,
        domain_role,
        domain_facing,
        binary_facing,
    })
}

fn optional_bool(
    name: &str,
    metadata: &Map<String, Value>,
    key: &str,
) -> Result<Option<bool>, String> {
    metadata
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("package={name} mfm.{key} must be a boolean"))
        })
        .transpose()
}

fn valid_domain_id(domain: &str) -> bool {
    if domain.is_empty() || domain.len() > 64 {
        return false;
    }
    let bytes = domain.as_bytes();
    if !bytes[0].is_ascii_lowercase() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return false;
    }
    let mut previous_hyphen = false;
    for byte in bytes {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            previous_hyphen = false;
        } else if *byte == b'-' && !previous_hyphen {
            previous_hyphen = true;
        } else {
            return false;
        }
    }
    true
}

fn domain_roles(packages: &[WorkspacePackage]) -> Result<BTreeMap<String, DomainRole>, String> {
    let mut roles = BTreeMap::new();
    for package in packages
        .iter()
        .filter(|package| package.semantics.layer == Layer::Domain)
    {
        let domain = package
            .semantics
            .domain
            .as_ref()
            .expect("domain metadata shape was parsed");
        let role = package
            .semantics
            .domain_role
            .expect("domain role metadata shape was parsed");
        if let Some(previous) = roles.insert(domain.clone(), role) {
            if previous != role {
                return Err(format!(
                    "domain role conflict domain={domain} first={} package={} second={}",
                    previous.as_str(),
                    package.name,
                    role.as_str()
                ));
            }
        }
    }

    for package in packages
        .iter()
        .filter(|package| package.semantics.layer == Layer::Live)
    {
        let domain = package
            .semantics
            .domain
            .as_ref()
            .expect("live domain metadata shape was parsed");
        if !roles.contains_key(domain) {
            return Err(format!(
                "live package={} domain={domain} has no matching domain package",
                package.name
            ));
        }
    }

    Ok(roles)
}

fn validate_target_coherence(package: &WorkspacePackage) -> Result<(), String> {
    let has_bin = package
        .target_kinds
        .iter()
        .any(|kinds| kinds.contains("bin"));
    if has_bin && !matches!(package.semantics.layer, Layer::Binary | Layer::Test) {
        return Err(format!(
            "package={} has a bin target but declares layer={}",
            package.name,
            package.semantics.layer.as_str()
        ));
    }
    if package.semantics.layer == Layer::Binary && !has_bin {
        return Err(format!(
            "package={} declares layer=binary without a bin target",
            package.name
        ));
    }

    let has_proc_macro = package
        .target_kinds
        .iter()
        .any(|kinds| kinds.contains("proc-macro"));
    if has_proc_macro {
        if package.semantics.layer == Layer::Binary || has_bin {
            return Err(format!(
                "package={} proc-macro target must be non-binary",
                package.name
            ));
        }
        let non_test_targets = package
            .target_kinds
            .iter()
            .filter(|kinds| {
                !kinds.contains("test") && !kinds.contains("bench") && !kinds.contains("example")
            })
            .collect::<Vec<_>>();
        let proc_macro_kind = BTreeSet::from(["proc-macro".to_owned()]);
        if non_test_targets.len() != 1 || *non_test_targets[0] != proc_macro_kind {
            return Err(format!(
                "package={} proc-macro target must remain in a dedicated package",
                package.name
            ));
        }
    }

    Ok(())
}

fn validate_dependency_matrix(
    metadata: &Value,
    root: &Path,
    packages: &[WorkspacePackage],
) -> Result<(), String> {
    let roles = domain_roles(packages)?;
    let by_name = packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    let by_manifest_dir = packages
        .iter()
        .map(|package| (package.manifest_dir_rel.as_str(), package))
        .collect::<BTreeMap<_, _>>();

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
            if dependency.get("kind").and_then(Value::as_str) == Some("dev") {
                continue;
            }
            let Some(path) = dependency.get("path").and_then(Value::as_str) else {
                continue;
            };
            let dependency_dir = repo_relative(root, path);
            let Some(dependency_package) = by_manifest_dir.get(dependency_dir.as_str()) else {
                continue;
            };
            if dependency_allowed(
                &source_package.semantics,
                &dependency_package.semantics,
                &roles,
            ) {
                continue;
            }

            return Err(format!(
                "semantic dependency violation source={} source_layer={} dependency={} dependency_layer={}",
                source_package.name,
                source_package.semantics.layer.as_str(),
                dependency_package.name,
                dependency_package.semantics.layer.as_str()
            ));
        }
    }

    Ok(())
}

fn dependency_allowed(
    source: &PackageSemantics,
    dependency: &PackageSemantics,
    roles: &BTreeMap<String, DomainRole>,
) -> bool {
    match source.layer {
        Layer::Kernel => dependency.layer == Layer::Kernel,
        Layer::Signing => {
            dependency.layer == Layer::Kernel && dependency.domain_facing == Some(true)
        }
        Layer::Domain => domain_dependency_allowed(source, dependency),
        Layer::Live => live_dependency_allowed(source, dependency, roles),
        Layer::SecretProvider => match dependency.layer {
            Layer::Kernel => dependency.domain_facing == Some(true),
            Layer::Signing | Layer::SecretProvider => true,
            _ => false,
        },
        Layer::Storage => dependency.layer == Layer::Kernel,
        Layer::Assembly => !matches!(dependency.layer, Layer::Binary | Layer::Test),
        // The final binary-facing refinement becomes active when implementation construction has
        // moved behind app. Even before that cut, domain, live, signing, and storage edges are
        // forbidden and mixed lib/bin packages are evaluated as one binary package.
        Layer::Binary => matches!(
            dependency.layer,
            Layer::Kernel | Layer::Assembly | Layer::SecretProvider
        ),
        Layer::Test => true,
    }
}

fn domain_dependency_allowed(source: &PackageSemantics, dependency: &PackageSemantics) -> bool {
    let source_domain = source
        .domain
        .as_deref()
        .expect("domain metadata shape was parsed");
    let source_role = source
        .domain_role
        .expect("domain role metadata shape was parsed");

    match dependency.layer {
        Layer::Kernel => dependency.domain_facing == Some(true),
        Layer::Signing => true,
        Layer::Domain => {
            let dependency_domain = dependency
                .domain
                .as_deref()
                .expect("domain metadata shape was parsed");
            let dependency_role = dependency
                .domain_role
                .expect("domain role metadata shape was parsed");
            match source_role {
                // The aggregate edge is removed with the last Bitcoin-to-portfolio state contract.
                DomainRole::Source => {
                    dependency_domain == source_domain || dependency_role == DomainRole::Aggregate
                }
                DomainRole::Aggregate => {
                    dependency_domain == source_domain || dependency_role == DomainRole::Source
                }
            }
        }
        _ => false,
    }
}

fn live_dependency_allowed(
    source: &PackageSemantics,
    dependency: &PackageSemantics,
    roles: &BTreeMap<String, DomainRole>,
) -> bool {
    let source_domain = source
        .domain
        .as_deref()
        .expect("live metadata shape was parsed");
    let source_role = roles
        .get(source_domain)
        .copied()
        .expect("live domain ownership was validated");

    match dependency.layer {
        Layer::Kernel | Layer::Signing => true,
        Layer::Domain => {
            let dependency_domain = dependency
                .domain
                .as_deref()
                .expect("domain metadata shape was parsed");
            let dependency_role = dependency
                .domain_role
                .expect("domain role metadata shape was parsed");
            match source_role {
                DomainRole::Source => {
                    dependency_role == DomainRole::Source && dependency_domain == source_domain
                }
                DomainRole::Aggregate => {
                    dependency_role == DomainRole::Source
                        || (dependency_role == DomainRole::Aggregate
                            && dependency_domain == source_domain)
                }
            }
        }
        Layer::Live => {
            let dependency_domain = dependency
                .domain
                .as_deref()
                .expect("live metadata shape was parsed");
            dependency_domain == source_domain
                && roles.get(dependency_domain).copied() == Some(source_role)
        }
        _ => false,
    }
}

fn fixture_package(
    name: &str,
    semantics: PackageSemantics,
    target_kinds: &[&[&str]],
) -> WorkspacePackage {
    WorkspacePackage {
        name: name.to_owned(),
        manifest_dir_rel: format!("fixtures/{name}"),
        semantics,
        target_kinds: target_kinds
            .iter()
            .map(|kinds| kinds.iter().map(|kind| (*kind).to_owned()).collect())
            .collect(),
    }
}

fn fixture_domain_roles() -> BTreeMap<String, DomainRole> {
    BTreeMap::from([
        ("bitcoin".to_owned(), DomainRole::Source),
        ("evm".to_owned(), DomainRole::Source),
        ("portfolio".to_owned(), DomainRole::Aggregate),
        ("reporting".to_owned(), DomainRole::Aggregate),
    ])
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

fn repo_relative(root: &Path, path: &str) -> String {
    let path = Path::new(path);
    let rel = if path.is_absolute() {
        path.strip_prefix(root).unwrap_or(path)
    } else {
        path
    };
    rel.to_string_lossy().replace('\\', "/")
}
