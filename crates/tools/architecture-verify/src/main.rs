use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SHARED_RATIO_MIN: f64 = 0.80;

fn main() {
    let repo_root = repo_root();
    let mut failures = Vec::new();

    check_expand_boundary(&repo_root, &mut failures);
    check_utility_duplication(&repo_root, &mut failures);
    check_shared_state_ratio(&repo_root, &mut failures);

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
    let shared_root = repo_root.join("crates/ops/common/src/states");
    let ops_root = repo_root.join("crates/ops");

    let shared_count = count_state_impls_under(&shared_root);

    let mut local_count = 0usize;
    if let Ok(entries) = fs::read_dir(&ops_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.file_name().and_then(|s| s.to_str()) == Some("common") {
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

fn count_state_impls_under(root: &Path) -> usize {
    let mut files = Vec::new();
    collect_rs_files(root, &mut files);
    files
        .into_iter()
        .map(PathBuf::from)
        .map(|p| count_state_impls_in_file(&p))
        .sum()
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
