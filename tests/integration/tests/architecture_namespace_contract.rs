use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TEST_HARNESS_PATHS: &[&str] = &["tests/integration/tests/architecture_namespace_contract.rs"];

const FORBIDDEN_TYPED_SURFACE_FIELDS: &[&str] = &[
    "rpc_url",
    "authorization",
    "keystore_path_env",
    "password_file_env",
    "password",
    "private_key",
    "mnemonic",
    "raw_transaction",
];

const TYPED_SURFACE_FIELD_SCAN_SKIP_PATHS: &[&str] = &[
    "crates/kernel/values/src/lib.rs",
    "crates/kernel/values/src/tests.rs",
];

const TEMPORARY_TYPED_FIELD_ALLOWLIST: &[(&str, &str, usize)] = &[
    ("crates/portfolio/model/src/portfolio.rs", "password", 1),
    ("crates/portfolio/model/src/portfolio.rs", "mnemonic", 2),
    ("crates/portfolio/model/src/symbol.rs", "authorization", 2),
    ("crates/app/src/lib.rs", "authorization", 74),
    ("crates/kernel/certify/src/lib.rs", "authorization", 79),
    ("crates/kernel/program/src/lib.rs", "authorization", 15),
    ("crates/kernel/runtime/src/tests.rs", "authorization", 11),
    ("tests/integration/src/test_support.rs", "authorization", 6),
    ("tests/integration/src/test_support.rs", "rpc_url", 4),
    (
        "tests/integration/src/test_support.rs",
        "password_file_env",
        7,
    ),
    ("tests/integration/src/test_support.rs", "password", 16),
    ("tests/integration/src/test_support.rs", "mnemonic", 2),
];

#[test]
fn active_public_namespace_has_no_stale_workflow_recipe_terms() {
    let root = repo_root();
    let entries = repo_text_entries(&root);
    let forbidden_terms = forbidden_public_name_terms();

    assert_forbidden_terms_are_allowlisted(
        "public namespace",
        &entries,
        &forbidden_terms,
        &[],
        |_path, _source| true,
    );
}

#[test]
fn typed_surface_runtime_fields_are_temporarily_allowlisted_by_path() {
    let root = repo_root();
    let entries = repo_text_entries(&root);

    assert_forbidden_terms_are_allowlisted(
        "typed surface field",
        &entries,
        FORBIDDEN_TYPED_SURFACE_FIELDS,
        TEMPORARY_TYPED_FIELD_ALLOWLIST,
        |path, source| {
            !TEST_HARNESS_PATHS.contains(&path)
                && !TYPED_SURFACE_FIELD_SCAN_SKIP_PATHS.contains(&path)
                && is_typed_surface_candidate(source)
        },
    );
}

#[test]
fn repository_text_entries_include_tracked_dot_config_surfaces() {
    let root = repo_root();
    let entries = repo_text_entries(&root);

    assert!(
        entries
            .iter()
            .any(|entry| entry.path.starts_with(".github/workflows/")),
        "namespace scan must cover tracked workflow files"
    );
    assert!(
        entries.iter().any(|entry| entry.path.starts_with(".env")),
        "namespace scan must cover tracked env example files"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.path == "contracts/src/ConfigurableCounter.sol"),
        "namespace scan must cover tracked Solidity files"
    );
    assert!(
        entries.iter().any(|entry| {
            entry.path
                == "crates/storages/stream-store-postgres/migrations/0001_typed_run_event_store.sql"
        }),
        "namespace scan must cover tracked SQL files"
    );
    assert!(
        entries
            .iter()
            .all(|entry| !entry.path.starts_with("migrations/")),
        "Postgres migrations must live under the owning storage crate"
    );
}

#[test]
fn docs_do_not_describe_synthetic_admission_as_executed_genesis_state() {
    let root = repo_root();
    let entries = repo_text_entries(&root);
    let forbidden = [
        "executes the sealed `BootstrapRun` genesis state",
        "executes the sealed BootstrapRun genesis state",
        "`BootstrapRun` remains certified",
        "BootstrapRun remains certified",
        "legacy bundled bootstrap evidence",
        "bundled bootstrap evidence",
    ];

    assert_forbidden_terms_are_allowlisted(
        "stale admission docs",
        &entries,
        &forbidden,
        &[],
        |path, _source| path != "tests/integration/tests/architecture_namespace_contract.rs",
    );
}

#[test]
fn repository_does_not_expose_raw_prepared_commit_escape_hatch_names() {
    let root = repo_root();
    let entries = repo_text_entries(&root);
    let forbidden = [
        "PreparedTypedCommit",
        "append_prepared_typed_commit",
        "stage_prepared_typed_run_commit",
        "build_prepared_committed_batch",
        "prepared_commit_fingerprint",
        "into_typed_commit",
    ];

    assert_forbidden_terms_are_allowlisted(
        "raw prepared commit escape hatch",
        &entries,
        &forbidden,
        &[],
        |path, _source| path != "tests/integration/tests/architecture_namespace_contract.rs",
    );
}

#[test]
fn runtime_docs_assign_public_output_render_to_framework_lifecycle() {
    let root = repo_root();
    let entries = repo_text_entries(&root);
    let forbidden = [
        "`attempt`: ordinary and public-output render attempt lifecycle",
        "`framework_lifecycle`: started-before-run framework attempt lifecycle for retention and terminal states",
    ];

    assert_forbidden_terms_are_allowlisted(
        "runtime framework lifecycle docs",
        &entries,
        &forbidden,
        &[],
        |path, _source| path == "crates/kernel/runtime/README.md",
    );

    let readme =
        fs::read_to_string(root.join("crates/kernel/runtime/README.md")).expect("runtime README");
    assert!(
        readme.contains(
            "`framework_lifecycle`: started-before-run framework attempt lifecycle for public-output rendering"
        ),
        "runtime README must assign public-output rendering to framework_lifecycle"
    );
}

#[test]
fn design_docs_do_not_make_public_output_projected_a_transition_decision() {
    let root = repo_root();
    let design = fs::read_to_string(root.join("docs/design.md")).expect("design doc");
    let forbidden = [
        "report projected public output completion",
        "projected public output completion",
        "projected public output is a frontier transition decision",
        "report blocked completion",
    ];

    for phrase in forbidden {
        assert!(
            !design.contains(phrase),
            "docs/design.md must not describe public-output projected status as a transition decision: {phrase}"
        );
    }
    assert!(
        design.contains("already projected public output is a scheduler facade status"),
        "docs/design.md must scope already-projected public output to scheduler facade status"
    );
}

#[test]
fn evm_contract_lifecycle_runners_live_in_adapter_not_app() {
    let root = repo_root();
    let app = fs::read_to_string(root.join("crates/app/src/evm_contracts.rs"))
        .expect("app evm contracts source");
    let adapter = fs::read_to_string(root.join("crates/adapters/evm-contracts/src/lib.rs"))
        .expect("evm contract adapter source");

    for forbidden in [
        "impl ErasedNodeRunner",
        "ErasedRunnerBinding",
        "register_capability_set",
        "ContractMutationRunner",
        "ContractValidateRunner",
        "run_deploy_mutation",
        "run_configure_mutation",
        "run_validate",
        "side_effect_prepare",
        "mfm-app-evm-contract-lifecycle",
    ] {
        assert!(
            !app.contains(forbidden),
            "app EVM wiring must not own contract lifecycle runner behavior: {forbidden}"
        );
    }

    for required in [
        "pub fn register_contract_lifecycle_runners_with_factory",
        "impl ErasedNodeRunner for ContractMutationRunner",
        "impl ErasedNodeRunner for ContractValidateRunner",
        "mfm-adapters-evm-contracts-built-in",
    ] {
        assert!(
            adapter.contains(required),
            "EVM contract adapter must own runner behavior: {required}"
        );
    }

    for forbidden in [
        "FsTypedArtifactStore",
        "EvmJsonRpcClient",
        "KeystoreSignerProvider",
        "KeystoreSignerRegistryEntry",
        "MFM_EVM_SIGNERS_JSON",
        "RuntimeSignerConfig",
        "from_env_sources",
    ] {
        assert!(
            !adapter.contains(forbidden),
            "EVM contract adapter must not own concrete process wiring: {forbidden}"
        );
    }
}

#[test]
fn namespace_guard_rejects_synthetic_stale_names() {
    let forbidden_terms = forbidden_public_name_terms();
    let stale_route_base = format!("/v1/evm/{}", stale_recipe_abbrev());
    let stale_route = format!("{stale_route_base}/deploy");
    let entries = vec![TextEntry {
        path: "crates/new-public-surface/src/lib.rs".to_owned(),
        source: format!("const ROUTE: &str = \"{stale_route}\";"),
    }];

    let error = forbidden_term_report(
        "synthetic namespace",
        &entries,
        &forbidden_terms,
        &[],
        |_path, _source| true,
    )
    .expect_err("synthetic stale name must fail");

    assert!(
        error.contains("crates/new-public-surface/src/lib.rs") && error.contains(&stale_route_base),
        "unexpected error: {error}"
    );
}

#[test]
fn namespace_guard_rejects_synthetic_stale_path_names() {
    let forbidden_terms = forbidden_public_name_terms();
    let stale_path = format!("crates/transports/evm-{}/src/lib.rs", stale_recipe_abbrev());
    let entries = vec![TextEntry {
        path: stale_path.clone(),
        source: "pub struct ContractTransport;".to_owned(),
    }];

    let error = forbidden_term_report(
        "synthetic namespace",
        &entries,
        &forbidden_terms,
        &[],
        |_path, _source| true,
    )
    .expect_err("synthetic stale path must fail");

    assert!(error.contains(&stale_path), "unexpected error: {error}");
}

#[test]
fn typed_surface_guard_rejects_synthetic_runtime_fields() {
    let entries = vec![TextEntry {
        path: "crates/new-config/src/lib.rs".to_owned(),
        source: "#[derive(MfmConfig)] struct Bad { rpc_url: String }".to_owned(),
    }];

    let error = forbidden_term_report(
        "synthetic typed field",
        &entries,
        FORBIDDEN_TYPED_SURFACE_FIELDS,
        &[],
        |_path, source| is_typed_surface_candidate(source),
    )
    .expect_err("synthetic runtime field must fail");

    assert!(
        error.contains("crates/new-config/src/lib.rs") && error.contains("rpc_url"),
        "unexpected error: {error}"
    );
}

#[test]
fn typed_surface_guard_rejects_extra_runtime_field_in_allowlisted_path() {
    let entries = vec![TextEntry {
        path: "crates/transports/runtime-fixture/src/lib.rs".to_owned(),
        source: "#[derive(MfmValue)] struct Bad { rpc_url: String, another_rpc_url: String }"
            .to_owned(),
    }];

    let error = forbidden_term_report(
        "synthetic typed field",
        &entries,
        &["rpc_url"],
        &[("crates/transports/runtime-fixture/src/lib.rs", "rpc_url", 1)],
        |_path, source| is_typed_surface_candidate(source),
    )
    .expect_err("extra runtime field in allowlisted path must fail");

    assert!(
        error.contains("expected=1") && error.contains("actual=2"),
        "unexpected error: {error}"
    );
}

#[derive(Debug)]
struct TextEntry {
    path: String,
    source: String,
}

fn stale_recipe_abbrev() -> String {
    ["d", "cv"].concat()
}

fn forbidden_public_name_terms() -> Vec<String> {
    let lower = stale_recipe_abbrev();
    let upper = lower.to_ascii_uppercase();
    let title = lower
        .chars()
        .enumerate()
        .flat_map(|(index, ch)| {
            if index == 0 {
                ch.to_uppercase().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect::<String>();
    let deploy = "deploy";
    let configure = "configure";
    let validate = "validate";
    let operation_words = [deploy, configure, validate];

    vec![
        lower.clone(),
        upper,
        title.clone(),
        format!("Evm{title}"),
        format!("evm_{lower}"),
        format!("evm-{lower}"),
        format!("mfm.evm.{lower}"),
        format!("/v1/evm/{lower}"),
        format!("typed-evm-{lower}"),
        ["Deploy", "Configure", "Validate"].concat(),
        operation_words.join("_"),
        operation_words.join("-"),
        format!("mfm-transports-evm-{lower}"),
    ]
}

fn assert_forbidden_terms_are_allowlisted<T: AsRef<str>>(
    label: &str,
    entries: &[TextEntry],
    terms: &[T],
    allowlist: &[(&str, &str, usize)],
    include: impl Fn(&str, &str) -> bool,
) {
    if let Err(error) = forbidden_term_report(label, entries, terms, allowlist, include) {
        panic!("{error}");
    }
}

fn forbidden_term_report<T: AsRef<str>>(
    label: &str,
    entries: &[TextEntry],
    terms: &[T],
    allowlist: &[(&str, &str, usize)],
    include: impl Fn(&str, &str) -> bool,
) -> Result<(), String> {
    let expected = allowlist
        .iter()
        .map(|(path, term, count)| (((*path).to_owned(), (*term).to_owned()), *count))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeMap::<(String, String), usize>::new();

    for entry in entries {
        if !include(&entry.path, &entry.source) {
            continue;
        }

        for (term, count) in term_counts_in_entry(entry, terms) {
            actual.insert((entry.path.clone(), term), count);
        }
    }

    let unexpected = actual
        .iter()
        .filter(|(hit, _count)| !expected.contains_key(hit))
        .map(|((path, term), count)| format!("{path} [{term} actual={count}]"))
        .collect::<Vec<_>>();
    if !unexpected.is_empty() {
        return Err(format!(
            "{label} guard found unallowlisted terms: {}",
            unexpected.join("; ")
        ));
    }

    let stale = expected
        .keys()
        .filter(|hit| !actual.contains_key(hit))
        .map(|(path, term)| format!("{path} [{term}]"))
        .collect::<Vec<_>>();
    if !stale.is_empty() {
        return Err(format!(
            "{label} guard has stale allowlist hits: {}",
            stale.join(", ")
        ));
    }

    let mismatched = expected
        .iter()
        .filter_map(|(hit, expected_count)| {
            let actual_count = actual.get(hit)?;
            (actual_count != expected_count).then(|| {
                format!(
                    "{} [{} expected={} actual={}]",
                    hit.0, hit.1, expected_count, actual_count
                )
            })
        })
        .collect::<Vec<_>>();
    if !mismatched.is_empty() {
        return Err(format!(
            "{label} guard found allowlist count mismatches: {}",
            mismatched.join("; ")
        ));
    }

    Ok(())
}

fn term_counts_in_entry<T: AsRef<str>>(entry: &TextEntry, terms: &[T]) -> Vec<(String, usize)> {
    terms
        .iter()
        .filter_map(|term| {
            let term = term.as_ref();
            let count = entry.path.matches(term).count() + entry.source.matches(term).count();
            (count > 0).then_some((term.to_owned(), count))
        })
        .collect()
}

fn is_typed_surface_candidate(source: &str) -> bool {
    [
        "MfmConfig",
        "MfmValue",
        "PublicOutputs",
        "OperationOutput",
        "StateInput",
    ]
    .iter()
    .any(|marker| source.contains(marker))
}

fn repo_text_entries(root: &Path) -> Vec<TextEntry> {
    let tracked_paths = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .ok()
        .and_then(|output| {
            output.status.success().then(|| {
                output
                    .stdout
                    .split(|byte| *byte == 0)
                    .filter(|raw| !raw.is_empty())
                    .map(|raw| {
                        std::str::from_utf8(raw)
                            .unwrap_or_else(|error| panic!("tracked path is not utf-8: {error}"))
                            .to_owned()
                    })
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_else(|| repo_text_paths_from_materialized_tree(root));

    let mut entries = tracked_paths
        .into_iter()
        .filter_map(|rel| {
            let path = root.join(&rel);
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
                Err(error) => panic!("read tracked file {}: {error}", path.display()),
            };
            String::from_utf8(bytes).ok().map(|source| TextEntry {
                path: rel.replace('\\', "/"),
                source,
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

fn repo_text_paths_from_materialized_tree(root: &Path) -> Vec<String> {
    fn visit(root: &Path, dir: &Path, paths: &mut Vec<String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read directory {}: {error}", dir.display()))
            .map(|entry| {
                entry.unwrap_or_else(|error| {
                    panic!("read directory entry {}: {error}", dir.display())
                })
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());

        for entry in entries {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or_else(|error| {
                panic!("strip repository root from {}: {error}", path.display())
            });
            let rel = rel
                .to_str()
                .unwrap_or_else(|| panic!("repository path is not utf-8: {}", path.display()))
                .replace('\\', "/");

            if fallback_scan_skips_path(&rel) {
                continue;
            }

            let file_type = entry
                .file_type()
                .unwrap_or_else(|error| panic!("read file type {}: {error}", path.display()));
            if file_type.is_dir() {
                visit(root, &path, paths);
            } else if file_type.is_file() {
                paths.push(rel);
            }
        }
    }

    let mut paths = Vec::new();
    visit(root, root, &mut paths);
    paths.sort();
    paths
}

fn fallback_scan_skips_path(rel: &str) -> bool {
    rel == ".git"
        || rel.starts_with(".git/")
        || rel == "target"
        || rel.starts_with("target/")
        || rel == "result"
        || rel.starts_with("result-")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("integration crate lives under tests/integration")
        .to_path_buf()
}
