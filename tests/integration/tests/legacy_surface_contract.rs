use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn production_tree_has_no_retired_runtime_or_history_surface() {
    let root = repository_root();
    for removed in [
        "crates/kernel/executor",
        "crates/storages/executor-file",
        "crates/storages/executor-postgres",
    ] {
        assert!(
            !root.join(removed).exists(),
            "retired path {removed} exists"
        );
    }

    let forbidden = [
        ["mfm", "-executor"].concat(),
        ["mfm-storage-", "executor"].concat(),
        ["NonDomain", "Failure"].concat(),
        ["non_domain_", "failure"].concat(),
        ["Dependency", "Skipped"].concat(),
        ["Blocking", "Source"].concat(),
        ["Effect", "Requested"].concat(),
        ["Effect", "Settled"].concat(),
        ["DidNot", "Enter"].concat(),
        ["Indeter", "minate"].concat(),
        ["executor_", "frontier"].concat(),
        ["ready_", "node"].concat(),
        ["application/vnd.mfm.run-export-stream.v1+", "json-seq"].concat(),
    ];
    let mut violations = Vec::new();
    for relative in ["Cargo.toml", "nixfied.nix", "crates", "bin"] {
        scan_path(&root.join(relative), &root, &forbidden, &mut violations);
    }
    assert!(
        violations.is_empty(),
        "retired production surface remains:\n{}",
        violations.join("\n")
    );

    let retired_test_imports = [
        "mfm_qualified_run_test_support",
        "QualifiedRunFixture",
        "StateExecutionKind",
        "QualifiedStateRegistration",
        "AuthorizedAdmissionPlan",
        "InMemoryRunJournalBackend",
    ];
    let mut test_violations = Vec::new();
    scan_test_sources(
        &root.join("crates"),
        &root,
        &retired_test_imports,
        &mut test_violations,
    );
    assert!(
        test_violations.is_empty(),
        "retired graph-era test source remains:\n{}",
        test_violations.join("\n")
    );
}

fn scan_path(path: &Path, root: &Path, forbidden: &[String], violations: &mut Vec<String>) {
    if path.is_dir() {
        if path.file_name().and_then(|name| name.to_str()) == Some("tests") {
            return;
        }
        for entry in fs::read_dir(path).expect("read source directory") {
            scan_path(
                &entry.expect("directory entry").path(),
                root,
                forbidden,
                violations,
            );
        }
        return;
    }

    let extension = path.extension().and_then(|value| value.to_str());
    let supported = matches!(extension, Some("rs" | "toml" | "sql" | "nix"))
        || path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml");
    if !supported
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "tests.rs" || name.ends_with("_tests.rs"))
    {
        return;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    for token in forbidden {
        if text.contains(token) {
            violations.push(format!(
                "{} contains {token}",
                path.strip_prefix(root).expect("repository path").display()
            ));
        }
    }
}

fn scan_test_sources(path: &Path, root: &Path, forbidden: &[&str], violations: &mut Vec<String>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).expect("read source directory") {
            scan_test_sources(
                &entry.expect("directory entry").path(),
                root,
                forbidden,
                violations,
            );
        }
        return;
    }
    if path.extension().and_then(|value| value.to_str()) != Some("rs") {
        return;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    for token in forbidden {
        if text.contains(token) {
            violations.push(format!(
                "{} contains {token}",
                path.strip_prefix(root).expect("repository path").display()
            ));
        }
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("integration crate must live under tests/integration")
        .to_path_buf()
}
