use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::{format_keys_table, handle_command_result, KeyDisplay};
use crate::support::{app_services, run_stores};
use clap::Args;
use mfm_op_keystore_admin::{
    keystore_list_report_context_key, KeystoreListOpConfig, KeystoreListReport, KeystoreListSortBy,
    KEYSTORE_ADMIN_OP_VERSION, KEYSTORE_LIST_OP_ID,
};
use mfm_sdk::unstable::{execute_single_op_report, SingleOpReportRequest};
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

#[derive(Args)]
pub struct ListArgs {
    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Include Ethereum addresses in output
    #[arg(long, default_value = "true")]
    pub show_addresses: bool,

    /// Filter by label pattern (regex supported)
    #[arg(long)]
    pub filter_label: Option<String>,

    /// Sort by: label, created, type
    #[arg(long, default_value = "created")]
    pub sort_by: SortBy,
}

#[derive(clap::ValueEnum, Clone)]
pub enum SortBy {
    #[value(name = "label")]
    Label,
    #[value(name = "created")]
    Created,
    #[value(name = "type")]
    Type,
}

#[derive(Serialize)]
pub struct ListResponse {
    keys: Vec<KeyDisplay>,
    show_addresses: bool,
}

impl fmt::Display for ListResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.keys.is_empty() {
            write!(f, "No keys found in keystore")
        } else {
            write!(f, "{}", format_keys_table(&self.keys, self.show_addresses))
        }
    }
}

pub async fn execute(ctx: &CommandContext, args: &ListArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ListArgs) -> CommandResult<ListResponse> {
    let op_config = KeystoreListOpConfig {
        keystore_path: None,
        keystore_path_hex: args
            .keystore
            .as_ref()
            .map(|path| hex::encode(path.to_string_lossy().as_bytes())),
        show_addresses: args.show_addresses,
        filter_label: args.filter_label.clone(),
        sort_by: match args.sort_by {
            SortBy::Label => KeystoreListSortBy::Label,
            SortBy::Created => KeystoreListSortBy::Created,
            SortBy::Type => KeystoreListSortBy::Type,
        },
    };

    let report_key = keystore_list_report_context_key();
    let bundle = app_services::make_engine_bundle();
    let report: KeystoreListReport = execute_single_op_report(
        bundle.engine,
        run_stores::make_ephemeral_stores(None),
        bundle.registry,
        bundle.planner,
        SingleOpReportRequest {
            op_id: KEYSTORE_LIST_OP_ID.to_string(),
            op_version: KEYSTORE_ADMIN_OP_VERSION.to_string(),
            op_config: serde_json::to_value(op_config)
                .expect("keystore list op config should serialize to json value"),
            report_context_key: report_key.0,
        },
    )
    .await
    .map_err(app_services::command_error_from_single_op_report_error)?;

    let keys = report
        .keys
        .into_iter()
        .map(|key| KeyDisplay {
            id: key.id,
            label: key.label,
            key_type: match key.key_type.as_str() {
                "raw" => "privatekey".to_string(),
                "hd" => "mnemonic".to_string(),
                other => other.to_string(),
            },
            address: key.address,
            created: key.created,
        })
        .collect();

    Ok(CommandOutput::new(ListResponse {
        keys,
        show_addresses: report.show_addresses,
    }))
}
