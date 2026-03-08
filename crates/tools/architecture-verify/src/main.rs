#![allow(clippy::disallowed_methods)]
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let repo_root = repo_root();
    let mut failures = Vec::new();

    check_expand_boundary(&repo_root, &mut failures);
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

    if let Ok(cwd) = std::env::current_dir() {
        if let Some(root) = find_repo_root(&cwd) {
            return root;
        }
    }

    find_repo_root(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("repo root")
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|candidate| {
        if candidate.join("Cargo.toml").is_file() && candidate.join("crates").is_dir() {
            Some(candidate.to_path_buf())
        } else {
            None
        }
    })
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

        let normalized = normalize_path_for_report(Path::new(&file));
        if normalized.contains("/crates/tools/architecture-verify/") {
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

fn check_evm_transport_invariants(repo_root: &Path, failures: &mut Vec<String>) {
    check_runtime_evm_namespace_call_sites(repo_root, failures);
    check_keystore_tx_sign_stays_local(repo_root, failures);
}

fn check_runtime_evm_namespace_call_sites(repo_root: &Path, failures: &mut Vec<String>) {
    let allowed_call_site =
        normalize_path_for_report(&repo_root.join("crates/collectors/evm/src/lib.rs"));

    let mut files = Vec::new();
    collect_rs_files(&repo_root.join("crates"), &mut files);
    collect_rs_files(&repo_root.join("bin"), &mut files);

    let mut saw_allowed_call_site = false;
    let direct_namespace_patterns = ["namespace: \"evm\"", "namespace: NAMESPACE_EVM.to_string()"];

    for file in files {
        if !is_runtime_source_file(&file) {
            continue;
        }

        let normalized = normalize_path_for_report(Path::new(&file));
        if normalized.contains("/crates/tools/architecture-verify/") {
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
        let has_evm_namespace_call = direct_namespace_patterns
            .iter()
            .any(|pattern| non_test_content.contains(pattern));
        if !has_evm_namespace_call {
            continue;
        }

        if normalized == allowed_call_site {
            saw_allowed_call_site = true;
            continue;
        }

        failures.push(format!(
            "{normalized}: runtime code must not introduce direct EVM namespace call sites outside {allowed_call_site}"
        ));
    }

    if !saw_allowed_call_site {
        failures.push(format!(
            "expected direct EVM namespace bridge call site in {allowed_call_site}"
        ));
    }
}

fn check_keystore_tx_sign_stays_local(repo_root: &Path, failures: &mut Vec<String>) {
    let keystore_tx_state = repo_root.join("crates/states/keystore/src/states/tx.rs");
    let keystore_tx_state_normalized = normalize_path_for_report(&keystore_tx_state);

    let content = match fs::read_to_string(&keystore_tx_state) {
        Ok(c) => c,
        Err(err) => {
            failures.push(format!(
                "failed to read {keystore_tx_state_normalized}: {err}"
            ));
            return;
        }
    };

    let non_test_content = strip_cfg_test_items(&content);
    let Some(sign_impl_start) = non_test_content.find("impl State for KeystoreTxSignState") else {
        failures.push(format!(
            "{keystore_tx_state_normalized}: expected `impl State for KeystoreTxSignState`"
        ));
        return;
    };

    let Some(sign_impl_body_rel) = non_test_content[sign_impl_start..].find('{') else {
        failures.push(format!(
            "{keystore_tx_state_normalized}: expected `KeystoreTxSignState` impl body"
        ));
        return;
    };
    let sign_impl_body_start = sign_impl_start + sign_impl_body_rel;
    let Some(sign_impl_body_end) = find_matching_brace(&non_test_content, sign_impl_body_start)
    else {
        failures.push(format!(
            "{keystore_tx_state_normalized}: expected `KeystoreTxSignState` impl body end"
        ));
        return;
    };
    let sign_impl = &non_test_content[sign_impl_start..=sign_impl_body_end];

    if !sign_impl.contains("LocalKeystoreIoClient") {
        failures.push(format!(
            "{keystore_tx_state_normalized}: expected keystore tx-sign to stay on the typed local-keystore client path"
        ));
    }

    if sign_impl.contains("namespace: \"evm\"")
        || sign_impl.contains("send_raw_transaction_via_io(")
    {
        failures.push(format!(
            "{keystore_tx_state_normalized}: keystore tx-sign must remain local-only and must not route through remote EVM submission helpers"
        ));
    }
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
