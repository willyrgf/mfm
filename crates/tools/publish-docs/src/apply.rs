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
};

/// Executes the publishable subset of the plan serially.
pub(crate) fn apply_plan(
    workspace_root: &Path,
    workspace: &WorkspaceState,
    plan: &Plan,
    _generated_readme: &str,
    ledger: &mut ReleaseLedger,
    allow_dirty: bool,
) -> Result<ApplyResult, PublishDocsError> {
    tracing::info!(
        target: "mfm_publish_docs",
        run_id = %plan.run_id,
        planned_packages = plan.packages.len(),
        allow_dirty,
        "starting publish-docs apply"
    );
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
                tracing::debug!(
                    target: "mfm_publish_docs",
                    run_id = %plan.run_id,
                    package = %package.name,
                    action = ?package.action,
                    "skipping apply for already-published package"
                );
                completed_actions.push(CompletedAction {
                    name: package.name.clone(),
                    action: package.action,
                    result: "already-published".to_string(),
                });
            }
            PlanAction::Publish => {
                let local = required_local_package(&packages_by_name, &package.name)?;
                tracing::info!(
                    target: "mfm_publish_docs",
                    run_id = %plan.run_id,
                    package = %package.name,
                    local_version = %local.version,
                    action = ?package.action,
                    "starting package publish"
                );
                publish_and_record(workspace_root, local, ledger, allow_dirty)?;
                tracing::info!(
                    target: "mfm_publish_docs",
                    run_id = %plan.run_id,
                    package = %package.name,
                    local_version = %local.version,
                    action = ?package.action,
                    "completed package publish"
                );
                completed_actions.push(CompletedAction {
                    name: package.name.clone(),
                    action: package.action,
                    result: "published".to_string(),
                });
            }
            PlanAction::RefreshUmbrella => {
                tracing::info!(
                    target: "mfm_publish_docs",
                    run_id = %plan.run_id,
                    package = %package.name,
                    action = ?package.action,
                    "apply blocked pending umbrella sync"
                );
                blocked_actions.push(BlockedAction {
                    name: package.name.clone(),
                    action: package.action,
                    reason: "umbrella-sync-required; run `publish-docs sync-umbrella` first"
                        .to_string(),
                });
            }
            PlanAction::WaitDependencies
            | PlanAction::WaitRegistry
            | PlanAction::WaitDocsRs
            | PlanAction::NeedsVersionBump
            | PlanAction::ManualReview
            | PlanAction::Yank => {
                tracing::info!(
                    target: "mfm_publish_docs",
                    run_id = %plan.run_id,
                    package = %package.name,
                    action = ?package.action,
                    reason = %package.reason,
                    "apply blocked for package"
                );
                blocked_actions.push(BlockedAction {
                    name: package.name.clone(),
                    action: package.action,
                    reason: package.reason.clone(),
                });
            }
        }
    }

    let result = ApplyResult {
        schema_version: 1,
        run_id: plan.run_id.clone(),
        completed_actions,
        next_resume_hint: has_resumable_blockers(&blocked_actions).then(|| plan.run_id.clone()),
        blocked_actions,
    };
    tracing::info!(
        target: "mfm_publish_docs",
        run_id = %result.run_id,
        completed_actions = result.completed_actions.len(),
        blocked_actions = result.blocked_actions.len(),
        next_resume_hint = ?result.next_resume_hint,
        "finished publish-docs apply"
    );
    Ok(result)
}

/// Executes `cargo yank` for one package and version.
pub(crate) fn yank_version(
    workspace_root: &Path,
    package: &str,
    version: &Version,
) -> Result<YankResult, PublishDocsError> {
    tracing::info!(
        target: "mfm_publish_docs",
        package,
        version = %version,
        "starting cargo yank"
    );
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

    tracing::info!(
        target: "mfm_publish_docs",
        package,
        version = %version,
        "completed cargo yank"
    );
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
    tracing::debug!(
        target: "mfm_publish_docs",
        package,
        allow_dirty,
        workspace_root = %workspace_root.display(),
        "running cargo publish command"
    );
    let mut command = Command::new("cargo");
    command.current_dir(workspace_root);
    command.args(["publish", "--locked", "-p", package]);
    if allow_dirty {
        command.arg("--allow-dirty");
    }

    let status = command.status()?;
    if status.success() {
        tracing::debug!(
            target: "mfm_publish_docs",
            package,
            "cargo publish command succeeded"
        );
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
