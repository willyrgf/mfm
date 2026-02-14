use clap::Args;
use mfm_app::{FeatureExecutionResult, PortfolioSnapshotRequest, PortfolioSnapshotResponse};

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services};
use crate::support::run_stores::{make_stores, RunStoresArgs};

#[derive(Args)]
pub struct SnapshotArgs {
    /// Wallet public address (0x...)
    pub address: String,

    /// EVM chain id to snapshot (default: 1)
    #[arg(long, default_value_t = 1)]
    pub chain_id: u64,

    /// Optional tokens JSON array to include/override allowlisted tokens (default: [])
    #[arg(long, default_value = "[]")]
    pub tokens_json: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

pub async fn execute(ctx: &CommandContext, args: &SnapshotArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &SnapshotArgs) -> CommandResult<FeatureExecutionResult> {
    let tokens = mfm_app::parse_portfolio_tokens_json(&args.tokens_json)
        .map_err(command_error_from_app_error)?;

    let stores = make_stores(
        args.stores.artifact_root.clone(),
        args.stores.database_url.clone(),
    )
    .await?;

    let services = make_app_services(stores);
    let response: PortfolioSnapshotResponse = services
        .start_portfolio_snapshot(PortfolioSnapshotRequest {
            address: args.address.clone(),
            chain_id: Some(args.chain_id),
            tokens,
        })
        .await
        .map_err(command_error_from_app_error)?;

    let result = FeatureExecutionResult {
        feature_id: "portfolio.snapshot".to_string(),
        result: serde_json::to_value(response)
            .map_err(|_| CommandError::new("SerializationError", "Failed to serialize result"))?,
    };

    Ok(CommandOutput::new(result))
}
