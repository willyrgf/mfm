use std::path::PathBuf;

use clap::Args;

use crate::commands::result::{CommandOutput, CommandResult, PublicError};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::output_file::{self, CreateNewFileError};

/// Arguments for `mfm keystore tx-sign`.
#[derive(Args)]
pub(crate) struct TxSignArgs {
    /// Exact signer ref from the runtime configuration
    #[arg(long)]
    pub signer_ref: String,

    /// Expected sender address (0x-prefixed)
    #[arg(long = "from")]
    pub expected_from: String,

    /// Destination address (0x-prefixed)
    #[arg(long)]
    pub to: String,

    /// Transfer value in wei (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub value_wei: String,

    /// EVM chain id (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub chain_id: String,

    /// Sender nonce (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub nonce: String,

    /// Max fee per gas in wei (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub max_fee_per_gas: String,

    /// Max priority fee per gas in wei (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub max_priority_fee_per_gas: String,

    /// Gas limit (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub gas_limit: String,

    /// Explicit bearer-output file for the signed raw transaction hex
    #[arg(long)]
    pub out: PathBuf,

    /// Replace an existing regular output file
    #[arg(long)]
    pub overwrite: bool,

    /// Transaction calldata (0x-prefixed hex), defaults to empty calldata
    #[arg(long, default_value = "0x")]
    pub data: String,

    /// Runtime configuration file (default: $MFM_RUNTIME_CONFIG_FILE)
    #[arg(long)]
    pub runtime_config: Option<PathBuf>,
}

/// Executes the signing command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &TxSignArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(
    args: &TxSignArgs,
) -> CommandResult<mfm_app::EvmTransactionSigningMetadata> {
    let request = mfm_app::EvmTransactionSigningRequest {
        runtime_config_path: args.runtime_config.clone(),
        signer_ref: args.signer_ref.clone(),
        envelope: mfm_app::EvmTransactionSigningEnvelopeInput {
            expected_from: args.expected_from.clone(),
            to: args.to.clone(),
            value_wei: args.value_wei.clone(),
            chain_id: args.chain_id.clone(),
            nonce: args.nonce.clone(),
            max_fee_per_gas: args.max_fee_per_gas.clone(),
            max_priority_fee_per_gas: args.max_priority_fee_per_gas.clone(),
            gas_limit: args.gas_limit.clone(),
            data: args.data.clone(),
        },
    };
    let signed = mfm_app::sign_evm_transaction_command(request).await?;
    let (metadata, bearer) = signed.into_parts();
    let output_path = args.out.clone();
    let overwrite = args.overwrite;
    tokio::task::spawn_blocking(move || {
        let raw_transaction_hex = format!("0x{}", hex::encode(bearer.bytes()));
        output_file::publish_bearer_atomic(&output_path, raw_transaction_hex.as_bytes(), overwrite)
            .map_err(public_output_error)
    })
    .await
    .map_err(|_| {
        PublicError::internal(
            "file_write_error",
            "Signed transaction output worker failed",
        )
    })??;

    Ok(CommandOutput::new(metadata))
}

fn public_output_error(error: CreateNewFileError) -> PublicError {
    match error {
        CreateNewFileError::TargetExists => PublicError::bad_request(
            "file_write_error",
            "Output file already exists; pass --overwrite to replace it",
        ),
        CreateNewFileError::InvalidPath => PublicError::bad_request(
            "file_write_error",
            "Signed transaction output path is unsafe or invalid",
        ),
        CreateNewFileError::WriteFailed => PublicError::internal(
            "file_write_error",
            "Failed to publish signed transaction output",
        ),
    }
}
