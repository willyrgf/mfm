use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const HISTORICAL_ARCHITECTURE_DOCS: &[&str] = &["PLAN_IMPL_PROBLEM_ARCH.md", "PROBLEM_ARCH_DCV.md"];

const SCAN_SKIP_PATHS: &[&str] = &["tests/integration/tests/architecture_namespace_contract.rs"];

const FORBIDDEN_PUBLIC_NAME_TERMS: &[&str] = &[
    "dcv",
    "DCV",
    "Dcv",
    "EvmDcv",
    "evm_dcv",
    "evm-dcv",
    "mfm.evm.dcv",
    "/v1/evm/dcv",
    "typed-evm-dcv",
    "DeployConfigureValidate",
    "deploy_configure_validate",
    "deploy-configure-validate",
    "mfm-transports-evm-dcv",
];

const TEMPORARY_PUBLIC_NAME_ALLOWLIST: &[(&str, &str, usize)] = &[
    ("Cargo.lock", "dcv", 10),
    ("Cargo.lock", "evm-dcv", 10),
    ("Cargo.lock", "deploy-configure-validate", 9),
    ("Cargo.lock", "mfm-transports-evm-dcv", 3),
    ("Cargo.toml", "dcv", 3),
    ("Cargo.toml", "evm-dcv", 3),
    ("Cargo.toml", "deploy-configure-validate", 2),
    ("bin/cli/Cargo.toml", "deploy-configure-validate", 2),
    ("bin/cli/README.md", "dcv", 4),
    ("bin/cli/README.md", "DCV", 1),
    ("bin/cli/README.md", "deploy-configure-validate", 1),
    ("bin/cli/src/commands/evm/dcv.rs", "dcv", 17),
    ("bin/cli/src/commands/evm/dcv.rs", "DCV", 2),
    ("bin/cli/src/commands/evm/dcv.rs", "Dcv", 30),
    ("bin/cli/src/commands/evm/dcv.rs", "EvmDcv", 10),
    ("bin/cli/src/commands/evm/dcv.rs", "evm_dcv", 4),
    (
        "bin/cli/src/commands/evm/dcv.rs",
        "DeployConfigureValidate",
        10,
    ),
    (
        "bin/cli/src/commands/evm/dcv.rs",
        "deploy_configure_validate",
        7,
    ),
    (
        "bin/cli/src/commands/evm/dcv.rs",
        "deploy-configure-validate",
        1,
    ),
    ("bin/cli/src/commands/evm/mod.rs", "dcv", 2),
    ("bin/cli/src/commands/evm/mod.rs", "DCV", 1),
    ("bin/cli/src/commands/evm/mod.rs", "Dcv", 3),
    ("bin/rest-api/Cargo.toml", "deploy-configure-validate", 2),
    ("bin/rest-api/README.md", "dcv", 4),
    ("bin/rest-api/README.md", "DCV", 1),
    ("bin/rest-api/README.md", "/v1/evm/dcv", 4),
    ("bin/rest-api/README.md", "deploy-configure-validate", 1),
    ("bin/rest-api/src/lib.rs", "dcv", 39),
    ("bin/rest-api/src/lib.rs", "Dcv", 42),
    ("bin/rest-api/src/lib.rs", "EvmDcv", 36),
    ("bin/rest-api/src/lib.rs", "evm_dcv", 23),
    ("bin/rest-api/src/lib.rs", "/v1/evm/dcv", 4),
    ("bin/rest-api/src/lib.rs", "DeployConfigureValidate", 8),
    ("bin/rest-api/src/lib.rs", "deploy_configure_validate", 5),
    ("bin/rest-api/src/lib.rs", "deploy-configure-validate", 1),
    ("crates/app/Cargo.toml", "dcv", 2),
    ("crates/app/Cargo.toml", "evm-dcv", 2),
    ("crates/app/Cargo.toml", "deploy-configure-validate", 2),
    ("crates/app/Cargo.toml", "mfm-transports-evm-dcv", 1),
    ("crates/app/src/lib.rs", "dcv", 8),
    ("crates/app/src/lib.rs", "Dcv", 1),
    ("crates/app/src/lib.rs", "EvmDcv", 1),
    ("crates/app/src/lib.rs", "evm_dcv", 7),
    ("crates/app/src/lib.rs", "deploy_configure_validate", 1),
    ("crates/app/tests/typed_transport_boundaries.rs", "dcv", 2),
    ("crates/app/tests/typed_transport_boundaries.rs", "Dcv", 2),
    (
        "crates/app/tests/typed_transport_boundaries.rs",
        "EvmDcv",
        2,
    ),
    (
        "crates/app/tests/typed_transport_boundaries.rs",
        "evm-dcv",
        2,
    ),
    ("crates/docs/README.md", "dcv", 6),
    ("crates/docs/README.md", "evm-dcv", 6),
    ("crates/docs/README.md", "deploy-configure-validate", 4),
    ("crates/docs/README.md", "mfm-transports-evm-dcv", 1),
    ("crates/docs/catalog.toml", "dcv", 6),
    ("crates/docs/catalog.toml", "evm-dcv", 6),
    ("crates/docs/catalog.toml", "deploy-configure-validate", 4),
    ("crates/docs/catalog.toml", "mfm-transports-evm-dcv", 1),
    ("crates/evm-dcv-model/Cargo.toml", "dcv", 1),
    ("crates/evm-dcv-model/Cargo.toml", "evm-dcv", 1),
    ("crates/evm-dcv-model/README.md", "dcv", 1),
    ("crates/evm-dcv-model/README.md", "DCV", 1),
    ("crates/evm-dcv-model/README.md", "evm-dcv", 1),
    ("crates/evm-dcv-model/src/lib.rs", "dcv", 34),
    ("crates/evm-dcv-model/src/lib.rs", "Dcv", 1),
    ("crates/evm-dcv-model/src/lib.rs", "evm_dcv", 2),
    ("crates/evm-dcv-model/src/lib.rs", "mfm.evm.dcv", 32),
    (
        "crates/evm-deploy-configure-validate-config/Cargo.toml",
        "dcv",
        2,
    ),
    (
        "crates/evm-deploy-configure-validate-config/Cargo.toml",
        "evm-dcv",
        2,
    ),
    (
        "crates/evm-deploy-configure-validate-config/Cargo.toml",
        "deploy-configure-validate",
        1,
    ),
    (
        "crates/evm-deploy-configure-validate-config/src/lib.rs",
        "dcv",
        21,
    ),
    (
        "crates/evm-deploy-configure-validate-config/src/lib.rs",
        "evm_dcv",
        1,
    ),
    (
        "crates/evm-deploy-configure-validate-config/src/lib.rs",
        "mfm.evm.dcv",
        20,
    ),
    (
        "crates/evm-deploy-configure-validate-config/src/lib.rs",
        "DeployConfigureValidate",
        91,
    ),
    (
        "crates/evm-deploy-configure-validate-config/src/lib.rs",
        "deploy_configure_validate",
        50,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/Cargo.toml",
        "dcv",
        4,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/Cargo.toml",
        "evm-dcv",
        4,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/Cargo.toml",
        "deploy-configure-validate",
        3,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/README.md",
        "DCV",
        1,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/README.md",
        "deploy-configure-validate",
        1,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "dcv",
        72,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "DCV",
        21,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "Dcv",
        67,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "EvmDcv",
        18,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "evm_dcv",
        12,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "mfm.evm.dcv",
        10,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "DeployConfigureValidate",
        47,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "deploy_configure_validate",
        14,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/tests/compile_fail.rs",
        "deploy-configure-validate",
        1,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/tests/ui/fail/validate_before_configure.rs",
        "DeployConfigureValidate",
        4,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/tests/ui/fail/validate_before_configure.rs",
        "deploy_configure_validate",
        1,
    ),
    ("crates/states/evm-dcv/Cargo.toml", "dcv", 3),
    ("crates/states/evm-dcv/Cargo.toml", "evm-dcv", 3),
    (
        "crates/states/evm-dcv/Cargo.toml",
        "deploy-configure-validate",
        2,
    ),
    ("crates/states/evm-dcv/README.md", "dcv", 2),
    ("crates/states/evm-dcv/README.md", "evm-dcv", 2),
    (
        "crates/states/evm-dcv/README.md",
        "mfm-transports-evm-dcv",
        1,
    ),
    ("crates/states/evm-dcv/src/lib.rs", "dcv", 74),
    ("crates/states/evm-dcv/src/lib.rs", "DCV", 7),
    ("crates/states/evm-dcv/src/lib.rs", "Dcv", 99),
    ("crates/states/evm-dcv/src/lib.rs", "EvmDcv", 91),
    ("crates/states/evm-dcv/src/lib.rs", "evm_dcv", 14),
    ("crates/states/evm-dcv/src/lib.rs", "evm-dcv", 2),
    ("crates/states/evm-dcv/src/lib.rs", "mfm.evm.dcv", 56),
    ("crates/states/evm-dcv/src/lib.rs", "typed-evm-dcv", 2),
    (
        "crates/states/evm-dcv/src/lib.rs",
        "DeployConfigureValidate",
        26,
    ),
    (
        "crates/states/evm-dcv/src/lib.rs",
        "deploy_configure_validate",
        1,
    ),
    ("crates/states/evm-dcv/tests/compile_fail.rs", "dcv", 1),
    ("crates/states/evm-dcv/tests/compile_fail.rs", "evm-dcv", 1),
    (
        "crates/states/evm-dcv/tests/ui/fail/protected_raw_transaction_value.rs",
        "dcv",
        1,
    ),
    (
        "crates/states/evm-dcv/tests/ui/fail/protected_raw_transaction_value.rs",
        "evm_dcv",
        1,
    ),
    (
        "crates/states/evm-dcv/tests/ui/fail/protected_raw_transaction_value.stderr",
        "dcv",
        2,
    ),
    (
        "crates/states/evm-dcv/tests/ui/fail/protected_raw_transaction_value.stderr",
        "evm_dcv",
        2,
    ),
    ("crates/transports/evm-dcv/Cargo.toml", "dcv", 3),
    ("crates/transports/evm-dcv/Cargo.toml", "evm-dcv", 3),
    (
        "crates/transports/evm-dcv/Cargo.toml",
        "deploy-configure-validate",
        2,
    ),
    (
        "crates/transports/evm-dcv/Cargo.toml",
        "mfm-transports-evm-dcv",
        1,
    ),
    ("crates/transports/evm-dcv/README.md", "dcv", 1),
    ("crates/transports/evm-dcv/README.md", "DCV", 1),
    ("crates/transports/evm-dcv/README.md", "evm-dcv", 1),
    (
        "crates/transports/evm-dcv/README.md",
        "mfm-transports-evm-dcv",
        1,
    ),
    ("crates/transports/evm-dcv/src/lib.rs", "dcv", 54),
    ("crates/transports/evm-dcv/src/lib.rs", "DCV", 36),
    ("crates/transports/evm-dcv/src/lib.rs", "Dcv", 196),
    ("crates/transports/evm-dcv/src/lib.rs", "EvmDcv", 196),
    ("crates/transports/evm-dcv/src/lib.rs", "evm_dcv", 43),
    ("crates/transports/evm-dcv/src/lib.rs", "evm-dcv", 5),
    ("crates/transports/evm-dcv/src/lib.rs", "mfm.evm.dcv", 6),
    ("crates/transports/evm-dcv/src/lib.rs", "typed-evm-dcv", 1),
    (
        "crates/transports/evm-dcv/src/lib.rs",
        "DeployConfigureValidate",
        20,
    ),
    (
        "crates/transports/evm-dcv/src/lib.rs",
        "deploy_configure_validate",
        1,
    ),
    (
        "crates/transports/evm-dcv/src/lib.rs",
        "mfm-transports-evm-dcv",
        4,
    ),
    ("docs/architecture.md", "DCV", 1),
    ("docs/docs-rs-readiness.md", "dcv", 1),
    ("docs/docs-rs-readiness.md", "DCV", 2),
    ("docs/docs-rs-readiness.md", "evm-dcv", 1),
    ("docs/evm-rpc-routing.md", "DCV", 2),
    ("docs/repo-index.json", "dcv", 6),
    ("docs/repo-index.json", "evm-dcv", 6),
    ("docs/repo-index.json", "deploy-configure-validate", 4),
    ("docs/repo-map.md", "dcv", 3),
    ("docs/repo-map.md", "evm-dcv", 3),
    ("docs/repo-map.md", "deploy-configure-validate", 2),
    ("nixfied/project/module.nix", "dcv", 2),
    ("nixfied/project/module.nix", "evm-dcv", 2),
    ("nixfied/project/module.nix", "mfm-transports-evm-dcv", 2),
    ("tests/integration/Cargo.toml", "dcv", 2),
    ("tests/integration/Cargo.toml", "evm-dcv", 2),
    (
        "tests/integration/Cargo.toml",
        "deploy-configure-validate",
        2,
    ),
    ("tests/integration/Cargo.toml", "mfm-transports-evm-dcv", 1),
    ("tests/integration/src/test_support.rs", "DCV", 2),
    (
        "tests/integration/tests/cargo_metadata_contract.rs",
        "dcv",
        5,
    ),
    (
        "tests/integration/tests/cargo_metadata_contract.rs",
        "evm-dcv",
        5,
    ),
    (
        "tests/integration/tests/cargo_metadata_contract.rs",
        "deploy-configure-validate",
        1,
    ),
    (
        "tests/integration/tests/cargo_metadata_contract.rs",
        "mfm-transports-evm-dcv",
        4,
    ),
    (
        "tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs",
        "dcv",
        33,
    ),
    (
        "tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs",
        "DCV",
        27,
    ),
    (
        "tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs",
        "Dcv",
        6,
    ),
    (
        "tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs",
        "EvmDcv",
        4,
    ),
    (
        "tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs",
        "evm_dcv",
        22,
    ),
    (
        "tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs",
        "/v1/evm/dcv",
        3,
    ),
    (
        "tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs",
        "deploy_configure_validate",
        6,
    ),
];

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
    (
        "crates/evm-deploy-configure-validate-config/src/lib.rs",
        "keystore_path_env",
        8,
    ),
    (
        "crates/evm-deploy-configure-validate-config/src/lib.rs",
        "password_file_env",
        8,
    ),
    (
        "crates/evm-deploy-configure-validate-config/src/lib.rs",
        "password",
        9,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "keystore_path_env",
        1,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "password_file_env",
        1,
    ),
    (
        "crates/ops/evm-deploy-configure-validate-op/src/lib.rs",
        "password",
        1,
    ),
    ("crates/portfolio/model/src/portfolio.rs", "password", 1),
    ("crates/portfolio/model/src/portfolio.rs", "mnemonic", 2),
    ("crates/portfolio/model/src/symbol.rs", "authorization", 2),
    ("crates/states/evm-dcv/src/lib.rs", "keystore_path_env", 4),
    ("crates/states/evm-dcv/src/lib.rs", "password_file_env", 4),
    ("crates/states/evm-dcv/src/lib.rs", "password", 4),
    ("crates/transports/evm-dcv/src/lib.rs", "rpc_url", 6),
    ("crates/transports/evm-dcv/src/lib.rs", "authorization", 7),
    (
        "crates/transports/evm-dcv/src/lib.rs",
        "keystore_path_env",
        3,
    ),
    (
        "crates/transports/evm-dcv/src/lib.rs",
        "password_file_env",
        3,
    ),
    ("crates/transports/evm-dcv/src/lib.rs", "password", 24),
    ("crates/transports/evm-dcv/src/lib.rs", "private_key", 2),
    (
        "crates/transports/evm-dcv/src/lib.rs",
        "raw_transaction",
        16,
    ),
    ("crates/transports/portfolio/src/lib.rs", "rpc_url", 6),
    ("crates/transports/portfolio/src/lib.rs", "authorization", 7),
    ("tests/integration/src/test_support.rs", "rpc_url", 5),
    (
        "tests/integration/src/test_support.rs",
        "keystore_path_env",
        1,
    ),
    (
        "tests/integration/src/test_support.rs",
        "password_file_env",
        7,
    ),
    ("tests/integration/src/test_support.rs", "password", 16),
    ("tests/integration/src/test_support.rs", "mnemonic", 2),
];

#[test]
fn stale_public_namespace_terms_are_temporarily_allowlisted_by_path() {
    let root = repo_root();
    let entries = repo_text_entries(&root);

    assert_forbidden_terms_are_allowlisted(
        "public namespace",
        &entries,
        FORBIDDEN_PUBLIC_NAME_TERMS,
        TEMPORARY_PUBLIC_NAME_ALLOWLIST,
        |path, _source| {
            !HISTORICAL_ARCHITECTURE_DOCS.contains(&path) && !SCAN_SKIP_PATHS.contains(&path)
        },
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
            !HISTORICAL_ARCHITECTURE_DOCS.contains(&path)
                && !SCAN_SKIP_PATHS.contains(&path)
                && !TYPED_SURFACE_FIELD_SCAN_SKIP_PATHS.contains(&path)
                && is_typed_surface_candidate(source)
        },
    );
}

#[test]
fn namespace_guard_rejects_synthetic_stale_names() {
    let entries = vec![TextEntry {
        path: "crates/new-public-surface/src/lib.rs".to_owned(),
        source: "const ROUTE: &str = \"/v1/evm/dcv/deploy\";".to_owned(),
    }];

    let error = forbidden_term_report(
        "synthetic namespace",
        &entries,
        FORBIDDEN_PUBLIC_NAME_TERMS,
        &[],
        |_path, _source| true,
    )
    .expect_err("synthetic stale name must fail");

    assert!(
        error.contains("crates/new-public-surface/src/lib.rs") && error.contains("/v1/evm/dcv"),
        "unexpected error: {error}"
    );
}

#[test]
fn namespace_guard_rejects_extra_stale_name_in_allowlisted_path() {
    let entries = vec![TextEntry {
        path: "bin/rest-api/src/lib.rs".to_owned(),
        source: "/v1/evm/dcv /v1/evm/dcv".to_owned(),
    }];

    let error = forbidden_term_report(
        "synthetic namespace",
        &entries,
        &["/v1/evm/dcv"],
        &[("bin/rest-api/src/lib.rs", "/v1/evm/dcv", 1)],
        |_path, _source| true,
    )
    .expect_err("extra stale name in allowlisted path must fail");

    assert!(
        error.contains("expected=1") && error.contains("actual=2"),
        "unexpected error: {error}"
    );
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
        path: "crates/transports/evm-dcv/src/lib.rs".to_owned(),
        source: "#[derive(MfmValue)] struct Bad { rpc_url: String, another_rpc_url: String }"
            .to_owned(),
    }];

    let error = forbidden_term_report(
        "synthetic typed field",
        &entries,
        &["rpc_url"],
        &[("crates/transports/evm-dcv/src/lib.rs", "rpc_url", 1)],
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

fn assert_forbidden_terms_are_allowlisted(
    label: &str,
    entries: &[TextEntry],
    terms: &[&str],
    allowlist: &[(&str, &str, usize)],
    include: impl Fn(&str, &str) -> bool,
) {
    if let Err(error) = forbidden_term_report(label, entries, terms, allowlist, include) {
        panic!("{error}");
    }
}

fn forbidden_term_report(
    label: &str,
    entries: &[TextEntry],
    terms: &[&str],
    allowlist: &[(&str, &str, usize)],
    include: impl Fn(&str, &str) -> bool,
) -> Result<(), String> {
    let expected = allowlist
        .iter()
        .map(|(path, term, count)| ((*path, *term), *count))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeMap::<(&str, &str), usize>::new();

    for entry in entries {
        if !include(&entry.path, &entry.source) {
            continue;
        }

        for (term, count) in term_counts_in_source(&entry.source, terms) {
            actual.insert((entry.path.as_str(), term), count);
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

fn term_counts_in_source<'a>(source: &str, terms: &'a [&'a str]) -> Vec<(&'a str, usize)> {
    terms
        .iter()
        .copied()
        .filter_map(|term| {
            let count = source.matches(term).count();
            (count > 0).then_some((term, count))
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
    let mut entries = Vec::new();
    collect_text_entries(root, root, &mut entries);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

fn collect_text_entries(root: &Path, dir: &Path, entries: &mut Vec<TextEntry>) {
    let read_dir = fs::read_dir(dir).unwrap_or_else(|error| {
        panic!("read directory {}: {error}", dir.display());
    });

    for entry in read_dir {
        let entry = entry.unwrap_or_else(|error| {
            panic!("read directory entry {}: {error}", dir.display());
        });
        let path = entry.path();
        let file_type = entry.file_type().unwrap_or_else(|error| {
            panic!("read file type {}: {error}", path.display());
        });
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if file_type.is_dir() {
            if should_skip_dir(&name) {
                continue;
            }
            collect_text_entries(root, &path, entries);
            continue;
        }

        if file_type.is_file() && should_scan_file(&path) {
            let rel = repo_relative(root, &path);
            let source = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!("read text file {}: {error}", path.display());
            });
            entries.push(TextEntry { path: rel, source });
        }
    }
}

fn should_skip_dir(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }

    matches!(name, "target" | "result" | "result-bin")
}

fn should_scan_file(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.lock") {
        return true;
    }

    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("rs" | "toml" | "md" | "json" | "nix" | "stderr" | "txt" | "yaml" | "yml")
    )
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("integration crate lives under tests/integration")
        .to_path_buf()
}

fn repo_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
