use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use mfm_app::{ExportRequest, PublicError, PublicJsonResponse, ReplayRequest};

use super::CommandContext;
use crate::presentation::output::{handle_public_result, print_error};
use crate::support::access::read_access_credential;
use crate::support::application::{connect_application, parse_run_id, ApplicationConnectionArgs};
use crate::support::output_file::{self, CreateNewFileError};
use crate::support::portable_export::read_portable_export_input;

/// Subcommands under `mfm run`.
#[derive(Subcommand)]
pub(crate) enum RunCommand {
    /// Admit one exact logical root.
    Admit {
        /// Exact published entry-point id.
        #[arg(value_name = "ENTRY_POINT_ID")]
        entry_point_id: String,
        /// Canonical lower-case UUIDv4 invocation identity.
        #[arg(long, value_name = "UUID")]
        invocation_identity: String,
        /// Configured entry-point target.
        #[arg(long, value_name = "TARGET")]
        target: String,
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
    /// Execute at most one legal run action.
    Drive {
        /// Exact typed run id.
        #[arg(value_name = "RUN_ID")]
        run_id: String,
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
    /// Show the sole ordinary public run view.
    Show {
        /// Exact typed run id.
        #[arg(value_name = "RUN_ID")]
        run_id: String,
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
    /// Perform one callback-free replay operation.
    Replay {
        /// Exact typed run id.
        #[arg(value_name = "RUN_ID")]
        run_id: String,
        /// Replay mode.
        #[arg(long, value_enum)]
        mode: ReplayModeArg,
        /// Caller-held canonical semantic export required by non-verify modes.
        #[arg(long, value_name = "PATH")]
        portable_export: Option<PathBuf>,
        /// Strict canonical JSON ContentRef sidecar required by non-verify modes.
        #[arg(long, value_name = "PATH")]
        portable_export_ref_file: Option<PathBuf>,
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
    /// Read one fixed-head transition-trace page.
    Trace {
        /// Exact typed run id.
        #[arg(value_name = "RUN_ID")]
        run_id: String,
        /// Opaque cursor returned by the preceding page.
        #[arg(long)]
        cursor: Option<String>,
        /// Number of entries from 1 through 500.
        #[arg(long)]
        limit: Option<u16>,
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
    /// Read one fixed-head safe access-audit page.
    Audit {
        /// Exact typed run id.
        #[arg(value_name = "RUN_ID")]
        run_id: String,
        /// Opaque cursor returned by the preceding page.
        #[arg(long)]
        cursor: Option<String>,
        /// Number of entries from 1 through 500.
        #[arg(long)]
        limit: Option<u16>,
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
    /// Atomically write one canonical portable run export.
    Export {
        /// Exact typed run id.
        #[arg(value_name = "RUN_ID")]
        run_id: String,
        /// Portable export scope.
        #[arg(long, value_enum)]
        kind: ExportKindArg,
        /// New output path; existing files are never overwritten.
        #[arg(long, value_name = "PATH")]
        output: PathBuf,
        /// New canonical ContentRef sidecar path; existing files are never overwritten.
        #[arg(long, value_name = "PATH")]
        ref_output: PathBuf,
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
}

/// CLI replay modes.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ReplayModeArg {
    /// Callback-free verification.
    Verify,
    /// Qualified exact reproduction.
    Reproduce,
    /// Non-authoritative current candidate comparison.
    CompareCurrent,
}

/// CLI portable-export kinds.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ExportKindArg {
    /// Semantic history and proof closure.
    Semantic,
    /// Audit history through the current physical head.
    Audit,
}

impl ExportKindArg {
    fn into_app(self) -> mfm_app::ExportKind {
        match self {
            Self::Semantic => mfm_app::ExportKind::Semantic,
            Self::Audit => mfm_app::ExportKind::Audit,
        }
    }
}

impl RunCommand {
    /// Dispatches the selected purpose-authorized run command.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            Self::Admit {
                entry_point_id,
                invocation_identity,
                target,
                connection,
            } => {
                let result =
                    admit(ctx, connection, entry_point_id, invocation_identity, target).await;
                handle_public_result(result, &ctx.output_format, public_text)
            }
            Self::Drive { run_id, connection } => {
                let result = drive(ctx, connection, run_id).await;
                handle_public_result(result, &ctx.output_format, public_text)
            }
            Self::Show { run_id, connection } => {
                let result = show(ctx, connection, run_id).await;
                handle_public_result(result, &ctx.output_format, public_text)
            }
            Self::Replay {
                run_id,
                mode,
                portable_export,
                portable_export_ref_file,
                connection,
            } => {
                let result = replay(
                    ctx,
                    connection,
                    run_id,
                    *mode,
                    portable_export.as_deref(),
                    portable_export_ref_file.as_deref(),
                )
                .await;
                handle_public_result(result, &ctx.output_format, public_text)
            }
            Self::Trace {
                run_id,
                cursor,
                limit,
                connection,
            } => {
                let result = trace(ctx, connection, run_id, cursor.clone(), *limit).await;
                handle_public_result(result, &ctx.output_format, public_text)
            }
            Self::Audit {
                run_id,
                cursor,
                limit,
                connection,
            } => {
                let result = audit(ctx, connection, run_id, cursor.clone(), *limit).await;
                handle_public_result(result, &ctx.output_format, public_text)
            }
            Self::Export {
                run_id,
                kind,
                output,
                ref_output,
                connection,
            } => finish_export(
                export(
                    ctx,
                    connection,
                    run_id,
                    *kind,
                    output.clone(),
                    ref_output.clone(),
                )
                .await,
                &ctx.output_format,
            ),
        }
    }
}

async fn application(
    connection: &ApplicationConnectionArgs,
) -> Result<mfm_app::Application, PublicError> {
    connect_application(connection).await
}

async fn credential(ctx: &CommandContext) -> Result<mfm_app::SecretCredential, PublicError> {
    read_access_credential(ctx.access_token_file.as_deref()).await
}

async fn admit(
    ctx: &CommandContext,
    connection: &ApplicationConnectionArgs,
    entry_point_id: &str,
    invocation_identity: &str,
    target: &str,
) -> Result<mfm_app::AdmitRunResponse, PublicError> {
    let credential = credential(ctx).await?;
    let bytes = serde_json::to_vec(&serde_json::json!({
        "version": mfm_app::ADMIT_RUN_REQUEST_VERSION,
        "entry_point_id": entry_point_id,
        "invocation_identity": invocation_identity,
        "input": { "target": target },
    }))
    .map_err(|_| {
        PublicError::internal(
            "AdmissionRequestConstructionFailed",
            "Admission request could not be constructed",
        )
    })?;
    let request = mfm_app::AdmitRunRequest::decode_json(&bytes)?;
    application(connection)
        .await?
        .admit_run(credential, request)
        .await
}

async fn drive(
    ctx: &CommandContext,
    connection: &ApplicationConnectionArgs,
    run_id: &str,
) -> Result<mfm_app::DriveResponse, PublicError> {
    let credential = credential(ctx).await?;
    let run_id = parse_run_id(run_id)?;
    application(connection)
        .await?
        .drive_once(credential, run_id)
        .await
}

async fn show(
    ctx: &CommandContext,
    connection: &ApplicationConnectionArgs,
    run_id: &str,
) -> Result<mfm_app::PublicRunView, PublicError> {
    let credential = credential(ctx).await?;
    let run_id = parse_run_id(run_id)?;
    application(connection)
        .await?
        .read_public_run(credential, run_id)
        .await
}

async fn replay(
    ctx: &CommandContext,
    connection: &ApplicationConnectionArgs,
    run_id: &str,
    mode: ReplayModeArg,
    portable_export: Option<&std::path::Path>,
    portable_export_ref_file: Option<&std::path::Path>,
) -> Result<mfm_app::ReplayResponse, PublicError> {
    let credential = credential(ctx).await?;
    let run_id = parse_run_id(run_id)?;
    let request = match mode {
        ReplayModeArg::Verify => {
            if portable_export.is_some() || portable_export_ref_file.is_some() {
                return Err(PublicError::replay_artifact_invalid());
            }
            ReplayRequest::Verify
        }
        ReplayModeArg::Reproduce | ReplayModeArg::CompareCurrent => {
            let (Some(portable_export), Some(portable_export_ref_file)) =
                (portable_export, portable_export_ref_file)
            else {
                return Err(PublicError::replay_artifact_invalid());
            };
            let input =
                read_portable_export_input(portable_export, portable_export_ref_file).await?;
            if matches!(mode, ReplayModeArg::Reproduce) {
                ReplayRequest::Reproduce(input)
            } else {
                ReplayRequest::CompareCurrent(input)
            }
        }
    };
    application(connection)
        .await?
        .replay_run(credential, run_id, request)
        .await
}

async fn trace(
    ctx: &CommandContext,
    connection: &ApplicationConnectionArgs,
    run_id: &str,
    cursor: Option<String>,
    limit: Option<u16>,
) -> Result<mfm_app::TransitionTracePage, PublicError> {
    let credential = credential(ctx).await?;
    let run_id = parse_run_id(run_id)?;
    let page = mfm_app::PageRequest::new(cursor, limit).map_err(|_| {
        PublicError::bad_request("PageLimitInvalid", "Page limit must be between 1 and 500")
    })?;
    application(connection)
        .await?
        .read_transition_trace(credential, run_id, page)
        .await
}

async fn audit(
    ctx: &CommandContext,
    connection: &ApplicationConnectionArgs,
    run_id: &str,
    cursor: Option<String>,
    limit: Option<u16>,
) -> Result<mfm_app::AccessAuditPage, PublicError> {
    let credential = credential(ctx).await?;
    let run_id = parse_run_id(run_id)?;
    let page = mfm_app::PageRequest::new(cursor, limit).map_err(|_| {
        PublicError::bad_request("PageLimitInvalid", "Page limit must be between 1 and 500")
    })?;
    application(connection)
        .await?
        .read_access_audit(credential, run_id, page)
        .await
}

async fn export(
    ctx: &CommandContext,
    connection: &ApplicationConnectionArgs,
    run_id: &str,
    kind: ExportKindArg,
    output: PathBuf,
    ref_output: PathBuf,
) -> Result<(), PublicError> {
    let credential = credential(ctx).await?;
    let run_id = parse_run_id(run_id)?;
    tokio::task::spawn_blocking({
        let output = output.clone();
        let ref_output = ref_output.clone();
        move || output_file::preflight_new_atomic_pair(&output, &ref_output)
    })
    .await
    .map_err(|_| export_write_error())?
    .map_err(map_export_write_error)?;
    let export = application(connection)
        .await?
        .export_run(credential, run_id, ExportRequest::new(kind.into_app()))
        .await?;
    let content_ref_bytes = serde_json::to_vec(export.content_ref()).map_err(|_| {
        PublicError::internal(
            "ExportContentRefInvalid",
            "Export ContentRef could not be rendered",
        )
    })?;
    output_file::create_new_atomic_pair_from_reader(
        output,
        export.into_reader(),
        ref_output,
        content_ref_bytes,
    )
    .await
    .map_err(map_export_write_error)
}

fn map_export_write_error(error: CreateNewFileError) -> PublicError {
    match error {
        CreateNewFileError::TargetExists => {
            PublicError::bad_request("ExportPathExists", "Export output path already exists")
        }
        CreateNewFileError::InvalidPath => PublicError::bad_request(
            "ExportPathInvalid",
            "Export output path is not a safe new regular file",
        ),
        CreateNewFileError::WriteFailed => export_write_error(),
    }
}

fn export_write_error() -> PublicError {
    PublicError::internal("ExportWriteFailed", "Export could not be written")
}

fn public_text<T: PublicJsonResponse>(value: &T) -> Result<String, PublicError> {
    value.public_json().map(|value| format!("{value}\n"))
}

fn finish_export(result: Result<(), PublicError>, format: &super::OutputFormat) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            print_error(error, format);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use mfm_app::ErrorClass;

    use super::{map_export_write_error, CreateNewFileError};

    #[test]
    fn destination_copy_failures_use_only_the_fixed_export_write_contract() {
        let error = map_export_write_error(CreateNewFileError::WriteFailed);
        assert_eq!(error.class, ErrorClass::Internal);
        assert_eq!(error.code, "ExportWriteFailed");
        assert_eq!(error.message, "Export could not be written");
    }
}
