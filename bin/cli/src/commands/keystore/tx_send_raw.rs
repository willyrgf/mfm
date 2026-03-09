use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{app_services, command_defaults, run_stores};
use clap::Args;
use mfm_op_keystore_tx::{
    tx_send_raw_report_context_key, TxSendRawOpConfig, TxSendRawReport, TX_OP_VERSION,
    TX_SEND_RAW_OP_ID,
};
use mfm_sdk::unstable::{execute_single_op_report, SingleOpReportRequest};
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// Arguments for `mfm keystore tx-send-raw`.
#[derive(Args)]
pub(crate) struct TxSendRawArgs {
    /// EVM RPC source ID (falls back to MFM_EVM_RPC_SOURCE_ID)
    #[arg(long, env = "MFM_EVM_RPC_SOURCE_ID")]
    pub source_id: Option<String>,

    /// Input file path with 0x-prefixed signed raw tx hex
    #[arg(long = "in")]
    pub input: PathBuf,
}

/// Response returned after broadcasting a signed raw transaction.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TxSendRawResponse {
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

/// Executes the raw-transaction submission command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &TxSendRawArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &TxSendRawArgs) -> CommandResult<TxSendRawResponse> {
    let source_id = command_defaults::resolve_rpc_source_id(args.source_id.as_deref())?;
    let op_config = TxSendRawOpConfig {
        source_id: Some(source_id),
        input_path: args.input.display().to_string(),
    };

    let report_key = tx_send_raw_report_context_key();
    let bundle = app_services::make_engine_bundle();
    let report: TxSendRawReport = execute_single_op_report(
        bundle.engine,
        run_stores::make_ephemeral_stores(None),
        bundle.registry,
        bundle.planner,
        SingleOpReportRequest {
            op_id: TX_SEND_RAW_OP_ID.to_string(),
            op_version: TX_OP_VERSION.to_string(),
            op_config: app_services::serialize_op_config(&op_config, "tx send raw")?,
            report_context_key: report_key.0,
        },
    )
    .await
    .map_err(app_services::command_error_from_single_op_report_error)?;

    Ok(CommandOutput::new(TxSendRawResponse {
        tx_hash: report.tx_hash,
        rpc_url_host: report.rpc_url_host,
        submitted_at: report.submitted_at,
    }))
}
