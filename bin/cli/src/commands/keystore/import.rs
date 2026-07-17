use crate::commands::result::{CommandOutput, CommandResult, PublicError};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{keystore, keystore_selection};
use clap::Args;
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

    /// Runtime configuration file for keystore profile selection
    #[arg(long)]
    pub runtime_config: Option<PathBuf>,

    /// Keystore profile ref inside the runtime config (default: default)
    #[arg(long)]
    pub keystore_ref: Option<String>,

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
            import_display_key_type(&self.key_type),
            self.id
        )
    }
}

/// Executes the import command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &ImportArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ImportArgs) -> CommandResult<ImportResponse> {
    let access =
        keystore_selection::resolve_keystore_access(keystore_selection::KeystoreSelectionArgs {
            keystore: args.keystore.as_ref(),
            runtime_config: args.runtime_config.as_ref(),
            keystore_ref: args.keystore_ref.as_deref(),
        })?;
    let bip39_extra = resolve_bip39_extra(args)?;
    let response = keystore::import_key(keystore::ImportKeyRequest {
        kind: match args.import_type {
            ImportType::PrivateKey => keystore::ImportKind::PrivateKey,
            ImportType::Mnemonic => keystore::ImportKind::Mnemonic,
        },
        label: args.label.clone(),
        derivation_path: args.derivation_path.clone(),
        stdin: args.stdin,
        access,
        bip39_extra,
    })?;

    Ok(CommandOutput::new(ImportResponse {
        id: response.id,
        label: response.label,
        key_type: response.key_type,
        address: response.address,
        created_at: response.created_at,
    }))
}

fn import_display_key_type(key_type: &str) -> &str {
    match key_type {
        "privatekey" => "private key",
        other => other,
    }
}

fn resolve_bip39_extra(args: &ImportArgs) -> Result<keystore::Bip39ExtraSource, PublicError> {
    let source = match (args.passphrase_prompt, args.passphrase_file.as_ref()) {
        (true, None) => keystore::Bip39ExtraSource::Prompt,
        (false, Some(path)) => keystore::Bip39ExtraSource::FilePath(path.clone()),
        (false, None) => keystore::Bip39ExtraSource::None,
        (true, Some(_)) => {
            return Err(PublicError::bad_request(
                "InvalidArgument",
                "choose only one BIP-39 extra input source",
            ));
        }
    };

    if !matches!(source, keystore::Bip39ExtraSource::None)
        && !matches!(args.import_type, ImportType::Mnemonic)
    {
        return Err(PublicError::bad_request(
            "InvalidArgument",
            "BIP-39 extra input is only valid for mnemonic imports",
        ));
    }

    if args.stdin && matches!(source, keystore::Bip39ExtraSource::Prompt) {
        return Err(PublicError::bad_request(
            "InvalidArgument",
            "BIP-39 prompt input cannot be combined with stdin material",
        ));
    }

    Ok(source)
}
