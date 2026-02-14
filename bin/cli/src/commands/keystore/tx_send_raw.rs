use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::evm_rpc::send_raw_transaction;
use clap::Args;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

#[derive(Args)]
pub struct TxSendRawArgs {
    /// RPC URL (falls back to MFM_EVM_RPC_URL)
    #[arg(long, env = "MFM_EVM_RPC_URL")]
    pub rpc_url: Option<String>,

    /// Input file path with 0x-prefixed signed raw tx hex
    #[arg(long = "in")]
    pub input: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct TxSendRawResponse {
    tx_hash: String,
    rpc_url_host: String,
    submitted_at: String,
}

impl fmt::Display for TxSendRawResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Submitted raw transaction {} to {} at {}",
            self.tx_hash, self.rpc_url_host, self.submitted_at
        )
    }
}

pub async fn execute(ctx: &CommandContext, args: &TxSendRawArgs) -> ! {
    let result = execute_internal(ctx, args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(
    _ctx: &CommandContext,
    args: &TxSendRawArgs,
) -> CommandResult<TxSendRawResponse> {
    let rpc_url = args.rpc_url.as_deref().ok_or_else(|| {
        CommandError::missing_argument("Must provide --rpc-url or set MFM_EVM_RPC_URL")
    })?;

    let raw_tx_file = std::fs::read_to_string(&args.input).map_err(|e| {
        CommandError::new(
            "InputReadError",
            format!("Failed to read input file '{}': {e}", args.input.display()),
        )
    })?;
    let raw_tx_hex = raw_tx_file.trim();
    if raw_tx_hex.is_empty() {
        return Err(CommandError::new(
            "InvalidRawTransaction",
            "input file did not contain a raw transaction payload",
        ));
    }

    let submission = send_raw_transaction(rpc_url, raw_tx_hex).await?;
    Ok(CommandOutput::new(TxSendRawResponse {
        tx_hash: submission.tx_hash,
        rpc_url_host: submission.rpc_url_host,
        submitted_at: submission.submitted_at,
    }))
}
