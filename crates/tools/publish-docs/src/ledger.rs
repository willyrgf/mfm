use std::{fs, path::Path, process::Command};

use time::OffsetDateTime;

use crate::{
    error::PublishDocsError,
    model::{ReleaseLedger, ReleaseRecord},
};

/// Relative path to the release provenance ledger.
pub(crate) const RELEASE_LEDGER_PATH: &str = "crates/docs/releases.json";

/// Loads the committed release ledger.
pub(crate) fn load_release_ledger(
    workspace_root: &Path,
) -> Result<ReleaseLedger, PublishDocsError> {
    let path = workspace_root.join(RELEASE_LEDGER_PATH);
    if !path.exists() {
        return Ok(empty_ledger());
    }
    let bytes = fs::read(&path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Writes the release ledger back to disk.
pub(crate) fn save_release_ledger(
    workspace_root: &Path,
    ledger: &ReleaseLedger,
) -> Result<(), PublishDocsError> {
    let path = workspace_root.join(RELEASE_LEDGER_PATH);
    let bytes = serde_json::to_vec_pretty(ledger)?;
    fs::write(path, bytes)?;
    Ok(())
}

/// Appends a release record immutably to the ledger.
pub(crate) fn append_release_record(
    ledger: &mut ReleaseLedger,
    package: &str,
    record: ReleaseRecord,
) -> Result<(), PublishDocsError> {
    let entries = ledger.packages.entry(package.to_string()).or_default();
    if entries.iter().any(|entry| entry.version == record.version) {
        return Err(PublishDocsError::CommandFailed {
            message: format!(
                "release ledger already contains package={} version={}",
                package, record.version
            ),
        });
    }
    entries.push(record);
    Ok(())
}

/// Returns the latest known release record for a package.
pub(crate) fn latest_release<'a>(
    ledger: &'a ReleaseLedger,
    package: &str,
) -> Option<&'a ReleaseRecord> {
    ledger
        .packages
        .get(package)
        .and_then(|entries| entries.last())
}

/// Builds a release record for a successful publish.
pub(crate) fn current_release_record(
    workspace_root: &Path,
    version: semver::Version,
    dirty: bool,
) -> Result<ReleaseRecord, PublishDocsError> {
    Ok(ReleaseRecord {
        version,
        git_commit: current_git_commit(workspace_root)?,
        dirty,
        published_at: OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|error| PublishDocsError::Other(error.into()))?,
        registry: "crates-io".to_string(),
    })
}

/// Returns the current git commit hash if available.
pub(crate) fn current_git_commit(
    workspace_root: &Path,
) -> Result<Option<String>, PublishDocsError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

/// Returns whether the working tree is currently dirty.
pub(crate) fn workspace_is_dirty(workspace_root: &Path) -> Result<bool, PublishDocsError> {
    let unstaged = Command::new("git")
        .args(["diff", "--quiet", "--ignore-submodules", "HEAD", "--"])
        .current_dir(workspace_root)
        .status()?;
    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet", "--ignore-submodules", "--"])
        .current_dir(workspace_root)
        .status()?;
    Ok(!(unstaged.success() && staged.success()))
}

/// Returns whether a package path changed since the recorded release commit.
pub(crate) fn package_changed_since_release(
    workspace_root: &Path,
    workspace_path: &Path,
    record: &ReleaseRecord,
) -> Result<bool, PublishDocsError> {
    let Some(commit) = &record.git_commit else {
        return Ok(false);
    };
    let status = Command::new("git")
        .args([
            "diff",
            "--quiet",
            commit,
            "--",
            &workspace_path.to_string_lossy(),
        ])
        .current_dir(workspace_root)
        .status()?;
    Ok(!status.success())
}

fn empty_ledger() -> ReleaseLedger {
    ReleaseLedger {
        schema_version: 1,
        packages: std::collections::BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{append_release_record, empty_ledger, latest_release};
    use crate::model::ReleaseRecord;

    #[test]
    fn appends_unique_versions() {
        let mut ledger = empty_ledger();
        append_release_record(
            &mut ledger,
            "mfm-machine",
            ReleaseRecord {
                version: Version::parse("0.1.0").expect("version"),
                git_commit: Some("abc".into()),
                dirty: false,
                published_at: "2026-03-08T00:00:00Z".into(),
                registry: "crates-io".into(),
            },
        )
        .expect("append");
        assert_eq!(
            latest_release(&ledger, "mfm-machine")
                .expect("latest")
                .version,
            Version::parse("0.1.0").expect("version")
        );
    }
}
