use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{app_services, run_stores};
use clap::Args;
use mfm_op_keystore_admin::{
    keystore_delete_report_context_key, KeystoreDeleteOpConfig, KeystoreDeleteReport,
    KEYSTORE_ADMIN_OP_VERSION, KEYSTORE_DELETE_OP_ID,
};
use mfm_sdk::unstable::{execute_single_op_report, SingleOpReportRequest};
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// Arguments for `mfm keystore delete`.
#[derive(Args)]
pub struct DeleteArgs {
    /// Key ID (UUID) to delete
    pub id: Option<String>,

    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Delete by label instead of ID
    #[arg(long)]
    pub by_label: Option<String>,
}

/// Response returned after successfully deleting a key.
#[derive(Serialize)]
pub struct DeleteResponse {
    id: String,
    label: String,
}

impl fmt::Display for DeleteResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key '{}' deleted successfully", self.label)
    }
}

/// Executes the delete command and terminates the process.
pub async fn execute(ctx: &CommandContext, args: &DeleteArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &DeleteArgs) -> CommandResult<DeleteResponse> {
    let op_config = KeystoreDeleteOpConfig {
        id: args.id.clone(),
        by_label: None,
        by_label_hex: args
            .by_label
            .as_ref()
            .map(|label| hex::encode(label.as_bytes())),
        yes: args.yes,
        keystore_path: None,
        keystore_path_hex: args
            .keystore
            .as_ref()
            .map(|path| hex::encode(path.to_string_lossy().as_bytes())),
    };

    let report_key = keystore_delete_report_context_key();
    let bundle = app_services::make_engine_bundle();
    let report: KeystoreDeleteReport = execute_single_op_report(
        bundle.engine,
        run_stores::make_ephemeral_stores(None),
        bundle.registry,
        bundle.planner,
        SingleOpReportRequest {
            op_id: KEYSTORE_DELETE_OP_ID.to_string(),
            op_version: KEYSTORE_ADMIN_OP_VERSION.to_string(),
            op_config: serde_json::to_value(op_config)
                .expect("keystore delete op config should serialize to json value"),
            report_context_key: report_key.0,
        },
    )
    .await
    .map_err(app_services::command_error_from_single_op_report_error)?;

    Ok(CommandOutput::new(DeleteResponse {
        id: report.id,
        label: report.label,
    }))
}
