use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TEST_HARNESS_PATHS: &[&str] = &["tests/integration/tests/architecture_namespace_contract.rs"];
const SOURCE_OF_TRUTH_DOC_PATHS: &[&str] = &[
    "PLAN_IMPL_RFC_PG_TRANS.md",
    "RFC_REFAC_PG_TRANS.md",
    "docs/persisted-public-surfaces.md",
];
const FORBIDDEN_SEMANTIC_SURFACE_FIELDS: &[&str] = &[
    "rpc_url",
    "authorization",
    "keystore_path_env",
    "password_file_env",
    "password",
    "private_key",
    "mnemonic",
    "raw_transaction",
];

const SEMANTIC_SURFACE_FIELD_SCAN_SKIP_PATHS: &[&str] = &[
    "PLAN_IMPL_RFC_PG_TRANS.md",
    "RFC_REFAC_PG_TRANS.md",
    "crates/kernel/values/src/lib.rs",
    "crates/kernel/values/src/tests.rs",
];

const SEMANTIC_FIELD_EXCEPTION_COUNTS: &[(&str, &str, usize)] = &[
    ("crates/portfolio/model/src/portfolio.rs", "password", 1),
    ("crates/portfolio/model/src/portfolio.rs", "mnemonic", 2),
    ("crates/portfolio/model/src/symbol.rs", "authorization", 2),
    ("crates/kernel/certify/src/lib.rs", "authorization", 79),
    ("crates/kernel/program/src/lib.rs", "authorization", 15),
    ("crates/kernel/runtime/src/tests.rs", "authorization", 10),
    ("crates/ops/proof-op/src/lib.rs", "authorization", 5),
];

#[test]
fn semantic_surface_runtime_fields_match_tracked_exception_counts() {
    let root = repo_root();
    let entries = repo_text_entries(&root);

    assert_forbidden_terms_are_allowlisted(
        "semantic surface runtime field",
        &entries,
        FORBIDDEN_SEMANTIC_SURFACE_FIELDS,
        SEMANTIC_FIELD_EXCEPTION_COUNTS,
        |path, source| {
            !TEST_HARNESS_PATHS.contains(&path)
                && !SEMANTIC_SURFACE_FIELD_SCAN_SKIP_PATHS.contains(&path)
                && is_semantic_surface_candidate(source)
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
            entry.path == "crates/storages/stream-store-postgres/migrations/0001_run_store.sql"
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
fn persisted_public_surface_inventory_covers_postgres_cutover_surfaces() {
    let root = repo_root();
    let inventory = fs::read_to_string(root.join("docs/persisted-public-surfaces.md"))
        .expect("persisted/public surface inventory");

    for required in [
        "Artifact bytes",
        "Artifact evidence",
        "Event canonical bytes",
        "Resource lane authority rows",
        "Observation summaries",
        "Observation provenance",
        "Cursor metadata",
        "Store metadata",
        "Run list/watch output",
        "Run status output",
        "Run stream output",
        "Public-output rendering",
        "Keystore list output",
    ] {
        assert!(
            inventory.contains(required),
            "persisted/public surface inventory must cover `{required}`"
        );
    }

    for classification in [
        "strict authority",
        "observation",
        "rebuildable cache",
        "operational telemetry",
        "public output",
    ] {
        assert!(
            inventory.contains(classification),
            "persisted/public surface inventory must define/use `{classification}`"
        );
    }

    for secret_boundary in [
        "No secrets allowed",
        "`cursor_secret` is secret storage metadata",
        "must never be logged or returned",
        "must not expose commit ids, append XIDs, sort keys, cursor versions, key ids, or store",
    ] {
        assert!(
            inventory.contains(secret_boundary),
            "persisted/public surface inventory must state secret/public boundary `{secret_boundary}`"
        );
    }
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
        "const READ_FACTORY: &str = \"read_external\"",
        "const SIDE_EFFECT_FACTORY: &str = \"apply_side_effect\"",
    ] {
        assert!(
            adapter.contains(required),
            "EVM contract adapter must own runner behavior: {required}"
        );
    }

    for forbidden in [
        concat!("artifact", "_store", "_fs"),
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
fn postgres_migrations_do_not_reintroduce_removed_storage_surfaces() {
    let root = repo_root();
    let migration_dir = root.join("crates/storages/stream-store-postgres/migrations");
    let mut migrations = fs::read_dir(&migration_dir)
        .unwrap_or_else(|error| panic!("read migration dir {}: {error}", migration_dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("read migration dir entry: {error}"))
                .path()
        })
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sql"))
        .collect::<Vec<_>>();
    migrations.sort();

    for path in migrations {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read migration {}: {error}", path.display()));
        for forbidden in [
            "typed_",
            "payload_json JSONB",
            "jsonb::text",
            "commit_pos",
            "change_pos",
            "CREATE SEQUENCE",
            "LOCK TABLE",
        ] {
            assert!(
                !source.contains(forbidden),
                "Postgres migrations must not reintroduce removed RFC storage surface `{forbidden}` in {}",
                path.display()
            );
        }
    }

    let baseline =
        fs::read_to_string(migration_dir.join("0001_run_store.sql")).expect("baseline migration");
    for required in [
        "CREATE TABLE run_events",
        "CONSTRAINT artifact_blobs_byte_len_max CHECK (byte_len <= 16777216)",
        "CREATE TABLE run_commit_log",
        "CONSTRAINT run_commit_log_sort_key_v1_length CHECK (octet_length(commit_sort_key) = 32)",
        "CONSTRAINT run_commit_log_sort_key_v1_prefix CHECK (get_byte(commit_sort_key, 0) = 1)",
        "CREATE TABLE run_observation_cursors",
    ] {
        assert!(
            baseline.contains(required),
            "baseline migration must keep RFC target storage surface `{required}`"
        );
    }
}

#[test]
fn postgres_migrations_do_not_claim_maintenance_role_boundaries() {
    let root = repo_root();
    let migration_dir = root.join("crates/storages/stream-store-postgres/migrations");
    let mut migrations = fs::read_dir(&migration_dir)
        .unwrap_or_else(|error| panic!("read migration dir {}: {error}", migration_dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("read migration dir entry: {error}"))
                .path()
        })
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sql"))
        .collect::<Vec<_>>();
    migrations.sort();

    for path in migrations {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read migration {}: {error}", path.display()));
        for forbidden in [
            "mfm_allow_orphan_artifact_blob_delete_only",
            "artifact_blobs_orphan_delete_only",
            "maintenance_artifact_blob_sweep",
            "reseed_store_epoch",
            "DISABLE TRIGGER",
            "ENABLE TRIGGER",
        ] {
            assert!(
                !source.contains(forbidden),
                "Postgres migrations must not claim underdesigned maintenance boundary `{forbidden}` in {}",
                path.display()
            );
        }
    }

    let baseline =
        fs::read_to_string(migration_dir.join("0001_run_store.sql")).expect("baseline migration");
    for required in [
        "CREATE TRIGGER artifact_blobs_no_update",
        "BEFORE UPDATE OR DELETE OR TRUNCATE ON artifact_blobs",
    ] {
        assert!(
            baseline.contains(required),
            "baseline migration must keep append-owned artifact blob immutability guard `{required}`"
        );
    }
}

#[test]
fn postgres_production_code_rejects_removed_storage_shortcuts() {
    let root = repo_root();
    let entries = repo_text_entries(&root);
    let forbidden = [
        "CREATE TABLE typed_",
        "INSERT INTO typed_",
        "UPDATE typed_",
        "DELETE FROM typed_",
        "FROM typed_",
        "JOIN typed_",
        "payload_json JSONB",
        "jsonb::text",
        "commit_pos",
        "change_pos",
        "global_commit",
        "global_change",
    ];

    assert_forbidden_terms_are_allowlisted(
        "Postgres removed storage shortcut",
        &entries,
        &forbidden,
        &[],
        |path, _source| {
            path.starts_with("crates/storages/stream-store-postgres/")
                && !path.ends_with("/src/schema.rs")
                && !is_test_support_path(path)
        },
    );
}

#[test]
fn postgres_production_code_rejects_fake_maintenance_boundaries() {
    let root = repo_root();
    let entries = repo_text_entries(&root);
    let forbidden = [
        "PostgresMaintenance",
        "ReadModelBuildMode",
        "ReadModelBuildReport",
        "ReadModelHighWatermark",
        "ArtifactBlobSweepReport",
        "build_projection_version",
        "read_model_high_watermark",
        "reseed_store_epoch",
        "sweep_orphan",
        "maintenance_artifact_blob_sweep",
        "mfm_allow_orphan",
        "artifact_blobs_orphan",
        "precommit_large_artifact",
        "LARGE_ARTIFACT_PRECOMMIT",
    ];

    assert_forbidden_terms_are_allowlisted(
        "Postgres fake maintenance boundary",
        &entries,
        &forbidden,
        &[],
        |path, _source| {
            path.starts_with("crates/storages/stream-store-postgres/")
                && !TEST_HARNESS_PATHS.contains(&path)
                && !SOURCE_OF_TRUTH_DOC_PATHS.contains(&path)
                && !is_test_support_path(path)
                && !path.contains("/.sqlx/")
                && !path.ends_with(".md")
        },
    );

    let mut trigger_offenders = Vec::new();
    for entry in entries {
        if !entry
            .path
            .starts_with("crates/storages/stream-store-postgres/src/")
            || is_test_support_path(&entry.path)
        {
            continue;
        }

        for term in ["DISABLE TRIGGER", "ENABLE TRIGGER"] {
            if entry.source.contains(term)
                && !all_term_occurrences_after_cfg_test_module(&entry.source, term)
            {
                trigger_offenders.push(format!("{} [{term}]", entry.path));
            }
        }
    }

    assert!(
        trigger_offenders.is_empty(),
        "Postgres trigger bypasses must stay test-only corruption fixtures; offenders: {}",
        trigger_offenders.join(", ")
    );
}

#[test]
fn postgres_sqlx_metadata_is_checked_in_and_wired_to_gates() {
    let root = repo_root();
    let sqlx_dir = root.join("crates/storages/stream-store-postgres/.sqlx");
    let sqlx_files = fs::read_dir(&sqlx_dir)
        .unwrap_or_else(|error| panic!("read sqlx metadata dir {}: {error}", sqlx_dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("read sqlx metadata dir entry: {error}"))
                .path()
        })
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    assert!(
        !sqlx_files.is_empty(),
        "Postgres storage must keep checked-in .sqlx query metadata"
    );

    let nixfied = fs::read_to_string(root.join("nixfied.nix")).expect("nixfied model");
    let check_start = nixfied
        .find("    check = {")
        .expect("check composite in nixfied model");
    let test_start = nixfied[check_start..]
        .find("\n    test = {")
        .expect("test composite after check composite")
        + check_start;
    let check_block = &nixfied[check_start..test_start];
    assert!(
        check_block.contains("\"postgres-sqlx-offline-check\""),
        ".#check must compile Postgres storage against checked-in SQLx metadata"
    );

    let offline_start = nixfied
        .find("    postgres-sqlx-offline-check = cargoLeaf")
        .expect("offline SQLx metadata task");
    let live_start = nixfied
        .find("    postgres-sqlx-check = cargoLeaf")
        .expect("live SQLx prepare task");
    let offline_block = &nixfied[offline_start..live_start];
    for required in [
        "SQLX_OFFLINE = \"true\";",
        "DATABASE_URL = \"postgresql://offline/offline\";",
        "\"mfm-stream-store-postgres\"",
        "\"--features\"",
        "\"parity-tests\"",
        "\"--all-targets\"",
    ] {
        assert!(
            offline_block.contains(required),
            "offline SQLx metadata task must keep `{required}`"
        );
    }

    let test_db_start = nixfied
        .find("    test-db = {")
        .expect("test-db composite in nixfied model");
    let ci_start = nixfied[test_db_start..]
        .find("\n    # The full gate")
        .expect("ci comment after test-db composite")
        + test_db_start;
    let test_db_block = &nixfied[test_db_start..ci_start];
    assert!(
        test_db_block.contains("postgres-sqlx-check.task = \"postgres-sqlx-check\""),
        ".#test-db must run the live SQLx prepare check before Postgres parity tests"
    );

    let live_block = &nixfied[live_start..test_db_start];
    for required in [
        "cargo sqlx prepare --check -- --all-targets --features parity-tests",
        "env = postgresSqlxEnv;",
        "requires = [ \"postgres\" ];",
    ] {
        assert!(
            live_block.contains(required),
            "live SQLx prepare task must keep `{required}`"
        );
    }
    assert!(
        nixfied.contains("SQLX_OFFLINE = \"false\";"),
        "live SQLx prepare env must keep SQLX_OFFLINE=false"
    );
}

#[test]
fn in_memory_run_store_is_test_support_only() {
    let root = repo_root();
    let store_path = "crates/kernel/store/src/lib.rs";
    let store_source = fs::read_to_string(root.join(store_path)).expect("store source");
    assert!(
        store_source.contains("#[cfg(any(test, feature = \"test-support\"))]\n    #[derive(Debug, Clone, Default)]\n    pub struct AsyncInMemoryRunStore"),
        "AsyncInMemoryRunStore must remain cfg-gated to tests or the test-support feature"
    );

    let lines = store_source.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !(trimmed.contains("pub struct AsyncInMemoryRunStore")
            || (trimmed.starts_with("impl ") && trimmed.contains("AsyncInMemoryRunStore")))
        {
            continue;
        }

        let context = lines[index.saturating_sub(3)..index].join("\n");
        assert!(
            context.contains("#[cfg(any(test, feature = \"test-support\"))]"),
            "{store_path}:{} exposes AsyncInMemoryRunStore without the required cfg",
            index + 1
        );
    }

    let entries = repo_text_entries(&root);
    let mut offenders = Vec::new();
    for entry in entries {
        if TEST_HARNESS_PATHS.contains(&entry.path.as_str())
            || entry.path == store_path
            || is_test_support_path(&entry.path)
            || !entry.source.contains("AsyncInMemoryRunStore")
        {
            continue;
        }

        if !all_term_occurrences_after_cfg_test_module(&entry.source, "AsyncInMemoryRunStore") {
            offenders.push(entry.path);
        }
    }

    assert!(
        offenders.is_empty(),
        "AsyncInMemoryRunStore references must be limited to tests/test-support; offenders: {}",
        offenders.join(", ")
    );
}

#[test]
fn public_observation_surfaces_do_not_expose_postgres_cursor_internals() {
    let root = repo_root();
    let entries = repo_text_entries(&root);
    let forbidden = [
        "cursor_key_id",
        "store_epoch",
        "append_xid",
        "commit_sort_key",
        "CURSOR_VERSION",
        "run_observation_cursors",
    ];

    assert_forbidden_terms_are_allowlisted(
        "public observation cursor internal",
        &entries,
        &forbidden,
        &[],
        |path, _source| {
            !TEST_HARNESS_PATHS.contains(&path)
                && !SOURCE_OF_TRUTH_DOC_PATHS.contains(&path)
                && !is_test_support_path(path)
                && !path.starts_with("crates/storages/stream-store-postgres/")
                && (path.starts_with("bin/")
                    || path.starts_with("crates/")
                    || path.starts_with("docs/")
                    || path == "README.md")
        },
    );
}

#[test]
fn artifact_blob_table_access_stays_inside_postgres_storage() {
    let root = repo_root();
    let entries = repo_text_entries(&root);

    assert_forbidden_terms_are_allowlisted(
        "external artifact blob table",
        &entries,
        &["artifact_blobs"],
        &[],
        |path, _source| {
            !TEST_HARNESS_PATHS.contains(&path)
                && !SOURCE_OF_TRUTH_DOC_PATHS.contains(&path)
                && !is_test_support_path(path)
                && !path.starts_with("crates/storages/stream-store-postgres/")
        },
    );
}

#[test]
fn jsonb_text_canonicalization_shortcuts_are_not_used() {
    let root = repo_root();
    let entries = repo_text_entries(&root);

    assert_forbidden_terms_are_allowlisted(
        "jsonb text canonicalization shortcut",
        &entries,
        &["jsonb::text", "payload_json JSONB"],
        &[],
        |path, _source| {
            !TEST_HARNESS_PATHS.contains(&path) && !SOURCE_OF_TRUTH_DOC_PATHS.contains(&path)
        },
    );
}

#[test]
fn semantic_surface_guard_rejects_synthetic_runtime_fields() {
    let entries = vec![TextEntry {
        path: "crates/new-config/src/lib.rs".to_owned(),
        source: "#[derive(MfmConfig)] struct Bad { rpc_url: String }".to_owned(),
    }];

    let error = forbidden_term_report(
        "synthetic typed field",
        &entries,
        FORBIDDEN_SEMANTIC_SURFACE_FIELDS,
        &[],
        |_path, source| is_semantic_surface_candidate(source),
    )
    .expect_err("synthetic runtime field must fail");

    assert!(
        error.contains("crates/new-config/src/lib.rs") && error.contains("rpc_url"),
        "unexpected error: {error}"
    );
}

#[test]
fn semantic_surface_guard_rejects_extra_runtime_field_in_allowlisted_path() {
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
        |_path, source| is_semantic_surface_candidate(source),
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

fn is_semantic_surface_candidate(source: &str) -> bool {
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

fn is_test_support_path(path: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.ends_with("/src/tests.rs")
        || path.ends_with("/test_support.rs")
        || path.contains("/test_support/")
}

fn all_term_occurrences_after_cfg_test_module(source: &str, term: &str) -> bool {
    let lines = source.lines().collect::<Vec<_>>();
    let cfg_test_module_line = lines.windows(2).position(|window| {
        let attr = window[0].trim();
        attr.starts_with("#[cfg(")
            && attr.contains("test")
            && window[1].trim_start().starts_with("mod tests")
    });

    let Some(cfg_test_module_line) = cfg_test_module_line else {
        return false;
    };

    lines
        .iter()
        .enumerate()
        .filter(|(_index, line)| line.contains(term))
        .all(|(index, _line)| index > cfg_test_module_line)
}

fn repo_text_entries(root: &Path) -> Vec<TextEntry> {
    let repo_paths = git_ls_files(root, &["ls-files", "-z"]).map(|tracked_paths| {
        let mut paths = tracked_paths.into_iter().collect::<BTreeSet<_>>();
        if let Some(untracked_paths) =
            git_ls_files(root, &["ls-files", "--others", "--exclude-standard", "-z"])
        {
            paths.extend(untracked_paths);
        }
        paths.into_iter().collect::<Vec<_>>()
    });

    let paths = repo_paths.unwrap_or_else(|| repo_text_paths_from_materialized_tree(root));

    let mut entries = paths
        .into_iter()
        .filter_map(|rel| {
            let path = root.join(&rel);
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
                Err(error) => panic!("read repository file {}: {error}", path.display()),
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

fn git_ls_files(root: &Path, args: &[&str]) -> Option<Vec<String>> {
    Command::new("git")
        .args(args)
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
