use std::collections::{BTreeSet, VecDeque};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const PERSISTED_TRAITS: &[&str] = &[
    "MfmValue",
    "MfmConfig",
    "PublicOutputDescriptor",
    "PublicOutputs",
    "StateInput",
    "OperationOutput",
];

#[derive(Debug, Default)]
struct Config {
    root: PathBuf,
    report_file: Option<PathBuf>,
    self_test: bool,
}

#[derive(Debug, Default)]
struct Report {
    rust_file_count: usize,
    manual_impl_count: usize,
    provenance_forgery_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Ident(String),
    Punct(char),
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    line: usize,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ERROR: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let config = parse_args()?;

    if config.self_test {
        run_self_tests()?;
    }

    let report = check_root(&config.root, true)?;
    write_report(config.report_file.as_deref(), &report)?;

    if report.manual_impl_count != 0 || report.provenance_forgery_count != 0 {
        eprintln!(
            "ERROR: manual persisted impl source-boundary check failed manual_impls={} provenance_forgery={}",
            report.manual_impl_count, report.provenance_forgery_count
        );
        std::process::exit(1);
    }

    println!(
        "OK: manual persisted impl source-boundary check passed files={}",
        report.rust_file_count
    );
    Ok(())
}

fn parse_args() -> Result<Config, String> {
    let mut config = Config {
        root: PathBuf::from("."),
        report_file: None,
        self_test: false,
    };

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--root requires a value".to_owned())?;
                config.root = PathBuf::from(value);
            }
            "--report-file" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--report-file requires a value".to_owned())?;
                config.report_file = Some(PathBuf::from(value));
            }
            "--self-test" => {
                config.self_test = true;
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }

    Ok(config)
}

fn print_usage() {
    println!(
        "usage: check-manual-persisted-impls-parser --root PATH [--report-file PATH] [--self-test]"
    );
}

fn run_self_tests() -> Result<(), String> {
    let tmp_root = env::temp_dir().join(format!("mfm-manual-impl-check-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp_root);
    fs::create_dir_all(tmp_root.join("crates/kernel/values/src"))
        .map_err(|error| format!("create self-test framework dir: {error}"))?;
    fs::create_dir_all(tmp_root.join("crates/domain/src"))
        .map_err(|error| format!("create self-test domain dir: {error}"))?;

    fs::write(
        tmp_root.join("crates/kernel/values/src/lib.rs"),
        r#"
impl<T: MfmValue> MfmValue for MaybeValue<T> {}
fn framework_owned() {
    let _ = SchemaAudit::__derive_generated("mfm-values", "mfm_values::Example", "mfm-program-derive/0.1.0");
}
"#,
    )
    .map_err(|error| format!("write self-test framework source: {error}"))?;

    let framework_report = check_root(&tmp_root, false)?;
    if framework_report.manual_impl_count != 0 || framework_report.provenance_forgery_count != 0 {
        let _ = fs::remove_dir_all(&tmp_root);
        return Err("self-test allowed framework path failed".to_owned());
    }

    fs::write(
        tmp_root.join("crates/domain/src/lib.rs"),
        r#"
use mfm_values::MfmConfig as ConfigTrait;
use mfm_values::SchemaAudit as Audit;

impl
    ConfigTrait
for DomainConfig {}

fn forge() {
    let _ = Audit::__derive_generated("domain", "DomainConfig", "mfm-program-derive/0.1.0");
}
"#,
    )
    .map_err(|error| format!("write self-test domain source: {error}"))?;

    let bad_report = check_root(&tmp_root, false)?;
    if bad_report.manual_impl_count == 0 {
        let _ = fs::remove_dir_all(&tmp_root);
        return Err("self-test did not reject aliased multiline manual impl".to_owned());
    }
    if bad_report.provenance_forgery_count == 0 {
        let _ = fs::remove_dir_all(&tmp_root);
        return Err("self-test did not reject aliased derive provenance call".to_owned());
    }

    let _ = fs::remove_dir_all(&tmp_root);
    println!("OK: source-boundary self-tests passed");
    Ok(())
}

fn check_root(root: &Path, emit_diagnostics: bool) -> Result<Report, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("canonicalize root {}: {error}", root.display()))?;
    let mut files = Vec::new();
    collect_rust_files(&root, &mut files)?;
    files.sort();

    let mut parsed = Vec::new();
    let mut aliases = BTreeSet::new();

    for path in &files {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("read rust source {}: {error}", path.display()))?;
        let tokens = lex(&text);
        collect_persisted_trait_aliases(&tokens, &mut aliases);
        let rel = relative_path(&root, path);
        parsed.push((rel, tokens));
    }

    let mut report = Report {
        rust_file_count: parsed.len(),
        manual_impl_count: 0,
        provenance_forgery_count: 0,
    };

    for (rel, tokens) in parsed {
        let impl_allowed = is_allowed_authored_persisted_impl_path(&rel);
        let provenance_allowed = is_allowed_derive_provenance_path(&rel);

        if !impl_allowed {
            for line in manual_persisted_impl_lines(&tokens, &aliases) {
                report.manual_impl_count += 1;
                if emit_diagnostics {
                    println!(
                        "ERROR: manual persisted trait impl outside framework allowlist file={rel}:{line}"
                    );
                }
            }
        }

        if !provenance_allowed {
            for line in derive_provenance_forgery_lines(&tokens) {
                report.provenance_forgery_count += 1;
                if emit_diagnostics {
                    println!(
                        "ERROR: derive provenance constructor outside framework derive path file={rel}:{line}"
                    );
                }
            }
        }
    }

    Ok(report)
}

fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(path) = queue.pop_front() {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("stat {}: {error}", path.display()))?;
        if metadata.is_dir() {
            if should_prune_dir(root, &path) {
                continue;
            }
            let mut entries = fs::read_dir(&path)
                .map_err(|error| format!("read dir {}: {error}", path.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("read dir entry {}: {error}", path.display()))?;
            entries.sort_by_key(|entry| entry.path());
            for entry in entries {
                queue.push_back(entry.path());
            }
        } else if metadata.is_file() && path.extension() == Some(OsStr::new("rs")) {
            files.push(path);
        }
    }
    Ok(())
}

fn should_prune_dir(root: &Path, path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    if path == root {
        return false;
    }
    matches!(name, ".git" | ".direnv" | "target")
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn is_allowed_authored_persisted_impl_path(path: &str) -> bool {
    path == "crates/kernel/values/src/lib.rs"
        || path == "crates/kernel/values/src/tests.rs"
        || path == "crates/kernel/program-derive/src/lib.rs"
        || path.starts_with("crates/kernel/program-derive/tests/")
}

fn is_allowed_derive_provenance_path(path: &str) -> bool {
    path == "crates/kernel/values/src/lib.rs" || path == "crates/kernel/program-derive/src/lib.rs"
}

fn collect_persisted_trait_aliases(tokens: &[Token], aliases: &mut BTreeSet<String>) {
    let mut index = 0;
    while index < tokens.len() {
        if !is_ident(&tokens[index], "use") {
            index += 1;
            continue;
        }

        let end = statement_end(tokens, index + 1);
        let mut cursor = index + 1;
        while cursor < end {
            if token_ident(tokens, cursor).is_some_and(is_persisted_trait_name) {
                let mut lookahead = cursor + 1;
                while lookahead < end {
                    if matches!(
                        tokens[lookahead].kind,
                        TokenKind::Punct(',') | TokenKind::Punct('}')
                    ) {
                        break;
                    }
                    if is_ident(&tokens[lookahead], "as") {
                        if let Some(alias) = token_ident(tokens, lookahead + 1) {
                            aliases.insert(alias.to_owned());
                        }
                        break;
                    }
                    lookahead += 1;
                }
            }
            cursor += 1;
        }

        index = end.saturating_add(1);
    }
}

fn manual_persisted_impl_lines(tokens: &[Token], aliases: &BTreeSet<String>) -> Vec<usize> {
    let mut lines = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if !is_ident(&tokens[index], "impl") {
            index += 1;
            continue;
        }

        let line = tokens[index].line;
        let mut cursor = index + 1;
        if token_punct(tokens, cursor) == Some('<') {
            cursor = skip_balanced(tokens, cursor, '<', '>');
        }

        let mut depth = 0i32;
        let mut trait_idents = Vec::new();
        let mut found_for = false;

        while cursor < tokens.len() {
            match &tokens[cursor].kind {
                TokenKind::Ident(value) if value == "for" && depth == 0 => {
                    found_for = true;
                    break;
                }
                TokenKind::Ident(value) => {
                    if depth == 0 {
                        trait_idents.push(value.as_str());
                    }
                }
                TokenKind::Punct('<' | '(' | '[' | '{') => depth += 1,
                TokenKind::Punct('>' | ')' | ']' | '}') => {
                    if depth > 0 {
                        depth -= 1;
                    } else {
                        break;
                    }
                }
                TokenKind::Punct(';') => break,
                _ => {}
            }
            cursor += 1;
        }

        if found_for
            && trait_idents
                .iter()
                .any(|ident| is_persisted_trait_name(ident) || aliases.contains(*ident))
        {
            lines.push(line);
        }

        index += 1;
    }
    lines
}

fn derive_provenance_forgery_lines(tokens: &[Token]) -> Vec<usize> {
    tokens
        .iter()
        .filter_map(|token| match &token.kind {
            TokenKind::Ident(value)
                if value == "__derive_generated" || value == "__mfm_values_derive_schema_audit" =>
            {
                Some(token.line)
            }
            _ => None,
        })
        .collect()
}

fn is_persisted_trait_name(value: &str) -> bool {
    PERSISTED_TRAITS.contains(&value)
}

fn statement_end(tokens: &[Token], start: usize) -> usize {
    let mut cursor = start;
    while cursor < tokens.len() {
        if matches!(tokens[cursor].kind, TokenKind::Punct(';')) {
            return cursor;
        }
        cursor += 1;
    }
    tokens.len()
}

fn skip_balanced(tokens: &[Token], start: usize, open: char, close: char) -> usize {
    let mut depth = 0i32;
    let mut cursor = start;
    while cursor < tokens.len() {
        if token_punct(tokens, cursor) == Some(open) {
            depth += 1;
        } else if token_punct(tokens, cursor) == Some(close) {
            depth -= 1;
            if depth == 0 {
                return cursor + 1;
            }
        }
        cursor += 1;
    }
    cursor
}

fn token_ident(tokens: &[Token], index: usize) -> Option<&str> {
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::Ident(value)) => Some(value),
        _ => None,
    }
}

fn token_punct(tokens: &[Token], index: usize) -> Option<char> {
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::Punct(value)) => Some(*value),
        _ => None,
    }
}

fn is_ident(token: &Token, expected: &str) -> bool {
    matches!(&token.kind, TokenKind::Ident(value) if value == expected)
}

fn lex(input: &str) -> Vec<Token> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut line = 1;

    while index < chars.len() {
        let ch = chars[index];

        if ch == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if ch.is_whitespace() {
            index += 1;
            continue;
        }
        if ch == '/' && chars.get(index + 1) == Some(&'/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if ch == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            let mut depth = 1;
            while index < chars.len() && depth > 0 {
                if chars[index] == '\n' {
                    line += 1;
                    index += 1;
                } else if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                    depth += 1;
                    index += 2;
                } else if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if ch == '"' {
            index = skip_quoted(&chars, index, &mut line, '"');
            continue;
        }
        if ch == '\'' {
            index = skip_quoted(&chars, index, &mut line, '\'');
            continue;
        }
        if is_raw_string_start(&chars, index) {
            index = skip_raw_string(&chars, index, &mut line);
            continue;
        }
        if ch == 'b' && chars.get(index + 1) == Some(&'"') {
            index = skip_quoted(&chars, index + 1, &mut line, '"');
            continue;
        }
        if ch == 'b' && chars.get(index + 1) == Some(&'\'') {
            index = skip_quoted(&chars, index + 1, &mut line, '\'');
            continue;
        }
        if ch == 'b' && chars.get(index + 1) == Some(&'r') && is_raw_string_start(&chars, index + 1)
        {
            index = skip_raw_string(&chars, index + 1, &mut line);
            continue;
        }
        if is_ident_start(ch) {
            let start = index;
            index += 1;
            while index < chars.len() && is_ident_continue(chars[index]) {
                index += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Ident(chars[start..index].iter().collect()),
                line,
            });
            continue;
        }

        tokens.push(Token {
            kind: TokenKind::Punct(ch),
            line,
        });
        index += 1;
    }

    tokens
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn skip_quoted(chars: &[char], mut index: usize, line: &mut usize, quote: char) -> usize {
    index += 1;
    while index < chars.len() {
        if chars[index] == '\n' {
            *line += 1;
            index += 1;
        } else if chars[index] == '\\' {
            index = (index + 2).min(chars.len());
        } else if chars[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    index
}

fn is_raw_string_start(chars: &[char], index: usize) -> bool {
    if chars.get(index) != Some(&'r') {
        return false;
    }
    let mut cursor = index + 1;
    while chars.get(cursor) == Some(&'#') {
        cursor += 1;
    }
    chars.get(cursor) == Some(&'"')
}

fn skip_raw_string(chars: &[char], index: usize, line: &mut usize) -> usize {
    let mut hash_count = 0;
    let mut cursor = index + 1;
    while chars.get(cursor) == Some(&'#') {
        hash_count += 1;
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'"') {
        return index + 1;
    }
    cursor += 1;

    while cursor < chars.len() {
        if chars[cursor] == '\n' {
            *line += 1;
            cursor += 1;
            continue;
        }
        if chars[cursor] == '"' {
            let mut hashes = 0;
            while hashes < hash_count && chars.get(cursor + 1 + hashes) == Some(&'#') {
                hashes += 1;
            }
            if hashes == hash_count {
                return cursor + 1 + hash_count;
            }
        }
        cursor += 1;
    }

    cursor
}

fn write_report(path: Option<&Path>, report: &Report) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };

    fs::write(
        path,
        format!(
            "rust_file_count={}\nmanual_impl_count={}\nprovenance_forgery_count={}\n",
            report.rust_file_count, report.manual_impl_count, report.provenance_forgery_count
        ),
    )
    .map_err(|error| format!("write report {}: {error}", path.display()))
}
