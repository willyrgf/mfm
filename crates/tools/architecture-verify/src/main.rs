use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SHARED_RATIO_MIN: f64 = 0.80;
const DEFAULT_SHARED_STATE_ROOTS: &[&str] = &[
    "crates/ops/common/src/states",
    "crates/states/common/src/states",
    "crates/ops/keystore-common/src/states",
    "crates/states/keystore/src/states",
    "crates/ops/aave-v3-common/src",
    "crates/states/aave-v3/src",
    "crates/evm-runtime/src/states",
];

fn main() {
    let repo_root = repo_root();
    let mut failures = Vec::new();

    check_expand_boundary(&repo_root, &mut failures);
    check_utility_duplication(&repo_root, &mut failures);
    check_shared_state_ratio(&repo_root, &mut failures);
    check_evm_transport_invariants(&repo_root, &mut failures);

    if failures.is_empty() {
        println!("mfm-architecture-verify: all checks passed");
        return;
    }

    eprintln!("mfm-architecture-verify: checks failed");
    for failure in failures {
        eprintln!("- {failure}");
    }
    std::process::exit(1);
}

fn repo_root() -> PathBuf {
    if let Ok(raw) = std::env::var("MFM_ARCH_VERIFY_ROOT") {
        return PathBuf::from(raw);
    }

    // crates/tools/architecture-verify -> repo root is 3 ancestors up
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn check_expand_boundary(repo_root: &Path, failures: &mut Vec<String>) {
    let mut files = Vec::new();
    collect_rs_files(&repo_root.join("crates/ops"), &mut files);

    for file in files {
        if !file.ends_with("/src/lib.rs") {
            continue;
        }
        if file.contains("/common/") {
            continue;
        }

        let content = match fs::read_to_string(&file) {
            Ok(c) => c,
            Err(err) => {
                failures.push(format!("failed to read {file}: {err}"));
                continue;
            }
        };

        let mut start = 0usize;
        while let Some(pos) = content[start..].find("fn expand(") {
            let fn_start = start + pos;
            let Some(body_start_rel) = content[fn_start..].find('{') else {
                failures.push(format!("{file}: unable to locate expand() body start"));
                break;
            };
            let body_start = fn_start + body_start_rel;

            let Some(body_end) = find_matching_brace(&content, body_start) else {
                failures.push(format!("{file}: unable to locate expand() body end"));
                break;
            };

            let body = &content[body_start..=body_end];
            for forbidden in [
                ".await",
                "std::fs::",
                "tokio::fs::",
                "std::net::",
                "reqwest::",
                "ureq::",
            ] {
                if body.contains(forbidden) {
                    failures.push(format!(
                        "{file}: expand() contains forbidden token `{forbidden}`"
                    ));
                }
            }

            start = body_end + 1;
        }
    }
}

fn check_utility_duplication(repo_root: &Path, failures: &mut Vec<String>) {
    let mut files = Vec::new();
    collect_rs_files(&repo_root.join("crates"), &mut files);

    let max_allowed: BTreeMap<&str, usize> = BTreeMap::from([
        ("normalize_hex_str", 3),
        ("hex_to_bytes", 3),
        ("bytes_to_hex_prefixed", 3),
        ("parse_abi", 3),
        ("encode_params", 2),
        ("parse_value_wei_to_hex", 2),
        ("trim_leading_zero_bytes", 1),
        ("u128_to_min_be", 1),
        ("rlp_encode_bytes", 1),
        ("rlp_encode_list", 1),
        ("usize_to_min_be", 1),
    ]);

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();

    for file in files {
        let content = match fs::read_to_string(&file) {
            Ok(c) => c,
            Err(err) => {
                failures.push(format!("failed to read {file}: {err}"));
                continue;
            }
        };

        for name in max_allowed.keys() {
            let needle = format!("fn {name}(");
            let n = content.matches(&needle).count();
            if n > 0 {
                *counts.entry(name).or_insert(0) += n;
            }
        }
    }

    for (name, max) in max_allowed {
        let actual = counts.get(name).copied().unwrap_or(0);
        if actual > max {
            failures.push(format!(
                "utility duplication exceeded for `{name}`: found {actual}, max {max}"
            ));
        }
    }
}

fn check_shared_state_ratio(repo_root: &Path, failures: &mut Vec<String>) {
    let shared_roots = shared_state_roots(repo_root);
    let ops_root = repo_root.join("crates/ops");

    let shared_count: usize = shared_roots
        .iter()
        .map(|root| count_state_impls_under(root))
        .sum();
    let shared_ops_dirs = shared_ops_dirs(repo_root, &shared_roots);

    let mut local_count = 0usize;
    if let Ok(entries) = fs::read_dir(&ops_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if shared_ops_dirs.contains(&path) {
                continue;
            }
            let lib = path.join("src/lib.rs");
            if !lib.exists() {
                continue;
            }
            local_count += count_state_impls_in_file(&lib);
        }
    }

    let total = shared_count + local_count;
    if total == 0 {
        failures.push("state ratio check found zero `impl State for` definitions".to_string());
        return;
    }

    let ratio = shared_count as f64 / total as f64;
    if ratio < SHARED_RATIO_MIN {
        failures.push(format!(
            "shared state ratio below threshold: shared={shared_count}, local={local_count}, ratio={ratio:.2}, required={SHARED_RATIO_MIN:.2}"
        ));
    }
}

fn check_evm_transport_invariants(repo_root: &Path, failures: &mut Vec<String>) {
    check_app_evm_transport_wiring(repo_root, failures);
    check_runtime_evm_namespace_call_sites(repo_root, failures);
    check_keystore_tx_sign_stays_local(repo_root, failures);
}

fn check_app_evm_transport_wiring(repo_root: &Path, failures: &mut Vec<String>) {
    let app_lib = repo_root.join("crates/app/src/lib.rs");
    let app_lib_normalized = normalize_path_for_report(&app_lib);
    let content = match fs::read_to_string(&app_lib) {
        Ok(c) => c,
        Err(err) => {
            failures.push(format!("failed to read {app_lib_normalized}: {err}"));
            return;
        }
    };

    let factory_wiring_count = content
        .matches("EvmJsonRpcHttpTransportFactory::from_env()")
        .count();
    if factory_wiring_count != 1 {
        failures.push(format!(
            "{app_lib_normalized}: expected exactly one EVM transport factory wiring to evm-jsonrpc-http, found {factory_wiring_count}"
        ));
    }

    let uses_legacy_route_insert = content.contains("routes.insert(\"evm\".to_string()");
    if uses_legacy_route_insert {
        failures.push(format!(
            "{app_lib_normalized}: legacy direct route insertion for `evm` is no longer allowed; use transport registry wiring"
        ));
    }

    let has_registry_route_assembly = content.contains("HashMapTransportRegistry::new()")
        && content.contains("RouterLiveIoTransportFactory::from_registry(&transports)");
    if !has_registry_route_assembly {
        failures.push(format!(
            "{app_lib_normalized}: expected transport registry based route assembly"
        ));
    }
}

fn check_runtime_evm_namespace_call_sites(repo_root: &Path, failures: &mut Vec<String>) {
    let allowed_call_site =
        normalize_path_for_report(&repo_root.join("crates/evm-runtime/src/rpc.rs"));

    let mut files = Vec::new();
    collect_rs_files(&repo_root.join("crates"), &mut files);
    collect_rs_files(&repo_root.join("bin"), &mut files);

    let mut saw_allowed_call_site = false;

    for file in files {
        if !is_runtime_source_file(&file) {
            continue;
        }

        let content = match fs::read_to_string(&file) {
            Ok(c) => c,
            Err(err) => {
                failures.push(format!("failed to read {file}: {err}"));
                continue;
            }
        };

        let non_test_content = strip_cfg_test_items(&content);
        let has_evm_namespace_call = non_test_content.contains("namespace: \"evm\"");
        if !has_evm_namespace_call {
            continue;
        }

        let normalized = normalize_path_for_report(Path::new(&file));
        if normalized == allowed_call_site {
            saw_allowed_call_site = true;
            continue;
        }

        failures.push(format!(
            "{normalized}: runtime code must not introduce direct `namespace: \"evm\"` call sites outside {allowed_call_site}"
        ));
    }

    if !saw_allowed_call_site {
        failures.push(format!(
            "expected runtime `namespace: \"evm\"` call site in {allowed_call_site}"
        ));
    }
}

fn check_keystore_tx_sign_stays_local(repo_root: &Path, failures: &mut Vec<String>) {
    let keystore_tx_state_candidates = [
        repo_root.join("crates/states/keystore/src/states/tx.rs"),
        repo_root.join("crates/ops/keystore-common/src/states/tx.rs"),
    ];

    let Some(keystore_tx_state) = keystore_tx_state_candidates
        .iter()
        .find(|path| path.exists())
    else {
        let candidates = keystore_tx_state_candidates
            .iter()
            .map(|p| normalize_path_for_report(p))
            .collect::<Vec<_>>()
            .join(", ");
        failures.push(format!(
            "failed to locate keystore tx-sign state source file; checked: {candidates}"
        ));
        return;
    };

    let keystore_tx_state_normalized = normalize_path_for_report(keystore_tx_state);

    let content = match fs::read_to_string(keystore_tx_state) {
        Ok(c) => c,
        Err(err) => {
            failures.push(format!(
                "failed to read {keystore_tx_state_normalized}: {err}"
            ));
            return;
        }
    };

    let non_test_content = strip_cfg_test_items(&content);
    if !non_test_content.contains("\"local.keystore.tx_sign\"") {
        failures.push(format!(
            "{keystore_tx_state_normalized}: expected local tx-sign namespace `local.keystore.tx_sign`"
        ));
    }

    if non_test_content.contains("namespace: \"evm\"") {
        failures.push(format!(
            "{keystore_tx_state_normalized}: keystore tx-sign must remain local and must not call `namespace: \"evm\"`"
        ));
    }
}

fn count_state_impls_under(root: &Path) -> usize {
    if root.is_file() {
        return count_state_impls_in_file(root);
    }

    let mut files = Vec::new();
    collect_rs_files(root, &mut files);
    files
        .into_iter()
        .map(PathBuf::from)
        .map(|p| count_state_impls_in_file(&p))
        .sum()
}

fn shared_state_roots(repo_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(raw) = std::env::var("MFM_ARCH_VERIFY_SHARED_STATE_ROOTS") {
        for value in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            roots.push(repo_root.join(value));
        }
    } else {
        roots.extend(
            DEFAULT_SHARED_STATE_ROOTS
                .iter()
                .map(|path| repo_root.join(path)),
        );
    }

    roots.retain(|path| path.exists());
    roots
}

fn shared_ops_dirs(repo_root: &Path, shared_roots: &[PathBuf]) -> BTreeSet<PathBuf> {
    shared_roots
        .iter()
        .filter_map(|root| root.strip_prefix(repo_root).ok())
        .filter_map(|rel| {
            let mut components = rel.components();
            let first = components.next()?.as_os_str();
            let second = components.next()?.as_os_str();
            if first != "crates" || second != "ops" {
                return None;
            }
            components.next().map(|name| name.as_os_str().to_owned())
        })
        .map(|name| repo_root.join("crates/ops").join(name))
        .collect()
}

fn count_state_impls_in_file(file: &Path) -> usize {
    let content = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let non_test_content = strip_cfg_test_items(&content);
    non_test_content.matches("impl State for ").count()
}

fn collect_rs_files(root: &Path, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().and_then(|v| v.to_str()) == Some("rs") {
            files.push(path.to_string_lossy().to_string());
        }
    }
}

fn normalize_path_for_report(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_runtime_source_file(file: &str) -> bool {
    let normalized = file.replace('\\', "/");
    normalized.contains("/src/") && !normalized.contains("/tests/")
}

fn find_matching_brace(content: &str, open_brace: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    if bytes.get(open_brace)? != &b'{' {
        return None;
    }

    let mut depth = 0usize;
    for (idx, b) in bytes.iter().enumerate().skip(open_brace) {
        match *b {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_cfg_test_items(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0usize;

    while let Some(rel) = content[cursor..].find("#[cfg(test)]") {
        let attr_start = cursor + rel;
        out.push_str(&content[cursor..attr_start]);

        let mut item_start = attr_start + "#[cfg(test)]".len();
        item_start = skip_ws(content, item_start);
        item_start = skip_extra_attrs(content, item_start);

        let next_word = next_word(content, item_start);
        let semicolon_pos = content[item_start..].find(';').map(|p| item_start + p);
        let brace_pos = content[item_start..].find('{').map(|p| item_start + p);

        // `#[cfg(test)] use ...;` is common and may contain braces in import paths.
        let prefers_semicolon = matches!(next_word, "use" | "extern" | "const" | "static" | "type");

        cursor = match (semicolon_pos, brace_pos, prefers_semicolon) {
            (Some(semicolon), _, true) => semicolon + 1,
            (Some(semicolon), Some(brace), false) if semicolon < brace => semicolon + 1,
            (_, Some(brace), false) => find_matching_brace(content, brace)
                .map(|end| end + 1)
                .unwrap_or(content.len()),
            (Some(semicolon), None, false) => semicolon + 1,
            (None, Some(brace), _) => find_matching_brace(content, brace)
                .map(|end| end + 1)
                .unwrap_or(content.len()),
            (None, None, _) => content.len(),
        };
    }

    out.push_str(&content[cursor..]);
    out
}

fn skip_ws(content: &str, mut idx: usize) -> usize {
    while idx < content.len() && content.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }
    idx
}

fn skip_extra_attrs(content: &str, mut idx: usize) -> usize {
    loop {
        let Some(rest) = content.get(idx..) else {
            return idx;
        };
        if !rest.starts_with("#[") {
            return idx;
        }

        let Some(end_rel) = rest.find(']') else {
            return content.len();
        };
        idx += end_rel + 1;
        idx = skip_ws(content, idx);
    }
}

fn next_word(content: &str, idx: usize) -> &str {
    let Some(rest) = content.get(idx..) else {
        return "";
    };
    rest.split_whitespace().next().unwrap_or("")
}
