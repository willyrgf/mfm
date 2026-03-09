use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::{
    apply::{apply_plan, yank_version},
    artifacts::write_run_artifacts,
    catalog::{catalog_entry, load_desired_catalog, load_publish_wave, select_packages},
    cli::{Cli, Command, ResumeArgs, SyncUmbrellaArgs, YankArgs},
    error::PublishDocsError,
    ledger::{
        current_git_commit, latest_release, load_release_ledger, package_changed_since_release,
    },
    model::{
        ApplyResult, DesiredCatalog, DocsRsObservation, Mode, OutputFormat, PackageFilter, Plan,
        RegistryObservation, SummaryCounts, SyncUmbrellaResult, WorkspaceState, YankResult,
    },
    plan::{build_plan, summarize_plan},
    remote::{docs_rs::DocsRsClient, index::IndexRegistryObserver},
    umbrella::{
        build_sync_state, load_current_readme, load_sync_state, render_readme, sync_readme,
        sync_state_matches_workspace, write_sync_state,
    },
    workspace::{
        ensure_clean_worktree, find_workspace_root, load_workspace_state, selected_packages,
        workspace_package,
    },
};

const README_PATH: &str = "crates/docs/README.md";
const AUTO_SYNC_ALLOW_DIRTY_MESSAGE: &str =
    "umbrella README was updated during apply; commit crates/docs/README.md or rerun with --allow-dirty";

/// Runs the selected CLI command and prints the final response.
pub async fn run(cli: Cli) -> ExitCode {
    let output_format = resolved_output_format(&cli);
    let mode = resolved_mode(&cli);
    tracing::info!(
        target: "mfm_publish_docs",
        ?mode,
        ?output_format,
        "publish-docs command starting"
    );
    match execute(cli).await {
        Ok(output) => {
            print_success(&output, output_format);
            ExitCode::from(0)
        }
        Err(error) => {
            let command_error = map_error(error);
            tracing::error!(
                target: "mfm_publish_docs",
                code = command_error.code,
                exit_code = command_error.exit_code,
                message = %command_error.message,
                "publish-docs command failed"
            );
            print_error(&command_error, output_format);
            ExitCode::from(command_error.exit_code)
        }
    }
}

#[derive(Debug, Serialize)]
struct SuccessResponse<T> {
    status: ResponseStatus,
    data: T,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    status: ResponseStatus,
    error: ErrorDetails,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum ResponseStatus {
    Success,
    Error,
}

#[derive(Debug, Serialize)]
struct ErrorDetails {
    code: String,
    message: String,
}

#[derive(Debug)]
struct CommandError {
    code: &'static str,
    message: String,
    exit_code: u8,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum CommandOutput {
    Plan(Plan),
    Apply(ApplyResult),
    SyncUmbrella(SyncUmbrellaResult),
    Yank(YankResult),
}

struct PreparedRun {
    workspace_root: PathBuf,
    artifact_dir: PathBuf,
    wave: crate::model::PublishWave,
    workspace: WorkspaceState,
    catalog: DesiredCatalog,
    registry: Vec<RegistryObservation>,
    docs: Vec<DocsRsObservation>,
    plan: Plan,
    summary: SummaryCounts,
    generated_readme: String,
    ledger: crate::model::ReleaseLedger,
}

async fn execute(cli: Cli) -> Result<CommandOutput, PublishDocsError> {
    match &cli.command {
        Some(Command::Resume(args)) => execute_resume(&cli, args).await,
        Some(Command::SyncUmbrella(args)) => execute_sync_umbrella(&cli, args).await,
        Some(Command::Yank(args)) => execute_yank(&cli, args).await,
        Some(Command::Plan(_)) | Some(Command::Apply(_)) | None => {
            execute_plan_or_apply(&cli).await
        }
    }
}

async fn execute_plan_or_apply(cli: &Cli) -> Result<CommandOutput, PublishDocsError> {
    let mode = resolved_mode(cli);
    let filter = PackageFilter {
        from: cli.from.clone(),
        only: cli.only.clone(),
    };
    let prepared = match mode {
        Mode::Apply => prepare_apply_run(cli, mode, filter, None).await?,
        _ => prepare_run(cli, mode, filter, None).await?,
    };

    match mode {
        Mode::Plan => {
            write_run_artifacts(
                &prepared.artifact_dir,
                &prepared.wave,
                &prepared.catalog,
                &prepared.workspace,
                &prepared.registry,
                &prepared.docs,
                &prepared.plan,
                &prepared.summary,
                &prepared.generated_readme,
                None,
            )?;
            Ok(CommandOutput::Plan(prepared.plan))
        }
        Mode::Apply => {
            let mut ledger = prepared.ledger;
            let result = apply_plan(
                &prepared.workspace_root,
                &prepared.workspace,
                &prepared.plan,
                &prepared.generated_readme,
                &mut ledger,
                cli.allow_dirty,
            )?;
            write_run_artifacts(
                &prepared.artifact_dir,
                &prepared.wave,
                &prepared.catalog,
                &prepared.workspace,
                &prepared.registry,
                &prepared.docs,
                &prepared.plan,
                &prepared.summary,
                &prepared.generated_readme,
                Some(&result),
            )?;
            Ok(CommandOutput::Apply(result))
        }
        _ => Err(PublishDocsError::CommandFailed {
            message: format!("unsupported plan/apply mode: {:?}", mode),
        }),
    }
}

async fn execute_resume(cli: &Cli, args: &ResumeArgs) -> Result<CommandOutput, PublishDocsError> {
    let workspace_root = workspace_root_from_cli()?;
    let prior_plan = load_prior_plan(&workspace_root, &args.run_id)?;
    let prepared = prepare_apply_run(
        cli,
        Mode::Resume,
        PackageFilter {
            from: prior_plan.selection_from.clone(),
            only: prior_plan.selection_only.clone(),
        },
        Some(args.run_id.clone()),
    )
    .await?;
    let mut ledger = prepared.ledger;
    let result = apply_plan(
        &workspace_root,
        &prepared.workspace,
        &prepared.plan,
        &prepared.generated_readme,
        &mut ledger,
        cli.allow_dirty,
    )?;
    write_run_artifacts(
        &prepared.artifact_dir,
        &prepared.wave,
        &prepared.catalog,
        &prepared.workspace,
        &prepared.registry,
        &prepared.docs,
        &prepared.plan,
        &prepared.summary,
        &prepared.generated_readme,
        Some(&result),
    )?;
    Ok(CommandOutput::Apply(result))
}

async fn execute_sync_umbrella(
    cli: &Cli,
    args: &SyncUmbrellaArgs,
) -> Result<CommandOutput, PublishDocsError> {
    let workspace_root = workspace_root_from_cli()?;
    let sync = run_sync_umbrella(&workspace_root, cli.allow_dirty, args.check, true).await?;
    Ok(CommandOutput::SyncUmbrella(sync))
}

async fn execute_yank(_cli: &Cli, args: &YankArgs) -> Result<CommandOutput, PublishDocsError> {
    let workspace_root = workspace_root_from_cli()?;
    let wave = load_publish_wave(&workspace_root)?;
    let workspace = load_workspace_state(&workspace_root, &wave.packages)?;
    let catalog = load_desired_catalog(&workspace_root, &workspace)?;
    let Some(entry) = catalog_entry(&catalog, &args.package) else {
        return Err(PublishDocsError::UnknownPackage {
            package: args.package.clone(),
        });
    };
    if !entry.allow_yank {
        return Err(PublishDocsError::CommandFailed {
            message: format!("catalog disallows yanking package={}", args.package),
        });
    }

    let local = workspace_package(&workspace, &args.package).ok_or_else(|| {
        PublishDocsError::PackageMissingFromMetadata {
            package: args.package.clone(),
        }
    })?;
    let version = match &args.version {
        Some(version) => semver::Version::parse(version)?,
        None => local.version.clone(),
    };

    Ok(CommandOutput::Yank(yank_version(
        &workspace_root,
        &args.package,
        &version,
    )?))
}

async fn prepare_run(
    cli: &Cli,
    mode: Mode,
    filter: PackageFilter,
    resumed_from_run_id: Option<String>,
) -> Result<PreparedRun, PublishDocsError> {
    prepare_run_with_options(cli, mode, filter, resumed_from_run_id, true).await
}

async fn prepare_apply_run(
    cli: &Cli,
    mode: Mode,
    filter: PackageFilter,
    resumed_from_run_id: Option<String>,
) -> Result<PreparedRun, PublishDocsError> {
    let mut prepared =
        prepare_run_with_options(cli, mode, filter.clone(), resumed_from_run_id.clone(), true)
            .await?;

    if !plan_requires_umbrella_sync(&prepared.plan) {
        return Ok(prepared);
    }

    tracing::info!(
        target: "mfm_publish_docs",
        run_id = %prepared.plan.run_id,
        "auto-syncing umbrella README before apply"
    );
    let sync = run_sync_umbrella(&prepared.workspace_root, cli.allow_dirty, false, false).await?;
    prepared = prepare_run_with_options(cli, mode, filter, resumed_from_run_id, false).await?;
    ensure_apply_can_continue_after_auto_sync(&prepared.plan, sync.changed, cli.allow_dirty)?;
    Ok(prepared)
}

async fn prepare_run_with_options(
    cli: &Cli,
    mode: Mode,
    filter: PackageFilter,
    resumed_from_run_id: Option<String>,
    enforce_clean_worktree: bool,
) -> Result<PreparedRun, PublishDocsError> {
    let workspace_root = workspace_root_from_cli()?;
    if enforce_clean_worktree && !cli.allow_dirty {
        ensure_clean_worktree(&workspace_root)?;
    }

    let run_id = new_run_id();
    let wave = load_publish_wave(&workspace_root)?;
    let selected_wave_packages = select_packages(&wave, &filter)?;
    let workspace = load_workspace_state(&workspace_root, &selected_wave_packages)?;
    let catalog = load_desired_catalog(&workspace_root, &workspace)?;
    let registry = observe_selected_registry(&workspace_root, &workspace).await?;
    let docs = observe_selected_docs(&workspace, &catalog, &registry).await?;
    let current_readme = load_current_readme(&workspace_root).unwrap_or_default();
    let current_commit = current_git_commit(&workspace_root)?;
    let (generated_readme, umbrella_synced) = if workspace
        .selected
        .iter()
        .any(|name| name == &catalog.umbrella_package)
    {
        match load_sync_state(&workspace_root).ok().flatten() {
            Some(state)
                if sync_state_matches_workspace(&state, current_commit.as_deref(), &catalog)
                    .unwrap_or(false)
                    && current_readme == state.generated_readme =>
            {
                (state.generated_readme, true)
            }
            _ => (current_readme.clone(), false),
        }
    } else {
        (current_readme.clone(), true)
    };
    let ledger = load_release_ledger(&workspace_root)?;
    let changed_since_release = changed_since_release_map(&workspace_root, &workspace, &ledger)?;
    let plan = build_plan(
        run_id.clone(),
        wave.wave.clone(),
        mode,
        filter.from.clone(),
        filter.only.clone(),
        resumed_from_run_id,
        &workspace,
        &catalog,
        &ledger,
        &registry,
        &docs,
        &changed_since_release,
        umbrella_synced,
    )?;
    let summary = summarize_plan(&plan);
    let artifact_dir = workspace_root
        .join(".mfm")
        .join("publish-docs")
        .join("runs")
        .join(&run_id);

    Ok(PreparedRun {
        workspace_root,
        artifact_dir,
        wave,
        workspace,
        catalog,
        registry,
        docs,
        plan,
        summary,
        generated_readme,
        ledger,
    })
}

async fn run_sync_umbrella(
    workspace_root: &Path,
    allow_dirty: bool,
    check_only: bool,
    enforce_clean_worktree: bool,
) -> Result<SyncUmbrellaResult, PublishDocsError> {
    if enforce_clean_worktree && !allow_dirty {
        ensure_clean_worktree(workspace_root)?;
    }

    let wave = load_publish_wave(workspace_root)?;
    let workspace = load_workspace_state(workspace_root, &wave.packages)?;
    let catalog = load_desired_catalog(workspace_root, &workspace)?;
    let registry = observe_catalog_registry(workspace_root, &workspace, &catalog).await?;
    let docs = observe_catalog_docs(&workspace, &catalog, &registry).await?;
    let generated_readme = generated_readme(&catalog, &registry, &docs);
    let changed = load_current_readme(workspace_root).unwrap_or_default() != generated_readme;
    let run_id = new_run_id();

    if changed && !check_only {
        sync_readme(workspace_root, &generated_readme)?;
    }
    if !check_only {
        write_sync_state(
            workspace_root,
            &build_sync_state(
                run_id.clone(),
                current_git_commit(workspace_root)?,
                &catalog,
                &registry,
                &docs,
                generated_readme,
            )?,
        )?;
    }

    Ok(SyncUmbrellaResult {
        schema_version: 1,
        run_id,
        changed,
        path: README_PATH.to_string(),
    })
}

fn plan_requires_umbrella_sync(plan: &Plan) -> bool {
    plan.packages
        .iter()
        .any(|package| matches!(package.action, crate::model::PlanAction::RefreshUmbrella))
}

fn plan_has_publish_actions(plan: &Plan) -> bool {
    plan.packages
        .iter()
        .any(|package| matches!(package.action, crate::model::PlanAction::Publish))
}

fn ensure_apply_can_continue_after_auto_sync(
    plan: &Plan,
    changed: bool,
    allow_dirty: bool,
) -> Result<(), PublishDocsError> {
    if changed && !allow_dirty && plan_has_publish_actions(plan) {
        return Err(PublishDocsError::CommandFailed {
            message: AUTO_SYNC_ALLOW_DIRTY_MESSAGE.to_string(),
        });
    }
    Ok(())
}

fn resolved_mode(cli: &Cli) -> Mode {
    match (&cli.command, cli.dry_run) {
        (Some(Command::Plan(_)), _) => Mode::Plan,
        (Some(Command::Apply(_)), _) => Mode::Apply,
        (Some(Command::Resume(_)), _) => Mode::Resume,
        (Some(Command::SyncUmbrella(_)), _) => Mode::SyncUmbrella,
        (Some(Command::Yank(_)), _) => Mode::Yank,
        (None, true) => Mode::Plan,
        (None, false) => Mode::Apply,
    }
}

fn resolved_output_format(cli: &Cli) -> OutputFormat {
    let json_flag = match &cli.command {
        Some(Command::Plan(args)) | Some(Command::Apply(args)) => args.json,
        Some(Command::SyncUmbrella(args)) => args.json,
        Some(Command::Resume(args)) => args.json,
        Some(Command::Yank(args)) => args.json,
        None => false,
    };
    if json_flag {
        OutputFormat::Json
    } else {
        cli.output_format
    }
}

fn new_run_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("run_{}_{}", now.as_secs(), now.subsec_nanos())
}

fn load_prior_plan(workspace_root: &Path, run_id: &str) -> Result<Plan, PublishDocsError> {
    let path = workspace_root
        .join(".mfm")
        .join("publish-docs")
        .join("runs")
        .join(run_id)
        .join("plan.json");
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn workspace_root_from_cli() -> Result<PathBuf, PublishDocsError> {
    let start_dir = env::current_dir()?;
    find_workspace_root(&start_dir)
}

fn observe_catalog_inputs<'a>(
    workspace: &'a WorkspaceState,
    catalog: &'a DesiredCatalog,
) -> Result<
    Vec<(
        &'a crate::model::LocalPackage,
        &'a crate::model::CatalogPackage,
    )>,
    PublishDocsError,
> {
    catalog
        .packages
        .iter()
        .map(|entry| {
            let local = workspace_package(workspace, &entry.name).ok_or_else(|| {
                PublishDocsError::PackageMissingFromMetadata {
                    package: entry.name.clone(),
                }
            })?;
            Ok((local, entry))
        })
        .collect()
}

async fn observe_selected_registry(
    workspace_root: &Path,
    workspace: &WorkspaceState,
) -> Result<Vec<RegistryObservation>, PublishDocsError> {
    let client = IndexRegistryObserver::new(workspace_root)?;
    let packages = selected_packages(workspace)?;
    Ok(client.observe_packages(&packages).await)
}

async fn observe_catalog_registry(
    workspace_root: &Path,
    workspace: &WorkspaceState,
    catalog: &DesiredCatalog,
) -> Result<Vec<RegistryObservation>, PublishDocsError> {
    let client = IndexRegistryObserver::new(workspace_root)?;
    let inputs = observe_catalog_inputs(workspace, catalog)?;
    let packages: Vec<_> = inputs.iter().map(|(local, _)| *local).collect();
    Ok(client.observe_packages(&packages).await)
}

async fn observe_selected_docs(
    workspace: &WorkspaceState,
    catalog: &DesiredCatalog,
    registry: &[RegistryObservation],
) -> Result<Vec<DocsRsObservation>, PublishDocsError> {
    let client = DocsRsClient::new()?;
    let registry_by_name: BTreeMap<_, _> = registry
        .iter()
        .map(|observation| (observation.package.as_str(), observation))
        .collect();
    let inputs = selected_packages(workspace)?
        .into_iter()
        .filter_map(|local| {
            let entry = catalog_entry(catalog, &local.name)?;
            let registry = registry_by_name.get(local.name.as_str()).copied()?;
            Some((local, entry, registry))
        })
        .collect::<Vec<_>>();
    Ok(client.observe_packages(&inputs).await)
}

async fn observe_catalog_docs(
    workspace: &WorkspaceState,
    catalog: &DesiredCatalog,
    registry: &[RegistryObservation],
) -> Result<Vec<DocsRsObservation>, PublishDocsError> {
    let client = DocsRsClient::new()?;
    let registry_by_name: BTreeMap<_, _> = registry
        .iter()
        .map(|observation| (observation.package.as_str(), observation))
        .collect();
    let inputs = observe_catalog_inputs(workspace, catalog)?
        .into_iter()
        .filter_map(|(local, entry)| {
            registry_by_name
                .get(local.name.as_str())
                .copied()
                .map(|registry| (local, entry, registry))
        })
        .collect::<Vec<_>>();
    Ok(client.observe_packages(&inputs).await)
}

fn changed_since_release_map(
    workspace_root: &Path,
    workspace: &WorkspaceState,
    ledger: &crate::model::ReleaseLedger,
) -> Result<BTreeMap<String, bool>, PublishDocsError> {
    let mut changed = BTreeMap::new();
    for package in selected_packages(workspace)? {
        let package_changed = latest_release(ledger, &package.name)
            .filter(|record| record.version == package.version)
            .map(|record| {
                package_changed_since_release(workspace_root, &package.workspace_path, record)
            })
            .transpose()?
            .unwrap_or(false);
        changed.insert(package.name.clone(), package_changed);
    }
    Ok(changed)
}

fn generated_readme(
    catalog: &DesiredCatalog,
    registry: &[RegistryObservation],
    docs: &[DocsRsObservation],
) -> String {
    let registry_by_name = registry
        .iter()
        .map(|observation| (observation.package.clone(), observation.clone()))
        .collect();
    let docs_by_name = docs
        .iter()
        .map(|observation| (observation.package.clone(), observation.clone()))
        .collect();
    render_readme(catalog, &registry_by_name, &docs_by_name)
}

fn map_error(error: PublishDocsError) -> CommandError {
    match error {
        PublishDocsError::WorkspaceRootNotFound { .. } => CommandError {
            code: "WorkspaceRootNotFound",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::EmptySelection => CommandError {
            code: "EmptySelection",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::UnknownPackage { .. } => CommandError {
            code: "UnknownPackage",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::PackageMissingFromMetadata { .. } => CommandError {
            code: "PackageMissingFromMetadata",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::DirtyWorkingTree => CommandError {
            code: "DirtyWorkingTree",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::CommandFailed { .. } => CommandError {
            code: "CommandFailed",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::Json(_) => CommandError {
            code: "JsonError",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::Http(_) => CommandError {
            code: "HttpError",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::Semver(_) => CommandError {
            code: "InvalidVersion",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::Io(_) => CommandError {
            code: "IoError",
            message: error.to_string(),
            exit_code: 1,
        },
        PublishDocsError::Other(_) => CommandError {
            code: "InternalError",
            message: error.to_string(),
            exit_code: 1,
        },
    }
}

fn print_success(output: &CommandOutput, format: OutputFormat) {
    match format {
        OutputFormat::Text => match output {
            CommandOutput::Plan(plan) => print_plan_text(plan, &summarize_plan(plan)),
            CommandOutput::Apply(result) => print_apply_text(result),
            CommandOutput::SyncUmbrella(result) => print_sync_umbrella_text(result),
            CommandOutput::Yank(result) => print_yank_text(result),
        },
        OutputFormat::Json => {
            let response = SuccessResponse {
                status: ResponseStatus::Success,
                data: output,
            };
            match serde_json::to_string_pretty(&response) {
                Ok(json) => println!("{json}"),
                Err(_) => println!(
                    r#"{{"status":"error","error":{{"code":"SerializationError","message":"Failed to serialize response"}}}}"#
                ),
            }
        }
    }
}

fn print_error(error: &CommandError, format: OutputFormat) {
    match format {
        OutputFormat::Text => eprintln!("Error: {}", error.message),
        OutputFormat::Json => {
            let response = ErrorResponse {
                status: ResponseStatus::Error,
                error: ErrorDetails {
                    code: error.code.to_string(),
                    message: error.message.clone(),
                },
            };
            match serde_json::to_string_pretty(&response) {
                Ok(json) => println!("{json}"),
                Err(_) => println!(
                    r#"{{"status":"error","error":{{"code":"SerializationError","message":"Failed to serialize error response"}}}}"#
                ),
            }
        }
    }
}

fn print_plan_text(plan: &Plan, summary: &SummaryCounts) {
    println!("PLAN wave={} run_id={}", plan.wave, plan.run_id);
    println!(
        "SUMMARY noop={} publish={} wait_dependencies={} wait_registry={} wait_docs_rs={} needs_version_bump={} refresh_umbrella={} manual_review={}",
        summary.noop,
        summary.publish,
        summary.wait_dependencies,
        summary.wait_registry,
        summary.wait_docs_rs,
        summary.needs_version_bump,
        summary.refresh_umbrella,
        summary.manual_review
    );
    for package in &plan.packages {
        let remote = package
            .remote_version
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "-".to_string());
        let docs_status = package
            .docs_status
            .map(|status| format!("{status:?}"))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "ACTION action={:?} package={} local_version={} remote_version={} docs_status={} reason={}",
            package.action,
            package.name,
            package.local_version,
            remote,
            docs_status,
            package.reason
        );
    }
}

fn print_apply_text(result: &ApplyResult) {
    println!("APPLY run_id={}", result.run_id);
    for action in &result.completed_actions {
        println!(
            "COMPLETED action={:?} package={} result={}",
            action.action, action.name, action.result
        );
    }
    for action in &result.blocked_actions {
        println!(
            "BLOCKED action={:?} package={} reason={}",
            action.action, action.name, action.reason
        );
    }
    if let Some(run_id) = &result.next_resume_hint {
        println!("RESUME run_id={run_id}");
    }
}

fn print_sync_umbrella_text(result: &SyncUmbrellaResult) {
    println!(
        "SYNC_UMBRELLA run_id={} changed={} path={}",
        result.run_id, result.changed, result.path
    );
}

fn print_yank_text(result: &YankResult) {
    println!(
        "YANK package={} version={} result={}",
        result.package, result.version, result.result
    );
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{
        ensure_apply_can_continue_after_auto_sync, plan_has_publish_actions,
        plan_requires_umbrella_sync, AUTO_SYNC_ALLOW_DIRTY_MESSAGE,
    };
    use crate::model::{Mode, Plan, PlanAction, PlannedPackage};

    fn plan_with_actions(actions: &[PlanAction]) -> Plan {
        Plan {
            schema_version: 1,
            run_id: "run_1".into(),
            mode: Mode::Apply,
            wave: "docs-rs-wave-1".into(),
            selection_from: None,
            selection_only: None,
            resumed_from_run_id: None,
            packages: actions
                .iter()
                .enumerate()
                .map(|(index, action)| PlannedPackage {
                    name: format!("pkg-{index}"),
                    local_version: Version::parse("0.1.0").expect("version"),
                    remote_version: None,
                    docs_status: None,
                    action: *action,
                    reason: "test".into(),
                    blocking_dependencies: Vec::new(),
                    synthetic: false,
                })
                .collect(),
        }
    }

    #[test]
    fn detects_refresh_umbrella_actions() {
        let plan = plan_with_actions(&[PlanAction::RefreshUmbrella, PlanAction::WaitRegistry]);
        assert!(plan_requires_umbrella_sync(&plan));
    }

    #[test]
    fn detects_publish_actions() {
        let plan = plan_with_actions(&[PlanAction::Noop, PlanAction::Publish]);
        assert!(plan_has_publish_actions(&plan));
    }

    #[test]
    fn auto_sync_can_continue_when_no_publish_will_run() {
        let plan = plan_with_actions(&[PlanAction::Noop]);
        assert!(ensure_apply_can_continue_after_auto_sync(&plan, true, false).is_ok());
    }

    #[test]
    fn auto_sync_requires_allow_dirty_when_publish_remains() {
        let plan = plan_with_actions(&[PlanAction::Publish]);
        let error =
            ensure_apply_can_continue_after_auto_sync(&plan, true, false).expect_err("error");
        assert_eq!(error.to_string(), AUTO_SYNC_ALLOW_DIRTY_MESSAGE);
    }
}
