use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{app_services, run_stores};
use clap::Args;
use mfm_op_keystore_tx::{
    tx_send_raw_report_context_key, TxSendRawOpConfig, TxSendRawReport, TX_OP_VERSION,
    TX_SEND_RAW_OP_ID,
};
use mfm_sdk::unstable::{execute_single_op_report, SingleOpReportRequest};
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
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &TxSendRawArgs) -> CommandResult<TxSendRawResponse> {
    let op_config = TxSendRawOpConfig {
        rpc_url: args.rpc_url.clone(),
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
            op_config: serde_json::to_value(op_config)
                .expect("tx send raw op config should serialize to json value"),
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
