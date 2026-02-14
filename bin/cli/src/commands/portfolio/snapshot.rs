use clap::Args;
use mfm_app::{FeatureCatalog, FeatureExecutionResult, FeatureRequest};

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
    let tokens: serde_json::Value = serde_json::from_str(&args.tokens_json)
        .map_err(|_| CommandError::new("InvalidJson", "Failed to parse --tokens-json as JSON"))?;
    if !tokens.is_array() {
        return Err(CommandError::new(
            "InvalidJson",
            "--tokens-json must be a JSON array",
        ));
    }

    let stores = make_stores(
        args.stores.artifact_root.clone(),
        args.stores.database_url.clone(),
    )
    .await?;

    let services = make_app_services(stores);

    let catalog = FeatureCatalog::with_builtins();
    let result = catalog
        .execute(
            &services,
            FeatureRequest {
                feature_id: "portfolio.snapshot".to_string(),
                payload: serde_json::json!({
                    "address": args.address,
                    "chain_id": args.chain_id,
                    "tokens": tokens,
                }),
            },
        )
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(result))
}
