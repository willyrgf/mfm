use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{app_services, command_defaults, run_stores};
use clap::Args;
use mfm_op_keystore_tx::{
    tx_sign_report_context_key, TxSignOpConfig, TxSignReport, TX_OP_VERSION, TX_SIGN_OP_ID,
};
use mfm_sdk::unstable::{execute_single_op_report, SingleOpReportRequest};
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// Arguments for `mfm keystore tx-sign`.
#[derive(Args)]
pub struct TxSignArgs {
    /// Key ID (UUID) to use for signing
    #[arg(long)]
    pub id: Option<String>,

    /// Key label to use for signing
    #[arg(long)]
    pub by_label: Option<String>,

    /// Destination address (0x-prefixed)
    #[arg(long)]
    pub to: String,

    /// Transfer value in wei (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub value_wei: String,

    /// EVM chain id
    #[arg(long)]
    pub chain_id: u64,

    /// Sender nonce
    #[arg(long)]
    pub nonce: u64,

    /// Max fee per gas (wei, decimal or 0x-prefixed hex)
    #[arg(long)]
    pub max_fee_per_gas: String,

    /// Max priority fee per gas (wei, decimal or 0x-prefixed hex)
    #[arg(long)]
    pub max_priority_fee_per_gas: String,

    /// Gas limit
    #[arg(long)]
    pub gas_limit: u64,

    /// Output file path where the signed raw transaction hex will be written
    #[arg(long)]
    pub out: PathBuf,

    /// Transaction calldata (0x-prefixed hex), defaults to empty calldata
    #[arg(long, default_value = "0x")]
    pub data: String,

    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,
}

/// Response returned after writing a signed transaction payload.
#[derive(Debug, Clone, Serialize)]
pub struct TxSignResponse {
    from: String,
    to: String,
    nonce: u64,
    chain_id: u64,
    tx_type: String,
    payload_hash: String,
    out_path: String,
}

impl fmt::Display for TxSignResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Signed EIP-1559 tx (type {}) from {} to {} and wrote payload to {}",
            self.tx_type, self.from, self.to, self.out_path
        )
    }
}

/// Executes the signing command and terminates the process.
pub async fn execute(ctx: &CommandContext, args: &TxSignArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &TxSignArgs) -> CommandResult<TxSignResponse> {
    let keystore_path = command_defaults::resolve_keystore_path(args.keystore.as_ref());
    let op_config = TxSignOpConfig {
        id: args.id.clone(),
        by_label: None,
        by_label_hex: args
            .by_label
            .as_ref()
            .map(|label| hex::encode(label.as_bytes())),
        to: args.to.clone(),
        value_wei: args.value_wei.clone(),
        chain_id: args.chain_id,
        nonce: args.nonce,
        max_fee_per_gas: args.max_fee_per_gas.clone(),
        max_priority_fee_per_gas: args.max_priority_fee_per_gas.clone(),
        gas_limit: args.gas_limit,
        out_path: args.out.display().to_string(),
        data: args.data.clone(),
        keystore_path: None,
        keystore_path_hex: Some(hex::encode(keystore_path.to_string_lossy().as_bytes())),
    };

    let report_key = tx_sign_report_context_key();
    let bundle = app_services::make_engine_bundle();
    let report: TxSignReport = execute_single_op_report(
        bundle.engine,
        run_stores::make_ephemeral_stores(None),
        bundle.registry,
        bundle.planner,
        SingleOpReportRequest {
            op_id: TX_SIGN_OP_ID.to_string(),
            op_version: TX_OP_VERSION.to_string(),
            op_config: serde_json::to_value(op_config)
                .expect("tx sign op config should serialize to json value"),
            report_context_key: report_key.0,
        },
    )
    .await
    .map_err(app_services::command_error_from_single_op_report_error)?;

    Ok(CommandOutput::new(TxSignResponse {
        from: report.from,
        to: report.to,
        nonce: report.nonce,
        chain_id: report.chain_id,
        tx_type: report.tx_type,
        payload_hash: report.payload_hash,
        out_path: report.out_path,
    }))
}
