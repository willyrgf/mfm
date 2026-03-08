use std::{collections::BTreeMap, path::Path, process::Command};

use semver::Version;

use crate::{
    error::PublishDocsError,
    ledger::{
        append_release_record, current_release_record, save_release_ledger, workspace_is_dirty,
    },
    model::{
        ApplyResult, BlockedAction, CompletedAction, LocalPackage, Plan, PlanAction, ReleaseLedger,
        WorkspaceState, YankResult,
    },
    umbrella::sync_readme,
};

/// Executes the publishable subset of the plan serially.
pub fn apply_plan(
    workspace_root: &Path,
    workspace: &WorkspaceState,
    plan: &Plan,
    generated_readme: &str,
    ledger: &mut ReleaseLedger,
    allow_dirty: bool,
) -> Result<ApplyResult, PublishDocsError> {
    let packages_by_name: BTreeMap<_, _> = workspace
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();
    let mut completed_actions = Vec::new();
    let mut blocked_actions = Vec::new();

    for package in &plan.packages {
        match package.action {
            PlanAction::Noop => {
                completed_actions.push(CompletedAction {
                    name: package.name.clone(),
                    action: package.action,
                    result: "already-published".to_string(),
                });
            }
            PlanAction::Publish => {
                let local = required_local_package(&packages_by_name, &package.name)?;
                publish_and_record(workspace_root, local, ledger, allow_dirty)?;
                completed_actions.push(CompletedAction {
                    name: package.name.clone(),
                    action: package.action,
                    result: "published".to_string(),
                });
            }
            PlanAction::RefreshUmbrella => {
                if !allow_dirty {
                    blocked_actions.push(BlockedAction {
                        name: package.name.clone(),
                        action: package.action,
                        reason: "umbrella-readme-stale; rerun with --allow-dirty or sync and commit first"
                            .to_string(),
                    });
                    continue;
                }

                let changed = sync_readme(workspace_root, generated_readme)?;
                let local = required_local_package(&packages_by_name, &package.name)?;
                publish_and_record(workspace_root, local, ledger, true)?;
                completed_actions.push(CompletedAction {
                    name: package.name.clone(),
                    action: package.action,
                    result: if changed {
                        "readme-synced-and-published".to_string()
                    } else {
                        "published".to_string()
                    },
                });
            }
            PlanAction::WaitDependencies
            | PlanAction::WaitRegistry
            | PlanAction::WaitDocsRs
            | PlanAction::NeedsVersionBump
            | PlanAction::ManualReview
            | PlanAction::Yank => {
                blocked_actions.push(BlockedAction {
                    name: package.name.clone(),
                    action: package.action,
                    reason: package.reason.clone(),
                });
            }
        }
    }

    Ok(ApplyResult {
        schema_version: 1,
        run_id: plan.run_id.clone(),
        completed_actions,
        next_resume_hint: has_resumable_blockers(&blocked_actions).then(|| plan.run_id.clone()),
        blocked_actions,
    })
}

/// Executes `cargo yank` for one package and version.
pub fn yank_version(
    workspace_root: &Path,
    package: &str,
    version: &Version,
) -> Result<YankResult, PublishDocsError> {
    let status = Command::new("cargo")
        .current_dir(workspace_root)
        .args(["yank", package, "--version", &version.to_string()])
        .status()?;
    if !status.success() {
        return Err(PublishDocsError::CommandFailed {
            message: format!(
                "cargo yank failed for package={} version={} status={}",
                package,
                version,
                status.code().unwrap_or(1)
            ),
        });
    }

    Ok(YankResult {
        schema_version: 1,
        package: package.to_string(),
        version: version.clone(),
        result: "yanked".to_string(),
    })
}

fn publish_and_record(
    workspace_root: &Path,
    package: &LocalPackage,
    ledger: &mut ReleaseLedger,
    allow_dirty: bool,
) -> Result<(), PublishDocsError> {
    run_publish(workspace_root, &package.name, allow_dirty)?;
    let dirty = workspace_is_dirty(workspace_root)?;
    let record = current_release_record(workspace_root, package.version.clone(), dirty)?;
    append_release_record(ledger, &package.name, record)?;
    save_release_ledger(workspace_root, ledger)?;
    Ok(())
}

fn required_local_package<'a>(
    packages_by_name: &'a BTreeMap<&str, &'a LocalPackage>,
    package: &str,
) -> Result<&'a LocalPackage, PublishDocsError> {
    packages_by_name.get(package).copied().ok_or_else(|| {
        PublishDocsError::PackageMissingFromMetadata {
            package: package.to_string(),
        }
    })
}

fn run_publish(
    workspace_root: &Path,
    package: &str,
    allow_dirty: bool,
) -> Result<(), PublishDocsError> {
    let mut command = Command::new("cargo");
    command.current_dir(workspace_root);
    command.args(["publish", "--locked", "-p", package]);
    if allow_dirty {
        command.arg("--allow-dirty");
    }

    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(PublishDocsError::CommandFailed {
            message: format!(
                "cargo publish failed for package={} status={}",
                package,
                status.code().unwrap_or(1)
            ),
        })
    }
}

fn has_resumable_blockers(blocked_actions: &[BlockedAction]) -> bool {
    blocked_actions.iter().any(|package| {
        matches!(
            package.action,
            PlanAction::WaitDependencies
                | PlanAction::WaitRegistry
                | PlanAction::WaitDocsRs
                | PlanAction::RefreshUmbrella
        )
    })
}
