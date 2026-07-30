use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn tracked_repository_has_only_current_mfm_v1_identities() {
    let root = repository_root();
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&root)
        .output()
        .expect("run git ls-files");
    assert!(output.status.success(), "git ls-files must succeed");

    let mut violations = Vec::new();
    for raw_path in output.stdout.split(|byte| *byte == 0) {
        if raw_path.is_empty() {
            continue;
        }
        let relative = std::str::from_utf8(raw_path).expect("tracked path is UTF-8");
        if path_has_superseded_version(relative) {
            violations.push(format!("{relative}: versioned tracked path"));
        }

        let path = root.join(relative);
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if let Some(reason) = content_violation(line) {
                violations.push(format!("{relative}:{}: {reason}", index + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "superseded MFM version residue:\n{}",
        violations.join("\n")
    );
}

#[test]
fn removed_rust_version_aliases_do_not_compile() {
    let root = repository_root();
    let temporary = tempfile::tempdir().expect("temporary compile probe");
    let manifest = format!(
        r#"[package]
name = "mfm-current-version-probe"
version = "0.0.0"
edition = "2021"

[workspace]

[dependencies]
mfm-canonical = {{ path = "{}" }}
mfm-journal = {{ path = "{}" }}
mfm-replay = {{ path = "{}" }}
mfm-spec = {{ path = "{}" }}
mfm-store = {{ path = "{}" }}
"#,
        root.join("crates/kernel/canonical").display(),
        root.join("crates/kernel/journal").display(),
        root.join("crates/kernel/replay").display(),
        root.join("crates/kernel/spec").display(),
        root.join("crates/kernel/store").display(),
    );
    fs::write(temporary.path().join("Cargo.toml"), manifest).expect("write probe manifest");
    fs::create_dir(temporary.path().join("src")).expect("create probe source directory");

    let removed = [
        ["mfm_canonical::RecoverabilityContractV", "3"].concat(),
        ["mfm_journal::", "v", "2::ValueRef"].concat(),
        ["mfm_replay::", "v", "2::verify_recorded_history"].concat(),
        ["mfm_spec::", "v", "1::AuthoredProgram"].concat(),
        ["mfm_store::", "v", "2::RunHistoryReader"].concat(),
    ];
    let source = removed
        .iter()
        .map(|path| format!("use {path};"))
        .chain(std::iter::once("fn main() {}".to_owned()))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(temporary.path().join("src/main.rs"), source).expect("write probe source");

    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .arg("check")
        .arg("--quiet")
        .current_dir(temporary.path());
    if let Some(target_dir) = env::var_os("CARGO_TARGET_DIR") {
        command.env("CARGO_TARGET_DIR", target_dir);
    } else {
        command.env("CARGO_TARGET_DIR", root.join("target"));
    }
    let output = command.output().expect("run compile probe");
    assert!(
        !output.status.success(),
        "superseded Rust aliases unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for path in removed {
        let terminal = path.rsplit("::").next().expect("removed terminal");
        assert!(
            stderr.contains(terminal),
            "compiler did not reject `{path}` explicitly:\n{stderr}"
        );
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("integration crate must live under tests/integration")
        .to_path_buf()
}

fn path_has_superseded_version(path: &str) -> bool {
    path.split(|character: char| !character.is_ascii_alphanumeric())
        .any(is_superseded_version_token)
}

fn is_superseded_version_token(token: &str) -> bool {
    token
        .strip_prefix('v')
        .and_then(|digits| digits.parse::<u64>().ok())
        .is_some_and(|version| version >= 2)
}

fn content_violation(line: &str) -> Option<&'static str> {
    if contains_marker_version(line, "::v", 2) && !line.contains("Keccak::v256") {
        return Some("versioned Rust module path");
    }
    if [
        "RecoverabilityContractV",
        "ValidatedCanonicalValueV",
        "CanonicalReferencePathV",
        "ReferenceTerminalKindV",
        "SchemaReferenceEdgeV",
    ]
    .iter()
    .any(|marker| contains_marker_version(line, marker, 2))
    {
        return Some("version-suffixed recoverability type");
    }
    if ["recoverability_v", "recoverability-v", "recoverability/v"]
        .iter()
        .any(|marker| contains_marker_version(line, marker, 2))
    {
        return Some("superseded recoverability name");
    }
    if mfm_identity_has_superseded_version(line) {
        return Some("MFM-owned identity above v1");
    }
    if schema_identity_has_superseded_version(line) {
        return Some("MFM-owned schema identity above v1");
    }
    if mfm_magic_has_superseded_version(line) {
        return Some("MFM persisted magic above 01");
    }
    None
}

fn contains_marker_version(text: &str, marker: &str, minimum: u64) -> bool {
    text.match_indices(marker).any(|(index, _)| {
        let digits = text[index + marker.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        !digits.is_empty()
            && digits
                .parse::<u64>()
                .is_ok_and(|version| version >= minimum)
    })
}

fn mfm_identity_has_superseded_version(line: &str) -> bool {
    line.match_indices("mfm.").any(|(start, _)| {
        let token = &line[start..];
        let end = token
            .find(|character: char| {
                character.is_ascii_whitespace()
                    || matches!(character, '"' | '\'' | '`' | ',' | ')' | ']' | '}')
            })
            .unwrap_or(token.len());
        contains_marker_version(&token[..end], ".v", 2)
    })
}

fn schema_identity_has_superseded_version(line: &str) -> bool {
    line.match_indices("schema:mfm.").any(|(start, _)| {
        let token = &line[start..];
        let Some(hash_marker) = token.find(":sha256-jcs-v1:") else {
            return false;
        };
        let prefix = &token[..hash_marker];
        let Some(version) = prefix.rsplit(':').next() else {
            return false;
        };
        version.parse::<u64>().is_ok_and(|version| version >= 2)
    })
}

fn mfm_magic_has_superseded_version(line: &str) -> bool {
    line.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| token.starts_with("MFM") && token.len() >= 5)
        .any(|token| {
            token
                .get(token.len().saturating_sub(2)..)
                .and_then(|digits| digits.parse::<u64>().ok())
                .is_some_and(|version| version >= 2)
        })
}
