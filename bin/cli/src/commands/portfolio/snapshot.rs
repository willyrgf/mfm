use std::path::PathBuf;

use clap::Args;
use mfm_app::{FeatureExecutionResult, PortfolioSnapshotResponse};

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services_from_args};
use crate::support::run_stores::RunStoresArgs;

/// Arguments for `mfm portfolio snapshot`.
#[derive(Args)]
pub(crate) struct SnapshotArgs {
    /// Canonical portfolio snapshot request JSON payload
    #[arg(long)]
    pub request_json: Option<String>,

    /// Path to a canonical portfolio snapshot request JSON file
    #[arg(long)]
    pub request_file: Option<PathBuf>,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Executes the portfolio snapshot command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &SnapshotArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &SnapshotArgs) -> CommandResult<FeatureExecutionResult> {
    let request = mfm_app::parse_portfolio_snapshot_request_input(
        args.request_json.clone(),
        args.request_file.clone(),
    )
    .map_err(command_error_from_app_error)?;

    let services = make_app_services_from_args(&args.stores).await?;
    let response: PortfolioSnapshotResponse = services
        .start_portfolio_snapshot(request)
        .await
        .map_err(command_error_from_app_error)?;

    let result = FeatureExecutionResult {
        feature_id: "portfolio.snapshot".to_string(),
        result: serde_json::to_value(response)
            .map_err(|_| CommandError::new("SerializationError", "Failed to serialize result"))?,
    };

    Ok(CommandOutput::new(result))
}
