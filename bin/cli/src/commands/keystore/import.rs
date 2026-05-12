use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{app_services, command_defaults, run_stores};
use clap::Args;
use mfm_op_keystore_admin::{
    keystore_import_report_context_key, KeystoreImportOpConfig, KeystoreImportReport,
    KeystoreImportType, KEYSTORE_ADMIN_OP_VERSION, KEYSTORE_IMPORT_OP_ID,
};
use mfm_sdk::unstable::{execute_single_op_report, SingleOpReportRequest};
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

const ENV_IMPORT_PASSPHRASE: &str = "MFM_KEYSTORE_IMPORT_BIP39_EXTRA";

/// Arguments for `mfm keystore import`.
#[derive(Args)]
pub(crate) struct ImportArgs {
    /// Import type: privatekey, mnemonic
    #[arg(short = 't', long, value_enum)]
    pub import_type: ImportType,

    /// Human-readable label for the key
    #[arg(short, long)]
    pub label: Option<String>,

    /// Derivation path for mnemonic (default: m/44'/60'/0'/0/0)
    #[arg(short = 'p', long, default_value = "m/44'/60'/0'/0/0")]
    pub derivation_path: String,

    /// Optional mnemonic passphrase (BIP39). Note: providing it via CLI may expose it in shell history.
    #[arg(long)]
    pub passphrase: Option<String>,

    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Interactive input mode (default if --stdin not specified)
    #[arg(short, long)]
    pub interactive: bool,

    /// Read from stdin
    #[arg(long)]
    pub stdin: bool,
}

/// Supported keystore import sources.
#[derive(clap::ValueEnum, Clone)]
pub(crate) enum ImportType {
    /// Import a raw private key.
    #[value(name = "privatekey")]
    PrivateKey,
    /// Import a BIP-39 mnemonic phrase.
    #[value(name = "mnemonic")]
    Mnemonic,
}

/// Response returned after successfully importing a key.
#[derive(Serialize)]
pub(crate) struct ImportResponse {
    id: String,
    label: String,
    key_type: String,
    address: String,
    created_at: String,
}

impl fmt::Display for ImportResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} imported successfully with ID: {}",
            self.key_type, self.id
        )
    }
}

/// Executes the import command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &ImportArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ImportArgs) -> CommandResult<ImportResponse> {
    let _passphrase_guard = args
        .passphrase
        .as_ref()
        .map(|passphrase| ScopedEnvVar::set(ENV_IMPORT_PASSPHRASE, passphrase));
    let keystore_path = command_defaults::resolve_keystore_path(args.keystore.as_ref());

    let op_config = KeystoreImportOpConfig {
        import_type: match args.import_type {
            ImportType::PrivateKey => KeystoreImportType::PrivateKey,
            ImportType::Mnemonic => KeystoreImportType::Mnemonic,
        },
        label: None,
        label_hex: args
            .label
            .as_ref()
            .map(|label| hex::encode(label.as_bytes())),
        derivation_path: args.derivation_path.clone(),
        keystore_path: None,
        keystore_path_hex: Some(hex::encode(keystore_path.to_string_lossy().as_bytes())),
        stdin: args.stdin,
    };

    let report_key = keystore_import_report_context_key();
    let bundle = app_services::make_engine_bundle();
    let report: KeystoreImportReport = execute_single_op_report(
        bundle.engine,
        run_stores::make_ephemeral_stores(None),
        bundle.registry,
        bundle.planner,
        SingleOpReportRequest {
            op_id: KEYSTORE_IMPORT_OP_ID.to_string(),
            op_version: KEYSTORE_ADMIN_OP_VERSION.to_string(),
            op_config: app_services::serialize_op_config(&op_config, "keystore import")?,
            report_context_key: report_key.0,
        },
    )
    .await
    .map_err(app_services::command_error_from_single_op_report_error)?;

    let key_type = match report.key_type.as_str() {
        "raw" => "private key",
        "hd_derived" => "hd_derived",
        other => other,
    };

    Ok(CommandOutput::new(ImportResponse {
        id: report.id,
        label: report.label,
        key_type: key_type.to_string(),
        address: report.address,
        created_at: report.created_at,
    }))
}

struct ScopedEnvVar {
    key: String,
    previous: Option<String>,
}

impl ScopedEnvVar {
    #[allow(clippy::disallowed_methods)]
    fn set(key: &str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self {
            key: key.to_string(),
            previous,
        }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(&self.key, previous);
        } else {
            std::env::remove_var(&self.key);
        }
    }
}
