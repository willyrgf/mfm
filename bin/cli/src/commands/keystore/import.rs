use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{app_services, command_defaults};
use clap::Args;
use mfm_app_legacy::{
    KeystoreBip39ExtraSource, KeystoreImportRequest, KeystoreImportType as AppKeystoreImportType,
};
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

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

    /// Prompt for an optional BIP-39 passphrase during mnemonic import.
    #[arg(long, conflicts_with = "passphrase_file")]
    pub passphrase_prompt: bool,

    /// Read an optional BIP-39 passphrase from a local file or FIFO.
    #[arg(long, value_name = "PATH", conflicts_with = "passphrase_prompt")]
    pub passphrase_file: Option<PathBuf>,

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
    let keystore_path = command_defaults::resolve_keystore_path(args.keystore.as_ref());
    let bip39_extra = resolve_bip39_extra(args)?;
    let services = app_services::make_ephemeral_app_services();
    let response = services
        .keystore_import(KeystoreImportRequest {
            import_type: match args.import_type {
                ImportType::PrivateKey => AppKeystoreImportType::PrivateKey,
                ImportType::Mnemonic => AppKeystoreImportType::Mnemonic,
            },
            label: args.label.clone(),
            derivation_path: args.derivation_path.clone(),
            stdin: args.stdin,
            keystore_path,
            bip39_extra,
        })
        .await
        .map_err(app_services::command_error_from_app_error)?;

    Ok(CommandOutput::new(ImportResponse {
        id: response.id,
        label: response.label,
        key_type: response.key_type,
        address: response.address,
        created_at: response.created_at,
    }))
}

fn resolve_bip39_extra(args: &ImportArgs) -> Result<KeystoreBip39ExtraSource, CommandError> {
    let source = match (args.passphrase_prompt, args.passphrase_file.as_ref()) {
        (true, None) => KeystoreBip39ExtraSource::Prompt,
        (false, Some(path)) => KeystoreBip39ExtraSource::FilePath(path.clone()),
        (false, None) => KeystoreBip39ExtraSource::None,
        (true, Some(_)) => {
            return Err(CommandError::new(
                "InvalidArgument",
                "choose only one BIP-39 extra input source",
            ));
        }
    };

    if !matches!(source, KeystoreBip39ExtraSource::None)
        && !matches!(args.import_type, ImportType::Mnemonic)
    {
        return Err(CommandError::new(
            "InvalidArgument",
            "BIP-39 extra input is only valid for mnemonic imports",
        ));
    }

    if args.stdin && matches!(source, KeystoreBip39ExtraSource::Prompt) {
        return Err(CommandError::new(
            "InvalidArgument",
            "BIP-39 prompt input cannot be combined with stdin material",
        ));
    }

    Ok(source)
}
