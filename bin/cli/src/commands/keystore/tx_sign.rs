use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{keystore, keystore_selection};
use clap::Args;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// Arguments for `mfm keystore tx-sign`.
#[derive(Args)]
pub(crate) struct TxSignArgs {
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

    /// Replace an existing regular output file
    #[arg(long)]
    pub overwrite: bool,

    /// Transaction calldata (0x-prefixed hex), defaults to empty calldata
    #[arg(long, default_value = "0x")]
    pub data: String,

    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Runtime configuration file for keystore profile selection
    #[arg(long)]
    pub runtime_config: Option<PathBuf>,

    /// Keystore profile ref inside the runtime config (default: default)
    #[arg(long)]
    pub keystore_ref: Option<String>,
}

/// Response returned after writing a signed transaction payload.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TxSignResponse {
    from: String,
    to: String,
    nonce: u64,
    chain_id: u64,
    tx_type: String,
    payload_hash: String,
}

impl fmt::Display for TxSignResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Signed EIP-1559 tx (type {}) from {} to {} and wrote payload locally",
            self.tx_type, self.from, self.to
        )
    }
}

/// Executes the signing command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &TxSignArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &TxSignArgs) -> CommandResult<TxSignResponse> {
    let access =
        keystore_selection::resolve_keystore_access(keystore_selection::KeystoreSelectionArgs {
            keystore: args.keystore.as_ref(),
            runtime_config: args.runtime_config.as_ref(),
            keystore_ref: args.keystore_ref.as_deref(),
        })?;
    let response = keystore::sign_transaction(keystore::TxSignRequest {
        id: args.id.clone(),
        by_label: args.by_label.clone(),
        to: args.to.clone(),
        value_wei: args.value_wei.clone(),
        chain_id: args.chain_id,
        nonce: args.nonce,
        max_fee_per_gas: args.max_fee_per_gas.clone(),
        max_priority_fee_per_gas: args.max_priority_fee_per_gas.clone(),
        gas_limit: args.gas_limit,
        out_path: args.out.clone(),
        out_write_mode: if args.overwrite {
            keystore::OutputWriteMode::Overwrite
        } else {
            keystore::OutputWriteMode::CreateNew
        },
        data: args.data.clone(),
        access,
    })?;

    Ok(CommandOutput::new(TxSignResponse {
        from: response.from,
        to: response.to,
        nonce: response.nonce,
        chain_id: response.chain_id,
        tx_type: response.tx_type,
        payload_hash: response.payload_hash,
    }))
}
